use std::io;
use std::path::PathBuf;

use crate::journal::LogPolicy;
use crate::runtime::{self, LogLevel, RunHeader};

/// High-level configuration used to initialize the eventline runtime.
///
/// This struct provides a convenient way to configure logging behavior
/// during application startup. It wraps the lower-level runtime functions
/// into a single, consistent initialization call.
///
/// All fields are optional or policy-based, allowing applications to define
/// their own logging behavior without enforcing a specific model.
#[derive(Debug, Clone)]
pub struct Setup {
    /// Enables verbose mode.
    ///
    /// When `true`:
    /// - Console output is enabled.
    /// - Log level defaults to [`LogLevel::Debug`] unless `level` is explicitly set.
    ///
    /// When `false`:
    /// - Console output is disabled.
    /// - Log level defaults to [`LogLevel::Info`] unless `level` is explicitly set.
    pub verbose: bool,

    /// Explicit log level override.
    ///
    /// If `Some`, this value takes precedence over the `verbose` flag.
    /// If `None`, the level is determined based on `verbose`.
    pub level: Option<LogLevel>,

    /// Optional console sink level override.
    ///
    /// When omitted, falls back to the shared `level` behavior.
    pub console_level: Option<LogLevel>,

    /// Optional file sink level override.
    ///
    /// When omitted, falls back to the shared `level` behavior.
    pub file_level: Option<LogLevel>,

    /// Optional file logging configuration.
    ///
    /// If `None`, file logging is left unchanged.
    /// Use [`FileSetup::Off`] to explicitly disable file output.
    pub file: Option<FileSetup>,

    /// Optional maximum number of records/scopes kept in the in-memory runtime journal.
    ///
    /// Durable history should use a file sink; this controls only the bounded
    /// in-process replay buffer returned by `eventline::records()` and
    /// `eventline::scopes()`.
    pub journal_retention: Option<usize>,
}

/// File logging configuration options.
///
/// Controls how (or if) log records are written to disk.
#[derive(Debug, Clone)]
pub enum FileSetup {
    /// Disable file output entirely.
    Off,

    /// Enable plain file logging without rotation.
    ///
    /// Logs will be appended to the provided path.
    Plain {
        /// Target log file path.
        path: PathBuf,
    },

    /// Enable JSONL file logging without rotation.
    ///
    /// Logs will be appended to the provided path as one JSON object per line.
    Jsonl {
        /// Target JSONL log file path.
        path: PathBuf,
    },

    /// Enable rotating file logging.
    ///
    /// Log files will rotate according to the provided [`LogPolicy`].
    /// An optional [`RunHeader`] may be written before structured records.
    Rotating {
        /// Target log file path.
        path: PathBuf,

        /// Rotation policy defining size and backup behavior.
        policy: LogPolicy,

        /// Optional header written at the beginning of the session.
        header: Option<RunHeader>,
    },
}

/// Initialize and configure the eventline runtime.
///
/// This function:
///
/// 1. Ensures the runtime is initialized (idempotent).
/// 2. Configures console output based on `verbose`.
/// 3. Applies console and file sink levels.
/// 4. Applies file logging configuration if provided.
///
/// The shared fallback level is determined as follows:
///
/// - If `Setup::level` is `Some`, that value is used.
/// - Otherwise:
///     - `LogLevel::Debug` when `verbose == true`
///     - `LogLevel::Info` when `verbose == false`
///
/// Sink levels are then resolved independently:
///
/// - `console_level`, when set, overrides the console sink threshold.
/// - `file_level`, when set, overrides the file sink threshold.
/// - otherwise both fall back to the shared `level` behavior above.
///
/// # Errors
///
/// Returns an [`io::Error`] if file logging initialization fails.
pub async fn setup(s: Setup) -> io::Result<()> {
    // Initialize runtime (safe to call multiple times).
    runtime::init_sync();

    // Console output is controlled directly by the verbose flag.
    runtime::enable_console_output(s.verbose);

    // Determine shared fallback level.
    let shared_level = match s.level {
        Some(l) => l,
        None if s.verbose => LogLevel::Debug,
        None => LogLevel::Info,
    };
    runtime::set_console_level(s.console_level.unwrap_or(shared_level));
    runtime::set_file_level(s.file_level.unwrap_or(shared_level));

    if let Some(max_records) = s.journal_retention {
        runtime::set_journal_retention(max_records);
    }

    // Apply file configuration if specified.
    match s.file {
        None => {}
        Some(FileSetup::Off) => runtime::disable_file_output(),
        Some(FileSetup::Plain { path }) => runtime::enable_file_output(path)?,
        Some(FileSetup::Jsonl { path }) => runtime::enable_file_output_jsonl(path)?,
        Some(FileSetup::Rotating {
            path,
            policy,
            header,
        }) => runtime::enable_file_output_rotating(path, policy, header)?,
    }

    Ok(())
}
