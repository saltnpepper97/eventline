# Changelog

## 0.10.0

### Added

- Executor-independent async scope propagation through `ScopeContext`,
  `WithScopeContext`, and `scope_async`. Scope context follows wrapped futures
  across polls and threads; dropped pending scopes record abortion.
- Result-aware scopes through `scope_result`, `scope_result!`,
  `scope_async_result`, and guard `exit_result` methods. `Ok` records success,
  `Err` records failure, and explicit outcomes remain available.
- Writer diagnostics exposing dropped output, failed operations, and the latest
  error. Background write failures are returned by `flush`; optional
  `WriteMode::Blocking` applies backpressure when the output queue is full.
- Saved JSONL replay through `load_jsonl`, `load_jsonl_files`, and `read_jsonl`,
  with run separation, scope reconstruction, and recovery issues for incomplete
  or conflicting data. Unfinished scopes retain unknown outcomes after a crash.
- `Journal::scope_elapsed` and runtime `scope_elapsed` for monotonic live and
  completed scope durations.
- Borrowed `ReplayIndex` for repeated record and scope queries, plus benchmarks
  for recording, concurrent logging, nesting, retention, and replay.

### Fixed

- File initialization and rotation errors are propagated instead of reporting a
  successful setup. Failed replacements preserve the previous output settings.
- Enabling file output preserves explicitly configured logging levels.
- Scope durations use `Instant` with nanosecond precision, independent of
  wall-clock adjustments. Human-readable timestamps retain wall-clock time.
- Indexed scope lookups and deque-backed retention avoid repeated scans and
  record shifting. Closed scopes can be evicted behind long-lived open scopes.
  Record-ID indexing is built lazily to avoid charging recording-only users.
- Replay outcome, tree, and JSONL snapshot construction index scope metadata
  rather than searching it separately for every record or child.

### Upgrade notes

- JSONL format version 1 adds `format_version`, `run_id`, and `type: "scope"`
  metadata entries. Snapshot line counts can exceed record counts. Update custom
  consumers to accept scope metadata entries. Text output is unchanged, and the
  loader accepts older unversioned logs. See the [format reference](docs/jsonl-format.md).
- Scope metadata includes configured exit messages. Nonfinite float fields use
  tagged string values so saved histories can recover them.
- Async context propagation is explicit: wrap each concurrent operation
  separately. It does not automatically propagate through an executor's spawn.
- Ordinary scope guards retain their existing normal-drop success behavior.
  Use a result-aware API when a returned `Err` should record failure.
- `Scope::elapsed` remains a portable wall-clock estimate. Use `scope_elapsed`
  for a live journal or saved exit-record `duration_ns` for completed work.
- Blocking writer mode prevents queue-pressure drops; it does not retry disk
  errors or provide `fsync` durability. Diagnostics remain available until the
  runtime is cleared.
- Dependency versions and the lockfile are refreshed. The minimum supported Rust
  version remains 1.89, and `log`/`tracing` integrations remain optional.
