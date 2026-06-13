//! A structured execution journal for Rust applications.
//!
//! `eventline` is designed as a small application flight recorder: it records
//! scoped execution history first, then renders that history to console, text
//! files, JSONL files, or replay helpers as secondary views.
//!
//! The core rule is that recording and emission are separate concerns. Sink log
//! levels control what users see in console/file output, while the recording
//! threshold controls what is retained in the runtime journal.
//!
//! ```
//! eventline::init_sync();
//! eventline::disable_all_output();
//! eventline::debug!("hidden from output but still recorded", answer = 42);
//!
//! assert_eq!(eventline::records().len(), 1);
//! ```

pub mod core;
pub mod integrations;
pub mod journal;
pub mod macros;
pub mod render;
pub mod replay;
pub mod runtime;

mod configure;

// -----------------------------------------------------------------------------
// Core exports
// -----------------------------------------------------------------------------
pub use core::{EventKind, ExitMessages, RecordId, RuntimeScopeGuard, ScopeGuard, ScopeId, Value};

// -----------------------------------------------------------------------------
// Journal exports
// -----------------------------------------------------------------------------
pub use journal::{FileWriter, Journal, MultiWriter, StdoutWriter, Writer};

pub use journal::rotation::LogPolicy;

// -----------------------------------------------------------------------------
// Runtime exports (flattened so users don’t type `runtime::`)
// -----------------------------------------------------------------------------
pub use runtime::{
    disable_all_output, disable_file_output, dropped_writer_records, emit, enable_console_color,
    enable_console_duration, enable_console_output, enable_console_scope_exits,
    enable_console_scope_labels, enable_console_timestamp, enable_file_output,
    enable_file_output_jsonl, enable_file_output_rotating, enter_scope, exit_scope, flush, init,
    init_sync, last_writer_error, records, records_jsonl, scope_guard, scopes, set_console_level,
    set_file_format, set_file_level, set_journal_retention, set_scope_exit_messages,
};

pub use render::FileFormat;

pub use runtime::run_header::RunHeader;
pub use runtime::{LogLevel, get_log_level, get_record_level, set_log_level, set_record_level};

// -----------------------------------------------------------------------------
// Setup helper (new sugar layer)
// -----------------------------------------------------------------------------
pub use configure::{FileSetup, Setup, setup};
