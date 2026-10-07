---
title: "The notifier"
description: "tayga-notifier: delivering log alerts to webhook and Slack targets."
---

`tayga-notifier` consumes the `tayga.alerts` topic and delivers new, spike and silence alerts to webhook and Slack targets. With no targets, the default, it logs `delivery disabled: no targets` once and keeps committing offsets. Its settings live in a TOML file (`targets` can only be set there); scalar keys can also be set with `TAYGA__NOTIFIER__*` environment variables.

:::caution[TODO: this page is being written]
It will cover:

- Every setting with its default (`targets`, `public_url`, `kinds`, `max_attempts`, `timeout_secs`, `max_age_secs`, `breaker_cooldown_secs`).
- Keeping webhook URLs secret.
- The notifier’s metrics.
:::
