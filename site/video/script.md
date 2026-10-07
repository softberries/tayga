# Tayga tour: narration script

The narrated product video on the landing page (`site/public/media/tayga-tour.mp4`). About four
minutes in ten segments, plus a title card and an end card.

This file is the source of truth for the narration: `tts.ts` reads the `>` lines under each
numbered heading and speaks them exactly as written. They are spelled the way they should be
said ("p ninety-nine", "hello at softberries dot dev"); the captions show the written form
(`p99`, `hello@softberries.dev`) through the substitutions in `assemble.ts`. The **On screen**
lists are what `record.ts` films for each segment.

Every statement is about a feature that exists in the open-source code, and every number comes
from the [Verified claims](../src/content/docs/verified.md) page:

| Narration | Source in `verified.md` |
|---|---|
| OTLP, no proprietary agent | Landing page rows "OTLP over gRPC and HTTP", "Tayga takes traces and logs only; no proprietary agent" |
| Redpanda, ClickHouse | "Redpanda / Kafka API, ClickHouse storage" |
| A trace closes after 10 s without new spans | "A trace closes after 10 s without new spans, or 60 s at most" |
| AGPL core | "Self-hosted, AGPLv3 core" |
| First payment story 45 to 95 s after the flip | `make e2e` runs: payment 95 s (2026-10-04), 55 s (2026-10-05), 45 s (run 2) |
| Root cause, path, critical path, comparison with normal, logs | "A story shows the root-cause span, the path, the critical path, what differs from normal, and the logs" |
| Fingerprint of kind, endpoint, root-cause span, masked message; numbers, IDs and hex masked | "Fingerprint of kind, endpoint, root-cause span and masked message", "Numbers, UUIDs and hex runs masked" |
| Map: rate, error ratio, p99; red at 5 % failed calls, amber above 2 × the 24 h p99 | "Service map: rate, error ratio and p99 …", "Health: error at ≥ 5 % failed server/consumer spans, slow above 2 × the 24 h p99" |
| 2.8 µs per log line | "2.8 µs per log line through Drain (2.818)" |
| New, spike and opt-in silence alerts to Slack and webhooks, with retries | "New, spike and opt-in silence alerts, to webhook and Slack with retries" |
| Pipeline page from a recorder inside the API | "Pipeline health recorded by the API itself" |
| Installer, Helm chart, OTel demo | `scripts/install.sh`, `deploy/helm/tayga`, the Getting started pages; "Tayga: Docker Compose and Helm" |
| Enterprise: SSO, role-based access, multi-tenancy, HA, SLA support, on request | "Enterprise: …" and "Enterprise available on request": **commercial offering (owner)** |

## 1. The problem

> A checkout request fails. Then another. Your dashboards show the error rate climbing, but not why. So someone opens a trace, scrolls through a hundred spans, and goes hunting through the logs for anything that looks related. Tayga does that first pass for you.

**On screen**

- Traces page, errors only: the cursor drifts over the duration scatter.
- Open a large failing trace: the waterfall scrolls past span after span.
- On "Tayga does that first pass", switch to the Stories page.

## 2. Beside your stack

> Tayga runs beside your stack. Your OpenTelemetry Collector sends it traces and logs over OTLP, with no proprietary agent. Tayga buffers them in Redpanda, closes each trace ten seconds after its last span, stores everything in ClickHouse, and analyses every request that failed or ran slow. The core is open source, under the AGPL.

**On screen**

- An architecture card in the Tayga style: your services and Collector, OTLP into ingest, Redpanda, the assembler and the logminer, ClickHouse, the API and the web app.
- Each part lights up as it is named; "AGPL" lights the licence badge.

## 3. A live failure

> Let's break something. The OpenTelemetry demo runs next to Tayga. We turn on its payment failure flag, so every charge fails, and watch the Stories page refresh in live mode. In our end-to-end runs, the first payment story arrived forty-five to ninety-five seconds after the flip. And here it is: payment charge failed, a new error story group.

**On screen**

- Stories, last 15 minutes, errors only, live.
- A terminal overlay types `make flag NAME=paymentFailure VARIANT=100%`; the flag is set for real at that moment.
- A badge counts the real seconds since the flip; the wait is shown as a time-lapse.
- The new "payment charge failed" group appears: ring round the row, the cursor selects it, the inspector fills.
- After filming: `make flags-reset`.

## 4. The story

> Open the story. At the top, the root cause: the service and span where the failure started, with its message. Then the request path, through checkout to payment, the failing service in red. The Compared with normal panel lists what differs from a typical request to this endpoint. Here, the operations that never got to run. The waterfall opens on the critical path, with the root-cause span selected. And below, the logs this trace wrote, each linked to its template.

**On screen**

- Select **Open story** in the inspector.
- Rings, in turn: the root-cause line, the request path, the "Compared with normal" panel, the waterfall's root-cause row, the logs table and a template link.

## 5. Story groups

> Stories with the same fingerprint, the same kind, endpoint, root-cause span and masked message, form one group. Numbers, IDs and hex strings are masked, so repeated failures become one row, with a count and a trend. The inspector previews the latest story, so you can triage with the arrow keys.

**On screen**

- Stories, last 24 hours. Rings on a group's summary, its trend sparkline and its count.
- The arrow keys step through the groups; the inspector follows.

## 6. The service map

> The service map draws every service in call order, with its call rate, error ratio and p ninety-nine. A service turns red when five percent or more of its calls fail, and amber when its p ninety-nine is more than twice its twenty-four hour level. Select one for its charts, and the stories whose root cause is there.

**On screen**

- The map, last 15 minutes (so the payment failure from segment 3 shows red): ring on the summary line, the cursor visits a few cards; the failing (dashed) calls run into payment.
- Select a degraded service: its drawer with the rate, error and p99 charts and its stories.

## 7. Log templates and alerts

> Tayga mines your logs too. Lines that differ only in their variable parts become templates, at about two point eight microseconds per line in our benchmark. It alerts on a new template, a spike above baseline, and silence, for templates you choose to watch. Each alert links to example traces, and the notifier sends it to Slack or any webhook, with retries.

**On screen**

- Alerts, last 7 days: the timeline and the table with new, spike and silence rows; rings on the kind badges.
- Open a spike's template: the hits chart, the latest hits linked to traces and stories, and the **Alert when silent** switch (shown, not changed).

## 8. Pipeline health

> And Tayga watches itself. The Pipeline page shows whether each service is up, charts every stage, and reads consumer lag straight from Kafka, all collected by the API itself, with no Prometheus needed. Prefer a light theme? One click.

**On screen**

- The status strip (ring), a smooth scroll through the charts, the consumer lag table (ring).
- Back to the top; the theme switch changes the app to light.

## 9. Deployment

> Getting started takes one line. The installer checks Docker, starts Tayga with ClickHouse and Redpanda, and prints the Collector snippet for your data. On Kubernetes, there's a Helm chart. Or run it next to the OpenTelemetry demo and flip a flag, like we just did.

**On screen**

- A terminal card: the `curl … install.sh | sh` one-liner and the installer's real closing output ("Tayga is running." with the URLs and the Collector exporter).
- Then `helm install tayga oci://ghcr.io/softberries/charts/tayga …`, then `make up` and `make flag NAME=paymentFailure VARIANT=100%`.

## 10. Enterprise and the docs

> For larger teams, an enterprise edition is available on request: single sign-on, role-based access, multi-tenancy, high availability, and support with an SLA. Write to hello at softberries dot dev. And for everything else, head to the docs site. Tayga: from a failing request to its root cause, in one story.

**On screen**

- The docs site's landing page: the enterprise section (ring on the contact), then the top of the page.
- The end card follows: the docs URL, the GitHub repository and the contact.
