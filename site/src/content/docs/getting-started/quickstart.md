---
title: "Quickstart"
description: "Install Tayga with the one-line installer and send it your first traces."
---

The quickest way to a running Tayga is the installer script: it checks Docker and Compose, starts Tayga with ClickHouse and Redpanda, waits until the services are healthy, and prints the URLs and an OpenTelemetry Collector snippet to send data with.

:::caution[TODO: this page is being written]
It will cover:

- Prerequisites: Docker with Compose, free ports, supported platforms.
- The `curl -fsSL …/install.sh | sh` one-liner, its options (version pinning, `--uninstall`, `--purge`) and what it changes on the machine.
- Sending the first traces and logs, and where the first story appears.
- Next steps: connecting your own Collector, Helm for Kubernetes, the OTel demo.
:::
