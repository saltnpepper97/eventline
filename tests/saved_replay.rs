use eventline::core::{Outcome, Value};
use eventline::journal::{AsyncWriter, Fields};
use eventline::render::{FileFormat, FileStyle};
use eventline::replay::{self, ReplayIssueKind};
use eventline::{FileWriter, Journal, LogPolicy, RunHeader, WriteMode};
use serde_json::{Value as Json, json};
use std::fs;
use std::io::{self, Cursor, Read};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);
struct TempDir(PathBuf);
impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "eventline-replay-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn jsonl(lines: &[Json]) -> String {
    lines
        .iter()
        .map(Json::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}
fn read(lines: &[Json]) -> replay::SavedJournal {
    replay::read_jsonl(Cursor::new(jsonl(lines))).unwrap()
}
fn scope(id: u64, parent: Option<u64>) -> Json {
    json!({"id":id,"parent":parent,"entered_at":10,"name":format!("scope-{id}"),"exited_at":null})
}
fn metadata(id: u64, parent: Option<u64>, run: &str) -> Json {
    json!({"format_version":1,"run_id":run,"type":"scope","scope":scope(id,parent)})
}
fn event(id: u64, run: Option<&str>, scope: Option<Json>) -> Json {
    let mut value = json!({"id":id,"time_ns":10_000_000,"type":"event","scope_id":scope.as_ref().map(|scope|scope["id"].as_u64().unwrap()),"level":"info","message":format!("event-{id}"),"fields":{}});
    if let Some(run) = run {
        value["format_version"] = json!(1);
        value["run_id"] = json!(run);
    }
    if let Some(scope) = scope {
        value["scope"] = scope;
    }
    value
}
fn snapshot(journal: &Journal) -> String {
    journal.records_jsonl().join("\n")
}
fn style() -> FileStyle {
    FileStyle {
        format: FileFormat::Jsonl,
        ..FileStyle::default()
    }
}

#[test]
fn snapshot_round_trip_restores_fields_hierarchy_outcomes_and_durations() {
    let mut journal = Journal::new();
    let root = journal.enter_scope("request");
    let child = journal.enter_scope("lookup");
    journal.set_scope_exit_messages(
        child,
        eventline::ExitMessages {
            failure: Some("not found".into()),
            ..Default::default()
        },
    );
    let mut fields = Fields::new();
    fields.insert("string", "hello\nworld");
    fields.insert("int", -7i64);
    fields.insert("uint", u64::MAX);
    fields.insert("float", 1.25f64);
    fields.insert("nan", f64::NAN);
    fields.insert("infinity", f64::INFINITY);
    fields.insert("negative_infinity", f64::NEG_INFINITY);
    fields.insert("bool", true);
    fields.insert("duration", std::time::Duration::from_millis(42));
    fields.insert("null", Value::Null);
    journal.record(eventline::EventKind::Warning, "missing", fields);
    journal.exit_scope(child, Outcome::Failure);
    journal.exit_scope(root, Outcome::Success);

    let saved = replay::read_jsonl(Cursor::new(snapshot(&journal))).unwrap();
    assert!(saved.issues.is_empty(), "{:?}", saved.issues);
    assert_eq!(saved.runs.len(), 1);
    let run = &saved.runs[0];
    assert_eq!(run.run_id.as_deref(), Some(journal.run_id()));
    assert_eq!(
        serde_json::to_value(&run.records).unwrap(),
        serde_json::to_value(journal.records()).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&run.scopes).unwrap(),
        serde_json::to_value(journal.scopes()).unwrap()
    );
    assert!(run.unfinished_scopes().is_empty());
    assert!(matches!(run.records[0].get_field("nan"), Some(Value::Float(value)) if value.is_nan()));
    assert!(
        matches!(run.records[0].get_field("infinity"), Some(Value::Float(value)) if *value == f64::INFINITY)
    );
    assert!(
        matches!(run.records[0].get_field("negative_infinity"), Some(Value::Float(value)) if *value == f64::NEG_INFINITY)
    );
    let tree = replay::scope_tree(&run.scopes);
    assert_eq!(tree[0].children[0].scope.id, child);
    assert_eq!(
        replay::failed_scopes(&run.records, &run.scopes)[0].scope.id,
        child
    );
    let outcomes = replay::scope_outcomes(&run.records, &run.scopes);
    assert_eq!(
        outcomes[0].duration_ns as u128,
        journal.scope_elapsed(child).unwrap().as_nanos()
    );
}

#[test]
fn appended_runs_with_reused_ids_remain_separate() {
    let dir = TempDir::new();
    let path = dir.0.join("runs.jsonl");
    let mut ids = Vec::new();
    for name in ["first", "second"] {
        let mut journal = Journal::with_writer(FileWriter::with_style(&path, style()).unwrap());
        ids.push(journal.run_id().to_string());
        let id = journal.enter_scope(name);
        journal.record(eventline::EventKind::Info, name, Fields::new());
        journal.exit_scope(id, Outcome::Success);
        journal.flush().unwrap();
    }
    let saved = replay::load_jsonl(path).unwrap();
    assert!(saved.issues.is_empty(), "{:?}", saved.issues);
    assert_eq!(saved.runs.len(), 2);
    assert_eq!(saved.runs[0].run_id.as_ref(), Some(&ids[0]));
    assert_eq!(saved.runs[1].run_id.as_ref(), Some(&ids[1]));
    assert_eq!(saved.runs[0].records[0].id, saved.runs[1].records[0].id);
    assert_eq!(saved.runs[0].scopes[0].name.as_deref(), Some("first"));
    assert_eq!(saved.runs[1].scopes[0].name.as_deref(), Some("second"));
}

#[test]
fn crash_history_retains_silent_scopes_without_fabricating_outcomes() {
    let dir = TempDir::new();
    let path = dir.0.join("crash.jsonl");
    let mut journal = Journal::with_writer(
        AsyncWriter::try_spawn_with_mode(
            Box::new(FileWriter::with_style(&path, style()).unwrap()),
            8,
            WriteMode::Blocking,
        )
        .unwrap(),
    );
    journal.enter_scope("silent parent");
    journal.enter_scope("silent child");
    journal.flush().unwrap();
    let saved = replay::load_jsonl(&path).unwrap();
    let run = &saved.runs[0];
    assert!(saved.issues.is_empty(), "{:?}", saved.issues);
    assert!(run.records.is_empty());
    assert_eq!(run.unfinished_scopes().len(), 2);
    assert_eq!(
        replay::scope_tree(&run.scopes)[0].children[0]
            .scope
            .name
            .as_deref(),
        Some("silent child")
    );
    assert!(replay::scope_outcomes(&run.records, &run.scopes).is_empty());
}

#[test]
fn saved_snapshot_includes_scopes_that_never_emitted_events() {
    let mut journal = Journal::new();
    journal.enter_scope("open and silent");
    let saved = replay::read_jsonl(Cursor::new(snapshot(&journal))).unwrap();
    assert_eq!(saved.runs[0].unfinished_scopes().len(), 1);
    assert!(saved.runs[0].records.is_empty());
}

#[test]
fn older_unversioned_exports_restore_scopes_without_exit_messages() {
    let open = scope(0, None);
    let mut closed = open.clone();
    closed["exited_at"] = json!(12);
    let lines = [
        event(0, None, Some(open)),
        json!({"id":1,"time_ns":12_000_000,"scope_id":0,"type":"scope_exit","outcome":"failure","duration_ns":123_456,"scope":closed}),
    ];
    let saved = read(&lines);
    assert!(saved.issues.is_empty(), "{:?}", saved.issues);
    let run = &saved.runs[0];
    assert_eq!(run.run_id, None);
    assert_eq!(
        replay::failed_scopes(&run.records, &run.scopes)[0].duration_ns,
        123_456
    );
    assert!(run.unfinished_scopes().is_empty());
}

#[test]
fn legacy_headers_and_repeated_ids_supply_reported_run_boundaries() {
    let line = event(0, None, None).to_string();
    let text = format!(
        "{}\n{line}\n\n{}\n{line}\n{line}\n",
        RunHeader::without_pid("first run").render(),
        RunHeader::new("second run").render()
    );
    let saved = replay::read_jsonl(Cursor::new(text)).unwrap();
    assert_eq!(saved.runs.len(), 3);
    assert!(saved.runs[0].headers[0].contains("first run"));
    assert!(saved.runs[1].headers[0].contains("second run"));
    assert_eq!(saved.issues.len(), 1);
    assert_eq!(saved.issues[0].kind, ReplayIssueKind::InferredRunBoundary);
    assert_eq!(saved.issues[0].line, Some(6));
}

#[test]
fn damaged_lines_and_truncated_utf8_tail_preserve_surviving_records() {
    let first = event(0, Some("run"), None);
    let second = event(1, Some("run"), None);
    let mut text = format!("{first}\nnot json\n{second}\n{{\"type\":").into_bytes();
    text.extend_from_slice(&[0xf0, 0x9f]);
    let saved = replay::read_jsonl(Cursor::new(text)).unwrap();
    assert_eq!(saved.runs[0].records.len(), 2);
    assert_eq!(saved.issues.len(), 2);
    assert_eq!(
        saved
            .issues
            .iter()
            .map(|issue| issue.line.unwrap())
            .collect::<Vec<_>>(),
        [2, 4]
    );
    assert!(
        saved
            .issues
            .iter()
            .all(|issue| issue.kind == ReplayIssueKind::InvalidLine)
    );
}

#[test]
fn valid_final_line_without_newline_and_crlf_are_accepted() {
    let text = format!(
        "\r\n{}\r\n{}",
        event(0, Some("run"), None),
        event(1, Some("run"), None)
    );
    let saved = replay::read_jsonl(Cursor::new(text)).unwrap();
    assert!(saved.issues.is_empty());
    assert_eq!(saved.runs[0].records.len(), 2);
}

#[test]
fn future_versions_are_reported_before_attempting_to_decode_their_payload() {
    let saved = read(&[
        event(0, Some("run"), None),
        json!({"format_version":eventline::JSONL_FORMAT_VERSION+1,"type":"future_entry"}),
        event(1, Some("run"), None),
    ]);
    assert_eq!(saved.runs[0].records.len(), 2);
    assert_eq!(saved.issues.len(), 1);
    assert_eq!(saved.issues[0].kind, ReplayIssueKind::UnsupportedVersion);
    assert_eq!(saved.issues[0].line, Some(2));
}

#[test]
fn invalid_versions_missing_run_ids_and_unknown_levels_are_reported() {
    let mut invalid = Vec::new();
    for version in [json!(null), json!("1"), json!(-1), json!(1.5)] {
        let mut entry = event(0, Some("run"), None);
        entry["format_version"] = version;
        invalid.push(entry);
    }
    let mut missing = event(0, Some("run"), None);
    missing.as_object_mut().unwrap().remove("run_id");
    invalid.push(missing);
    let mut level = event(0, Some("run"), None);
    level["level"] = json!("unknown");
    invalid.push(level);
    let saved = read(&invalid);
    assert!(saved.runs.is_empty());
    assert_eq!(saved.issues.len(), 6);
    assert!(
        saved
            .issues
            .iter()
            .all(|issue| issue.kind == ReplayIssueKind::InvalidLine)
    );
}

#[test]
fn missing_ancestors_and_scope_metadata_have_explicit_placeholders() {
    let mut missing = event(1, Some("run"), None);
    missing["scope_id"] = json!(7);
    let saved = read(&[event(0, Some("run"), Some(scope(2, Some(1)))), missing]);
    let run = &saved.runs[0];
    assert_eq!(run.scopes.len(), 3);
    assert_eq!(run.missing_scope_metadata.len(), 2);
    assert!(
        saved
            .issues
            .iter()
            .all(|issue| issue.kind == ReplayIssueKind::MissingScopeMetadata)
    );
    let tree = replay::scope_tree(&run.scopes);
    let parent = tree.iter().find(|node| !node.children.is_empty()).unwrap();
    assert!(run.missing_scope_metadata.contains(&parent.scope.id));
    assert_eq!(parent.scope.name, None);
    assert_eq!(parent.children[0].scope.name.as_deref(), Some("scope-2"));
}

#[test]
fn cyclic_scope_parents_are_reported_and_the_scopes_remain_visible() {
    let saved = read(&[
        metadata(1, Some(2), "run"),
        metadata(2, Some(3), "run"),
        metadata(3, Some(1), "run"),
        metadata(4, Some(4), "run"),
    ]);
    assert_eq!(
        saved
            .issues
            .iter()
            .filter(|issue| issue.kind == ReplayIssueKind::InvalidHierarchy)
            .count(),
        2
    );
    let run = &saved.runs[0];
    assert_eq!(run.scopes.len(), 4);
    let rendered = replay::render_scope_tree(&run.scopes);
    for id in 1..=4 {
        assert!(rendered.contains(&format!("scope-{id}")));
    }
}

#[test]
fn overlapping_versioned_fragments_are_deduplicated_but_conflicts_are_preserved() {
    let first = event(0, Some("run"), None);
    let mut conflict = first.clone();
    conflict["message"] = json!("conflicting event");
    let saved = read(&[first.clone(), first, conflict]);
    assert_eq!(saved.runs[0].records.len(), 2);
    assert_eq!(saved.issues.len(), 1);
    assert_eq!(saved.issues[0].kind, ReplayIssueKind::ConflictingRecord);
}

#[test]
fn conflicting_scope_metadata_is_reported_without_silently_replacing_identity() {
    let first = metadata(0, None, "run");
    let mut second = first.clone();
    second["scope"]["name"] = json!("changed identity");
    let saved = read(&[first, second]);
    assert_eq!(saved.runs[0].scopes[0].name.as_deref(), Some("scope-0"));
    assert_eq!(saved.issues[0].kind, ReplayIssueKind::ConflictingScope);
}

#[test]
fn older_open_metadata_does_not_reopen_scopes_or_replace_final_exit_messages() {
    let mut closed = metadata(0, None, "run");
    closed["scope"]["exited_at"] = json!(12);
    closed["scope"]["exit_messages"] = json!({"success":"finished","failure":null,"aborted":null});
    let mut open = scope(0, None);
    open["exit_messages"] = json!({"success":"initial message","failure":null,"aborted":null});
    let saved = read(&[closed, event(0, Some("run"), Some(open))]);
    assert!(saved.issues.is_empty());
    assert_eq!(saved.runs[0].scopes[0].exited_at, Some(12));
    assert_eq!(
        saved.runs[0].scopes[0].exit_messages.success.as_deref(),
        Some("finished")
    );
    assert!(replay::scope_outcomes(&saved.runs[0].records, &saved.runs[0].scopes).is_empty());
}

#[test]
fn contradictory_exit_records_are_reported_instead_of_selecting_an_outcome() {
    let exit = |id, outcome| json!({"format_version":1,"run_id":"run","type":"scope_exit","id":id,"time_ns":12_000_000,"scope_id":0,"duration_ns":123,"outcome":outcome});
    let saved = read(&[
        metadata(0, None, "run"),
        exit(0, "success"),
        exit(1, "failure"),
    ]);
    assert_eq!(saved.runs[0].records.len(), 2);
    assert_eq!(saved.issues.len(), 1);
    assert_eq!(saved.issues[0].kind, ReplayIssueKind::ConflictingScope);
    assert!(saved.issues[0].message.contains("contradictory exit"));
}

#[test]
fn out_of_order_legacy_record_ids_do_not_invent_a_restart() {
    let saved = read(&[event(1, None, None), event(0, None, None)]);
    assert!(saved.issues.is_empty());
    assert_eq!(saved.runs.len(), 1);
    assert_eq!(saved.runs[0].records.len(), 2);
}

#[test]
fn nonfinite_float_records_are_deduplicated_without_conflating_different_values() {
    let mut nan = event(0, Some("run"), None);
    nan["fields"] = json!({"value":{"type":"Float","value":"NaN"}});
    let mut infinity = nan.clone();
    infinity["fields"]["value"]["value"] = json!("Infinity");
    let saved = read(&[nan.clone(), nan, infinity]);
    assert_eq!(saved.runs[0].records.len(), 2);
    assert_eq!(saved.issues.len(), 1);
    assert_eq!(saved.issues[0].kind, ReplayIssueKind::ConflictingRecord);
}

#[test]
fn scope_id_mismatches_and_exits_without_scope_ids_are_invalid_entries() {
    let mut mismatch = event(0, Some("run"), Some(scope(0, None)));
    mismatch["scope_id"] = json!(1);
    let saved = read(&[
        mismatch,
        json!({"format_version":1,"run_id":"run","id":0,"time_ns":10,"type":"scope_exit","outcome":"success","duration_ns":10}),
    ]);
    assert!(saved.runs.is_empty());
    assert_eq!(saved.issues.len(), 2);
}

#[test]
fn rotated_files_join_by_run_id_and_keep_file_and_line_diagnostics() {
    let dir = TempDir::new();
    let path = dir.0.join("rotated.jsonl");
    let writer =
        eventline::journal::RotatingFileWriter::with_style(&path, style(), LogPolicy::new(1, 5))
            .unwrap();
    let mut journal = Journal::with_writer(writer);
    let id = journal.enter_scope("parent");
    journal.record(eventline::EventKind::Info, "message", Fields::new());
    journal.exit_scope(id, Outcome::Success);
    journal.flush().unwrap();
    let paths = [
        dir.0.join("rotated.jsonl.2"),
        dir.0.join("rotated.jsonl.1"),
        path.clone(),
    ];
    let saved = replay::load_jsonl_files(&paths).unwrap();
    assert!(saved.issues.is_empty(), "{:?}", saved.issues);
    assert_eq!(saved.runs.len(), 1);
    assert_eq!(saved.runs[0].records.len(), 2);
    assert_eq!(saved.runs[0].scopes.len(), 1);
    fs::write(&path, "broken\n").unwrap();
    let saved = replay::load_jsonl(&path).unwrap();
    assert_eq!(saved.issues[0].source.as_ref(), Some(&path));
    assert_eq!(saved.issues[0].line, Some(1));
}

#[test]
fn missing_files_and_reader_io_errors_are_returned() {
    let dir = TempDir::new();
    assert_eq!(
        replay::load_jsonl(dir.0.join("missing"))
            .unwrap_err()
            .kind(),
        io::ErrorKind::NotFound
    );
    struct BrokenReader;
    impl Read for BrokenReader {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("read failed"))
        }
    }
    impl io::BufRead for BrokenReader {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            Err(io::Error::other("read failed"))
        }
        fn consume(&mut self, _: usize) {}
    }
    assert!(
        replay::read_jsonl(BrokenReader)
            .unwrap_err()
            .to_string()
            .contains("read failed")
    );
}

#[test]
fn empty_and_header_only_files_can_be_inspected() {
    assert!(
        replay::read_jsonl(Cursor::new("\n\r\n"))
            .unwrap()
            .runs
            .is_empty()
    );
    let saved = replay::read_jsonl(Cursor::new(RunHeader::new("empty run").render())).unwrap();
    assert_eq!(saved.runs.len(), 1);
    assert!(saved.runs[0].records.is_empty());
    assert!(saved.runs[0].headers[0].contains("empty run"));
}

#[test]
fn runtime_reconfiguration_keeps_run_identity_and_loading_does_not_mutate_live_history() {
    let dir = TempDir::new();
    eventline::disable_all_output();
    eventline::runtime::clear();
    eventline::set_writer_mode(WriteMode::Blocking).unwrap();
    let outer = eventline::scope_guard("parent opened before output");
    let path = dir.0.join("runtime.jsonl");
    eventline::enable_file_output_jsonl(&path).unwrap();
    let child = eventline::scope_guard("child");
    eventline::info!("before rebuild");
    eventline::enable_console_timestamp(true);
    eventline::info!("after rebuild");
    eventline::flush().unwrap();
    let before = serde_json::to_value(eventline::records()).unwrap();
    let saved = replay::load_jsonl(&path).unwrap();
    assert_eq!(serde_json::to_value(eventline::records()).unwrap(), before);
    assert!(saved.issues.is_empty(), "{:?}", saved.issues);
    assert_eq!(saved.runs.len(), 1);
    let run = &saved.runs[0];
    assert_eq!(run.scopes.len(), 2);
    assert_eq!(run.unfinished_scopes().len(), 2);
    assert_eq!(
        replay::scope_tree(&run.scopes)[0].scope.name.as_deref(),
        Some("parent opened before output")
    );
    child.success();
    outer.success();
    eventline::flush().unwrap();
    let saved = replay::load_jsonl(&path).unwrap();
    assert!(saved.issues.is_empty(), "{:?}", saved.issues);
    assert!(saved.runs[0].unfinished_scopes().is_empty());
    eventline::disable_all_output();
}
