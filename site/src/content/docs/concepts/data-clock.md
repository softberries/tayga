---
title: "The data clock"
description: "Why the logminer judges new templates in log time, not wall-clock time."
---

The logminer keeps a data clock: the newest mined log timestamp. New templates are judged against it rather than against the wall clock, so a template that appeared while the logminer was down or behind is still reported when it catches up, and a replayed backlog on a fresh install does not report its whole history. The lag between the wall clock and the data clock is exported as `tayga_logminer_data_lag_seconds` and charted on the Pipeline page.

:::caution[TODO: this page is being written]
It will cover:

- The per-partition clock and how it holds while a partition is behind.
- Fresh installs, restarts and the stored watermark.
- What is wall-clock based (spike windows) and the consequences.
:::
