//! Saving recordings as plain or zip-compressed ACMI.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

use acmi_core::writer::write_acmi;
use acmi_core::Recording;

#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error("cannot write file: {0}")]
    Io(#[from] io::Error),
    #[error("cannot write zip archive: {0}")]
    Zip(#[from] zip::result::ZipError),
}

/// Writes `recording` to `path`. With `compress`, the file is a zip archive
/// holding a single `.acmi` entry, as Tacview's `.zip.acmi` files are.
pub fn save_file(recording: &Recording, path: &Path, compress: bool) -> Result<(), SaveError> {
    let file = BufWriter::new(File::create(path)?);
    if compress {
        let stem = path.file_name().and_then(|n| n.to_str()).unwrap_or("recording");
        let stem = stem.trim_end_matches(".acmi").trim_end_matches(".zip");
        let mut archive = zip::ZipWriter::new(file);
        archive.start_file(format!("{stem}.txt.acmi"), zip::write::SimpleFileOptions::default())?;
        write_acmi(recording, &mut archive)?;
        archive.finish()?.flush()?;
    } else {
        let mut file = file;
        write_acmi(recording, &mut file)?;
        file.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load::{load_file, LoadOptions, LoadProgress};
    use acmi_core::Parser;

    #[test]
    fn plain_and_zip_round_trip() {
        let mut recording = Recording::new();
        let text = "FileType=text/acmi/tacview\nFileVersion=2.2\n0,Title=Saved\n#0\n1,T=1|2|3,Name=A\n#1\n1,T=2|2|3\n";
        Parser::new().feed_text(&mut recording, text, |_| {}).unwrap();

        let dir = std::env::temp_dir().join(format!("openacmi-save-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, compress) in [("plain.acmi", false), ("packed.zip.acmi", true)] {
            let path = dir.join(name);
            save_file(&recording, &path, compress).unwrap();
            let loaded = load_file(&path, LoadOptions::default(), &LoadProgress::default()).unwrap();
            assert_eq!(loaded.recording.meta.title(), "Saved");
            assert_eq!(loaded.recording.objects()[0].track.len(), 2);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
