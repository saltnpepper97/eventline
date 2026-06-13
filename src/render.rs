use crate::core::{EventKind, Outcome, Record, RecordKind, Scope, Value};
use crate::journal::Fields;
use serde::Serialize;
use time::{OffsetDateTime, UtcOffset};

/// Rendering preferences for console output.
#[derive(Debug, Clone, Copy)]
pub struct ConsoleStyle {
    pub color: bool,
    pub show_scope: bool,
    pub show_scope_exit: bool,
    pub show_duration: bool,
    pub show_timestamp: bool,
}

/// Rendering preferences for file output.
#[derive(Debug, Clone, Copy)]
pub struct FileStyle {
    pub show_timestamp: bool,
    pub show_scope: bool,
    pub format: FileFormat,
}

/// Output format for file sinks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileFormat {
    Text,
    Jsonl,
}

impl Default for ConsoleStyle {
    fn default() -> Self {
        Self {
            color: true,
            show_scope: true,
            show_scope_exit: true,
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
            format: FileFormat::Text,
        }
    }
}

pub fn render_console(record: &Record, scope: Option<&Scope>, style: ConsoleStyle) -> String {
    let mut out = String::new();

    if style.show_timestamp {
        out.push_str(&format!("[{}] ", format_timestamp_ns_local(record.time_ns)));
    }

    match &record.kind {
        RecordKind::Event { kind, name, fields } => {
            out.push_str(&console_level_prefix(*kind, style.color));
            out.push_str(name);

            append_fields_text(&mut out, fields);

            if style.show_scope
                && let Some(s) = scope
            {
                out.push(' ');
                out.push_str(&format!("({})", scope_label(s)));
            }
        }

        RecordKind::ScopeExit {
            outcome,
            duration_ns,
        } => {
            if !style.show_scope_exit {
                return String::new();
            }
            out.push_str(&console_done_prefix(*outcome, style.color));

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
    if style.format == FileFormat::Jsonl {
        return render_jsonl(record, scope);
    }

    let mut out = String::new();

    if style.show_timestamp {
        out.push_str(&format!("[{}] ", format_timestamp_ns_local(record.time_ns)));
    }

    match &record.kind {
        RecordKind::Event { kind, name, fields } => {
            out.push_str(text_level_prefix(*kind));
            out.push_str(name);

            append_fields_text(&mut out, fields);

            if style.show_scope
                && let Some(s) = scope
            {
                out.push(' ');
                out.push_str(&format!("({})", scope_label(s)));
            }
        }

        RecordKind::ScopeExit {
            outcome,
            duration_ns,
        } => {
            out.push_str(text_done_prefix());
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

pub fn render_jsonl(record: &Record, scope: Option<&Scope>) -> String {
    match serde_json::to_string(&JsonRecord::new(record, scope)) {
        Ok(line) => line,
        Err(e) => format!(
            r#"{{"type":"render_error","error":"{}"}}"#,
            escape_json_error(&e)
        ),
    }
}

// ----------------- helpers -----------------

fn scope_label(scope: &Scope) -> String {
    match scope.name.as_deref() {
        Some(name) => format!("{name}#{}", scope.id.0),
        None => format!("#{}", scope.id.0),
    }
}

fn append_fields_text(out: &mut String, fields: &Fields) {
    if fields.is_empty() {
        return;
    }

    let mut pairs = fields.iter().collect::<Vec<_>>();
    pairs.sort_by(|(a, _), (b, _)| a.cmp(b));

    out.push(' ');
    out.push('{');
    for (idx, (key, value)) in pairs.into_iter().enumerate() {
        if idx > 0 {
            out.push_str(", ");
        }
        out.push_str(key);
        out.push('=');
        out.push_str(&format_value_text(value));
    }
    out.push('}');
}

fn format_value_text(value: &Value) -> String {
    match value {
        Value::String(s) => format!("{:?}", s),
        _ => value.to_string(),
    }
}

fn escape_json_error(e: &serde_json::Error) -> String {
    e.to_string().replace('"', "\\\"")
}

#[derive(Serialize)]
struct JsonRecord<'a> {
    id: u64,
    time_ns: u64,
    scope_id: Option<u64>,
    #[serde(flatten)]
    kind: JsonRecordKind<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scope: Option<JsonScope<'a>>,
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum JsonRecordKind<'a> {
    #[serde(rename = "event")]
    Event {
        level: &'static str,
        message: &'a str,
        fields: &'a Fields,
    },
    #[serde(rename = "scope_exit")]
    ScopeExit {
        outcome: &'static str,
        duration_ns: u64,
    },
}

#[derive(Serialize)]
struct JsonScope<'a> {
    id: u64,
    parent: Option<u64>,
    entered_at: u64,
    exited_at: Option<u64>,
    name: Option<&'a str>,
}

impl<'a> JsonRecord<'a> {
    fn new(record: &'a Record, scope: Option<&'a Scope>) -> Self {
        let kind = match &record.kind {
            RecordKind::Event { kind, name, fields } => JsonRecordKind::Event {
                level: level_str_lower(*kind),
                message: name,
                fields,
            },
            RecordKind::ScopeExit {
                outcome,
                duration_ns,
            } => JsonRecordKind::ScopeExit {
                outcome: outcome_str_lower(*outcome),
                duration_ns: *duration_ns,
            },
        };

        Self {
            id: record.id.0,
            time_ns: record.time_ns,
            scope_id: record.scope.map(|id| id.0),
            kind,
            scope: scope.map(JsonScope::from),
        }
    }
}

impl<'a> From<&'a Scope> for JsonScope<'a> {
    fn from(scope: &'a Scope) -> Self {
        Self {
            id: scope.id.0,
            parent: scope.parent.map(|id| id.0),
            entered_at: scope.entered_at,
            exited_at: scope.exited_at,
            name: scope.name.as_deref(),
        }
    }
}

fn scope_exit_label(scope: &Scope, outcome: Outcome) -> String {
    let mut s = scope_label(scope);

    if let Some(msg) = scope.exit_message_for(outcome)
        && !msg.is_empty()
    {
        s.push(' ');
        s.push_str(msg);
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
    let label = text_level_prefix(kind);

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
        return text_done_prefix().to_string();
    }

    let c = "\x1b[36m";
    format!("{c}{}\x1b[0m", text_done_prefix())
}

fn text_level_prefix(kind: EventKind) -> &'static str {
    match kind {
        EventKind::Debug => "debug: ",
        EventKind::Info => "info:  ",
        EventKind::Warning => "warn:  ",
        EventKind::Error => "error: ",
    }
}

fn text_done_prefix() -> &'static str {
    "done:  "
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

#[cfg(test)]
mod tests {
    use super::{ConsoleStyle, FileFormat, FileStyle, render_console, render_file, render_jsonl};
    use crate::core::{EventKind, Outcome, Record, RecordId, RecordKind, Scope, ScopeId};
    use crate::journal::Fields;

    #[test]
    fn console_can_hide_scope_exit_lines() {
        let record = Record {
            id: RecordId(1),
            scope: Some(ScopeId(2)),
            time_ns: 0,
            kind: RecordKind::ScopeExit {
                outcome: Outcome::Success,
                duration_ns: 1_000,
            },
        };
        let scope = Scope {
            id: ScopeId(2),
            parent: None,
            entered_at: 0,
            name: Some("startup".to_string()),
            exited_at: Some(1),
            exit_messages: Default::default(),
        };

        let rendered = render_console(
            &record,
            Some(&scope),
            ConsoleStyle {
                show_scope_exit: false,
                ..ConsoleStyle::default()
            },
        );

        assert!(rendered.is_empty());
    }

    #[test]
    fn file_render_includes_sorted_fields() {
        let mut fields = Fields::new();
        fields.insert("method", "oauth");
        fields.insert("user_id", 42);

        let record = Record {
            id: RecordId(1),
            scope: None,
            time_ns: 0,
            kind: RecordKind::Event {
                kind: EventKind::Info,
                name: "user login".to_string(),
                fields,
            },
        };

        let rendered = render_file(
            &record,
            None,
            FileStyle {
                show_timestamp: false,
                show_scope: true,
                format: FileFormat::Text,
            },
        );

        assert_eq!(rendered, "info:  user login {method=\"oauth\", user_id=42}");
    }

    #[test]
    fn console_render_uses_padded_colon_prefixes() {
        let info = Record {
            id: RecordId(1),
            scope: None,
            time_ns: 0,
            kind: RecordKind::Event {
                kind: EventKind::Info,
                name: "server listening".to_string(),
                fields: Fields::new(),
            },
        };
        let done = Record {
            id: RecordId(2),
            scope: Some(ScopeId(3)),
            time_ns: 0,
            kind: RecordKind::ScopeExit {
                outcome: Outcome::Success,
                duration_ns: 1_000_000,
            },
        };
        let scope = Scope {
            id: ScopeId(3),
            parent: None,
            entered_at: 0,
            name: Some("startup".to_string()),
            exited_at: Some(1),
            exit_messages: Default::default(),
        };
        let style = ConsoleStyle {
            color: false,
            ..ConsoleStyle::default()
        };

        assert_eq!(
            render_console(&info, None, style),
            "info:  server listening"
        );
        assert_eq!(
            render_console(&done, Some(&scope), style),
            "done:  startup#3 [success] (1.0ms)"
        );
    }

    #[test]
    fn jsonl_render_is_machine_readable() {
        let mut fields = Fields::new();
        fields.insert("user_id", 42);

        let record = Record {
            id: RecordId(7),
            scope: None,
            time_ns: 123,
            kind: RecordKind::Event {
                kind: EventKind::Info,
                name: "user login".to_string(),
                fields,
            },
        };

        let rendered = render_jsonl(&record, None);
        let json: serde_json::Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(json["type"], "event");
        assert_eq!(json["level"], "info");
        assert_eq!(json["message"], "user login");
        assert_eq!(json["fields"]["user_id"]["value"], 42);
    }
}
