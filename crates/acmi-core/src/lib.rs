//! ACMI 2.x data model, parser and writer.
//!
//! This crate is deliberately free of I/O, threading and rendering so it can
//! be reused by tools, tests and other front-ends:
//!
//! * [`Recording`] owns everything known about a session: metadata, objects,
//!   their time series and the event log.
//! * [`Parser`] is an incremental, line-oriented state machine. The same
//!   parser handles files and real-time telemetry streams.
//! * [`writer::write_acmi`] serialises a recording back to ACMI text.
//! * [`geo`] holds the world-space conventions shared with the renderer.

pub mod derive;
pub mod event;
pub mod filter;
pub mod geo;
pub mod object;
pub mod parser;
pub mod props;
pub mod recording;
pub mod tags;
pub mod time;
pub mod writer;

pub use event::{Event, EventKind};
pub use object::{Kinematics, Object, ObjectId, Pose, Track};
pub use parser::{ParseError, ParseStats, Parser};
pub use props::{known, PropId, PropKind, PropRegistry, Unit};
pub use recording::{Metadata, ObjectHandle, Recording};
pub use tags::{Category, TagSet};
