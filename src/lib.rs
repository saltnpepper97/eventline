pub mod core;
pub mod journal;
pub mod macros;
pub mod render;
pub mod runtime;

mod configure;

// -----------------------------------------------------------------------------
// Core exports
// -----------------------------------------------------------------------------
pub use core::{
    EventKind,
    ScopeId,
    ScopeGuard,
    RuntimeScopeGuard,
    ExitMessages,
    RecordId,
    Value,
};

// -----------------------------------------------------------------------------
// Journal exports
// -----------------------------------------------------------------------------
pub use journal::{
    Journal,
    Writer,
    StdoutWriter,
    FileWriter,
    MultiWriter,
};

pub use journal::rotation::LogPolicy;

// -----------------------------------------------------------------------------
// Runtime exports (flattened so users don’t type `runtime::`)
// -----------------------------------------------------------------------------
pub use runtime::{
    init,
    enable_console_output,
    enable_console_color,
    enable_console_duration,
    enable_console_timestamp,
    enable_file_output,
    enable_file_output_rotating,
    disable_file_output,
    disable_all_output,
    flush,
    records,
    scopes,
    emit,
    scope_guard,
    enter_scope,
    set_scope_exit_messages,
    exit_scope,
};

pub use runtime::run_header::RunHeader;
pub use runtime::{LogLevel, get_log_level, set_log_level};

// -----------------------------------------------------------------------------
// Setup helper (new sugar layer)
// -----------------------------------------------------------------------------
pub use configure::{setup, Setup, FileSetup};
