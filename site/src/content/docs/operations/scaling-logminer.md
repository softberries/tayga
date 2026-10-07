---
title: "Scaling logminer replicas"
description: "Running several logminer replicas."
---

`LOGMINER_REPLICAS=2 make up` runs two logminer replicas; the default is 1. They share the consumer group `tayga-logminer`, so Kafka splits the 12 partitions of `tayga.logs` between them; a replica beyond 12 gets no partition and idles.

:::caution[TODO: this page is being written]
It will cover:

- Scaling up and down with Compose and Helm.
- Commands for logs, exec and partitions per replica.
- Metrics edges at a scale change.
:::
