use crate::core::{EventKind, Outcome, Record, RecordKind, Scope};

/// Rendering preferences for console output.
#[derive(Debug, Clone, Copy)]
pub struct ConsoleStyle {
    pub color: bool,
    pub show_scope: bool,
    pub show_duration: bool,
    pub show_timestamp: bool,
}

/// Rendering preferences for file output.
#[derive(Debug, Clone, Copy)]
pub struct FileStyle {
    pub show_timestamp: bool,
    pub show_scope: bool,
}

impl Default for ConsoleStyle {
    fn default() -> Self {
        Self {
            color: true,
            show_scope: true,
            show_duration: true,
            show_timestamp: false,
        }
    }
}

impl Default for FileStyle {
    fn default() -> Self {
        Self {
            show_timestamp: true,
            show_scope: true,
        }
    }
}

pub fn render_console(record: &Record, scope: Option<&Scope>, style: ConsoleStyle) -> String {
    let mut out = String::new();

    if style.show_timestamp {
        out.push_str(&format!("[{}] ", format_timestamp_ns_local(record.time_ns)));
    }

    match &record.kind {
        RecordKind::Event { kind, name, .. } => {
            out.push_str(&console_level_prefix(*kind, style.color));
            out.push_str(name);

            if style.show_scope {
                if let Some(s) = scope {
                    out.push(' ');
                    out.push_str(&format!("({})", scope_label(s)));
                }
            }
        }

        RecordKind::ScopeExit { outcome, duration_ns } => {
            out.push_str(&console_done_prefix(*outcome, style.color));
            out.push(' ');

            if let Some(s) = scope {
                out.push_str(&scope_exit_label(s, *outcome));
            } else {
                out.push_str("unknown-scope");
            }

            out.push(' ');
            out.push_str(&format!("[{}]", outcome_str_lower(*outcome)));

            if style.show_duration {
                out.push(' ');
                out.push_str(&format!("({})", format_duration_ns(*duration_ns)));
            }
        }
    }

    out
}

pub fn render_file(record: &Record, scope: Option<&Scope>, style: FileStyle) -> String {
    let mut out = String::new();

    if style.show_timestamp {
        out.push_str(&format!("[{}] ", format_timestamp_ns_local(record.time_ns)));
    }

    match &record.kind {
        RecordKind::Event { kind, name, .. } => {
            out.push_str(level_str_lower(*kind));
            out.push_str(": ");
            out.push_str(name);

            if style.show_scope {
                if let Some(s) = scope {
                    out.push(' ');
                    out.push_str(&format!("({})", scope_label(s)));
                }
            }
        }

        RecordKind::ScopeExit { outcome, duration_ns } => {
            out.push_str("done: ");
            if let Some(s) = scope {
                out.push_str(&scope_exit_label(s, *outcome));
            } else {
                out.push_str("unknown-scope");
            }

            out.push(' ');
            out.push_str(&format!("[{}]", outcome_str_lower(*outcome)));
            out.push(' ');
            out.push_str(&format!("({})", format_duration_ns(*duration_ns)));
        }
    }

    out
}

// ----------------- helpers -----------------

fn scope_label(scope: &Scope) -> String {
    match scope.name.as_deref() {
        Some(name) => format!("{name}#{}", scope.id.0),
        None => format!("#{}", scope.id.0),
    }
}

fn scope_exit_label(scope: &Scope, outcome: Outcome) -> String {
    let mut s = scope_label(scope);

    if let Some(msg) = scope.exit_message_for(outcome) {
        if !msg.is_empty() {
            s.push(' ');
            s.push_str(msg);
        }
    }

    s
}

fn level_str_lower(kind: EventKind) -> &'static str {
    match kind {
        EventKind::Debug => "debug",
        EventKind::Info => "info",
        EventKind::Warning => "warn",
        EventKind::Error => "error",
    }
}

fn outcome_str_lower(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Success => "success",
        Outcome::Failure => "failure",
        Outcome::Aborted => "aborted",
    }
}

fn console_level_prefix(kind: EventKind, color: bool) -> String {
    let label = match kind {
        EventKind::Debug => "debug: ",
        EventKind::Info => "info: ",
        EventKind::Warning => "warn: ",
        EventKind::Error => "error: ",
    };

    if !color {
        return label.to_string();
    }

    let c = match kind {
        EventKind::Debug => "\x1b[90m",
        EventKind::Info => "\x1b[32m",
        EventKind::Warning => "\x1b[33m",
        EventKind::Error => "\x1b[31m",
    };

    format!("{c}{label}\x1b[0m")
}

fn console_done_prefix(_outcome: Outcome, color: bool) -> String {
    if !color {
        return "done:".to_string();
    }

    let c = "\x1b[36m";
    format!("{c}done:\x1b[0m")
}

fn format_duration_ns(ns: u64) -> String {
    if ns < 1_000 {
        format!("{ns}ns")
    } else if ns < 1_000_000 {
        format!("{}us", ns / 1_000)
    } else if ns < 1_000_000_000 {
        let whole = ns / 1_000_000;
        let frac = (ns / 100_000) % 10;
        format!("{whole}.{frac}ms")
    } else {
        let whole = ns / 1_000_000_000;
        let frac = (ns / 100_000_000) % 10;
        format!("{whole}.{frac}s")
    }
}

/// Fast local-time formatter:
/// - On Unix: uses `localtime_r` (no subprocess).
/// - Elsewhere: falls back to UTC.
/// Local timestamp formatter (UTC-only, no libc, no FFI).
fn format_timestamp_ns_local(time_ns: u64) -> String {
    format_timestamp_ns_utc(time_ns)
}

fn format_timestamp_ns_utc(time_ns: u64) -> String {
    let ms_total = time_ns / 1_000_000;
    let secs = (ms_total / 1000) as i64;
    let millis = (ms_total % 1000) as u32;

    let (year, month, day, hour, min, sec) = unix_seconds_to_utc_ymdhms(secs);

    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        year, month, day, hour, min, sec, millis
    )
}

fn unix_seconds_to_utc_ymdhms(secs: i64) -> (i32, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);

    let hour = (sod / 3600) as u32;
    let min = ((sod % 3600) / 60) as u32;
    let sec = (sod % 60) as u32;

    let (year, month, day) = civil_from_days(days);

    (year, month, day, hour, min, sec)
}

fn civil_from_days(days_since_unix: i64) -> (i32, u32, u32) {
    let z = days_since_unix + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }).div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096).div_euclid(365);
    let y = (yoe as i32) + (era as i32) * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2).div_euclid(153);
    let d = doy - (153 * mp + 2).div_euclid(5) + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };

    (year, m as u32, d as u32)
}
