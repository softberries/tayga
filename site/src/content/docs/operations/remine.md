---
title: "Re-mining templates"
description: "Rebuilding log templates with tayga-devtools remine."
---

After a change to masking or a Drain setting, `tayga-devtools remine` rebuilds the templates from the stored logs of the last 3 days with the logminer's own code, so the templates reflect the new configuration. Run it with the logminer stopped; `--dry-run` previews the result without writing anything.

:::caution[TODO: this page is being written]
It will cover:

- The procedure step by step.
- The heartbeat guard, duration and the 3-day window.
- Template id churn and orphaned silence settings.
:::
