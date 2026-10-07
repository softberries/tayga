---
title: "Baselines"
description: "How Tayga learns what normal looks like for each endpoint."
---

Every 60 seconds the assembler refreshes a baseline per endpoint from the last 60 minutes of trace summaries, using non-error traces only. A baseline is trusted once it has at least 50 traces. It gives the root duration's p50, p95 and p99 and, per operation, how often it appears and its p95 duration. A story is compared with it to report new operations, missing operations and slower operations, and a request whose duration exceeds max(p99 × 1.5, p99 + 100 ms) becomes a slow story.

:::caution[TODO: this page is being written]
It will cover:

- The comparison rules and their defaults.
- How slow outliers and slow-storied traces are kept out of the baseline, and the carry-over during long slowdowns.
- Known limits, such as a failure that lasts longer than the window becoming part of the baseline.
:::
