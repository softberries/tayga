---
title: "Helm and Kubernetes"
description: "Install Tayga on Kubernetes with the Helm chart."
---

The Helm chart deploys Tayga's services (ingest, writer, assembler, logminer, API and, optionally, the notifier) with a migration job that runs before installs and upgrades. ClickHouse and Redpanda can be bundled for evaluation or pointed at existing clusters.

:::caution[TODO: this page is being written]
It will cover:

- Installing from the OCI chart registry, and the values that matter first.
- Bundled single-node ClickHouse and Redpanda versus `external.*` endpoints.
- Ingress, resources, probes and replica counts (logminer replicas included).
- The values reference, generated from `values.schema.json`.
:::
