//! Incremental ACMI 2.x text parser.
//!
//! The parser is a small state machine fed one line at a time, which makes it
//! equally usable for whole files ([`Parser::feed_text`]) and for real-time
//! telemetry arriving over a socket ([`Parser::feed_line`]). It holds no
//! object state of its own: everything lands in the [`Recording`].
//!
//! Format summary (see the Tacview "ACMI 2.2 file format" documentation):
//!
//! ```text
//! FileType=text/acmi/tacview          header, must come first
//! FileVersion=2.2
//! 0,ReferenceTime=2011-06-02T05:00:00Z   global property (object id 0)
//! #47.13                               start of a frame, seconds from ReferenceTime
//! 3000102,T=-129.1|43.2|2000|||90,Name=F-16C   object update, id in hex
//! -3000102                             object removed
//! ```
//!
//! Commas inside values are escaped as `\,`; a line ending in `\` continues
//! on the next line.

use std::borrow::Cow;

use crate::event::Event;
use crate::object::{ObjectId, TransformUpdate};
use crate::props::{known, PropKind};
use crate::recording::Recording;
use crate::tags::{Category, TagSet};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ParseError {
    #[error("line {line}: not an ACMI text file (FileType={found:?})")]
    InvalidFileType { line: u64, found: String },
    #[error("line {line}: unsupported ACMI version {found:?} (2.x required)")]
    UnsupportedVersion { line: u64, found: String },
    #[error("no ACMI header found (expected FileType=text/acmi/tacview)")]
    MissingHeader,
}

/// Counters describing what the parser has seen so far.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ParseStats {
    pub lines: u64,
    pub frames: u64,
    /// Lines or properties that could not be understood and were ignored.
    pub skipped: u64,
}

#[derive(Debug, Default)]
pub struct Parser {
    file_type_ok: bool,
    version_ok: bool,
    /// Time of the current frame.
    time: f64,
    /// Accumulates the pieces of a `\`-continued line.
    pending: String,
    stats: ParseStats,
}

impl Parser {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn stats(&self) -> ParseStats {
        self.stats
    }

    /// Time of the frame currently being parsed (the "live" time of a stream).
    #[inline]
    pub fn current_time(&self) -> f64 {
        self.time
    }

    #[inline]
    pub fn header_complete(&self) -> bool {
        self.file_type_ok && self.version_ok
    }

    /// Parses a complete document.
    ///
    /// `progress` is called periodically with the number of bytes consumed.
    pub fn feed_text(
        &mut self,
        recording: &mut Recording,
        text: &str,
        mut progress: impl FnMut(usize),
    ) -> Result<(), ParseError> {
        const PROGRESS_INTERVAL: u64 = 16_384;
        let base = text.as_ptr() as usize;
        for line in text.split('\n') {
            self.feed_line(recording, line)?;
            if self.stats.lines.is_multiple_of(PROGRESS_INTERVAL) {
                progress(line.as_ptr() as usize - base);
            }
        }
        progress(text.len());
        self.finish()
    }

    /// Call after the last line of a document to validate that it was ACMI at all.
    pub fn finish(&self) -> Result<(), ParseError> {
        if self.header_complete() {
            Ok(())
        } else {
            Err(ParseError::MissingHeader)
        }
    }

    /// Parses one line (without or with its trailing line break).
    ///
    /// Malformed lines are counted in [`ParseStats::skipped`] and ignored; an
    /// error is returned only when the input is not an ACMI 2.x document.
    pub fn feed_line(&mut self, recording: &mut Recording, raw: &str) -> Result<(), ParseError> {
        self.stats.lines += 1;
        let mut line = raw.trim_end_matches(['\r', '\n']);
        if self.stats.lines == 1 {
            line = line.trim_start_matches('\u{feff}');
        }

        if ends_with_continuation(line) {
            self.pending.push_str(&line[..line.len() - 1]);
            self.pending.push('\n');
            return Ok(());
        }
        if self.pending.is_empty() {
            self.process(recording, line)
        } else {
            self.pending.push_str(line);
            let full = std::mem::take(&mut self.pending);
            let result = self.process(recording, &full);
            // Keep the allocation for the next multi-line value.
            self.pending = full;
            self.pending.clear();
            result
        }
    }

    fn process(&mut self, recording: &mut Recording, line: &str) -> Result<(), ParseError> {
        let line = line.trim_start();
        let Some(&first) = line.as_bytes().first() else { return Ok(()) };
        if line.starts_with("//") {
            return Ok(());
        }

        if first == b'F' {
            if let Some(value) = line.strip_prefix("FileType=") {
                if value.trim() != "text/acmi/tacview" {
                    return Err(ParseError::InvalidFileType { line: self.stats.lines, found: value.trim().to_owned() });
                }
                self.file_type_ok = true;
                return Ok(());
            }
            if let Some(value) = line.strip_prefix("FileVersion=") {
                let supported = value.trim().parse::<f32>().is_ok_and(|v| (2.0..3.0).contains(&v));
                if !supported {
                    return Err(ParseError::UnsupportedVersion { line: self.stats.lines, found: value.trim().to_owned() });
                }
                self.version_ok = true;
                return Ok(());
            }
        }
        if !self.header_complete() {
            self.stats.skipped += 1;
            return Ok(());
        }

        match first {
            b'#' => match line[1..].trim().parse::<f64>() {
                Ok(t) if t.is_finite() => {
                    self.time = t;
                    self.stats.frames += 1;
                    // An empty frame still marks the passage of time.
                    if recording.time_range.is_some() {
                        recording.extend_time_range(t);
                    }
                }
                _ => self.stats.skipped += 1,
            },
            b'-' => match ObjectId::from_str_radix(line[1..].trim(), 16) {
                Ok(id) => self.remove_object(recording, id),
                Err(_) => self.stats.skipped += 1,
            },
            _ => {
                let parsed =
                    line.split_once(',').and_then(|(id, rest)| Some((ObjectId::from_str_radix(id.trim(), 16).ok()?, rest)));
                match parsed {
                    Some((0, rest)) => self.global_update(recording, rest),
                    Some((id, rest)) => self.object_update(recording, id, rest),
                    None => self.stats.skipped += 1,
                }
            }
        }
        Ok(())
    }

    fn remove_object(&mut self, recording: &mut Recording, id: ObjectId) {
        if let Some(handle) = recording.find(id) {
            recording.objects[handle.index()].set_removed(self.time);
            recording.extend_time_range(self.time);
            recording.revision += 1;
        }
    }

    fn global_update(&mut self, recording: &mut Recording, rest: &str) {
        for (key, raw) in Properties::new(rest) {
            let value = unescape(raw);
            if key == "Event" {
                recording.push_event(Event::parse(self.time, &value));
            } else {
                recording.meta.set(key, &value);
            }
        }
        recording.revision += 1;
    }

    fn object_update(&mut self, recording: &mut Recording, id: ObjectId, rest: &str) {
        let time = self.time;
        let handle = recording.obtain(id, time);
        let (ref_lon, ref_lat) = (recording.meta.reference_longitude, recording.meta.reference_latitude);

        let mut first_fix = false;
        let mut sampled = false;
        let mut identity_changed = false;
        {
            // Disjoint field borrows: the registry may grow while the object is edited.
            let Recording { objects, props, .. } = recording;
            let object = &mut objects[handle.index()];

            for (key, raw) in Properties::new(rest) {
                if key == "T" {
                    let had_fix = !object.track.is_empty();
                    if object.track.push(time, &parse_transform(raw, ref_lon, ref_lat)) {
                        sampled = true;
                        first_fix |= !had_fix;
                    }
                    continue;
                }
                let value = unescape(raw);
                let prop = match props.get(key) {
                    Some(prop) => prop,
                    None => props.intern(key, &value),
                };
                match props.kind(prop) {
                    PropKind::Text => identity_changed |= object.set_text(prop, time, &value),
                    kind => match value.trim().parse::<f32>() {
                        Ok(number) if number.is_finite() => object.set_number(prop, kind, time, number),
                        _ => self.stats.skipped += 1,
                    },
                }
            }
        }

        if sampled {
            recording.extend_time_range(time);
        }
        if identity_changed {
            recording.revision += 1;
        }
        if first_fix {
            let object = &recording.objects[handle.index()];
            let wants_parent = object.category() == Category::Weapon
                && !object.tags().contains(TagSet::SHELL)
                && object.text(known::PARENT).is_empty();
            if wants_parent {
                let parent = recording.infer_parent(handle);
                recording.objects[handle.index()].inferred_parent = parent;
            }
        }
    }
}

/// A line continues on the next one when it ends in an unescaped backslash.
#[inline]
fn ends_with_continuation(line: &str) -> bool {
    let trailing = line.bytes().rev().take_while(|&b| b == b'\\').count();
    trailing % 2 == 1
}

/// Iterator over the `Key=Value` pairs of a line, honouring `\,` escapes.
struct Properties<'a> {
    rest: &'a str,
}

impl<'a> Properties<'a> {
    fn new(rest: &'a str) -> Self {
        Self { rest }
    }
}

impl<'a> Iterator for Properties<'a> {
    type Item = (&'a str, &'a str);

    fn next(&mut self) -> Option<Self::Item> {
        while !self.rest.is_empty() {
            let bytes = self.rest.as_bytes();
            let mut end = bytes.len();
            let mut i = 0;
            while i < bytes.len() {
                match bytes[i] {
                    b'\\' => i += 1, // skip the escaped byte
                    b',' => {
                        end = i;
                        break;
                    }
                    _ => {}
                }
                i += 1;
            }
            let token = &self.rest[..end];
            self.rest = self.rest.get(end + 1..).unwrap_or("");
            if let Some((key, value)) = token.split_once('=') {
                return Some((key.trim(), value));
            }
        }
        None
    }
}

fn unescape(raw: &str) -> Cow<'_, str> {
    if raw.contains('\\') {
        Cow::Owned(raw.replace("\\,", ","))
    } else {
        Cow::Borrowed(raw)
    }
}

/// Parses the `T=` field. Supported layouts:
///
/// ```text
/// Lon|Lat|Alt
/// Lon|Lat|Alt|U|V
/// Lon|Lat|Alt|Roll|Pitch|Yaw
/// Lon|Lat|Alt|Roll|Pitch|Yaw|U|V|Heading
/// ```
///
/// `U`/`V` are the simulator's native flat-world coordinates and are not
/// needed on a globe. `Heading` (flat-world) stands in for yaw when the
/// latter is absent.
fn parse_transform(raw: &str, ref_lon: f64, ref_lat: f64) -> TransformUpdate {
    let mut fields = [""; 9];
    let mut count = 0;
    for (slot, part) in fields.iter_mut().zip(raw.split('|')) {
        *slot = part;
        count += 1;
    }
    let f64_at = |i: usize| fields[i].trim().parse::<f64>().ok().filter(|v| v.is_finite());
    let f32_at = |i: usize| fields[i].trim().parse::<f32>().ok().filter(|v| v.is_finite());

    let mut update = TransformUpdate {
        lon: f64_at(0).map(|v| v + ref_lon),
        lat: f64_at(1).map(|v| v + ref_lat),
        alt: f32_at(2),
        ..Default::default()
    };
    if count >= 6 {
        update.roll = f32_at(3);
        update.pitch = f32_at(4);
        update.yaw = f32_at(5);
        if count >= 9 && update.yaw.is_none() {
            update.yaw = f32_at(8);
        }
    }
    update
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::EventKind;
    use crate::props::Unit;

    const HEADER: &str = "FileType=text/acmi/tacview\nFileVersion=2.2\n";

    fn parse(body: &str) -> Recording {
        let mut recording = Recording::new();
        let text = format!("{HEADER}{body}");
        Parser::new().feed_text(&mut recording, &text, |_| {}).unwrap();
        recording
    }

    #[test]
    fn rejects_non_acmi_input() {
        let mut recording = Recording::new();
        let err = Parser::new().feed_text(&mut recording, "FileType=text/plain\n", |_| {}).unwrap_err();
        assert!(matches!(err, ParseError::InvalidFileType { line: 1, .. }));
        let err = Parser::new().feed_text(&mut recording, "FileType=text/acmi/tacview\nFileVersion=1.4\n", |_| {});
        assert!(matches!(err, Err(ParseError::UnsupportedVersion { line: 2, .. })));
        assert_eq!(Parser::new().feed_text(&mut recording, "hello\nworld\n", |_| {}), Err(ParseError::MissingHeader));
    }

    #[test]
    fn handles_bom_and_crlf() {
        let mut recording = Recording::new();
        let text = "\u{feff}FileType=text/acmi/tacview\r\nFileVersion=2.1\r\n0,Title=Test\r\n#1\r\n1,T=1|2|3\r\n";
        Parser::new().feed_text(&mut recording, text, |_| {}).unwrap();
        assert_eq!(recording.meta.title(), "Test");
        assert_eq!(recording.objects().len(), 1);
    }

    #[test]
    fn applies_reference_point_and_metadata() {
        let rec = parse(
            "0,ReferenceTime=2011-06-02T05:00:00Z\n0,ReferenceLongitude=-129\n0,ReferenceLatitude=43\n\
             #0\n10,T=0.5|0.25|2000,Type=Air+FixedWing,Name=F-16C,Pilot=Viper 1\n",
        );
        assert_eq!(rec.meta.reference_time, Some(1_306_990_800.0));
        let obj = rec.object(rec.find(0x10).unwrap());
        assert_eq!(obj.track.position(0), (43.25, -128.5, 2000.0));
        assert_eq!(obj.name(), "F-16C");
        assert_eq!(obj.text(known::PILOT), "Viper 1");
        assert_eq!(obj.category(), Category::Aircraft);
    }

    #[test]
    fn transform_layouts() {
        let rec = parse("#0\n1,T=1|2|3|4|5\n2,T=1|2|3|10|20|30\n3,T=1|2|3|||  |7|8|270\n4,T=1|2|3\n#1\n4,T=||50\n");
        let orientation = |id| rec.object(rec.find(id).unwrap()).track.recorded_orientation(0);
        assert!(orientation(1)[2].is_nan(), "U|V must not be read as attitude");
        assert_eq!(orientation(2), [10.0, 20.0, 30.0]);
        assert_eq!(orientation(3)[2], 270.0, "flat heading substitutes for missing yaw");
        // Empty fields keep their previous value.
        assert_eq!(rec.object(rec.find(4).unwrap()).track.position(1), (2.0, 1.0, 50.0));
    }

    #[test]
    fn escaped_commas_and_continuations() {
        let rec = parse("0,Briefing=Line one\\\nLine two\\, still two\n#0\n1,T=1|2|3,Name=Alpha\\, Bravo,Pilot=X\n");
        assert_eq!(rec.meta.get("Briefing"), Some("Line one\nLine two, still two"));
        let obj = rec.object(rec.find(1).unwrap());
        assert_eq!(obj.name(), "Alpha, Bravo");
        assert_eq!(obj.text(known::PILOT), "X");
    }

    #[test]
    fn numeric_properties_are_time_series() {
        let rec = parse("#0\n1,T=1|2|3,IAS=100,OnGround=1,FuelWeight7=900\n#10\n1,IAS=200,OnGround=0\n");
        let obj = rec.object(rec.find(1).unwrap());
        assert_eq!(obj.number_at(known::IAS, PropKind::Number(Unit::MetersPerSecond), 5.0), Some(150.0));
        assert_eq!(obj.number_at(known::ON_GROUND, PropKind::Flag, 5.0), Some(1.0));
        let fuel = rec.props.get("FuelWeight7").expect("unknown numeric property is registered");
        assert_eq!(obj.number_at(fuel, rec.props.kind(fuel), 99.0), Some(900.0));
    }

    #[test]
    fn removal_and_time_range() {
        let rec = parse("#5\n1,T=1|2|3\n#8\n1,T=1|2|4\n#9\n-1\n#12\n");
        let obj = rec.object(rec.find(1).unwrap());
        assert!(!obj.is_active_at(4.0));
        assert!(obj.is_active_at(9.0));
        assert!(!obj.is_active_at(9.5));
        assert_eq!(rec.time_range(), Some((5.0, 12.0)));
    }

    #[test]
    fn recycled_ids_create_new_objects() {
        let rec = parse("#0\n1,T=1|2|3,Name=First\n#1\n-1\n#2\n1,T=5|6|7,Name=Second\n");
        assert_eq!(rec.objects().len(), 2);
        assert_eq!(rec.objects()[0].name(), "First");
        assert_eq!(rec.object(rec.find(1).unwrap()).name(), "Second");
    }

    #[test]
    fn events_are_collected_in_order() {
        let rec = parse("#3\n0,Event=Destroyed|a1|\n#1\n0,Event=Bookmark|Start\n");
        let kinds: Vec<_> = rec.events().iter().map(|e| (e.time, e.kind.clone())).collect();
        assert_eq!(kinds, vec![(1.0, EventKind::Bookmark), (3.0, EventKind::Destroyed)]);
        assert_eq!(rec.events()[1].objects, vec![0xa1]);
    }

    #[test]
    fn weapons_get_a_parent() {
        let rec = parse(
            "#0\n1,T=10|10|5000,Type=Air+FixedWing\n2,T=12|12|5000,Type=Air+FixedWing\n\
             #1\n3,T=10.001|10|4990,Type=Weapon+Missile\n4,T=12|12|4990,Type=Weapon+Missile,Parent=1\n",
        );
        assert_eq!(rec.object(rec.find(3).unwrap()).parent(), Some(1));
        assert_eq!(rec.object(rec.find(4).unwrap()).parent(), Some(1), "explicit Parent wins");
    }

    #[test]
    fn malformed_lines_are_skipped() {
        let mut recording = Recording::new();
        let mut parser = Parser::new();
        let text = format!("{HEADER}#abc\nnot a line\nzz,T=1|2|3\n#1\n1,T=1|2|3,IAS=fast\n");
        parser.feed_text(&mut recording, &text, |_| {}).unwrap();
        assert_eq!(recording.objects().len(), 1);
        assert_eq!(parser.stats().skipped, 4);
    }

    #[test]
    fn streaming_line_by_line() {
        let mut recording = Recording::new();
        let mut parser = Parser::new();
        for line in ["FileType=text/acmi/tacview", "FileVersion=2.2", "#2.5", "a,T=1|1|1"] {
            parser.feed_line(&mut recording, line).unwrap();
        }
        assert_eq!(parser.current_time(), 2.5);
        assert!(recording.find(0xa).is_some());
    }
}
