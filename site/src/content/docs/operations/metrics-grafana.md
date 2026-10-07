---
title: "Metrics and Grafana"
description: "Prometheus metrics, the built-in recorder and the optional Grafana dashboards."
---

Every Tayga service exposes Prometheus metrics. `tayga-api` records them itself for the Pipeline page, so Prometheus is not required. `make up-extras` adds Prometheus and Grafana with four dashboards: stories, service map, pipeline health and logs.

:::caution[TODO: this page is being written]
It will cover:

- The metrics per service.
- The recorder (`record_secs`) and its scrape targets.
- The Grafana dashboards and the read-only ClickHouse user.
:::
