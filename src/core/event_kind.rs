//! Semantic classification for journal events.
//!
//! EventKind describes what kind of event occurred. It does not encode
//! severity, importance, or visibility. Filtering and rendering are handled
//! by writers and renderers.

use serde::{Deserialize, Serialize};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventKind {
    Info,
    Warning,
    Error,
    Debug,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::Info => "Info",
            EventKind::Warning => "Warn",
            EventKind::Error => "Error",
            EventKind::Debug => "Debug",
        }
    }
}
