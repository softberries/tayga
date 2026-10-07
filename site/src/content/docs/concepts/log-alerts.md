---
title: "Log alerts"
description: "New-template, rate-spike and silence alerts on log templates."
---

Every 60 seconds the logminer checks the templates for alerts. A **new** alert fires once when a template appears for the first time in a service that already had templates. A **spike** alert fires when a template's count in the last 5 minutes is at least 10 and at least 5 times its baseline. A **silence** alert, opt-in per template, fires when a template has had no log for N minutes while its service still sends other logs. Alerts carry example trace ids that link to error stories.

:::caution[TODO: this page is being written]
It will cover:

- Each rule with its settings and defaults, and the warmup that stops a fresh install from flooding.
- Spike baseline coverage and the opt-in seasonal mode.
- When an alert counts as active.
- How alerts reach the notifier.
:::
