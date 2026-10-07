---
title: "Connect your OpenTelemetry Collector"
description: "Point an existing OpenTelemetry Collector at Tayga."
---

Tayga ingests OTLP traces and logs, over gRPC and over HTTP (`/v1/traces` and `/v1/logs`). Metrics are not ingested. Any OpenTelemetry Collector can send to Tayga with an OTLP exporter added to its traces and logs pipelines, next to the exporters it already has.

:::caution[TODO: this page is being written]
It will cover:

- A complete exporter and pipeline snippet for gRPC and for HTTP.
- Sending to Tayga and to an existing backend at the same time.
- Ports per install method, TLS and network placement.
- Checking that data arrives: the Pipeline page and the ingest metrics.
:::
