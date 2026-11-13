//! User interface. Each submodule draws one region of the window from the
//! shared [`crate::app::App`] state; none of them owns data of its own
//! beyond what is in [`UiState`].

pub mod dialogs;
pub mod edit_object;
pub mod format;
pub mod graph;
pub mod menu;
pub mod panels;
pub mod preferences;
pub mod side_panel;
pub mod timeline_bar;
pub mod toolbar;
pub mod view_options;
pub mod viewport;

use acmi_core::{known, Category, ObjectHandle, ObjectId, Recording, TagSet};
use rustc_hash::FxHashMap;

use crate::session::Session;

/// Transient UI state (nothing here is persisted).
#[derive(Default)]
pub struct UiState {
    /// Message shown in a modal until dismissed.
    pub error: Option<String>,
    pub connection_open: bool,
    pub preferences_open: bool,
    /// The object being edited in the Edit Object window, if it is open.
    pub edit_object: Option<edit_object::EditObject>,
    /// The object the context menu of the 3D view was opened on.
    pub context_object: Option<ObjectHandle>,
    pub primary_filter: String,
    pub secondary_filter: String,
    pub event_filter: String,
    pub tracks: TrackList,
    pub labels: LabelCache,
    /// Last point clicked on the globe: `(latitude, longitude)`.
    pub clicked_geo: Option<(f64, f64)>,
}

/// One entry of the track selectors.
pub struct TrackItem {
    pub handle: ObjectHandle,
    pub label: String,
    /// Lower-case text matched by the filter box.
    pub search: String,
    /// 1 for weapons listed under the object that fired them.
    pub depth: u8,
}

/// Selectable objects, grouped the way the selectors show them: aircraft,
/// ground units and ships, each followed by its weapons, then weapons whose
/// launcher is unknown. Rebuilt only when the recording's objects change.
#[derive(Default)]
pub struct TrackList {
    pub items: Vec<TrackItem>,
    key: Option<(u64, u64)>,
}

impl TrackList {
    pub fn refresh(&mut self, session: &Session) {
        let recording = &session.recording;
        let key = (session.generation(), recording.revision());
        if self.key == Some(key) {
            return;
        }
        self.key = Some(key);
        self.items.clear();

        let selectable_weapon = |tags: TagSet| !tags.intersects(TagSet::SHELL.union(TagSet::BULLET).union(TagSet::SHRAPNEL));
        let mut weapons_by_parent: FxHashMap<ObjectId, Vec<ObjectHandle>> = FxHashMap::default();
        let mut orphans = Vec::new();
        for (handle, object) in recording.iter() {
            if object.category() == Category::Weapon && selectable_weapon(object.tags()) {
                // Only attach to a parent that is itself listed.
                let parent = object.parent().filter(|&id| {
                    recording.find(id).is_some_and(|p| {
                        matches!(recording.object(p).category(), Category::Aircraft | Category::Ground | Category::Ship)
                    })
                });
                match parent {
                    Some(id) => weapons_by_parent.entry(id).or_default().push(handle),
                    None => orphans.push(handle),
                }
            }
        }

        let push = |items: &mut Vec<TrackItem>, handle: ObjectHandle, depth: u8| {
            let label = track_label(recording, handle);
            items.push(TrackItem { handle, search: label.to_lowercase(), label, depth });
        };
        for category in [Category::Aircraft, Category::Ground, Category::Ship] {
            for (handle, object) in recording.iter().filter(|(_, o)| o.category() == category) {
                push(&mut self.items, handle, 0);
                // Ids can be recycled; only the latest holder owns the weapons.
                if recording.find(object.id) == Some(handle) {
                    for &weapon in weapons_by_parent.get(&object.id).into_iter().flatten() {
                        push(&mut self.items, weapon, 1);
                    }
                }
            }
        }
        for handle in orphans {
            push(&mut self.items, handle, 0);
        }
    }

    pub fn label(&self, handle: ObjectHandle) -> Option<&str> {
        self.items.iter().find(|item| item.handle == handle).map(|item| item.label.as_str())
    }
}

/// `Name | CallSign Pilot`, falling back to the hexadecimal id.
pub fn track_label(recording: &Recording, handle: ObjectHandle) -> String {
    let object = recording.object(handle);
    let mut label = match object.name() {
        "" => format!("{:x}", object.id),
        name => name.to_owned(),
    };
    let (call_sign, pilot) = (object.text(known::CALL_SIGN), object.text(known::PILOT));
    if !call_sign.is_empty() || !pilot.is_empty() {
        label.push_str(" |");
        for part in [call_sign, pilot] {
            if !part.is_empty() {
                label.push(' ');
                label.push_str(part);
            }
        }
    }
    label
}

/// Text of the in-world label of each object, by handle.
#[derive(Default)]
pub struct LabelCache {
    texts: Vec<Option<String>>,
    key: Option<(u64, u64)>,
}

impl LabelCache {
    pub fn refresh(&mut self, session: &Session) {
        let key = (session.generation(), session.recording.revision());
        if self.key == Some(key) {
            return;
        }
        self.key = Some(key);
        self.texts.clear();
        self.texts.extend(session.recording.objects().iter().map(|object| {
            // Small, numerous or fixed objects would only clutter the view.
            const UNLABELLED: TagSet = TagSet::SHELL
                .union(TagSet::BULLET)
                .union(TagSet::SHRAPNEL)
                .union(TagSet::FLARE)
                .union(TagSet::CHAFF)
                .union(TagSet::EXPLOSION);
            if object.tags().intersects(UNLABELLED) || object.category() == Category::Static {
                return None;
            }
            let who = match (object.text(known::PILOT), object.text(known::CALL_SIGN)) {
                ("", call_sign) => call_sign,
                (pilot, _) => pilot,
            };
            Some(match (who, object.name()) {
                ("", "") => format!("{:x}", object.id),
                ("", name) => name.to_owned(),
                (who, "") => who.to_owned(),
                (who, name) => format!("{who}\n{name}"),
            })
        }));
    }

    pub fn get(&self, handle: ObjectHandle) -> Option<&str> {
        self.texts.get(handle.index())?.as_deref()
    }
}
