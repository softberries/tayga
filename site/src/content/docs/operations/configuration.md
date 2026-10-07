---
title: "Configuration reference"
description: "Every setting and environment variable, with its default."
---

Each Tayga service reads its settings from a TOML file named by `TAYGA_CONFIG` and from `TAYGA__*` environment variables (for example `TAYGA__CLICKHOUSE__URL` or `TAYGA__LOGMINER__FINGERPRINTER`). Lists, such as notifier targets or the map's infrastructure services, can only be set in the file.

:::caution[TODO: this page is being written]
It will cover:

- Every setting per service, with its default and environment variable, checked against the code.
- How the file and environment variables combine.
- Example configurations.
:::
