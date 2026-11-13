//! Loading ACMI recordings: plain text or zip-compressed (`.zip.acmi`).

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use acmi_core::filter::decimate_frames;
use acmi_core::{ParseError, ParseStats, Parser, Recording};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadOptions {
    /// Keep one in every N frames (see [`acmi_core::filter`]). `1` is lossless.
    pub keep_every_nth: usize,
}

impl Default for LoadOptions {
    fn default() -> Self {
        Self { keep_every_nth: 1 }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("cannot read file: {0}")]
    Io(#[from] io::Error),
    #[error("cannot read zip archive: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("the archive contains no .acmi file")]
    NoAcmiInArchive,
    #[error(transparent)]
    Parse(#[from] ParseError),
    #[error("loading was cancelled")]
    Cancelled,
}

/// Result of a successful load.
#[derive(Debug)]
pub struct Loaded {
    pub recording: Recording,
    pub stats: ParseStats,
    pub elapsed: Duration,
}

/// Shared between a loading thread and whoever is watching it.
#[derive(Debug, Default)]
pub struct LoadProgress {
    permille: AtomicU32,
    cancelled: AtomicBool,
}

impl LoadProgress {
    /// Completed fraction in `0.0..=1.0`.
    pub fn fraction(&self) -> f32 {
        self.permille.load(Ordering::Relaxed) as f32 / 1000.0
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    fn set(&self, fraction: f64) {
        self.permille.store((fraction.clamp(0.0, 1.0) * 1000.0) as u32, Ordering::Relaxed);
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

/// Reads the ACMI text of a file, transparently unpacking zip archives.
pub fn read_text(path: &Path) -> Result<String, LoadError> {
    let bytes = fs::read(path)?;
    let bytes = if bytes.starts_with(b"PK") { unzip_acmi(&bytes)? } else { bytes };
    // ACMI is UTF-8; tolerate the odd invalid byte rather than refusing the file.
    Ok(match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(err) => String::from_utf8_lossy(err.as_bytes()).into_owned(),
    })
}

fn unzip_acmi(bytes: &[u8]) -> Result<Vec<u8>, LoadError> {
    let mut archive = zip::ZipArchive::new(io::Cursor::new(bytes))?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let name = entry.name()?.to_ascii_lowercase();
        if name.ends_with(".acmi") || name.ends_with(".txt") {
            let mut out = Vec::new();
            entry.read_to_end(&mut out)?;
            return Ok(out);
        }
    }
    Err(LoadError::NoAcmiInArchive)
}

/// Loads and parses a recording on the calling thread.
pub fn load_file(path: &Path, options: LoadOptions, progress: &LoadProgress) -> Result<Loaded, LoadError> {
    const CHECK_INTERVAL: usize = 8192;

    let started = Instant::now();
    let text = read_text(path)?;
    let ranges = decimate_frames(&text, options.keep_every_nth);

    let mut recording = Recording::new();
    let mut parser = Parser::new();
    let total = text.len().max(1) as f64;
    let mut countdown = CHECK_INTERVAL;
    for range in ranges {
        let mut offset = range.start;
        for line in text[range].split_inclusive('\n') {
            parser.feed_line(&mut recording, line)?;
            offset += line.len();
            countdown -= 1;
            if countdown == 0 {
                countdown = CHECK_INTERVAL;
                progress.set(offset as f64 / total);
                if progress.is_cancelled() {
                    return Err(LoadError::Cancelled);
                }
            }
        }
    }
    parser.finish()?;
    progress.set(1.0);

    let loaded = Loaded { recording, stats: parser.stats(), elapsed: started.elapsed() };
    log::info!(
        "loaded {} in {:.2?}: {} objects, {} lines ({} skipped)",
        path.display(),
        loaded.elapsed,
        loaded.recording.objects().len(),
        loaded.stats.lines,
        loaded.stats.skipped
    );
    Ok(loaded)
}

/// A load running on a background thread.
pub struct LoadJob {
    path: PathBuf,
    progress: Arc<LoadProgress>,
    result: Receiver<Result<Loaded, LoadError>>,
}

impl LoadJob {
    pub fn spawn(path: PathBuf, options: LoadOptions) -> Self {
        let progress = Arc::new(LoadProgress::default());
        let (tx, result) = mpsc::channel();
        let (thread_path, thread_progress) = (path.clone(), Arc::clone(&progress));
        std::thread::Builder::new()
            .name("acmi-loader".into())
            .spawn(move || {
                // The receiver may be gone if the job was dropped; that is fine.
                let _ = tx.send(load_file(&thread_path, options, &thread_progress));
            })
            .expect("failed to spawn loader thread");
        Self { path, progress, result }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn progress(&self) -> f32 {
        self.progress.fraction()
    }

    /// Returns the outcome once the load has finished; `None` while running.
    pub fn poll(&self) -> Option<Result<Loaded, LoadError>> {
        match self.result.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(LoadError::Io(io::Error::other("loader thread panicked")))),
        }
    }
}

impl Drop for LoadJob {
    fn drop(&mut self) {
        self.progress.cancel();
    }
}
