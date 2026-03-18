use crate::core::{EventKind, Outcome, Record, RecordKind, Scope};
use time::{OffsetDateTime, UtcOffset};

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

/// Format a timestamp using the runner's local time when available.
/// Falls back to UTC if the local offset cannot be determined.
fn format_timestamp_ns_local(time_ns: u64) -> String {
    let dt_utc = match datetime_from_unix_ns(time_ns) {
        Some(dt) => dt,
        None => return format_timestamp_ns_utc(time_ns),
    };

    match UtcOffset::current_local_offset() {
        Ok(offset) => format_offset_datetime(dt_utc.to_offset(offset), true),
        Err(_) => format_offset_datetime(dt_utc, false),
    }
}

fn format_timestamp_ns_utc(time_ns: u64) -> String {
    match datetime_from_unix_ns(time_ns) {
        Some(dt) => format_offset_datetime(dt, false),
        None => "1970-01-01 00:00:00.000".to_string(),
    }
}

fn datetime_from_unix_ns(time_ns: u64) -> Option<OffsetDateTime> {
    let secs = (time_ns / 1_000_000_000) as i64;
    let nanos = (time_ns % 1_000_000_000) as u32;

    let dt = OffsetDateTime::from_unix_timestamp(secs).ok()?;
    dt.replace_nanosecond(nanos).ok()
}

fn format_offset_datetime(dt: OffsetDateTime, include_offset: bool) -> String {
    let year = dt.year();
    let month = dt.month() as u8;
    let day = dt.day();
    let hour = dt.hour();
    let minute = dt.minute();
    let second = dt.second();
    let millis = dt.nanosecond() / 1_000_000;

    if include_offset {
        let offset = dt.offset();
        let total_secs = offset.whole_seconds();
        let sign = if total_secs >= 0 { '+' } else { '-' };
        let abs = total_secs.unsigned_abs();
        let off_h = abs / 3600;
        let off_m = (abs % 3600) / 60;

        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03} {}{:02}{:02}",
            year, month, day, hour, minute, second, millis, sign, off_h, off_m
        )
    } else {
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
            year, month, day, hour, minute, second, millis
        )
    }
}
