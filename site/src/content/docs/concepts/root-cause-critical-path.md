---
title: "Root cause and critical path"
description: "How Tayga picks the root-cause span and computes the critical path."
---

The root cause is chosen among the error leaves: error spans with no error descendants. The one that ended first is the root cause, and the others are listed as also failed. The critical path is found by walking back from the end of the root span through the children that determined when it finished; it yields ordered segments and the top three contributors by self-time. For slow stories, the top contributor is the root cause.

:::caution[TODO: this page is being written]
It will cover:

- The rules for the one-sentence explanation, including "could not reach" for client spans without a server child.
- The critical-path algorithm, async children and the 5 ms clock-skew tolerance.
- Worked examples from the OpenTelemetry demo.
:::
