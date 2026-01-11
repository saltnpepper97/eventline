//! Semantic classification for journal events.
//!
//! EventKind describes *what kind of event* occurred, not how important it is
//! or whether it should be shown. Filtering and rendering decisions are made
//! by writers and renderers, not at record time.

use serde::Serialize;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize)]
pub enum EventKind {
    /// Normal informational event.
    Info,
    /// Something unexpected but recoverable occurred.
    Warning,
    /// An error occurred; execution continued or was handled.
    Error,
    /// Low-level diagnostic information.
    Debug,
}

impl EventKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            EventKind::Info => "INFO",
            EventKind::Warning => "WARN",
            EventKind::Error => "ERROR",
            EventKind::Debug => "DEBUG",
        }
    }
}
