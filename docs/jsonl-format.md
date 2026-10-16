# JSONL journal format

`eventline::JSONL_FORMAT_VERSION` is currently `1`. Each nonblank structured line
is one UTF-8 JSON object. The version belongs to every entry so rotated file
fragments can be loaded independently. Unknown fields within a supported version
are ignored; unknown entry types are reported as invalid entries. Unsupported
versions are reported and skipped, never treated as version 1.

Files exported before versioning are accepted as legacy version `0`, identified
by an absent `format_version` field. Explicit version `0` is also accepted.

## Runs and IDs

Every version 1 entry has `format_version: 1` and a nonempty, opaque `run_id`
string. Each new `Journal` gets a distinct run identity. The runtime preserves
its identity through sink reconfiguration, and `Journal::records_jsonl()` uses
the same identity as writers attached to that journal. Record and scope IDs are
unsigned 64-bit integers local to a run and may repeat in a different run.

Standalone file writers have their own run identity. Custom writers can receive
a journal identity through the defaulted `Writer::set_run_id` callback. Custom
rendering of independent streams should use `render::render_jsonl_in_run` with
distinct run IDs; `render_jsonl` uses one default process-local identity.

## Entries

Scope metadata is written on entry, independently of event level filtering.
This preserves silent parent/child scopes when a process stops before an exit.
Enabling or rebuilding a JSONL sink also seeds retained scope metadata. Exports
from `records_jsonl()` include retained scope metadata before the records, so
the number of JSONL lines can exceed the number of records.

```json
{"format_version":1,"run_id":"example-run","type":"scope","scope":{"id":0,"parent":null,"entered_at":1700000000000,"name":"request","exited_at":null,"exit_messages":{"success":"served","failure":null,"aborted":null}}}
```

Events retain their original record ID, timestamp, scope association, level,
message, and typed fields. The embedded `scope` is optional when its metadata is
unavailable; when present, its ID must match `scope_id`.

```json
{"format_version":1,"run_id":"example-run","id":0,"time_ns":1700000000000000000,"scope_id":0,"type":"event","level":"info","message":"handling request","fields":{"request_id":{"type":"Int","value":7}},"scope":{"id":0,"parent":null,"entered_at":1700000000000,"name":"request","exited_at":null,"exit_messages":{"success":"served","failure":null,"aborted":null}}}
```

Event levels are `debug`, `info`, `warn`, and `error`. The reader also accepts
`warning` as an alias. Unscoped events use `scope_id: null` and omit `scope`.

Field values use the existing tagged representation: `String`, `Int`, `Uint`,
`Float`, `Bool`, and `DurationMs` have a `value`; `Null` is `{"type":"Null"}`.
Finite floats use JSON numbers. Nonfinite floats use the strings `NaN`,
`Infinity`, and `-Infinity` as their `Float` value so they can be recovered
without loss of meaning. Durations in fields remain milliseconds.

```json
{"format_version":1,"run_id":"example-run","id":1,"time_ns":1700000000001234567,"scope_id":0,"type":"scope_exit","outcome":"success","duration_ns":1234567,"scope":{"id":0,"parent":null,"entered_at":1700000000000,"name":"request","exited_at":1700000000001,"exit_messages":{"success":"served","failure":null,"aborted":null}}}
```

Exit outcomes are `success`, `failure`, and `aborted`. `duration_ns` is the
recorded monotonic duration and is authoritative for completed work; it is not
recalculated from timestamps. Record `time_ns` is Unix-epoch nanoseconds. Scope
`entered_at` and `exited_at` are Unix-epoch milliseconds. `exit_messages` defaults
to empty when reading older scope metadata that omitted it.

## Loading and crash recovery

`replay::load_jsonl(path)` loads one file. `load_jsonl_files(paths)` reads files
in the supplied order, normally oldest rotated backup first. `read_jsonl(reader)`
accepts any `BufRead`. None of these modifies the live journal, configuration,
or scope stack.

The result contains `runs`, each with ordinary `records` and `scopes` compatible
with the existing replay helpers. Runs appear in order of first observation;
records retain input order rather than being sorted by wall-clock time or ID.
Matching versioned run IDs join fragments across files. Identical overlapping
records are deduplicated. Conflicting record contents or scope outcomes are
retained as observed records and reported; conflicting scope identity metadata
keeps the first metadata and reports the conflict.

Blank lines and decorated `RunHeader` lines are accepted. A valid final JSON
object needs no trailing newline. Malformed, truncated, invalid-UTF-8, and
unsupported entries are skipped individually. Inspect `SavedJournal::issues`
for the source path, one-based line number, issue kind, and explanation.
Reader and filesystem I/O failures return `io::Error`.

`SavedRun::unfinished_scopes()` identifies scopes with neither an observed exit
record nor closed metadata. Missing exits never become fabricated success,
failure, or abortion outcomes. A closed metadata snapshot alone does not prove
an outcome: only an exit record supplies one. Instant clocks cannot be restored
across restarts; saved exit durations are retained verbatim.

When a referenced scope or ancestor has no surviving metadata, the loader adds
an explicit placeholder so its children and observed outcomes remain usable.
These IDs appear in `missing_scope_metadata` and in recovery issues. Placeholder
entry time `0`, missing name, and missing parent mean unknown metadata. Cyclic
parent links are reported and one link is removed per cycle to keep every scope
visible in hierarchy inspection.

Legacy files cannot prove every restart boundary. Decorated headers start a new
legacy run; a repeated record ID also starts one and reports that the boundary
was inferred. Out-of-order IDs alone do not imply a restart. New versioned logs
avoid this ambiguity with explicit run identities.

Recovery is limited to entries that reached disk. Queue-pressure drops,
filtered events, trimmed snapshots, missing rotation files, partial lines, and
writer failures can leave gaps that no replay loader can reconstruct. JSONL
scope metadata shares the bounded output queue, and dropped-output counters
include these metadata entries. Text sinks and ordinary custom writers retain
their existing output behavior; custom metadata-aware writers opt in through
`wants_scope_metadata` and `write_scope`.
