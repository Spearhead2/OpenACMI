//! Serialises a [`Recording`] back to ACMI 2.2 text.
//!
//! The output is a well-formed, chronologically ordered document carrying
//! every property series the recording holds, not just positions, so a
//! load → save round trip preserves telemetry.

use std::fmt::Write as _;
use std::io::{self, Write};

use crate::recording::Recording;

/// One thing that happens to one object at one instant.
#[derive(Clone, Copy)]
struct Change {
    time: f64,
    object: u32,
    kind: ChangeKind,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ChangeKind {
    /// Track sample index.
    Transform(u32),
    /// Index into the object's text properties, then the point within the series.
    Text(u32, u32),
    /// Index into the object's numeric properties, then the point within the series.
    Number(u32, u32),
    Removal,
}

pub fn write_acmi(recording: &Recording, out: &mut impl Write) -> io::Result<()> {
    out.write_all(b"FileType=text/acmi/tacview\nFileVersion=2.2\n")?;
    for (key, value) in recording.meta.iter() {
        writeln!(out, "0,{key}={}", escape(value))?;
    }

    let mut changes = collect_changes(recording);
    changes.sort_by(|a, b| a.time.total_cmp(&b.time).then(a.object.cmp(&b.object)).then(a.kind.cmp(&b.kind)));

    let events = recording.events();
    let mut next_event = 0;
    let mut current_time = f64::NAN;
    let mut line = String::with_capacity(256);

    let mut i = 0;
    while i < changes.len() || next_event < events.len() {
        // Whichever comes first: the next object change or the next event.
        let change_time = changes.get(i).map_or(f64::INFINITY, |c| c.time);
        let event_time = events.get(next_event).map_or(f64::INFINITY, |e| e.time);
        let time = change_time.min(event_time);
        if time != current_time {
            writeln!(out, "#{}", fmt_number(time, 3))?;
            current_time = time;
        }

        if event_time <= change_time {
            writeln!(out, "0,Event={}", escape(&events[next_event].to_acmi()))?;
            next_event += 1;
            continue;
        }

        // All changes of one object at this instant become one line.
        let object_index = changes[i].object;
        let object = &recording.objects()[object_index as usize];
        line.clear();
        let mut removed = false;
        while let Some(change) = changes.get(i).filter(|c| c.time == time && c.object == object_index) {
            match change.kind {
                ChangeKind::Transform(sample) => write_transform(&mut line, recording, object, sample as usize),
                ChangeKind::Text(prop, point) => {
                    let (id, series) = object.text_props().nth(prop as usize).unwrap();
                    let (_, value) = series.points().nth(point as usize).unwrap();
                    let _ = write!(line, ",{}={}", recording.props.name(id), escape(value));
                }
                ChangeKind::Number(prop, point) => {
                    let (id, series) = object.number_props().nth(prop as usize).unwrap();
                    let (_, value) = series.points().nth(point as usize).unwrap();
                    let _ = write!(line, ",{}={}", recording.props.name(id), fmt_number(value as f64, 4));
                }
                ChangeKind::Removal => removed = true,
            }
            i += 1;
        }
        if !line.is_empty() {
            writeln!(out, "{:x}{line}", object.id)?;
        }
        if removed {
            writeln!(out, "-{:x}", object.id)?;
        }
    }
    Ok(())
}

/// Convenience wrapper returning the document as a string.
pub fn to_acmi_string(recording: &Recording) -> String {
    let mut buffer = Vec::new();
    write_acmi(recording, &mut buffer).expect("writing to memory cannot fail");
    String::from_utf8(buffer).expect("writer emits UTF-8")
}

fn collect_changes(recording: &Recording) -> Vec<Change> {
    let mut changes = Vec::new();
    for (index, object) in recording.objects().iter().enumerate() {
        let object_index = index as u32;
        let mut push = |time: f64, kind: ChangeKind| changes.push(Change { time, object: object_index, kind });

        for (sample, &time) in object.track.times().iter().enumerate() {
            push(time, ChangeKind::Transform(sample as u32));
        }
        for (prop, (_, series)) in object.text_props().enumerate() {
            for (point, (time, _)) in series.points().enumerate() {
                push(time, ChangeKind::Text(prop as u32, point as u32));
            }
        }
        for (prop, (_, series)) in object.number_props().enumerate() {
            for (point, (time, _)) in series.points().enumerate() {
                push(time, ChangeKind::Number(prop as u32, point as u32));
            }
        }
        if let Some(time) = object.removed_at() {
            push(time, ChangeKind::Removal);
        }
    }
    changes
}

fn write_transform(line: &mut String, recording: &Recording, object: &crate::Object, sample: usize) {
    let (lat, lon, alt) = object.track.position(sample);
    let _ = write!(
        line,
        ",T={}|{}|{}",
        fmt_number(lon - recording.meta.reference_longitude, 7),
        fmt_number(lat - recording.meta.reference_latitude, 7),
        fmt_number(alt, 2),
    );
    let orientation = object.track.recorded_orientation(sample);
    if orientation.iter().any(|v| !v.is_nan()) {
        for value in orientation {
            line.push('|');
            if !value.is_nan() {
                line.push_str(&fmt_number(value as f64, 2));
            }
        }
    }
}

/// Fixed-point formatting with trailing zeros removed (`12.5`, not `12.500`).
fn fmt_number(value: f64, decimals: usize) -> String {
    let mut s = format!("{value:.decimals$}");
    if s.contains('.') {
        let trimmed = s.trim_end_matches('0').trim_end_matches('.').len();
        s.truncate(trimmed);
    }
    if s == "-0" {
        s.remove(0);
    }
    s
}

/// Escapes commas and encodes line breaks as continuations.
fn escape(value: &str) -> String {
    if !value.contains([',', '\n']) {
        return value.to_owned();
    }
    value.replace(',', "\\,").replace('\n', "\\\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::props::known;
    use crate::Parser;

    #[test]
    fn formats_numbers_compactly() {
        assert_eq!(fmt_number(12.5, 3), "12.5");
        assert_eq!(fmt_number(3.0, 2), "3");
        assert_eq!(fmt_number(-0.000_000_01, 7), "0");
        assert_eq!(fmt_number(-1.250, 2), "-1.25");
    }

    #[test]
    fn round_trip_preserves_content() {
        let source = "FileType=text/acmi/tacview\nFileVersion=2.2\n\
            0,ReferenceTime=2020-01-01T12:00:00Z\n0,ReferenceLongitude=30\n0,ReferenceLatitude=40\n\
            0,Title=Round\\, trip\n0,Briefing=one\\\ntwo\n\
            #0\n1,T=1|2|1000|5|10|90,Type=Air+FixedWing,Name=F-16C,IAS=150\n2,T=3|4|0,Type=Ground+Static+Building\n\
            #1.5\n1,T=1.01|2|1100|6|11|91,IAS=160\n0,Event=Bookmark|half way\n\
            #3\n1,T=1.02|2|1200\n3,T=1.02|2|1190,Type=Weapon+Missile,Parent=1\n\
            #4\n-3\n0,Event=Destroyed|3|\n";

        let mut first = Recording::new();
        Parser::new().feed_text(&mut first, source, |_| {}).unwrap();
        let written = to_acmi_string(&first);

        let mut second = Recording::new();
        Parser::new().feed_text(&mut second, &written, |_| {}).unwrap();

        assert_eq!(second.meta.title(), "Round, trip");
        assert_eq!(second.meta.get("Briefing"), Some("one\ntwo"));
        assert_eq!(second.meta.reference_time, first.meta.reference_time);
        assert_eq!(second.time_range(), first.time_range());
        assert_eq!(second.events(), first.events());
        assert_eq!(second.objects().len(), first.objects().len());

        for (a, b) in first.objects().iter().zip(second.objects()) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.type_str(), b.type_str());
            assert_eq!(a.removed_at(), b.removed_at());
            assert_eq!(a.track.times(), b.track.times());
            for i in 0..a.track.len() {
                let (pa, pb) = (a.track.position(i), b.track.position(i));
                assert!((pa.0 - pb.0).abs() < 1e-6 && (pa.1 - pb.1).abs() < 1e-6 && (pa.2 - pb.2).abs() < 0.01);
                assert_eq!(a.track.recorded_orientation(i).map(f32::to_bits), b.track.recorded_orientation(i).map(f32::to_bits));
            }
            let ias = |o: &crate::Object| o.number_series(known::IAS).map(|s| s.points().collect::<Vec<_>>());
            assert_eq!(ias(a), ias(b));
        }
    }
}
