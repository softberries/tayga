---
title: "Troubleshooting"
description: "Common problems and how to diagnose them."
---

Most problems show up on the Pipeline page first: a component that is down, a growing consumer lag or a data lag on the logminer. The services log in JSON and expose Prometheus metrics; the API answers 503 `storage unavailable` when ClickHouse fails and 504 `storage timeout` when a read runs past its limit.

:::caution[TODO: this page is being written]
It will cover:

- No stories appear: checking ingest, the topics and the assembler.
- Disk filling up, and Redpanda retention.
- API errors and what they mean.
- Collecting logs and metrics for a bug report.
:::
