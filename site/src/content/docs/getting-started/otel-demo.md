---
title: "Demo with the OpenTelemetry demo"
description: "Run Tayga next to the OpenTelemetry demo, trigger a failure and watch it become an error story."
---

The repository vendors the [OpenTelemetry demo](https://github.com/open-telemetry/opentelemetry-demo) as a git submodule, pinned to 3.1.0. `make up` builds the `tayga:dev` image and starts the demo together with Tayga; the demo's collector forwards OTLP to Tayga. A demo feature flag such as `paymentFailure` then produces real error stories.

```sh
git clone --recurse-submodules git@github.com:softberries/tayga.git tayga
cd tayga
make up
make flag NAME=paymentFailure VARIANT=100%
make flags-reset        # restore the demo's default flags
```

The web app is at `http://localhost:8090` and the demo shop at `http://localhost:8080`.

:::caution[TODO: this page is being written]
It will cover:

- Requirements (Docker with Compose, `make`, a Rust toolchain for the dev commands) and resource needs.
- A guided walk: which flags to flip, what story each produces, and how long it takes to appear.
- Optional Grafana and Prometheus with `make up-extras`.
- Ports and the network exposure of the demo, and how to stop the stack.
:::
