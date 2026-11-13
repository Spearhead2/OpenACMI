//! I/O for OpenACMI: reading and writing recordings on disk and receiving
//! real-time telemetry. Everything here is UI-agnostic; long operations run
//! on worker threads and report back through channels and atomics.

pub mod load;
pub mod rtt;
pub mod save;

pub use load::{load_file, LoadError, LoadJob, LoadOptions, LoadProgress, Loaded};
pub use rtt::{RttClient, RttConfig, RttMessage, RttStatus};
pub use save::{save_file, SaveError};
