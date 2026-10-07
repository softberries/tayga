---
title: "Story groups and fingerprints"
description: "How one underlying problem becomes one story group."
---

Stories are grouped by a fingerprint: a hash of the story kind, the endpoint, the root-cause service and span name, and the root-cause message with numbers, hex runs and UUIDs masked. Every story with the same fingerprint lands in the same group, so a recurring failure shows up as one row with a trend instead of hundreds of traces.

:::caution[TODO: this page is being written]
It will cover:

- The fingerprint inputs and masking in detail.
- Why the same failure can form one group per calling endpoint.
- Group trends, example stories and the group view in the app.
:::
