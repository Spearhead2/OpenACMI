//! The open session: a recording, where it came from, the playback clock
//! and the user's selection.

use std::path::PathBuf;

use acmi_core::{ObjectHandle, Parser, Recording};

use crate::timeline::Timeline;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Source {
    #[default]
    None,
    File(PathBuf),
    /// Real-time telemetry from `host:port`.
    Live(String),
}

#[derive(Default)]
pub struct Session {
    pub recording: Recording,
    pub source: Source,
    pub timeline: Timeline,
    /// The object the camera, telemetry panel and exports refer to.
    pub primary: Option<ObjectHandle>,
    /// Second object, for relative measurements.
    pub secondary: Option<ObjectHandle>,
    /// Changes whenever `recording` is replaced, so caches keyed on object
    /// handles know to reset.
    generation: u64,
    /// Parser of the live stream, if any.
    live_parser: Parser,
}

impl Session {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Replaces the session with a freshly loaded recording.
    pub fn open(&mut self, recording: Recording, source: Source) {
        let start = recording.time_range().map_or(0.0, |(start, _)| start);
        self.recording = recording;
        self.source = source;
        self.timeline = Timeline { time: start, ..Default::default() };
        self.primary = None;
        self.secondary = None;
        self.generation += 1;
    }

    pub fn close(&mut self) {
        self.open(Recording::new(), Source::None);
    }

    /// Prepares for a new telemetry stream. With `clear`, existing data is
    /// dropped; otherwise the stream is merged into the current recording.
    pub fn begin_live(&mut self, label: String, clear: bool) {
        if clear {
            self.close();
        }
        self.source = Source::Live(label);
        self.live_parser = Parser::new();
        self.timeline.follow_live = true;
        self.timeline.playing = false;
    }

    /// Feeds lines received from the telemetry stream.
    pub fn feed_live(&mut self, lines: &str) {
        for line in lines.lines() {
            if let Err(err) = self.live_parser.feed_line(&mut self.recording, line) {
                log::error!("telemetry stream: {err}");
            }
        }
    }

    pub fn is_live(&self) -> bool {
        matches!(self.source, Source::Live(_))
    }

    /// Short name of the session for the window title.
    pub fn title(&self) -> String {
        match &self.source {
            Source::None => String::new(),
            Source::File(path) => path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            Source::Live(label) => format!("live: {label}"),
        }
    }

    pub fn swap_selection(&mut self) {
        std::mem::swap(&mut self.primary, &mut self.secondary);
    }
}
