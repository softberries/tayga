---
title: "HTTP API"
description: "Every route of the tayga-api JSON API, with parameters and response shapes."
---

`tayga-api` serves the web app and a JSON API on port 8090. The data routes are under `/api/v1/`; `GET /healthz` returns `ok` and `GET /metrics` serves Prometheus metrics. Time windows use `since` (`<n>[smhd]`, from `1s` to `7d`) and an optional `until`; errors are JSON `{"error": "..."}`.

:::caution[TODO: this page is being written]
It will cover:

- Every route, parameter and response shape, with examples checked against the live API.
- Time windows, bucketing and limits.
- Authentication for scripts.
- Error codes.
:::
