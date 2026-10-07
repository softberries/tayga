---
title: "Error stories"
description: "What an error story is, when Tayga creates one, and what it contains."
---

An error story is Tayga's explanation of one failing request. When the assembler closes a trace that has an error span (a span with error status, an `exception` event, or an attached log with severity ERROR or higher), it writes a story: the root-cause span with a one-sentence summary, the path from the root span to the root cause, the critical path, the comparison with the endpoint's baseline, the related logs (at most 50, errors first) and the other spans that also failed. A request that did not fail but was much slower than normal becomes a slow story instead.

:::caution[TODO: this page is being written]
It will cover:

- The story record field by field, and how it maps to the Story page.
- Error versus slow stories, and the `incomplete` and `truncated` flags.
- Late spans and logs, and why they are stored but not re-analysed.
- Where stories go: ClickHouse `error_stories` and the `tayga.stories` topic.
:::
