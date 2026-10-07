---
title: "Delivery semantics"
description: "Retries, the per-target circuit breaker, deduplication and limits."
---

Each alert is delivered once per target, when the notifier first sees it. A 2xx is delivered; 429, 5xx, network errors and timeouts are retried with exponential backoff up to `max_attempts`; other statuses fail at once. A circuit breaker per target keeps a dead target from holding back healthy ones, and delivery state per alert and target is stored in ClickHouse so a re-published alert is not sent twice.

:::caution[TODO: this page is being written]
It will cover:

- The retry ladder and `Retry-After` handling.
- The circuit breaker in detail.
- Deduplication, offsets and stopping.
- The accepted cases where an alert can be sent twice, and why receivers should deduplicate on `alert_id`.
:::
