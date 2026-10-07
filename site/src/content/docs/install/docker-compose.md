---
title: "Docker Compose"
description: "Run Tayga, ClickHouse and Redpanda with Docker Compose, without the OpenTelemetry demo."
---

A standalone Compose deployment runs Tayga with ClickHouse and Redpanda and nothing else: no demo services. You point your own OpenTelemetry Collector, or any OTLP source, at Tayga's ingest.

:::caution[TODO: this page is being written]
It will cover:

- The `deploy/standalone/` bundle: services, the pinned image version (`TAYGA_VERSION`) and host ports set through `.env`.
- Starting, checking health, and opening the app.
- The included `otel-collector.yaml` snippet.
- Persistent volumes, resource limits and production notes.
:::
