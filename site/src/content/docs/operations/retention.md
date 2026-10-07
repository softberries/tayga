---
title: "Retention and disk"
description: "How long Tayga keeps data, and how to keep the disk in check."
---

Each ClickHouse table has a TTL: raw spans and logs keep 3 days, trace summaries 2 days, stories, service edges and log alerts 7 days, and log templates 30 days. Kafka topics that Tayga creates get a retention of 24 hours (`TAYGA__KAFKA__RETENTION_MS`); Tayga never alters an existing topic.

:::caution[TODO: this page is being written]
It will cover:

- The full TTL table.
- Changing topic retention on an existing stack.
- Sizing the disk and watching it.
:::
