use crate::core::{EventKind, ExitMessages, Outcome, Record, RecordId, RecordKind, Scope, ScopeId};
use crate::journal::Fields;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};

/// Recovered history, grouped by run. Inspect `issues` before treating it as
/// complete: malformed lines and unsupported versions are reported and skipped.
#[derive(Debug, Clone, Default)]
pub struct SavedJournal {
    pub runs: Vec<SavedRun>,
    pub issues: Vec<ReplayIssue>,
}

/// One independent journal. Record and scope IDs are local to this run.
#[derive(Debug, Clone, Default)]
pub struct SavedRun {
    /// Version 1 run identity; `None` for legacy files without run IDs.
    pub run_id: Option<String>,
    /// Decorated run headers encountered in the input, if any.
    pub headers: Vec<String>,
    pub records: Vec<Record>,
    pub scopes: Vec<Scope>,
    /// IDs represented by placeholders because no scope metadata survived.
    /// Their name/parent/entry time are unknown (`None`/`None`/`0`). They are
    /// included so child relationships and observed outcomes remain inspectable.
    pub missing_scope_metadata: Vec<ScopeId>,
}

impl SavedRun {
    /// Scopes with no observed exit record or closed metadata. These remain
    /// unfinished; loading a crash log never fabricates a success or abortion.
    pub fn unfinished_scopes(&self) -> Vec<Scope> {
        let exited: HashSet<_> = self
            .records
            .iter()
            .filter_map(|record| {
                matches!(record.kind, RecordKind::ScopeExit { .. })
                    .then_some(record.scope)
                    .flatten()
            })
            .collect();
        self.scopes
            .iter()
            .filter(|scope| scope.exited_at.is_none() && !exited.contains(&scope.id))
            .cloned()
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayIssueKind {
    InvalidLine,
    UnsupportedVersion,
    InferredRunBoundary,
    ConflictingRecord,
    ConflictingScope,
    MissingScopeMetadata,
    InvalidHierarchy,
}

/// An explicit recovery limitation. Input line numbers are one-based; a missing
/// line number indicates a reconstruction issue found after reading the input.
#[derive(Debug, Clone)]
pub struct ReplayIssue {
    pub source: Option<PathBuf>,
    pub line: Option<usize>,
    pub kind: ReplayIssueKind,
    pub message: String,
}

/// Load a saved JSONL file without changing the live runtime.
///
/// Valid entries survive truncated or malformed lines, with details in
/// [`SavedJournal::issues`]. Unknown format versions are reported and skipped,
/// never interpreted as the current version. File and reader I/O errors return
/// `Err`. Existing unversioned exports are accepted as legacy version 0.
///
/// ```no_run
/// let saved = eventline::replay::load_jsonl("app.jsonl")?;
/// for run in &saved.runs {
///     println!("{}", eventline::replay::render_scope_tree(&run.scopes));
///     let failures = eventline::replay::failed_scopes(&run.records, &run.scopes);
///     println!("{} failures, {} unfinished scopes", failures.len(), run.unfinished_scopes().len());
/// }
/// for issue in &saved.issues { eprintln!("{issue:?}"); }
/// # Ok::<(), std::io::Error>(())
/// ```
pub fn load_jsonl(path: impl AsRef<Path>) -> io::Result<SavedJournal> {
    load_jsonl_files([path])
}

/// Read multiple files in the supplied order (normally oldest backup first).
/// Versioned run IDs join fragments across rotation boundaries. Identical
/// overlapping records are deduplicated within a versioned run. Conflicting
/// records remain available, with an issue describing the conflict.
pub fn load_jsonl_files<I, P>(paths: I) -> io::Result<SavedJournal>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    let mut loader = Loader::default();
    for path in paths {
        let path = path.as_ref();
        loader.read(BufReader::new(File::open(path)?), Some(path.to_path_buf()))?;
    }
    Ok(loader.finish())
}

/// Reconstruct history from any buffered input, with the same recovery behavior
/// as [`load_jsonl`]. The input is consumed line by line, not read as one string.
pub fn read_jsonl(reader: impl BufRead) -> io::Result<SavedJournal> {
    let mut loader = Loader::default();
    loader.read(reader, None)?;
    Ok(loader.finish())
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Level {
    Debug,
    Info,
    #[serde(alias = "warning")]
    Warn,
    Error,
}
impl From<Level> for EventKind {
    fn from(level: Level) -> Self {
        match level {
            Level::Debug => Self::Debug,
            Level::Info => Self::Info,
            Level::Warn => Self::Warning,
            Level::Error => Self::Error,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum ExitOutcome {
    Success,
    Failure,
    Aborted,
}
impl From<ExitOutcome> for Outcome {
    fn from(outcome: ExitOutcome) -> Self {
        match outcome {
            ExitOutcome::Success => Self::Success,
            ExitOutcome::Failure => Self::Failure,
            ExitOutcome::Aborted => Self::Aborted,
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Entry {
    Scope {
        scope: Scope,
    },
    Event {
        id: RecordId,
        time_ns: u64,
        scope_id: Option<ScopeId>,
        scope: Option<Scope>,
        level: Level,
        message: String,
        #[serde(default, deserialize_with = "crate::format::deserialize_fields")]
        fields: Fields,
    },
    ScopeExit {
        id: RecordId,
        time_ns: u64,
        scope_id: ScopeId,
        scope: Option<Scope>,
        outcome: ExitOutcome,
        duration_ns: u64,
    },
}

impl Entry {
    fn into_parts(self) -> Result<(Option<Record>, Option<Scope>), &'static str> {
        let (record, scope) = match self {
            Self::Scope { scope } => return Ok((None, Some(scope))),
            Self::Event {
                id,
                time_ns,
                scope_id,
                scope,
                level,
                message,
                fields,
            } => (
                Record {
                    id,
                    scope: scope_id,
                    time_ns,
                    kind: RecordKind::Event {
                        kind: level.into(),
                        name: message,
                        fields,
                    },
                },
                scope,
            ),
            Self::ScopeExit {
                id,
                time_ns,
                scope_id,
                scope,
                outcome,
                duration_ns,
            } => (
                Record {
                    id,
                    scope: Some(scope_id),
                    time_ns,
                    kind: RecordKind::ScopeExit {
                        outcome: outcome.into(),
                        duration_ns,
                    },
                },
                scope,
            ),
        };
        if let Some(scope) = &scope
            && record.scope != Some(scope.id)
        {
            return Err("embedded scope ID does not match the record's scope_id");
        }
        Ok((Some(record), scope))
    }
}

struct RunBuilder {
    run: SavedRun,
    source: Option<PathBuf>,
    records: HashMap<RecordId, usize>,
    scopes: HashMap<ScopeId, usize>,
    outcomes: HashMap<ScopeId, (Outcome, u64)>,
}

impl RunBuilder {
    fn new(run_id: Option<String>, source: Option<PathBuf>) -> Self {
        Self {
            run: SavedRun {
                run_id,
                ..SavedRun::default()
            },
            source,
            records: HashMap::new(),
            scopes: HashMap::new(),
            outcomes: HashMap::new(),
        }
    }

    fn add_scope(&mut self, incoming: Scope) -> Option<String> {
        let Some(&index) = self.scopes.get(&incoming.id) else {
            self.scopes.insert(incoming.id, self.run.scopes.len());
            self.run.scopes.push(incoming);
            return None;
        };
        let existing = &mut self.run.scopes[index];
        if existing.parent != incoming.parent
            || existing.entered_at != incoming.entered_at
            || existing.name != incoming.name
            || matches!((existing.exited_at, incoming.exited_at), (Some(a), Some(b)) if a != b)
        {
            return Some(format!(
                "conflicting metadata for scope {:?}; retained the first metadata",
                incoming.id
            ));
        }
        let accept_messages = existing.exited_at.is_none() || incoming.exited_at.is_some();
        existing.exited_at = existing.exited_at.or(incoming.exited_at);
        if accept_messages && incoming.exit_messages.success.is_some() {
            existing.exit_messages.success = incoming.exit_messages.success;
        }
        if accept_messages && incoming.exit_messages.failure.is_some() {
            existing.exit_messages.failure = incoming.exit_messages.failure;
        }
        if accept_messages && incoming.exit_messages.aborted.is_some() {
            existing.exit_messages.aborted = incoming.exit_messages.aborted;
        }
        None
    }

    fn add_record(&mut self, record: Record) -> Option<(ReplayIssueKind, String)> {
        let mut conflict = None;
        if let Some(&index) = self.records.get(&record.id) {
            let wire_value = |record: &Record| {
                serde_json::from_str::<serde_json::Value>(&crate::render::render_jsonl_in_run(
                    record,
                    None,
                    "comparison",
                ))
                .expect("loaded record has a valid wire representation")
            };
            if wire_value(&self.run.records[index]) == wire_value(&record) {
                return None;
            }
            conflict = Some((
                ReplayIssueKind::ConflictingRecord,
                format!(
                    "record {:?} has conflicting contents; retained both entries",
                    record.id
                ),
            ));
        } else {
            self.records.insert(record.id, self.run.records.len());
        }
        if let RecordKind::ScopeExit {
            outcome,
            duration_ns,
        } = &record.kind
            && let Some(id) = record.scope
        {
            if let Some(previous) = self.outcomes.get(&id) {
                if previous != &(*outcome, *duration_ns) && conflict.is_none() {
                    conflict = Some((
                        ReplayIssueKind::ConflictingScope,
                        format!(
                            "scope {id:?} has contradictory exit records; retained the observed records without choosing an outcome"
                        ),
                    ));
                }
            } else {
                self.outcomes.insert(id, (*outcome, *duration_ns));
            }
        }
        self.run.records.push(record);
        conflict
    }

    fn finish(mut self) -> (SavedRun, Vec<ReplayIssue>) {
        let mut issues = Vec::new();
        let mut needed: Vec<_> = self
            .run
            .scopes
            .iter()
            .filter_map(|scope| scope.parent)
            .collect();
        needed.extend(self.run.records.iter().filter_map(|record| record.scope));
        for id in needed {
            if self.scopes.contains_key(&id) {
                continue;
            }
            self.scopes.insert(id, self.run.scopes.len());
            self.run.scopes.push(Scope {
                id,
                parent: None,
                entered_at: 0,
                name: None,
                exited_at: None,
                exit_messages: ExitMessages::default(),
            });
            self.run.missing_scope_metadata.push(id);
            issues.push(ReplayIssue { source: self.source.clone(), line: None, kind: ReplayIssueKind::MissingScopeMetadata, message: format!("scope {id:?} has no surviving metadata; inserted an explicitly marked placeholder") });
        }
        // Repair cycles without losing the component. Missing ancestors above
        // are roots, so every retained scope can be inspected with scope_tree.
        let mut done = HashSet::new();
        for start in self
            .run
            .scopes
            .iter()
            .map(|scope| scope.id)
            .collect::<Vec<_>>()
        {
            let mut path = HashSet::new();
            let mut current = Some(start);
            while let Some(id) = current {
                if done.contains(&id) {
                    break;
                }
                let index = self.scopes[&id];
                if !path.insert(id) {
                    self.run.scopes[index].parent = None;
                    issues.push(ReplayIssue {
                        source: self.source.clone(),
                        line: None,
                        kind: ReplayIssueKind::InvalidHierarchy,
                        message: format!("cycle at scope {id:?}; removed its parent link"),
                    });
                    break;
                }
                current = self.run.scopes[index].parent;
            }
            done.extend(path);
        }
        (self.run, issues)
    }
}

#[derive(Default)]
struct Loader {
    runs: Vec<RunBuilder>,
    by_id: HashMap<String, usize>,
    legacy: Option<usize>,
    headers: Vec<String>,
    issues: Vec<ReplayIssue>,
}

impl Loader {
    fn issue(
        &mut self,
        source: &Option<PathBuf>,
        line: usize,
        kind: ReplayIssueKind,
        message: impl Into<String>,
    ) {
        self.issues.push(ReplayIssue {
            source: source.clone(),
            line: Some(line),
            kind,
            message: message.into(),
        });
    }

    fn read(&mut self, mut reader: impl BufRead, source: Option<PathBuf>) -> io::Result<()> {
        let mut bytes = Vec::new();
        let mut line = 0;
        loop {
            bytes.clear();
            if reader.read_until(b'\n', &mut bytes)? == 0 {
                break;
            }
            line += 1;
            let trimmed = bytes.trim_ascii();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.starts_with(b"==")
                && trimmed.ends_with(b"==")
                && let Ok(header) = std::str::from_utf8(trimmed)
            {
                self.headers.push(header.to_string());
                self.legacy = None;
                continue;
            }
            self.read_line(trimmed, &source, line);
        }
        Ok(())
    }

    fn read_line(&mut self, bytes: &[u8], source: &Option<PathBuf>, line: usize) {
        let value: serde_json::Value = match serde_json::from_slice(bytes) {
            Ok(value) => value,
            Err(error) => {
                self.issue(
                    source,
                    line,
                    ReplayIssueKind::InvalidLine,
                    error.to_string(),
                );
                return;
            }
        };
        let version = match value.get("format_version") {
            None => 0,
            Some(version) => match version.as_u64() {
                Some(version) => version,
                None => {
                    self.issue(
                        source,
                        line,
                        ReplayIssueKind::InvalidLine,
                        "format_version must be an unsigned integer",
                    );
                    return;
                }
            },
        };
        if version > u64::from(crate::JSONL_FORMAT_VERSION) {
            self.issue(
                source,
                line,
                ReplayIssueKind::UnsupportedVersion,
                format!("unsupported JSONL format version {version}"),
            );
            return;
        }
        let run_id = if version == 0 {
            None
        } else {
            match value
                .get("run_id")
                .and_then(serde_json::Value::as_str)
                .filter(|id| !id.is_empty())
            {
                Some(id) => Some(id.to_string()),
                None => {
                    self.issue(
                        source,
                        line,
                        ReplayIssueKind::InvalidLine,
                        "versioned entries require a nonempty run_id",
                    );
                    return;
                }
            }
        };
        let entry: Entry = match serde_json::from_value(value) {
            Ok(entry) => entry,
            Err(error) => {
                self.issue(
                    source,
                    line,
                    ReplayIssueKind::InvalidLine,
                    error.to_string(),
                );
                return;
            }
        };
        let (record, scope) = match entry.into_parts() {
            Ok(parts) => parts,
            Err(error) => {
                self.issue(source, line, ReplayIssueKind::InvalidLine, error);
                return;
            }
        };
        let index = if let Some(run_id) = run_id {
            self.legacy = None;
            if let Some(&index) = self.by_id.get(&run_id) {
                index
            } else {
                let index = self.runs.len();
                self.by_id.insert(run_id.clone(), index);
                self.runs
                    .push(RunBuilder::new(Some(run_id), source.clone()));
                index
            }
        } else {
            if let Some(index) = self.legacy
                && let Some(record) = &record
                && self.runs[index].records.contains_key(&record.id)
            {
                self.issue(source, line, ReplayIssueKind::InferredRunBoundary, "legacy record ID repeated; inferred a new run (unversioned boundaries can be ambiguous)");
                self.legacy = None;
            }
            *self.legacy.get_or_insert_with(|| {
                self.runs.push(RunBuilder::new(None, source.clone()));
                self.runs.len() - 1
            })
        };
        self.runs[index].run.headers.append(&mut self.headers);
        if let Some(scope) = scope
            && let Some(message) = self.runs[index].add_scope(scope)
        {
            self.issue(source, line, ReplayIssueKind::ConflictingScope, message);
        }
        if let Some(record) = record
            && let Some((kind, message)) = self.runs[index].add_record(record)
        {
            self.issue(source, line, kind, message);
        }
    }

    fn finish(mut self) -> SavedJournal {
        if !self.headers.is_empty() {
            let mut run = RunBuilder::new(None, None);
            run.run.headers = self.headers;
            self.runs.push(run);
        }
        let mut saved = SavedJournal {
            runs: Vec::new(),
            issues: self.issues,
        };
        for run in self.runs {
            let (run, mut issues) = run.finish();
            saved.runs.push(run);
            saved.issues.append(&mut issues);
        }
        saved
    }
}
