---
title: "Performance tuning"
description: "The settings that change how Tayga uses CPU, including the fingerprinter."
---

The main tuning knob for log mining is `logminer.fingerprinter` (`TAYGA__LOGMINER__FINGERPRINTER`): `scalar` (the default), `parallel`, `gpu` (needs a build with the `gpu` feature, which the Docker images do not have) or `off`. At the logminer's real batch size, about 5 log lines per Kafka record, the backends measure the same, which is why `scalar` is the default.

:::caution[TODO: this page is being written]
It will cover:

- Each fingerprinter backend and when it helps.
- Query timeouts and other settings that affect ClickHouse load.
- Where to look when something is slow.
:::
