# Sub-project 4: performance report

This report measures where Tayga spends CPU: Drain log mining (micro-benchmarks over a log corpus), the ingest, writer and assembler conversion stages (micro-benchmarks over `fixtures/healthy.pb.gz`), the live stack's load, and ClickHouse's query cost. Micro-benchmarks ran natively on macOS (release build, criterion). Live numbers come from the running stack.

Hardware and software:

| Item | Value | Source |
|---|---|---|
| Host | Apple M3 Max, 14 cores, 96 GiB | `sysctl machdep.cpu.brand_string hw.ncpu hw.memsize` |
| Docker Desktop VM | 14 CPUs, 31.5 GiB | `docker info` |
| Rust | 1.98.1 | `rustc --version` |
| ClickHouse | 26.8.15.10 | `SELECT version()` |
| GPU | Apple M3 Max, wgpu backend Metal, integrated, compute shaders yes, `SHADER_INT64` yes | wgpu 30.0.1 `Adapter::get_info`, `features`, `get_downlevel_capabilities` |

Corpus: `fixtures/log_corpus.jsonl.gz`, the oldest 50,000 rows of the last hour (18:50:23–19:13:01 UTC, 22 min), from 17 services. Exported read-only with:

```sh
curl -s 'http://localhost:18123/?database=tayga' --data-binary "SELECT service_name AS service, toUnixTimestamp64Nano(l.ts) AS ts_ns, severity_number AS sev, body FROM logs AS l WHERE l.ts > now() - INTERVAL 1 HOUR ORDER BY l.ts LIMIT 50000 FORMAT JSONEachRow" | gzip -9 > fixtures/log_corpus.jsonl.gz
```

The export was scanned for `password|passwd|authorization|bearer |secret|api[_-]?key|token=` (case-insensitive): 0 matches.

## Live load

The hardware table above and the Live load and ClickHouse sections are copied from spec §2.1, §2.6 and §2.7 (`docs/superpowers/specs/2026-10-06-tayga-sp4-performance-design.md`), observed on 2026-10-06, and cannot be reproduced from the benches; the spec has the queries and commands.

Sample of 2026-10-06, between 18:30 and 19:00 UTC, 60 s.

| Measure | Value | Source |
|---|---|---|
| Logs mined | 2,456 in 61 s = **40 lines/s** | `tayga_logminer_logs_mined_total` delta |
| `tayga.logs` records | 473 in 61 s, so **5.2 logs per record**: the miner's real batch | `tayga_ingest_log_records_published_total{topic="tayga.logs"}` delta |
| CPU (average of 30 `docker stats` samples, % of one core) | ClickHouse 21.1, ingest 1.81, assembler 1.55, **logminer 1.07**, writer 0.75, api 0.21 | `docker stats --no-stream` |
| Drain's share of the logminer | 40 lines/s × 2.818 µs per line (the `drain_add` row of the Drain breakdown below) = 0.113 ms/s, about 0.011 % of a core and **about 1 % of the logminer's own CPU** (0.011 / 1.07) | live rate × bench µs per line |

The logminer's CPU is fixed overhead: Kafka polling, the 200 ms `recv` timeout loop and the 60 s detection pass. ClickHouse is the largest consumer.

## ClickHouse

Last hour, `system.query_log`, SELECT, ordered by CPU.

| Rank | Query (code) | Runs/h | Avg ms | CPU s/h | Read per run |
|---|---|---|---|---|---|
| 1 | service map nodes (`repo.rs` `service_graph`, 24 h scan of `spans`) | 1,006 | 449 | **558** | 10.0 M rows, 162 MiB |
| 2 | trace search (`TRACE_SEARCH`, `trace_summaries FINAL`, 24 h) | 172 | 521 | 147 | 4.3 M rows, 226 MiB |
| 3 | `op_stats` (7a baseline caps) | 60 | 354 | 97 | 8.6 M rows, 1.22 GiB |
| 4 | `endpoint_stats` (7a baseline caps) | 60 | 300 | 70 | 8.6 M rows, 468 MiB |
| 5 | `search` services (`SELECT DISTINCT service_name FROM spans`, 24 h) | 171 | 429 | 47 | 10.5 M rows |

The top 15 query shapes used about 1,070 CPU s/h; ClickHouse averaged 21 % of a core. The service map runs every 10 s per open UI tab: 1,006 runs/h is about 3 tabs.

Isolated re-runs on the same data (read-only, `use_query_cache=0`, best of 3, CPU from `ProfileEvents['OSCPUVirtualTimeMicroseconds']`):

| Variant | ms | CPU ms | Rows read | Bytes read |
|---|---|---|---|---|
| `op_stats` as deployed (`indexOf`) | 154 | 1,511 | 8.55 M | 1.21 GiB |
| `op_stats` with `transform` instead of `indexOf` | 153 | 1,382 | 8.55 M | 1.21 GiB |
| `trace_summaries FINAL WHERE ts > now() - 60 min` (count only) | 95 | 770 | 4.27 M | 964 MiB |
| same without `FINAL` | 16 | 17 | 0.17 M | 27 MiB |
| `endpoint_stats` as deployed | 114 | 999 | 8.79 M | 478 MiB |
| `endpoint_stats` with `argMax(…, span_count) GROUP BY trace_id` over the ts-filtered rows, without `op_durations` (§3.8) | 70 | 265 | 0.39 M | 21 MiB |
| same, carrying `op_durations` (the `op_stats` shape) | 141 | 538 | 0.38 M | 91 MiB |
| service map as deployed (15 min window + 24 h baseline) | 31 | 269 | 9.81 M | 163 MiB |
| service map, window part only | 8 | 12 | 0.19 M | 3.3 MiB |

- **`indexOf` is not the 7a M2 cost.** `transform` changes nothing. The cost is `FINAL` on `trace_summaries`. That table is `ORDER BY trace_id`, so under `FINAL` the `ts` filter prunes nothing and every column is read for all 3.6 M rows.
- **Equivalence of the rewrite.** The `argMax` rewrite of `endpoint_stats` returned the same 61 endpoints with identical `seen`, `kept` and `excluded` counts, run back to back with the deployed query.
- **Quantile differences are noise.** Quantiles differed in 2 rows. Two runs of the deployed query differed in 14 rows: `quantile` is sampling-based.

## Benchmarks

Baseline, criterion, release build, one thread. Time and throughput are criterion's point estimate: the middle value of the confidence interval, not the median. The `convert`, `flatten` and `assemble` element counts are derived (time × throughput), not printed by criterion.

| Bench | Elements | Time (estimate) | Throughput (estimate) |
|---|---|---|---|
| `stages/split` | 50,000 lines | 26.972 ms | 1.8538 Melem/s |
| `stages/tokens` | 50,000 lines | 122.88 ms | 406.89 Kelem/s |
| `stages/drain_add` | 50,000 lines | 140.89 ms | 354.90 Kelem/s |
| `convert/trace_records` | about 3,267 spans | 6.2875 ms | 519.60 Kelem/s |
| `flatten/rows_from_envelope` | about 2,217 envelopes | 8.0848 ms | 274.22 Kelem/s |
| `assemble/ingest_close_process` | about 2,217 envelopes | 4.4374 ms | 499.62 Kelem/s |

Reproduce:

```sh
cargo bench -p tayga-drain --bench mining
cargo bench -p tayga-ingest --bench convert
cargo bench -p tayga-store --bench flatten
cargo bench -p tayga-assembler --bench close
```

Optional flamegraph (needs `sudo` on macOS): `cargo flamegraph --bench mining -p tayga-drain -- --bench stages`

## Drain breakdown

From `stages` (50,000 lines per iteration).

| Part | µs per line | Share of `drain_add` |
|---|---|---|
| `drain_add` (whole) | 2.818 | 100 % |
| `tokens` (split, mask, normalise) | 2.458 | 87.2 % |
| of which split and `String` allocation only | 0.539 | 19.1 % |
| masking (`tokens` − `split`) | 1.919 | 68.1 % |
| tree (`drain_add` − `tokens`) | 0.360 | 12.8 % |

`tokens − split` also includes the per-token allocation difference between the two benches.

Spike (spec §2.3): masking 70 %, tree 11 %. Masking dominates, as in the spike.

## Fingerprint backends

| Batch | scalar | parallel | gpu | parallel / scalar | gpu / scalar | gpu time / parallel time |
|---|---|---|---|---|---|---|
| 5 | 2.44 µs (2.05 Melem/s) | 2.44 µs (2.05 Melem/s) | 2.43 µs (2.05 Melem/s), CPU path | 1.00× | 1.00× | 1.00 (both on the calling thread) |
| 64 | 21.0 µs (3.05 Melem/s) | 20.9 µs (3.06 Melem/s) | 21.1 µs (3.03 Melem/s), CPU path | 1.00× | 0.99× | 1.01 (both on the calling thread) |
| 512 | 184 µs (2.79 Melem/s) | 90.8 µs (5.64 Melem/s) | 182 µs (2.81 Melem/s), CPU path | 2.02× | 1.01× | 2.00: **gpu slower** |
| 2,048 | 694 µs (2.95 Melem/s) | 144 µs (14.19 Melem/s) | 725 µs (2.82 Melem/s) | 4.80× | **0.96×** | 5.02: **gpu slower** |
| 5,000 | 1.69 ms (2.96 Melem/s) | 252 µs (19.84 Melem/s) | 924 µs (5.41 Melem/s) | 6.71× | 1.83× | 3.67: **gpu slower** |
| 50,000 | 16.6 ms (3.00 Melem/s) | 1.95 ms (25.66 Melem/s) | 3.10 ms (16.14 Melem/s) | 8.54× | 5.37× | 1.59: **gpu slower** |

Criterion's point estimate of `cargo bench -p tayga-drain --bench mining --features gpu -- fingerprint`, all three backends from one run on 2026-10-07 (this run replaces the earlier scalar/parallel-only table; its scalar times were within 3 % of it). `gpu` is `GpuFingerprinter` on the M3 Max through Metal (wgpu 30.0.1).

- **`parallel` uses rayon's global pool**, which by default has one thread per logical CPU: 14 on this host (`hw.ncpu` 14).
- **The spike's parallel numbers were not reproduced.** The spike (spec §2.5) measured 23.4 M elem/s at 5,000 bodies and 36.7 M at 50,000, and the Task 4 brief estimated parallel above 10× scalar from 5,000 bodies. Measured: 19.44 M (6.52×) at 5,000 and 28.76 M (9.60×) at 50,000 in the Task 4 run (`cb51bf4`, scalar and parallel only), and 19.84 M (6.71×) and 25.66 M (8.54×) in the table above. The ratio is the comparable figure: the Task 4 run's scalar was about 21 % slower than the first scalar-only run, so its absolute times are not.
- **Below `GPU_MIN_BATCH` (2,048) the GPU backend runs scalar**, so 5, 64 and 512 equal scalar. Batches under 512 run on the calling thread for `parallel` too (`PAR_MIN_BATCH`).
- **The GPU is slower than `parallel` at every size**: 5.0× slower at 2,048, 3.7× at 5,000, 1.6× at 50,000.
- **Against one thread** the GPU is 4 % slower at 2,048 (its first GPU size), 1.83× faster at 5,000 and 5.37× faster at 50,000. The spike (spec §2.5) measured 5.6 and 15.6 M elem/s at 5,000 and 50,000; this run measured 5.41 and 16.14.
- **Fixed dispatch cost: 279 µs** (criterion point estimate; interval 276–284 µs), from `fingerprint_gpu_dispatch`: 2,048 empty bodies, so the kernel does almost nothing and buffer creation, upload, dispatch and readback dominate. That is 40 % of the 725 µs at 2,048. A line through the 5,000 and 50,000 points has an intercept of about 680 µs, so the per-body cost is not linear at small batches; the 279 µs is the direct measurement.

Correctness: `parallel` and `gpu` equal `ScalarFingerprinter` on the corpus (50,000 bodies), on NUL, non-ASCII and empty edge cases either side of their thresholds (for `gpu` also 2,048 + 1, + 63, + 64, + 65, which end inside, at and past a 64-invocation workgroup), and on random batches (`tests/backends.rs`; the GPU proptest has 32 cases of up to 3,000 bodies). `gpu::tests::the_kernel_equals_fingerprint_body_on_small_batches` runs the kernel itself, without the CPU threshold, on batches of 1, 2, 63, 64, 65 and 129 bodies; it fails when the kernel's NUL rule is removed (checked by mutation, then reverted). Every ASCII body equals `reference_fingerprint` (`tests/fingerprint.rs`); NUL and non-ASCII bodies take the Drain path.

**`mine_batch` blocks the calling thread.** The logminer calls `Miner::mine_batch` synchronously inside its consume loop (`on_message` in `crates/tayga-logminer/src/main.rs`), on a tokio worker thread. `parallel` waits there for rayon's pool to finish the batch; `gpu` waits for the readback, at most `POLL_TIMEOUT` (5 s) before the batch falls back to scalar. At the live batch of about 5 bodies both run on the calling thread anyway, so this matters only for the opt-in backends at large batches.

**The GPU backend is opt-in.** It is compiled only with `--features gpu` (`tayga-drain`, forwarded by `tayga-logminer`), the Docker images build default features and do not contain it, and it is slower than `parallel` at every measured size. Two of its fallbacks are untested: the 5 s poll timeout (a hang cannot be forced) and the no-adapter path of `GpuFingerprinter::new` (this Mac always has an adapter). `TAYGA_REQUIRE_GPU=1` makes the GPU tests fail instead of skipping when there is no adapter.

Reproduce (GPU rows need a build with the feature and an adapter): `cargo bench -p tayga-drain --bench mining --features gpu -- fingerprint` and `TAYGA_REQUIRE_GPU=1 cargo test -p tayga-drain --features gpu` (without `TAYGA_REQUIRE_GPU=1` the GPU tests skip when no adapter exists).

## Cache

The corpus through `Drain::add` and through `Drain::add_fingerprinted` with `fingerprint_body`. Each iteration starts with a fresh Drain, so the first sighting of every sequence is a miss. Times are criterion's point estimate, as in Benchmarks.

| Bench | Elements | Time (estimate) | Throughput (estimate) |
|---|---|---|---|
| `cached/add` | 50,000 lines | 144.29 ms | 346.52 Kelem/s |
| `cached/add_fingerprinted` | 50,000 lines | 20.678 ms | 2.4180 Melem/s |

The cache makes mining **6.98×** faster (144.29 / 20.678). The spike measured 8.4× with the prototype crate on its main corpus (spec §2.5: 2.94 M against 352 k lines/s).

**Hit rate.** `tests/differential.rs` asserts at least 95 % hits on the corpus under each of five Drain configurations. Measured: 49,716 of 50,000 lines (99.43 %) for the default, `max_clusters_per_service = 20` and `max_children = 2`; 49,715 (99.43 %) for `sim_threshold = 0.75`; 49,746 (99.49 %) for `keep_http_status = false`.

**The differential test fails when invalidation is missing.** Mutations, each run with `cargo test -p tayga-drain` and then reverted:

| Mutation | Failed test |
|---|---|
| No cache reset on generalisation (`if false && generalised …`) | `the_cache_assigns_exactly_like_drain_on_random_logs`, a `(Hit)` line with another template. Minimal case: six lines of `svc0`, `max_children = 1`, `cap = 2`, `keep = false`. Also `generalisation_empties_only_that_services_cache` |
| A key match counts as a hit whatever its `check` | `colliding_keys_are_caught_by_the_check` |
| `restore` keeps the service's cache | `restore_empties_the_service_cache` (unit test). The differential test does not catch this one: it restores before any line is cached |

Reproduce: `cargo bench -p tayga-drain --bench mining -- cached` and `cargo test -p tayga-drain --test differential`.

### The cache live (Task 7, 2026-10-07)

Deploy at 04:30:52 UTC with the default `scalar` backend; readings of the logminer's `/metrics`:

| Measure | Value |
|---|---|
| Lines mined in the first 30 minutes (04:30:50 to 05:01:14) | 69,569: 69,359 cache hits, 210 misses, so **99.70 % hits** |
| Collisions, cache resets | 0; 0 `generalised`, 0 `full` |
| Templates created | 0 (control window before the deploy, 10 min 38 s: also 0) |
| Records mined | 13,470, so 5.16 logs per record |
| Mean `mine_batch` time, `scalar` | 0.10756 s / 13,470 = **8.0 µs per record** (Pipeline chart p50 8.6–9.9 µs, p99 57–61 µs) |
| Mean `mine_batch` time, `off` (kill switch, 05:37:24 to about 05:40:05) | 0.043595 s / 1,251 = **34.8 µs per record** |

The live gain, 34.8 against 8.0 µs per record (4.4×), is a mean over two different short windows, not a controlled benchmark. Either way mining stays a negligible share of a core at about 40 lines/s.

Real-data differential: the last hour of `logs` exported at 04:31:37 UTC (130,378 lines, 17 services, 03:31:37 to 04:31:36) passed `TAYGA_CORPUS=<file> cargo test -p tayga-drain --release --test differential --test fingerprint` (5 and 2 tests).

## Hotspots before and after

The fixes are spec §3.8:
- the service map's 24 h baseline is its own query, ending at the window end's minute floor and cached for that minute (up to 60 s stale);
- `endpoint_stats` and `op_stats` deduplicate with `argMax(…, span_count) GROUP BY trace_id` over the `ts`-filtered rows instead of `trace_summaries FINAL`.

Each figure below is labelled by its source, and comparisons are made only within one source:
- **live 10-06**: the 2026-10-06 `system.query_log` hour of spec §2.7 (old code, real UI tabs: 1,006 map runs/h, about 3 tabs);
- **isolated**: Task 6's back-to-back runs of the old and new SQL on 2026-10-07 around 02:20 UTC (best of 3, `use_query_cache=0`), multiplied by the spec §2.7 run rates (1,006 map runs/h, 60 baseline runs/h). These are estimates;
- **live 10-07**: Task 7's `system.query_log` on the running stack, old code before the deploy at 04:30:52 UTC and new code after it, with the same synthetic map load.

**Like for like, isolated (estimates).**

| Query | Before: CPU s/h | After: CPU s/h | Ratio |
|---|---|---|---|
| service map | 236 (1,006 × 235 ms) | 23 (1,006 × 9.8 ms + 60 × 216 ms) | 10× |
| `endpoint_stats` | 49 (60 × 818 ms) | 8 (60 × 127 ms) | 6.4× |
| `op_stats` | 78 (60 × 1,302 ms) | 22 (60 × 373 ms) | 3.5× |

The live 10-06 figures (558, 70 and 97 CPU s/h) are higher than the isolated "before" estimates. The isolated figures are best of 3, while live runs share ClickHouse with ingest and the other queries; that is the likely cause, not investigated. Comparing live 10-06 with isolated "after" (558 against 23) would overstate the gain.

**Like for like, live 10-07 (measured).** The service map ran only when requested, and no UI tab was open, so Task 7 generated the map load itself: `GET /api/v1/service-map` (default window) from a script on the host.
- **Synchronised load:** 3 requests at once every 10 s, as three tabs refreshing in step. Old code 04:19:42–04:29:42; new code over the hour to 05:31:03.
- **Staggered load:** three loops each every 10 s, offset by 0, 3.3 and 6.6 s, as three tabs opened at different times. New code 05:38–05:52 (the old code's cost per refresh does not depend on timing: it has no cache).

| Query | Old code, live 10-07 | New code, live 10-07 | Ratio |
|---|---|---|---|
| service map, synchronised load, before the single-flight | 180 runs in 10 min, 291.5 CPU ms per refresh, 165.3 MiB read: 315 CPU s/h at 1,080 runs/h | 1,070 window runs (24.9 ms, 8.3 MiB) and 177 baseline runs (280.1 ms, 165.4 MiB) in the hour: 76.3 CPU s/h, 71.3 ms per refresh | 4.1×: below the 5× target |
| service map, synchronised load, with the single-flight (`2e09ac2`) | 291.5 CPU ms per refresh (as above) | 1,083 window runs (25.1 ms, 8.5 MiB) and 60 baseline runs (270.1 ms, 165.4 MiB) in the hour 06:41:25–07:41:25: 43.4 CPU s/h, 40.0 ms per refresh | **7.3×** (5.9× against the isolated 235 ms) |
| service map, staggered load | 291.5 CPU ms per refresh (as above) | 250 window runs (23.6 ms, 8.7 MiB) and 14 baseline runs (312.5 ms, 165.7 MiB) in 14 min: 41.1 ms per refresh, about 44 CPU s/h at 1,071 runs/h | **7.1×** |
| `endpoint_stats` | 60 runs/h, 70.9 CPU s/h, 505.7 MiB per run (hour to 04:19:15) | 60 runs/h, 15.2 CPU s/h, 18.3 MiB and 0.34 M rows per run (hour to 05:31:03) | CPU 4.7×, read **27.7×** |
| `op_stats` | 60 runs/h, 98.5 CPU s/h, 1.32 GiB per run | 60 runs/h, 31.2 CPU s/h, 79.3 MiB and 0.34 M rows per run | CPU 3.2×, read **17.0×** |

- **Why the synchronised load first missed 5×, and the fix.** Before the final-review fix the baseline ran 177 times in the hour, not about 60: three requests that arrived together at a minute rollover all missed the one-entry cache and each computed the baseline, as there was no single-flight (Task 6 concern 3). The fix (`2e09ac2`) puts a `tokio::sync::OnceCell` in the cache slot, so concurrent requests for the same minute share one query. Re-measured with the same load (deployed 06:36:34, hour 06:41:25–07:41:25): 60 baseline runs, exactly one in each minute; 43.4 CPU s/h, 40.0 ms per refresh, **7.3×** against the live old code (291.5 ms) and 5.9× against the isolated "before" (235 ms per refresh; 236 against 43.4 CPU s/h is 5.4× at the two different run rates). The 5× target is met. Staggered requests before the fix gave 7.1×.
- **The window query costs more live than isolated**: 23.6–24.9 CPU ms per run against 9.8 ms, with 8.3–8.7 MiB read against 2.7 MiB.
- **`endpoint_stats` and `op_stats` read 17–28× fewer bytes per run**, past the 4× target. Their CPU falls less (4.7× and 3.2×), close to the isolated 6.4× and 3.5×.

Isolated variants before the fix (spec §2.7, read-only, `use_query_cache=0`, best of 3, on 2026-10-06):

| Variant | ms | CPU ms | Rows read | Bytes read |
|---|---|---|---|---|
| `endpoint_stats` as deployed | 114 | 999 | 8.79 M | 478 MiB |
| `endpoint_stats` with the `argMax` dedup | 70 | 265 | 0.39 M | 21 MiB |
| `op_stats` as deployed | 154 | 1,511 | 8.55 M | 1.21 GiB |
| the `argMax` dedup carrying `op_durations` | 141 | 538 | 0.38 M | 91 MiB |
| service map as deployed (15 min window + 24 h baseline) | 31 | 269 | 9.81 M | 163 MiB |
| service map, window part only | 8 | 12 | 0.19 M | 3.3 MiB |

### Task 6: the shipped queries on live data

Re-run on 2026-10-07 around 02:20 UTC against the live `tayga` database. The settings were `readonly=2` and `use_query_cache=0`, with no caps (`[]`, the bootstrap path). Each query ran 3 times. The figures come from `system.query_log` (`QueryFinish`): best wall ms, best and mean `ProfileEvents['OSCPUVirtualTimeMicroseconds']`, then `read_rows` and `read_bytes`. "Before" is the pre-Task-6 SQL; "after" is the SQL of this commit.

| Query | Wall ms | CPU ms, best (mean) | Rows read | Bytes read | Result rows |
|---|---|---|---|---|---|
| `endpoint_stats` before | 89 | 818 (921) | 8.81 M | 478.9 MiB | 68 |
| `endpoint_stats` after | 51 | 127 (134) | 0.36 M | 19.5 MiB | 68 |
| `op_stats` before | 125 | 1,302 (1,340) | 8.81 M | 1.25 GiB | 453 |
| `op_stats` after | 148 | 373 (383) | 0.36 M | 83.8 MiB | 453 |
| service map before (15 min window + 24 h baseline, one query) | 28 | 235 (257) | 10.60 M | 165.3 MiB | 17 |
| service map after: window query, every request | 7 | 9.8 (11) | 0.16 M | 2.7 MiB | 17 |
| service map after: baseline query, once per minute | 28 | 216 (225) | 10.60 M | 165.9 MiB | 17 |

- **The cost moves out of the per-refresh path (isolated estimate).** At spec §2.7's rate of 1,006 map runs/h and one baseline a minute, the estimate is 1,006 × 9.8 ms + 60 × 216 ms, about **23 CPU s/h**, against 236 for the old query at the same rate and from the same isolated runs (the live 10-06 hour measured 558). Live tabs share it, since their window ends fall in the same minute. The cache holds one entry, so a request for a past window (another minute floor) computes its own baseline; since `1e2ade7` it does not replace a newer cached one. The live measurement is in "Like for like, live 10-07" above.
- **The baselines' CPU falls by 6.4x and 3.5x (isolated estimate).** At 60 runs/h, `endpoint_stats` drops from about 49 to about 8 CPU s/h and `op_stats` from about 78 to about 22 (the live 10-06 hour measured 70 and 97).
- **`op_stats` wall time is not lower** (148 against 125 ms best of 3) even though CPU falls 3.5x. The cause was not investigated. CPU and bytes read are what the stack pays for.

**Equivalence on live data.**
- **Setup.** The comparison pinned `now()` to a fixed instant 5 minutes earlier and added `ts <= that instant` to every `ts` filter, in both versions, so ingest during the runs could not move the window. It ran before, after, then before again as a control.
- **As shipped:**
  - the same 67 endpoints and 450 ops;
  - `seen` equal everywhere;
  - 1 endpoint and 2 ops differ in `kept`/`excluded`/`present`, while the before/before control differs in 0.
  - The cause is the bootstrap cap, 10 × `quantile(0.5)`. `quantile` samples, and the two versions feed it rows in a different order.
- **With `quantileExact` for the p50** (both versions): every `seen`, `kept`, `excluded` and `present` is equal. The remaining quantile differences are 2 endpoint rows and 5 op rows, the same number as the before/before control.

The integration test `argmax_baselines_equal_final_over_duplicates` (`crates/tayga-store/tests/store_it.rs`) pins the semantics on deliberate duplicates. It compares the shipped queries with the `FINAL` reference over:
- several versions in separate parts;
- a newer version that changes the endpoint or turns the trace into an error;
- a version tie;
- a newer version that moves a trace into the window;
- slow stories and caps.

It also pins the documented difference: a newer version that moves a trace out of the window. `FINAL` drops that trace; the `argMax` dedup counts its in-window version.

## Decisions

- **Default backend: `scalar`.** The logminer's batch is one Kafka record, 5.2 logs on average (Live load above). At 5 bodies `parallel` and `gpu` both run on the calling thread and measure the same as scalar (2.44, 2.44 and 2.43 µs), so neither earns a default.
- **GPU: kept behind the `gpu` feature, not recommended.** It is slower than `parallel` at every measured size and passes one thread only from about 5,000 bodies (4 % slower at 2,048), a batch the logminer never forms. It is not in the Docker images; `logminer.fingerprinter = "gpu"` in a build without the feature is a startup error.
- **Arrow: not added** (spec §3.9).
