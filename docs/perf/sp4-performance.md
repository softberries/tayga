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

Filled by Task 2/4/5.

## Cache

Filled by Task 3.

## Hotspots before and after

Filled by Task 6/7.

## Decisions

Filled by Task 7.
