---
title: "Webhook and Slack formats"
description: "The JSON webhook payload and the Slack message."
---

A webhook target receives a `POST` with a JSON body per alert: its id, kind, service, template, times, count and baseline, a one-line summary, example trace ids and links back into the app. A Slack target receives a message built from blocks, with the template in a code block and buttons to the template and up to 3 traces.

:::caution[TODO: this page is being written]
It will cover:

- The full webhook payload, field by field, with an example.
- The Slack message layout and setting up an incoming webhook.
- Escaping and the summary text per alert kind.
:::
