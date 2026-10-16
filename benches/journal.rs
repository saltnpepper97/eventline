//! Dependency-free, end-to-end microbenchmarks. See docs/performance.md.
use eventline::core::{EventKind, Outcome, Record, Scope, ScopeId};
use eventline::journal::Fields;
use eventline::{Journal, replay};
use std::hint::black_box;
use std::sync::Barrier;
use std::time::{Duration, Instant};

fn measure(name: &str, units: usize, mut run: impl FnMut() -> Duration) {
    if let Some(filter) = std::env::args().skip(1).find(|arg| arg != "--bench")
        && !name.contains(&filter)
    {
        return;
    }
    let samples: usize = std::env::var("EVENTLINE_BENCH_SAMPLES")
        .ok()
        .map(|value| {
            value
                .parse()
                .expect("EVENTLINE_BENCH_SAMPLES must be positive")
        })
        .unwrap_or(7);
    assert!(samples > 0);
    black_box(run()); // Warm allocations, code, and scheduler before sampling.
    let mut elapsed: Vec<_> = (0..samples).map(|_| run().as_nanos() as f64).collect();
    elapsed.sort_by(f64::total_cmp);
    println!(
        "{name},{units},{samples},{:.2},{:.2},{:.2}",
        elapsed[samples / 2] / units as f64,
        elapsed[0] / units as f64,
        elapsed[samples - 1] / units as f64
    );
}

fn history(count: usize) -> (Journal, Vec<ScopeId>) {
    let mut journal = Journal::new();
    let mut ids = Vec::with_capacity(count);
    for i in 0..count {
        let parent = (i > 0).then(|| ids[(i - 1) / 8]);
        let id = journal.enter_scope_with_parent("work", parent);
        ids.push(id);
        journal.record_no_write_in_scope(EventKind::Info, "event", Fields::new(), Some(id));
        journal.exit_scope(id, Outcome::Success);
    }
    (journal, ids)
}

fn recording(retained: usize, scoped: bool, indexed: bool) -> Duration {
    let mut journal = Journal::new();
    let active = if scoped {
        for _ in 0..retained {
            let id = journal.enter_scope_with_parent("previous", None);
            journal.exit_scope(id, Outcome::Success);
        }
        Some(journal.enter_scope_with_parent("active", None))
    } else {
        None
    };
    for _ in 0..retained {
        journal.record_no_write_in_scope(EventKind::Info, "warm", Fields::new(), active);
    }
    journal.trim_records(retained);
    if indexed {
        let id = journal.records()[0].id;
        black_box(journal.get_record(id));
    }
    let start = Instant::now();
    for i in 0..50_000 {
        black_box(journal.record_no_write_in_scope(
            EventKind::Info,
            "event",
            Fields::new(),
            active,
        ));
        if i % 128 == 0 {
            journal.trim_records(retained);
            journal.trim_scopes(retained);
        }
    }
    start.elapsed()
}

fn concurrent(threads: usize, retained: usize) -> Duration {
    eventline::runtime::clear();
    eventline::set_journal_retention(retained);
    let ready = Barrier::new(threads + 1);
    let start_gate = Barrier::new(threads + 1);
    let done = Barrier::new(threads + 1);
    std::thread::scope(|workers| {
        for _ in 0..threads {
            workers.spawn(|| {
                let id = eventline::enter_scope("worker");
                ready.wait();
                start_gate.wait();
                for _ in 0..20_000 {
                    eventline::emit(EventKind::Info, "event".into(), Fields::new());
                }
                done.wait();
                eventline::exit_scope(id, Outcome::Success);
            });
        }
        ready.wait();
        let start = Instant::now();
        start_gate.wait();
        done.wait();
        start.elapsed()
    })
}

fn nesting(depth: usize) -> Duration {
    let mut journal = Journal::new();
    let start = Instant::now();
    for _ in 0..40 {
        let mut ids = Vec::with_capacity(depth);
        for _ in 0..depth {
            ids.push(journal.enter_scope("nested"));
            journal.record(EventKind::Info, "inside", Fields::new());
        }
        for id in ids.into_iter().rev() {
            black_box(journal.exit_scope(id, Outcome::Success));
        }
        journal.trim_records(500);
        journal.trim_scopes(500);
    }
    start.elapsed()
}

fn scope_churn(retained: usize) -> Duration {
    let mut journal = Journal::new();
    // A long-lived open scope must not pin all the subsequently closed scopes.
    let _open = journal.enter_scope("long-lived");
    let start = Instant::now();
    for _ in 0..10_000 {
        let id = journal.enter_scope("short-lived");
        black_box(journal.exit_scope(id, Outcome::Success));
        journal.trim_records(retained);
        journal.trim_scopes(retained);
    }
    start.elapsed()
}

fn replay_cases(count: usize) {
    let (journal, ids) = history(count);
    let records: Vec<Record> = journal.records();
    let scopes: Vec<Scope> = journal.scopes();
    measure(&format!("lookup/record/{count}"), 10_000, || {
        let start = Instant::now();
        for i in 0..10_000 {
            black_box(journal.get_record(records[(i * 8191) % records.len()].id));
        }
        start.elapsed()
    });
    measure(&format!("lookup/scope/{count}"), 10_000, || {
        let start = Instant::now();
        for i in 0..10_000 {
            black_box(journal.get_scope(ids[(i * 8191) % ids.len()]));
        }
        start.elapsed()
    });
    measure(&format!("replay/outcomes/{count}"), count, || {
        let start = Instant::now();
        black_box(replay::scope_outcomes(&records, &scopes));
        start.elapsed()
    });
    measure(&format!("replay/tree/{count}"), count, || {
        let start = Instant::now();
        black_box(replay::scope_tree(&scopes));
        start.elapsed()
    });
    measure(&format!("replay/128-scope-queries/{count}"), 128, || {
        let start = Instant::now();
        for id in ids.iter().cycle().take(128) {
            black_box(replay::records_for_scope(&records, *id));
        }
        start.elapsed()
    });
    measure(
        &format!("replay/index-build/{count}"),
        records.len() + scopes.len(),
        || {
            let start = Instant::now();
            black_box(replay::ReplayIndex::new(&records, &scopes));
            start.elapsed()
        },
    );
    let index = replay::ReplayIndex::new(&records, &scopes);
    measure(
        &format!("replay/indexed-128-scope-queries/{count}"),
        128,
        || {
            let start = Instant::now();
            for id in ids.iter().cycle().take(128) {
                for record in index.records_for_scope(*id) {
                    black_box(record);
                }
            }
            start.elapsed()
        },
    );
    measure(
        &format!("replay/build-and-128-scope-queries/{count}"),
        128,
        || {
            let start = Instant::now();
            let index = replay::ReplayIndex::new(&records, &scopes);
            for id in ids.iter().cycle().take(128) {
                for record in index.records_for_scope(*id) {
                    black_box(record);
                }
            }
            black_box(index);
            start.elapsed()
        },
    );
}

fn main() {
    eventline::disable_all_output();
    println!("case,units,samples,median_ns_per_unit,min_ns_per_unit,max_ns_per_unit");
    for retained in [500, 10_000] {
        measure(&format!("record/unscoped/{retained}"), 50_000, || {
            recording(retained, false, false)
        });
        measure(&format!("record/scoped/{retained}"), 50_000, || {
            recording(retained, true, false)
        });
        measure(
            &format!("record/unscoped-indexed/{retained}"),
            50_000,
            || recording(retained, false, true),
        );
        for threads in [1, 4] {
            measure(
                &format!("runtime/{threads}-threads/{retained}"),
                threads * 20_000,
                || concurrent(threads, retained),
            );
        }
        measure(&format!("scope/churn/{retained}"), 10_000, || {
            scope_churn(retained)
        });
    }
    for depth in [16, 256] {
        measure(&format!("scope/nested/{depth}"), 40 * depth, || {
            nesting(depth)
        });
    }
    for count in [500, 10_000] {
        replay_cases(count);
    }
}
