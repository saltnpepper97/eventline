//! Console output for live event streaming.
//!
//! This module provides immediate terminal output for events as they're recorded,
//! enabling a "dual output" mode where events are both journaled and printed.

use std::sync::atomic::{AtomicBool, Ordering};
use crate::journal::event_kind::EventKind;

/// Global flag controlling console output.
static CONSOLE_ENABLED: AtomicBool = AtomicBool::new(false);

/// Global flag controlling color output (runtime control).
static CONSOLE_COLOR: AtomicBool = AtomicBool::new(true);

/// Enable or disable automatic console output for events.
///
/// When enabled, events are printed to the console immediately as they're recorded,
/// in addition to being stored in the journal. This provides real-time feedback
/// similar to traditional logging libraries.
///
/// # Example
///
/// ```
/// use eventline::runtime;
///
/// runtime::init();
/// runtime::enable_console_output(true);
///
/// // This will both record AND print to console
/// runtime::info("Server started");
/// ```
pub fn enable_console_output(enable: bool) {
    CONSOLE_ENABLED.store(enable, Ordering::Relaxed);
}

/// Check if console output is currently enabled.
pub fn is_console_enabled() -> bool {
    CONSOLE_ENABLED.load(Ordering::Relaxed)
}

/// Enable or disable color output for console events.
///
/// This only has effect if the `color` feature is enabled at compile time.
/// Without the feature, output is always plain regardless of this setting.
///
/// # Example
///
/// ```
/// use eventline::runtime;
///
/// runtime::init();
/// runtime::enable_console_output(true);
/// runtime::enable_console_color(true); // Enable colored output
///
/// runtime::error("This will be red");
/// ```
pub fn enable_console_color(enable: bool) {
    CONSOLE_COLOR.store(enable, Ordering::Relaxed);
}

/// Check if console color is currently enabled.
pub fn is_console_color_enabled() -> bool {
    CONSOLE_COLOR.load(Ordering::Relaxed)
}

/// Print an event to the console with appropriate formatting.
///
/// This is called internally by the runtime when console output is enabled.
/// It applies consistent formatting based on the event kind:
/// - Debug: prefixed with "debug: " (blue if color enabled)
/// - Info: no prefix
/// - Warning: prefixed with "warning: " (yellow if color enabled)
/// - Error: prefixed with "error: " and sent to stderr (red if color enabled)
///
/// Color is automatically applied if:
/// 1. The `color` feature is enabled at compile time, AND
/// 2. Color is enabled at runtime via `enable_console_color(true)`
pub fn print_event(kind: EventKind, message: &str) {
    #[cfg(feature = "colour")]
    {
        if is_console_color_enabled() {
            print_event_colored(kind, message);
            return;
        }
    }
    
    // Plain output (no color)
    match kind {
        EventKind::Debug => println!("debug: {}", message),
        EventKind::Info => println!("{}", message),
        EventKind::Warning => println!("warning: {}", message),
        EventKind::Error => eprintln!("error: {}", message),
    }
}

/// Print an event with ANSI color codes.
///
/// Internal function - automatically called by `print_event` when color is enabled.
#[cfg(feature = "colour")]
fn print_event_colored(kind: EventKind, message: &str) {
    use crate::colour::{RESET, RED, YELLOW, BLUE};
    
    match kind {
        EventKind::Debug => println!("{}debug:{} {}", BLUE, RESET, message),
        EventKind::Info => println!("{}", message),
        EventKind::Warning => println!("{}warning:{} {}", YELLOW, RESET, message),
        EventKind::Error => eprintln!("{}error:{} {}", RED, RESET, message),
    }
}
