//! Exporting data out of a session.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

use acmi_core::time::format_utc;
use acmi_core::{ObjectHandle, Recording};

/// Writes the telemetry of one object as CSV: one row per recorded
/// position, with every numeric property sampled at that instant.
pub fn telemetry_csv(recording: &Recording, handle: ObjectHandle, path: &Path) -> io::Result<()> {
    let mut out = BufWriter::new(File::create(path)?);
    write_telemetry_csv(recording, handle, &mut out)?;
    out.flush()
}

fn write_telemetry_csv(recording: &Recording, handle: ObjectHandle, out: &mut impl Write) -> io::Result<()> {
    let object = recording.object(handle);
    let numeric: Vec<_> = object.number_props().map(|(id, _)| id).collect();

    write!(out, "Time,UTC,Longitude,Latitude,Altitude,Roll,Pitch,Heading")?;
    for &id in &numeric {
        write!(out, ",{}", recording.props.name(id))?;
    }
    writeln!(out)?;

    for &time in object.track.times() {
        let Some(pose) = object.pose_at(time) else { continue };
        let utc = recording.absolute_time(time).map(format_utc).unwrap_or_default();
        write!(
            out,
            "{time:.2},{utc},{:.7},{:.7},{:.2},{:.2},{:.2},{:.2}",
            pose.lon, pose.lat, pose.alt, pose.roll, pose.pitch, pose.heading
        )?;
        for &id in &numeric {
            match object.number_at(id, recording.props.kind(id), time) {
                Some(value) => write!(out, ",{value}")?,
                None => write!(out, ",")?,
            }
        }
        writeln!(out)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use acmi_core::Parser;

    #[test]
    fn csv_has_header_and_rows() {
        let mut recording = Recording::new();
        let text = "FileType=text/acmi/tacview\nFileVersion=2.2\n0,ReferenceTime=2020-01-01T00:00:00Z\n\
                    #0\n1,T=1|2|300|0|0|90,IAS=100\n#1\n1,T=1.1|2|310|0|0|90\n#2\n1,T=1.2|2|320|0|0|90,IAS=120\n";
        Parser::new().feed_text(&mut recording, text, |_| {}).unwrap();

        let mut csv = Vec::new();
        write_telemetry_csv(&recording, recording.find(1).unwrap(), &mut csv).unwrap();
        let csv = String::from_utf8(csv).unwrap();
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines[0], "Time,UTC,Longitude,Latitude,Altitude,Roll,Pitch,Heading,IAS");
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[2], "1.00,2020-01-01 00:00:01,1.1000000,2.0000000,310.00,0.00,0.00,90.00,110");
    }
}
