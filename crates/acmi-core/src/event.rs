//! Global events (`0,Event=…`).

use crate::object::ObjectId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventKind {
    Message,
    Bookmark,
    Debug,
    LeftArea,
    Destroyed,
    TakenOff,
    Landed,
    Timeout,
    /// Event type this crate has no special handling for.
    Other(Box<str>),
}

impl EventKind {
    pub fn from_name(name: &str) -> Self {
        match name {
            "Message" => Self::Message,
            "Bookmark" => Self::Bookmark,
            "Debug" => Self::Debug,
            "LeftArea" => Self::LeftArea,
            "Destroyed" => Self::Destroyed,
            "TakenOff" => Self::TakenOff,
            "Landed" => Self::Landed,
            "Timeout" => Self::Timeout,
            other => Self::Other(other.into()),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Message => "Message",
            Self::Bookmark => "Bookmark",
            Self::Debug => "Debug",
            Self::LeftArea => "LeftArea",
            Self::Destroyed => "Destroyed",
            Self::TakenOff => "TakenOff",
            Self::Landed => "Landed",
            Self::Timeout => "Timeout",
            Self::Other(name) => name,
        }
    }

    /// Kinds whose arguments are object ids even without a trailing text field.
    fn refers_to_objects(&self) -> bool {
        matches!(self, Self::LeftArea | Self::Destroyed | Self::TakenOff | Self::Landed | Self::Timeout)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub time: f64,
    pub kind: EventKind,
    /// Objects the event refers to, in file order.
    pub objects: Vec<ObjectId>,
    pub text: String,
}

impl Event {
    /// Parses the value of an `Event=` property:
    /// `Type|ObjectId|ObjectId|…|Text`.
    ///
    /// The specification puts the free text last, but several exporters omit
    /// the trailing separator on object-only events (`Destroyed|3f2`), so a
    /// final field that reads as an id on such an event is taken as one.
    pub fn parse(time: f64, value: &str) -> Self {
        let mut fields = value.split('|');
        let kind = EventKind::from_name(fields.next().unwrap_or("").trim());
        let args: Vec<&str> = fields.collect();

        let mut objects = Vec::new();
        let mut text_parts: Vec<&str> = Vec::new();
        let last = args.len().saturating_sub(1);
        for (i, arg) in args.iter().enumerate() {
            let id = ObjectId::from_str_radix(arg.trim(), 16).ok();
            let is_text_slot = i == last && !(args.len() == 1 && kind.refers_to_objects());
            match id {
                Some(id) if !is_text_slot => objects.push(id),
                _ if !arg.is_empty() => text_parts.push(arg),
                _ => {}
            }
        }
        Self { time, kind, objects, text: text_parts.join("|") }
    }

    /// Serialises back to the `Event=` value format.
    pub fn to_acmi(&self) -> String {
        let mut out = String::from(self.kind.name());
        for id in &self.objects {
            out.push_str(&format!("|{id:x}"));
        }
        out.push('|');
        out.push_str(&self.text);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_spec_forms() {
        let e = Event::parse(1.0, "Message|705|Maverick has violated ATC directives");
        assert_eq!(e.kind, EventKind::Message);
        assert_eq!(e.objects, vec![0x705]);
        assert_eq!(e.text, "Maverick has violated ATC directives");

        let e = Event::parse(1.0, "Bookmark|Starting precautionary landing practice session");
        assert!(e.objects.is_empty());
        assert_eq!(e.text, "Starting precautionary landing practice session");

        let e = Event::parse(1.0, "LeftArea|507|");
        assert_eq!((e.objects.as_slice(), e.text.as_str()), (&[0x507][..], ""));

        let e = Event::parse(1.0, "Landed|2a|Nellis");
        assert_eq!((e.objects.as_slice(), e.text.as_str()), (&[0x2a][..], "Nellis"));
    }

    #[test]
    fn tolerates_missing_trailing_separator() {
        let e = Event::parse(1.0, "Destroyed|3f2");
        assert_eq!(e.objects, vec![0x3f2]);
        // A bookmark's single argument is text even when it looks like hex.
        let e = Event::parse(1.0, "Bookmark|cafe");
        assert!(e.objects.is_empty());
        assert_eq!(e.text, "cafe");
    }

    #[test]
    fn unknown_kinds_round_trip() {
        let e = Event::parse(3.0, "Refuelled|1|2|done");
        assert_eq!(e.kind, EventKind::Other("Refuelled".into()));
        assert_eq!(Event::parse(3.0, &e.to_acmi()), e);
    }
}
