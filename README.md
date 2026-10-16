# eventline

An application flight recorder for Rust: structured execution history, scoped
outcomes, and logging views.

![GitHub last commit](https://img.shields.io/github/last-commit/saltnpepper97/eventline?style=for-the-badge)
![License](https://img.shields.io/badge/License-MIT-E5534B?style=for-the-badge)
![Rust](https://img.shields.io/badge/Rust-1.89%2B-orange?style=for-the-badge&logo=rust&logoColor=white)
![Status](https://img.shields.io/badge/Status-Active-28A745?style=for-the-badge)

Eventline records events and scope outcomes independently of what console and
file sinks display. Keep recent history in memory or save JSONL for inspection
after an application restarts.

- Structured fields and independently configured recording and output levels.
- Synchronous and async scopes with result-aware outcomes and monotonic durations.
- Console, text, and JSONL output, rotation, writer diagnostics, and backpressure.
- Saved-file replay, crash-log recovery, and indexed history queries.
- Optional `log` and `tracing` integrations.

## Quick start

Requires Rust 1.89 or newer.

```toml
[dependencies]
eventline = "0.10.0"
```

```rust
fn main() -> std::io::Result<()> {
    eventline::init_sync();
    eventline::scope_result!("startup", success = "ready", failure = "failed", {
        eventline::info!("loading configuration", source = "defaults");
        Ok::<(), std::io::Error>(())
    })?;
    eventline::flush()
}
```

`scope_result!` records success for `Ok` and failure for `Err`, including an early
`?`. For async work, use `scope_async_result` and wrap each concurrent operation
separately. Ordinary scope guards retain their normal-drop success behavior.

## Documentation

- [User guide](docs/guide.md): scopes, configuration, file output, replay, and integrations.
- [API reference](https://docs.rs/eventline).
- [JSONL format and crash recovery](docs/jsonl-format.md).
- [Performance measurements and benchmarks](docs/performance.md).
- [Changelog and 0.10.0 upgrade notes](CHANGELOG.md).
- [Runnable examples](examples/).

## License

MIT
