---
title: "Upgrading"
description: "Upgrade Tayga to a new version safely."
---

Migrations must run before `tayga-api`, `tayga-logminer` or `tayga-notifier` restart on a new version. With Compose, every Tayga service except ingest waits for the one-shot `tayga-migrate` service to finish; with Helm, the migration job runs as a pre-upgrade hook.

:::caution[TODO: this page is being written]
It will cover:

- The upgrade procedure for Compose, Helm and source builds.
- Version-specific notes, such as the move of the logminer to the `tayga.logs` topic.
- Reloading browser tabs opened before the upgrade.
- Rolling back.
:::
