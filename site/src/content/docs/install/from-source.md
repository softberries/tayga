---
title: "From source"
description: "Build Tayga from source with Cargo and Node."
---

Tayga is a Rust workspace. The services are `tayga-ingest`, `tayga-writer`, `tayga-assembler`, `tayga-logminer`, `tayga-notifier` and `tayga-api`; the web app in `ui/` is a React single-page app that is built into `ui/dist` and embedded in the `tayga-api` binary. Building the Rust crates needs no Node: without `ui/dist`, `tayga-api` serves a "UI not built" placeholder page.

:::caution[TODO: this page is being written]
It will cover:

- Toolchain requirements (Rust, Node 24 or newer for the UI).
- Building the image (`make up` builds `tayga:dev`) and building the binaries directly.
- Running the services against your own ClickHouse and Redpanda, and the migrations (`tayga-writer migrate`).
- Developer commands: tests, integration tests, end-to-end tests and benches.
:::
