---
title: "Log templates"
description: "How Tayga mines log lines into templates with Drain."
---

`tayga-logminer` groups log lines into templates with the Drain algorithm, one tree per service. A body is masked first: UUIDs, hex runs of 8 or more characters and numbers become `<*>`, so `Found 3 products from database` and `Found 12 products from database` are one template, `Found <*> products from database`. An HTTP status code in an access-log position is kept, so a burst of 500s can be its own template. A fingerprint cache skips the Drain tree for log lines whose masked shape was already seen; it gives identical results.

:::caution[TODO: this page is being written]
It will cover:

- Drain settings: similarity threshold, cluster limits and the `<overflow>` template.
- Masking rules, the HTTP status code rule and the masking epoch.
- The fingerprint cache and its backends.
- Storage and retention of templates and hits.
:::
