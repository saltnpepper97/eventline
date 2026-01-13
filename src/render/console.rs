//! Console output for live event streaming.
//!
//! This module provides immediate terminal output for events as they're recorded,
//! using the canonical "Narrative Structured" format for consistency with all
//! other output destinations.
//!
//! ## Design Philosophy
//!
//! Unscoped logs remain simple (no headers). Scoped logs use full canonical format.

use std::sync::atomic::{AtomicBool, Ordering};
use crate::core::event_kind::EventKind;

#[cfg(feature = "colour")]
use crate::render::colour::{RESET, GREEN, RED, YELLOW, BLUE};

/// Global flag controlling console output.
static CONSOLE_ENABLED: AtomicBool = AtomicBool::new(false);

/// Global flag controlling color output (runtime control).
static CONSOLE_COLOR: AtomicBool = AtomicBool::new(true);

/// Enable or disable automatic console output for events.
///
/// When enabled, events are printed to the console immediately as they're recorded,
/// in addition to being stored in the journal. Uses canonical format for consistency.
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
/// This only has effect if the `colour` feature is enabled at compile time.
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

/// Print an event to the console with canonical formatting.
///
/// This is called internally by the runtime when console output is enabled.
/// Uses the same format as journal writers and render tree for consistency.
///
/// **Format for unscoped logs** (simple):
/// ```text
/// info message
/// warning: disk space low
/// error: failed to connect
/// debug: packet received
/// ```
///
/// **Format for scoped logs** (canonical):
/// Uses full scope headers and event bullets matching the journal format.
///
/// Color is automatically applied if:
/// 1. The `colour` feature is enabled at compile time, AND
/// 2. Color is enabled at runtime via `enable_console_color(true)`
pub fn print_event(kind: EventKind, message: &str) {
    #[cfg(feature = "colour")]
    {
        if is_console_color_enabled() {
            print_event_colored(kind, message);
            return;
        }
    }
    
    // Plain output (no color) - simple format for unscoped logs
    match kind {
        EventKind::Debug => println!("debug: {}", message),
        EventKind::Info => println!("{}", message),
        EventKind::Warning => println!("warning: {}", message),
        EventKind::Error => eprintln!("error: {}", message),
    }
}

/// Print an event with ANSI color codes using canonical format.
///
/// Internal function - automatically called by `print_event` when color is enabled.
#[cfg(feature = "colour")]
fn print_event_colored(kind: EventKind, message: &str) {
    match kind {
        EventKind::Debug => println!("{}debug:  {} {}", BLUE, RESET, message),
        EventKind::Info  => println!("{}info:   {} {}", GREEN, RESET, message),
        EventKind::Warning => println!("{}warning:{} {}", YELLOW, RESET, message),
        EventKind::Error => eprintln!("{}error: {} {}", RED, RESET, message),
    }
}

/// Print a scoped event with full canonical formatting.
///
/// This is used when events belong to a scope and should match the journal format.
/// Uses bullet points and proper indentation.
///
/// **Format**:
/// ```text
///   • info      message here
///   • warning   disk space low
///   • error     connection failed
/// ```
pub fn print_scoped_event(kind: EventKind, message: &str, indent_level: usize) {
    let bullet = if cfg!(windows) { "*" } else { "•" };
    let indent = " ".repeat(2 * indent_level);
    
    #[cfg(feature = "colour")]
    {
        if is_console_color_enabled() {
            print_scoped_event_colored(kind, message, &indent, bullet);
            return;
        }
    }
    
    // Plain output with canonical alignment
    let kind_label = match kind {
        EventKind::Info => "info     ",
        EventKind::Warning => "warning  ",
        EventKind::Error => "error    ",
        EventKind::Debug => "debug    ",
    };
    
    match kind {
        EventKind::Error => eprintln!("{}{} {} {}", indent, bullet, kind_label, message),
        _ => println!("{}{} {} {}", indent, bullet, kind_label, message),
    }
}

/// Print a scoped event with color using canonical format.
#[cfg(feature = "colour")]
fn print_scoped_event_colored(kind: EventKind, message: &str, indent: &str, bullet: &str) {
    // Match canonical rendering: color the kind label, pad to alignment
    let (colored_label, padding) = match kind {
        EventKind::Warning => (format!("{}warning{}", YELLOW, RESET), "  "),
        EventKind::Error => (format!("{}error{}", RED, RESET), "    "),
        EventKind::Debug => (format!("{}debug{}", BLUE, RESET), "    "),
        EventKind::Info => ("info".to_string(), "     "),
    };
    
    match kind {
        EventKind::Error => eprintln!("{}{} {}{} {}", indent, bullet, colored_label, padding, message),
        _ => println!("{}{} {}{} {}", indent, bullet, colored_label, padding, message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_console_flags() {
        enable_console_output(true);
        assert!(is_console_enabled());
        
        enable_console_output(false);
        assert!(!is_console_enabled());
        
        enable_console_color(true);
        assert!(is_console_color_enabled());
        
        enable_console_color(false);
        assert!(!is_console_color_enabled());
    }
}
