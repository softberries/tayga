---
title: "Pipeline"
description: "The Pipeline page: component status, metric charts and consumer lag."
---

The Pipeline page (`/pipeline`) shows each component's status, metric history charts and consumer lag. The history comes from a recorder inside `tayga-api` that scrapes the services' `/metrics` every 15 seconds and stores the samples in ClickHouse, so the page needs no Prometheus.

:::caution[TODO: this page is being written]
It will cover:

- The status strip and what each state means.
- Every chart and how replicas are aggregated.
- Consumer lag per group and topic.
:::
