# Journal indexing and benchmarks

Run the dependency-free harness in an optimized build:

```sh
cargo bench --locked --bench journal > results.csv
EVENTLINE_BENCH_SAMPLES=15 cargo bench --locked --bench journal -- replay/
```

The optional argument filters case names by substring. Cargo diagnostics go to
stderr; stdout is CSV. Each case has one untimed warm-up invocation and seven
samples by default. CSV reports median, minimum, and maximum nanoseconds per
unit. `EVENTLINE_BENCH_SAMPLES` must be a positive integer. There are no timing
assertions in the test suite.

## Storage and query costs

Scope IDs use a hash index into ordered deque slots. Records also use a deque,
so removing old records does not shift the retained history. Their hash index
is lazy: the first record-ID lookup takes O(R) to build it, and later lookups
are expected O(1). Once built, the index is maintained on insertion/eviction.
Recording-only workloads avoid that index's allocation and hashing cost.

Closed scope positions are tracked in entry order, so a trim visits the scopes
being evicted rather than scanning all scopes to count closed entries. Open
scopes remain available for later exits and updates. Trimming behind an open
scope leaves vacant slots; compaction runs when more than half the slots are
vacant and there are more than 64 slots. This bounds slot growth even when a
very old scope stays open. Compaction rebuilds the indexes, amortizing movement
across evictions. Closed-position maintenance costs O(log S) per entry/exit or
eviction, and normal ID lookups are expected O(1).

Snapshots and positional access preserve insertion order. Positional
`Buffer::get_scope(index)` may scan vacant slots; prefer ID queries for repeated
access. Public `Buffer` users can still insert nonmonotonic or duplicate IDs:
lookups use the first retained occurrence. Removing that occurrence may scan
for the next duplicate. Journal-generated IDs are unique and avoid this path.
Clearing and trimming remove associated index entries and monotonic clocks.

One-off outcome reconstruction and scope-tree construction now take O(R + S)
and O(S), respectively, for unique IDs. Roots and siblings retain input order.
Traversal uses an explicit stack, including direct text-tree rendering. The
public `ScopeNode` result remains a recursive data structure. JSONL snapshot
export also indexes scope metadata once rather than searching it per record.
Snapshot shapes, file formats, retention caps, and clock behavior are unchanged.

`replay::ReplayIndex` borrows one immutable snapshot or saved run, building
scope-ID, record-ID, and scope-record indexes in O(R + S). Point queries are
expected O(1); a scope-record query visits only its matching records and returns
borrowed references. It preserves duplicate/conflicting records in input order,
with the first occurrence used for ID lookups. It requires additional memory
and build time, so a one-off linear filter can be cheaper. Input borrowing
prevents a stale index caused by mutating its source. Runs with independent ID
spaces need independent indexes.

## Workloads

All logging benchmarks disable console and file output. They measure in-memory
recording, timestamping, synchronization, and scope lookup, not disk throughput,
durability, or asynchronous executor throughput. Names and empty field sets are
fixed to isolate journal costs; real applications may spend more time preparing
messages and fields.

| Case | Work per sample | Unit |
| --- | --- | --- |
| `record/unscoped/N` | 50,000 direct-journal events, prefilled N records, trims every 128 events | event |
| `record/scoped/N` | Same, with N closed scopes before one active scope | event |
| `record/unscoped-indexed/N` | Same as unscoped, after querying an ID to activate the record index | event |
| `runtime/T-threads/N` | T synchronized workers emit 20,000 events each, one active scope each, runtime retention N | event across all workers |
| `scope/churn/N` | 10,000 enter/exit pairs, trim after each pair, one older scope stays open | pair |
| `scope/nested/D` | 40 batches of D nested scopes, one event each, exit in reverse order, retention 500 | scope/event/exit group |
| `lookup/record/N`, `lookup/scope/N` | 10,000 scrambled ID queries over N scopes and 2N records | lookup |
| `replay/outcomes/N` | All N exits in a snapshot with N scopes and 2N records | scope |
| `replay/tree/N` | Eight-way tree containing N scopes | scope |
| `replay/128-scope-queries/N` | 128 existing one-off filters, cloning their matches | query |
| `replay/index-build/N` | Build and destroy a borrowed index | input record or scope |
| `replay/indexed-128-scope-queries/N` | Visit matching records for 128 scopes with an existing index | query |
| `replay/build-and-128-scope-queries/N` | Build, query 128 times, and destroy an index | query including shared build cost |

N is 500 or 10,000; T is 1 or 4; D is 16 or 256. Fixtures are constructed
outside timed sections except for the explicitly named churn, nesting, and
index-build cases. Query results are consumed with `std::hint::black_box`.
Runtime workers start outside timing and synchronize their timed loops with
barriers. Retention remains periodic, as in the existing runtime, so its record
count can briefly exceed the configured cap between trim passes.

## Local comparison

Measurements from this change are included in
[indexing-before.csv](benchmarks/indexing-before.csv) and
[indexing-after.csv](benchmarks/indexing-after.csv). The baseline is commit `b2a675e`, before
indexing changes, with the same compatible benchmark workloads copied into its
checkout. Only the new `ReplayIndex` cases are absent from that baseline. Both
use the locked dependencies, default features, Cargo's optimized bench profile,
Rust 1.97.1, and an AMD Ryzen 9 5900X Linux workstation on 2026-10-15.

Representative medians (lower is better):

| Workload | Before | After |
| --- | ---: | ---: |
| Unscoped recording, 500 retained | 87 ns/event | 90 ns/event |
| Scoped recording, 500 retained scopes | 357 ns/event | 125 ns/event |
| Scoped recording, 10,000 retained scopes | 8.54 us/event | 135 ns/event |
| Scope churn, retention 500 | 2.47 us/pair | 424 ns/pair |
| Scope churn, retention 10,000 | 6.06 us/pair | 332 ns/pair |
| Nested scopes, depth 256 | 874 ns/group | 462 ns/group |
| Record-ID lookup, 20,000 records | 8.04 us/query | 55 ns/query |
| Scope-ID lookup, 10,000 scopes | 3.19 us/query | 82 ns/query |
| Reconstruct 10,000 outcomes | 36.7 ms | 734 us |
| Build a 10,000-scope tree | 99.0 ms | 1.27 ms |

The tradeoff is visible after activating the record index: unscoped recording
with retention 500 rose from 84 to 128 ns/event. The lazy index avoids that
maintenance cost until ID queries are used. At retention 10,000, cheaper deque
eviction offset it in this measurement (169 to 144 ns/event).

For 10,000 scopes and 20,000 records, 128 one-off scope queries took 2.24 ms.
Building a `ReplayIndex` and making the same 128 queries took 1.22 ms including
destruction. With an existing index, those queries took about 1.78 us in total;
they borrow matches instead of cloning them. Building and destroying the index
alone took about 1.21 ms, so building one for only a few queries may cost more
than scanning.

The runtime cases include locks, timestamps, and background-writer interaction,
which dominate these small active-scope sets. At retention 500, the single-worker
median rose from 1.09 to 1.26 us/event while the four-worker median fell from
1.01 us to 876 ns/event. Their sample ranges overlap substantially. These
results do not establish a consistent improvement in concurrent logging; the
clear gains here are in large retained histories, scope churn, and replay.

These are local microbenchmarks, not release guarantees or statistical
confidence intervals. CPU frequency, scheduling, and other host activity affect
results, especially concurrent cases. Compare identical case names and units;
the CSV keeps the full range rather than presenting only favorable samples.
