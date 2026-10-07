---
title: "Service map"
description: "How Tayga builds the service map and judges service health."
---

The service map is built from the calls between services found in assembled traces. Each service shows its rate, error ratio and p99, compared with its own p99 over the previous 24 hours, and a health of ok, slow or error. Infrastructure services (by default `flagd`) are hidden unless "Show infrastructure" is on.

:::caution[TODO: this page is being written]
It will cover:

- Where the edges come from (`service_edges`) and how they are counted.
- The health rules and the 24-hour baseline, cached once a minute.
- Infrastructure services: the `[map] infra_services` setting and the "+N infra" badges.
:::
