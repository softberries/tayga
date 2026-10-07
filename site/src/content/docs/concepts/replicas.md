---
title: "Replicas and partitioning"
description: "How Tayga spreads work across Kafka partitions and logminer replicas."
---

Spans and logs travel on the Redpanda topic `tayga.signals`, keyed by trace id, so all of a trace reaches the same assembler partition. Drain needs all of a service's logs in one place, so ingest also publishes logs to `tayga.logs`, keyed by service. Several logminer replicas share the consumer group `tayga-logminer` and split the 12 partitions; each replica judges only the services it owns, so two replicas never alert on the same service outside a hand-over.

:::caution[TODO: this page is being written]
It will cover:

- Topics, keys and partition counts.
- Ownership, rebalances and why re-reads are harmless.
- Duplicate-alert protection and alert republication.
- Metrics with several replicas.
:::
