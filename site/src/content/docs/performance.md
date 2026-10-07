---
title: "Performance"
description: "Benchmarks and the method behind them."
---

Tayga's performance numbers come from a measured report: criterion micro-benchmarks in a release build on an Apple M3 Max, and live measurements on the OpenTelemetry demo stack, each dated. For example, Drain takes 2.818 µs per log line on a 50,000-line corpus exported from the stack, the fingerprint cache makes mining 6.98× faster on the same corpus, and the logminer used 1.07 % of one core at the demo's live load of about 40 log lines per second.

:::caution[TODO: this page is being written]
It will cover:

- The full benchmark tables and how to reproduce them.
- Live load and ClickHouse query costs, before and after the optimisations.
- Fingerprint backends, including the GPU result.
- Hardware, software versions and dates.
:::
