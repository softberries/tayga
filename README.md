<p align="center">
  <img src="ui/public/logo.jpg" alt="Tayga logo: a brown and white ticked German spaniel in profile" width="180">
</p>

# Tayga

Tayga turns OpenTelemetry traces and logs into "error stories". For each failing or slow request it shows the root-cause span, the request path across services, the critical path, a diff against the endpoint's normal baseline, and the related logs. Stories are grouped by fingerprint, so one underlying problem shows up as one group rather than as hundreds of traces.

It runs next to the [OpenTelemetry demo](https://github.com/open-telemetry/opentelemetry-demo) (vendored as a git submodule, pinned to 3.1.0). The demo's collector forwards OTLP to Tayga; Tayga stores raw spans and logs in ClickHouse, assembles traces, and serves a web app and a JSON API.

## Architecture

```
OTel demo services ──► demo otel-collector ──OTLP gRPC──► tayga-ingest
                                                              │ one record per trace_id
                                                              ▼
                                       Redpanda topic `tayga.signals` (key = trace_id)
                                     ┌────────────────────────┴───────────────────────┐
                                     ▼                                                ▼
                              tayga-writer                                    tayga-assembler
                       (raw spans/logs → ClickHouse)            (session windows → analysis → stories,
                                     │                            trace summaries, service edges)
                                     ▼                                                │
                                 ClickHouse ◄─────────────────────────────────────────┤
                                     │                                                ▼
                         tayga-api (JSON API + web app)                  Redpanda topic `tayga.stories`
                         tayga-grafana, tayga-prometheus (optional: `make up-extras`)

Redpanda topic `tayga.signals` ──► tayga-logminer (separate consumer group, one replica)
                                     │  Drain template mining per service, detection every 60 s
                                     ├─► ClickHouse `log_templates`, `log_template_hits`, `log_alerts`
                                     └─► Redpanda topic `tayga.alerts`
```

Workspace crates (`crates/`): `tayga-ingest`, `tayga-writer`, `tayga-assembler`, `tayga-logminer`, `tayga-api` (services); `tayga-analysis`, `tayga-drain` (Drain mining and alert rules, no I/O), `tayga-model`, `tayga-kafka`, `tayga-store`, `tayga-common` (libraries); `tayga-devtools` (flag, capture, verify-raw, emit-log CLI); `tayga-e2e` (end-to-end tests).

## Quick start

Requires Docker with Compose, `make`, and a Rust toolchain (for the dev commands).

```sh
git clone --recurse-submodules git@github.com:softberries/tayga.git tayga
cd tayga
make up
```

`make up` initializes the submodule, builds the `tayga:dev` image (the web app is built inside the image; no Node needed on the host), and starts the demo plus Tayga. It does not start Grafana or Prometheus.

- Web app: http://localhost:8090
- Demo shop: http://localhost:8080
- Optional, after `make up-extras` (see [Grafana and Prometheus](#grafana-and-prometheus-optional)):
  - Grafana: http://localhost:3001 (anonymous Viewer access is enabled; admin password is `admin`)
  - Prometheus: http://localhost:19090

Trigger a failure with a demo feature flag, then watch a story appear:

```sh
make flag NAME=paymentFailure VARIANT=100%
make flags-reset        # restore the demo's default flags
```

Other targets: `make ps`, `make logs SERVICE=<name>`, `make down` (also removes Grafana and Prometheus if they are running).

## Web app

The app is a single-page React app in `ui/`, built into `ui/dist` and embedded in the `tayga-api` binary, so port 8090 serves the app and the JSON API. It replaced the earlier server-rendered pages. Every filter and the time range live in the URL, so a page can be shared as a link.

| Page | Path | What it shows |
|---|---|---|
| Stories | `/` | KPI tiles, the story-groups table (kind, service, endpoint filters, search), an inspector for the selected group (request path, compact waterfall, comparison with normal), a mini service map and the log alerts |
| Story | `/stories/{id}` | One story: root cause, group trend, full waterfall, comparison with normal, logs with their templates, related alerts |
| Traces | `/traces`, `/traces/{id}` | Trace explorer (filters, duration scatter with brush selection, results table); the trace page has the waterfall and a span drawer |
| Service map | `/map` | Services and their calls with health, rate, error ratio and p99; a node opens a drawer with RED charts and related stories |
| Logs | `/logs/alerts`, `/logs/templates`, `/logs/templates/{id}` | Log alerts, templates and a template's detail (`/logs` redirects to the alerts tab) |
| Pipeline | `/pipeline` | Component status, metric history charts and consumer lag, from the recorder below; needs no Prometheus |

Header and shortcuts:

- Time range: 15m, 1h, 24h or 7d (default 1h), each ending now, or "Custom…": a past window picked with "from" and "to" fields in local time, up to 7 days long and starting within the last 7 days. The custom range is kept in the URL as `since` and `until` (UTC) and shows in the header as, for example, "Oct 4 12:00 – 14:00"; links between pages keep it, and choosing a preset clears it.
- Live: refreshes every 10 s and pauses while the browser tab is hidden. It is off, and cannot be turned on, while a custom range is set. The degraded-services badge (always the last 15 minutes) and the Pipeline page's job status and consumer lag show the current state and keep refreshing in any range.
- A custom range in an old link may have aged past the 7 days of data: the pages then show the API's error with a "Show last 1h" button, and the URL is left alone until it is clicked.
- Theme: a switch that cycles light, dark and system (the default). The choice is stored in the browser.
- `Cmd+K` or `Ctrl+K` opens the command palette: jump to a page, a service, a trace id (32 hex characters), a story group or a template, or switch the theme and the time range (including "Custom range…", which opens the custom range fields).
- `g` then `s`, `t`, `m`, `l` or `p` goes to Stories, Traces, Service map, Log alerts or Pipeline. `?` lists the shortcuts. They are ignored while you type in a field or a dialog is open.
- "Open in Jaeger" (span drawer, trace page) links to the demo's Jaeger (`TAYGA__JAEGER_URL`, set in `deploy/compose.tayga.yaml`). "Open in Grafana" on the map appears only when `TAYGA__GRAFANA_URL` is set, which `make up-extras` does. Both are empty when unset.

Pipeline history comes from a recorder inside `tayga-api`: every 15 s (`record_secs`) it scrapes the `/metrics` of ingest, writer, assembler and logminer plus its own registry, and stores the samples in ClickHouse `metric_samples` (7-day TTL). The scrape targets are in `deploy/tayga-api.toml` (`TAYGA_CONFIG`); a list of targets cannot be set through `TAYGA__` environment variables. Consumer lag is read from Kafka on request, so `tayga-api` has Kafka settings.

### Grafana and Prometheus (optional)

They are the compose profile `extras`. `make up-extras` starts them and sets the Grafana link in the app; `make up` leaves them out. `make up` does not stop them if they are already running (compose leaves profiled services alone), so after upgrading an existing stack run `make down` once, or stop `tayga-grafana` and `tayga-prometheus` by hand. The Grafana dashboards (`tayga-stories`, `tayga-service-map`, `tayga-pipeline`, `tayga-logs`) are unchanged and still available there.

### Developing the UI

Node 24 or newer is needed only for UI development (`engines` in `ui/package.json`; Docker builds the app with `node:24`). Building the Rust crates needs no Node: without `ui/dist`, `tayga-api` compiles and serves a "UI not built" placeholder page.

```sh
npm --prefix ui ci
make ui-dev                  # Vite dev server; proxies /api and /metrics to http://127.0.0.1:8090 (TAYGA_API overrides)
npm --prefix ui test         # unit and component tests (vitest)
npm --prefix ui run lint
npm --prefix ui run typecheck
make ui-e2e                  # Playwright against the live app (see ui/playwright.config.ts)
```

## Log templates and alerts

`tayga-logminer` reads the logs from `tayga.signals` (its own consumer group) and groups them into templates with the Drain algorithm, one tree per service. A body is first masked (UUIDs, hex runs of 8 or more characters and numbers become `<*>`), so `Found 3 products from database` and `Found 12 products from database` are one template, `Found <*> products from database`. A template keeps its id when it generalizes. Because numbers are masked, an HTTP status code in an access log is a wildcard: a burst of 500s in Envoy logs is not separated from 200s (spans cover HTTP errors).

Per log, one row in `log_template_hits` (3-day TTL); per template, one row in `log_templates` (30-day TTL); alerts in `log_alerts` (7-day TTL) and as JSON on the topic `tayga.alerts`. Counting uses `uniqExact(log_id)`, so replayed records are not counted twice. Notification delivery (webhook, Slack) is not implemented.

Every 60 seconds the logminer runs two rules over `log_template_hits`:

| Rule | Fires when | Default |
|---|---|---|
| New template | the template's first log falls since the previous detection tick, in log time; its service had a template at least `new_template_warmup_min` before that first log (so a fresh install or new service does not flood); and it is not `<overflow>`. One alert per template, ever | warmup 15 min |
| Rate spike | the count in the last `spike_window_min` is at least `spike_min_count` and at least `spike_factor` times the mean per-window count over the preceding `baseline_window_min` (floored at 1), and the template is older than window + baseline (65 min; younger ones are covered by the new-template rule) | window 5 min, baseline 60 min, factor 5, min count 10 |

"In log time" means the logminer keeps a data clock, the newest mined log `ts`, and checks templates first seen after the previous tick's data clock minus 60 s. A template that appeared while the logminer was down or behind is therefore still reported when it catches up. On a fresh install (no hits yet) the clock starts at the wall clock minus 10 minutes, so a replayed backlog does not report its history. Spike windows are wall-clock based: a spike during such a gap is not reported. `tayga_logminer_data_lag_seconds` (wall clock minus the data clock; charted on the app's Pipeline page, and on the Grafana Pipeline health dashboard with `make up-extras`) shows the lag, and the logminer logs a warning when it exceeds 10 minutes.

A spike alert stays active while it was last confirmed within `alert_active_min` (10) of now; a tick that still fires updates it, otherwise a new alert starts. Each alert carries up to 5 example trace ids (newest first) that link to the error story when one exists, else to Jaeger. The other values are `TAYGA__LOGMINER__*` environment variables (keys in `LogminerSettings` in `crates/tayga-logminer/src/main.rs`). Other defaults: similarity threshold 0.5, at most 5,000 clusters per service (beyond that, unmatched logs go to the `<overflow>` template), flush at 5,000 logs or 1 s.

The API decides whether an alert is "active" (the `active` field, the `alerting` flag and the story-page badges) with its own constant, `ALERT_ACTIVE_MIN = 10` in `crates/tayga-api/src/repo.rs`, not with the logminer setting. If you change `TAYGA__LOGMINER__ALERT_ACTIVE_MIN`, change that constant to match, or the UI and the logminer disagree on which alerts are active.

Where to look:

- `/logs/alerts`: alerts with kind badge, count against baseline and example traces.
- `/logs/templates` and `/logs/templates/{id}`: templates by count, search, sparkline, sample and recent hits.
- Story pages: the log table has a Template column, with a `new` or `spike` badge when the template was alerting at the story's time.
- With `make up-extras`: Grafana "Tayga · Logs" dashboard (`tayga-logs`) and logminer panels in "Tayga · Pipeline health". The app's Pipeline page charts the logminer metrics too.

Limits: one logminer replica only (Drain state is global per service while records are partitioned by trace); no seasonal baselines; no alert when a template disappears; history is not re-mined. Open questions are in `docs/superpowers/followups.md`.

To probe the new-template rule by hand (the stack's ingest listens on 14318):

```sh
cargo run -p tayga-devtools -- emit-log --service tayga-e2e-probe --body "hello probe marker"
```

The command prints the trace id it used. An alert fires only if the service already had a template 15 minutes before. The e2e `new_template_from_probe` logs under its own service, `tayga-e2e-probe`, never a demo service. Every run emits a constant seed log, `tayga e2e probe seed`, and then a probe `{word} probe … probe marker` with a random 12-letter word and 4 to 58 tokens. The first run on a stack waits up to 16 minutes for the seed template to age past the warmup. Drain routes on the token count, then on the first word, so the probes spread over about 55 nodes of 100 children: about 4,000 runs (simulated: the first node fills at 4,000–4,650 runs) fit in the 30-day template TTL before probes start merging and the test fails.

## Developer commands

| Command | What it does |
|---|---|
| `cargo test --workspace` | Unit tests (integration and e2e tests are `#[ignore]`d) |
| `make up-extras` | Also starts Grafana and Prometheus (compose profile `extras`) and enables the app's Grafana link |
| `make ui-dev` | Starts the Vite dev server for the web app (Node 24 or newer; proxies to the API on 8090) |
| `make ui-e2e` | Runs the Playwright suite (`npm --prefix ui run e2e`) against the running app; needs `make up` first |
| `npm --prefix ui test` | UI unit and component tests (vitest) |
| `make it` | Starts Redpanda + ClickHouse standalone (compose project `tayga-it`) and runs the ignored integration tests (all crates except `tayga-e2e`; the ClickHouse tests each seed a uniquely named database). Run it with the full stack down: both use the same host ports 19092 and 18123 |
| `make infra-down` | Stops the standalone infra and removes its volumes |
| `make e2e` | Resets flags, then runs the end-to-end tests against the live stack (`make up` first). Nine tests: the seven from before (flag-driven story scenarios for payment, payment unreachable, shipping, product catalog and ad, the service map, and raw span counts versus Jaeger) plus `log_spike_on_payment_failure` and `new_template_from_probe`. The spike scenario fails up front if a payment spike alert is still active from an earlier run (wait about 10 minutes). The first run on a fresh stack waits up to 16 more minutes for the probe service warmup |
| `make verify-raw` | Compares per-trace span counts in ClickHouse with the demo's Jaeger |
| `make capture NAME=<n> ARGS="<args>"` | Captures a fixture to `fixtures/<n>.pb.gz` (see `cargo run -p tayga-devtools -- capture --help`) |

## Ports

Tayga's own published ports (8090, 3001, 19090, 19092, 18123, 14318) are bound to 127.0.0.1. 3001 and 19090 are published only after `make up-extras`. The upstream OpenTelemetry demo is not: it publishes 8080 (frontend proxy), 9090 (the demo's Prometheus), 10000 (Envoy admin) and 26 other container ports (on ephemeral host ports, counted on demo 3.1.0) on all interfaces, so they are reachable from your network. Run the stack only on a trusted network, or firewall those ports.

| Port | Service | Defined in |
|---|---|---|
| 8080 | OTel demo frontend proxy (shop, Jaeger UI under `/jaeger/ui`) | demo compose (not published by Tayga's compose files) |
| 8090 | tayga-api: web app, JSON API, `/healthz`, `/metrics` | `deploy/compose.tayga.yaml` |
| 3001 | Grafana (container port 3000), `extras` profile only | `deploy/compose.tayga.yaml` |
| 19090 | Prometheus (container port 9090), `extras` profile only | `deploy/compose.tayga.yaml` |
| 19092 | Redpanda Kafka API (external listener) | `deploy/compose.infra.yaml` |
| 18123 | ClickHouse HTTP (container port 8123) | `deploy/compose.infra.yaml` |
| 14318 | tayga-ingest OTLP/HTTP (container port 4318), used by `tayga-devtools emit-log` | `deploy/compose.tayga.yaml` |

## HTTP routes (tayga-api, port 8090)

`since` takes `<n>[smhd]`, from `1s` to `7d` on the API routes; the app's time range offers 15m, 1h, 24h and 7d, plus a custom range. Every route that takes `since` also takes an optional `until`: RFC 3339 (`2026-10-04T12:00:00Z`, any offset) or unix seconds, default now. The window is then `[until - since, until)`; without `until` it ends now, and the row lists (trace search, log alerts, a template's recent hits) also show rows stamped up to 60 s past now, so a producer clock running slightly ahead hides nothing. Counts, rates and bucketed series stop at the window's end, so a total always equals the sum of its buckets. `until` may be at most 60 s in the future, and the window must start within the last 7 days, the longest TTL of the tables these routes read (`error_stories`, `service_edges`, `log_alerts` and `metric_samples`; `spans`, `logs` and `log_template_hits` keep 3 days and `trace_summaries` 2 days, so older windows there are simply empty). Bucketed series use buckets on the epoch grid (multiples of `bucket_secs`), so a moving live window keeps its bucket edges; the first and last bucket may be partial. Alert `active` flags, template `alerting` and the overview's active alerts and data lag are as of the window's end. `pipeline/lag` is live only and ignores `until`. Invalid values return 400. Fingerprints are decimal u64 strings; story and trace ids are 32 hex characters.

JSON API:

| Route | Query | Returns |
|---|---|---|
| `GET /api/v1/story-groups` | `since` (default `1h`), `until`, `kind` (`error` or `slow`), `service` | Top 100 story groups, each with `buckets` (`[bucket_start_unix_s, stories]`, about 120 per window) and `bucket_secs` |
| `GET /api/v1/story-groups/{fingerprint}` | `since` (default `24h`), `until` | One group with example stories; 404 if absent |
| `GET /api/v1/stories/{story_id}` | none | Full story; 404 if absent |
| `GET /api/v1/traces/{trace_id}` | none | Spans and logs from the raw tables; 404 if neither exists |
| `GET /api/v1/service-map` | `since` (default `1h`), `until` | `{edges, nodes}`: `edges` are service-to-service calls (`parent`, `child`, `calls`, `errors`, `error_rate`, `avg_duration_ns`); `nodes` are per-service RED summaries (`service`, `calls`, `rate`, `error_ratio`, `p99_ns`, `baseline_p99_ns`, `health`: `ok`, `slow` or `error`). Before plan 5 the body was a plain array of edges |
| `GET /api/v1/log-alerts` | `since` (default `24h`), `until`, `kind` (`new` or `spike`), `service` | Up to 200 alerts that overlap the window (seen after its start, started by its end), newest `last_at` first; each has `active` and `example_traces` (`[{trace_id, story_id \| null}]`) |
| `GET /api/v1/log-templates` | `since` (default `1h`), `until`, `service`, `q` (substring, at most 200 chars) | Top 200 templates with hits in the window, by count; each has `alerting`, plus `bucket_secs` (the window / 120, rounded up to whole minutes) and `buckets` (`[bucket_start_unix_s, hits]`, oldest first; buckets without hits are left out) |
| `GET /api/v1/log-templates/{id}` | `since` (default `24h`), `until` | One template with `buckets`, the 20 most recent hits up to the window's end and its alerts of the 7 days before that end |
| `GET /api/v1/traces/{trace_id}/log-templates` | none | `[{log_id, template_id, template, alert}]` for the trace's logs |
| `GET /api/v1/overview` | `since`, `until` | KPI values and bucket series for the Stories page |
| `GET /api/v1/stories/series` | `since`, `until`, `kind`, `service` | Stories per bucket, for charts |
| `GET /api/v1/traces/search` | `since`, `until`, `service`, `touched` (0 or 1), `endpoint`, `min_ms`, `max_ms`, `errors` (0 or 1), `limit` (1 to 500, default 100) | Trace rows for the explorer, newest first, each with `story_id` and `story_kind` when a story exists |
| `GET /api/v1/services` | none | Service names |
| `GET /api/v1/services/{name}` | `since`, `until` | RED series (rate, error ratio, p50/p95/p99) for one service |
| `GET /api/v1/search` | `q` | Command palette: matching services, templates and story groups; a trace id when `q` is 32 hex characters |
| `GET /api/v1/pipeline/series` | `metric`, `kind` (required), `job`, `labels` (`k=v`), `since`, `until` | A rate, gauge or quantile series from the recorded metrics |
| `GET /api/v1/pipeline/lag` | none | Consumer lag per group (committed, end offset, lag) |
| `GET /api/v1/config` | none | `{jaeger_url, grafana_url}` (null when unset) |
| `GET /healthz` | none | `ok` |
| `GET /metrics` | none | Prometheus metrics |

Errors on these routes are JSON `{"error": "..."}`. A ClickHouse failure returns 503. `GET /api/v1/traces/{trace_id}` also carries the extra fields the app uses (span attributes, resource, events, self time). `GET /api/v1/service-map` changed shape: it returns an object, not an array, so a client that read the old array must read `edges`.

App routes (client-side; every path below serves `index.html`, and the app renders the page):

| Route | Query | Page |
|---|---|---|
| `/` | `since`, `until`, `kind`, `service`, `group` | Stories |
| `/stories/{story_id}` | `since`, `until` | Story |
| `/traces`, `/traces/{trace_id}` | `since`, `until`, filters | Trace explorer, trace |
| `/map` | `since`, `until` | Service map |
| `/logs/alerts`, `/logs/templates`, `/logs/templates/{id}` | `since`, `until`, filters | Logs |
| `/pipeline` | `since`, `until` | Pipeline health |

Old server-rendered URLs (`/service-map`, `/alerts`, `/templates`, `/groups/…`) are not redirected; they show the app's not-found page.

Static files: `/assets/*` is served with `Cache-Control: public, max-age=31536000, immutable`; `index.html` with `no-cache`. A GET to a path the app does not know serves `index.html` with status 200 (the app shows its own not-found page). A missing `/api/*` route or `/assets/*` file returns a JSON 404.

## Verified

Rows above the `Plan 4` row were checked 2026-10-03 on branch `feat/plan-3-api-ui-e2e`; rows from the `Plan 4` row on were checked 2026-10-04 on branch `feat/plan-4-log-templates`; rows from the `Plan 5` row on were checked 2026-10-05 on branch `feat/plan-5-ui`. The stack was running for all three. Rows about the removed server-rendered pages are kept as history and marked **superseded**.

| Claim | How verified | Result |
|---|---|---|
| Port 8090 = tayga-api | `ports` in `deploy/compose.tayga.yaml`; default `http_addr` in `crates/tayga-api/src/main.rs`; live `curl localhost:8090/healthz` returned `ok` | verified |
| Port 3001 = Grafana | `ports` in `deploy/compose.tayga.yaml`; live `curl localhost:3001/login` returned HTTP 200 | verified |
| Port 19090 = Prometheus | `ports` in `deploy/compose.tayga.yaml`; live `curl localhost:19090/-/ready` returned "Prometheus Server is Ready." | verified |
| Port 18123 = ClickHouse HTTP | `ports` in `deploy/compose.infra.yaml`; live `curl localhost:18123/ping` returned `Ok.` | verified |
| Port 19092 = Redpanda Kafka API | `ports` in `deploy/compose.infra.yaml`; `docker ps` shows `127.0.0.1:19092->19092/tcp` | verified (port mapping only; no Kafka client connection made) |
| Tayga's ports bind 127.0.0.1; the demo publishes 8080, 9090, 10000 and ephemeral service ports on all interfaces | `lsof -nP -iTCP -sTCP:LISTEN` showed `127.0.0.1:8090`, `:3001`, `:19090`, `:19092`, `:18123` and `*:8080`, `*:9090`, `*:10000`, `*:574xx`, `*:627xx`, `*:648xx`; `docker ps` mapped the `*` listeners to containers of compose project `opentelemetry-demo` (frontend-proxy, prometheus, otel-collector, flagd and the demo services) | verified live 2026-10-03 |
| Port 8080 = demo frontend proxy | `docker ps` shows `frontend-proxy` on 8080; `curl localhost:8080/` returned HTTP 200; the compose definition is in the submodule, not read | verified live, definition not read |
| Jaeger UI at `/jaeger/ui` on 8080 | default `jaeger_url` in `crates/tayga-api/src/main.rs` and `TAYGA__JAEGER_URL` in compose | verified in config only, URL not fetched |
| Grafana anonymous Viewer, admin password `admin` | `GF_AUTH_ANONYMOUS_*`, `GF_SECURITY_ADMIN_PASSWORD` in `deploy/compose.tayga.yaml` | verified in config; login not tried; since plan 5 Grafana runs only after `make up-extras` |
| API and UI routes and their query parameters | `crates/tayga-api/src/routes.rs`, `ui.rs`, `params.rs` | verified in code; **superseded** for the HTML routes (`ui.rs` is removed, see the Plan 5 rows) |
| `since` range 1s-7d, defaults 1h / 24h / 1h | `parse_since`, `group_filter`, `group`, `service_map` | verified in code; live `since=8d` returned 400, `since=1h&kind=error` returned 200 |
| `kind` accepts only `error` or `slow` | `group_filter` in `params.rs` | verified in code |
| `/metrics` on tayga-api | `tayga_common::metrics::router` merged in `main.rs`; live `curl localhost:8090/metrics` returned Prometheus text | verified |
| Unknown route returns plain 404 | live `curl localhost:8090/nope` returned 404 (body not inspected); no fallback in the routers | verified status; "plain" inferred from code; **superseded**: unknown paths now serve the app (see the Plan 5 rows) |
| `/` UI returns 200 | live `curl localhost:8090/` | verified (still 200, now the web app) |
| `make up/down/ps/logs/flag/flags-reset/it/e2e/verify-raw/capture/infra-down` | `Makefile` | verified in Makefile; only `up`-state commands were observed, `make it`, `make e2e`, `make down`, `make infra-up` and flag changes were not run |
| `make it` conflicts with the full stack | both compose files publish 19092 and 18123 on the host (`compose.infra.yaml`) | inferred from port definitions, not run |
| Demo pinned to 3.1.0 | `.gitmodules`, `git submodule status` shows `(3.1.0)`; `DEMO_VERSION` in Makefile | verified |
| Architecture diagram | copied from spec section 3; services in `deploy/compose.tayga.yaml`; topic `tayga.signals` seen in the code and spec | topology matches compose services; topic names not checked against running Redpanda |
| Crate list | `crates/*/Cargo.toml` | verified |
| Performance, scale, or latency claims | none made | n/a |
| **Plan 4** | | |
| Port 14318 = tayga-ingest OTLP/HTTP | `"127.0.0.1:14318:4318"` in `deploy/compose.tayga.yaml`; `emit-log` default endpoint in `crates/tayga-devtools/src/main.rs`; the e2e probe sends through it | verified in config; the e2e probe's 45 s alert (see below) is evidence it works |
| `tayga-logminer` runs as one replica, metrics on 9100, scraped by Prometheus | service in `deploy/compose.tayga.yaml`; default `metrics_addr` in `crates/tayga-logminer/src/main.rs`; `deploy/prometheus/prometheus.yml`; live `docker ps` shows `tayga-logminer` Up; `/api/v1/targets` showed 6 jobs, all `up` (writer, ingest, logminer, assembler, api, redpanda) | verified |
| Rule defaults (5 min window, 60 min baseline, factor 5, min count 10, warmup 15 min measured back from the template's first log, active 10 min, new = first seen after the previous tick's data clock minus 60 s, start watermark 10 min before the data clock, 65 min = baseline + window) | `DetectConfig::default`, `min_age`, `is_new`, `initial_watermark` and `NEW_TEMPLATE_MARGIN_NS` in `crates/tayga-drain/src/detect.rs`, with exact-boundary unit tests; logminer settings take the same values (`LogminerSettings::default` and its test) | verified in code; the e2e scenarios exercise one new-template and one spike case, not every threshold |
| Drain defaults (similarity 0.5, depth 4, 100 children, 5,000 clusters per service, 64 tokens) | `DrainConfig::default` in `drain.rs`, `MAX_TOKENS` in `preprocess.rs` | verified in code |
| Flush at 5,000 logs or 1 s, detect every 60 s, topic `tayga.alerts` | `LogminerSettings::default` | verified in code |
| TTLs 3 d (hits) / 30 d (templates) / 7 d (alerts) | `TTL` lines in `crates/tayga-store/migrations/0004*` | verified in code |
| New routes and their defaults (`since` 24h / 1h / 24h, 200 alert and template limit, `q` at most 200 chars) | `routes.rs`, `ui.rs`, `params.rs`, `repo.rs` (limit 200 at `log_alerts` and `TEMPLATES_IN_WINDOW`) | verified in code |
| `/alerts` and `/templates` return 200; `/api/v1/log-alerts?since=24h` | live `curl`: 200, 200; 8 alerts in the last 24 h | verified live 2026-10-04; **superseded**: `/alerts` and `/templates` are no longer redirected and show the app's not-found page |
| About 60-120 templates | live `log_templates FINAL`: 293 rows in total (all ever mined, 30-day TTL), 72 with `last_seen` in the last hour; `/api/v1/log-templates?since=1h` returned 72 | verified live; the 60-120 range is the plan's estimate, the live hourly count (72) is inside it |
| Golden Drain test: 64 templates on the 5,000-line sample, `frontend-proxy` 5, bound is 120 and 10 | `cargo test -p tayga-drain --test '*' -- --nocapture` printed `templates: 64 {... "frontend-proxy": 5 ...}`, 3 passed | verified |
| Restoring the first half of the golden sample and mining the rest gives every line the same template id as one pass | `restore_mid_corpus_matches_a_single_pass` in `crates/tayga-drain/tests/golden.rs`. It fails (116 of 5,000 lines differ) when the restore is skipped. It still passes when clusters are restored in reverse order, so the sample does not exercise leaf-order ties | verified 2026-10-04 |
| Grafana `tayga-logs` ("Tayga · Logs") has 5 panels: Log alerts by kind, Recent alerts, New templates per service, Top templates, Logs mined/s | `/api/dashboards/uid/tayga-logs`, 2026-10-04 | verified live (rendering in a browser not checked) |
| `tayga-pipeline` has 13 panels, 5 of them logminer panels (logs mined/s, templates, cluster cap hits, detect p99, data lag) | `/api/dashboards/uid/tayga-pipeline` returned 13 panels, the last "Logminer data lag", 2026-10-04 | verified live (rendering in a browser not checked) |
| `tayga_logminer_data_lag_seconds` is exported and small while the logminer keeps up | Prometheus query after `make up` returned 0.38 s, and 0.44 s an hour later | verified live 2026-10-04 |
| A template that first appears while the logminer is stopped is reported after it restarts | `docker stop tayga-logminer` at 14:09:06; probe 1 emitted at 14:09:06; probe 2 emitted at 14:21:19, after 12 min; `docker start` at 14:21:19. Both `new` alerts were written on the first detection tick, at 14:22:19, 60 s after the start. Probe 1 was 13 min old by then, so the old wall-clock 10-minute rule would have dropped it | single run, verified live 2026-10-04 |
| Settings come from `TAYGA__SECTION__KEY` environment variables | `load_settings` in `crates/tayga-common/src/lib.rs:20-30` | verified in code |
| `new_template_recent_min` (10 min: the start watermark offset, the lag-warning threshold and the minimum example window) is not configurable | `LogminerSettings` has no such key; the value comes from `DetectConfig::default` | verified in code |
| e2e: 9 tests, 2 of them new; timeouts 180 s default, 600 s shipping, 600 s spike, 180 s new template | `cargo test -p tayga-e2e -- --ignored --list` listed 9; `crates/tayga-e2e/src/lib.rs` | verified |
| Latest `make e2e` run, 2026-10-04, on commit `7c0d35a`: 8 of 9 passed. `shipping_slowdown_produces_slow_story_blaming_shipping` found no shipping slow story within 600 s. Run alone right after, on the same commit, it passed in 40 s. Alert times: log spike 200 s; new-template probe 45 s, after a 556 s first-run warmup wait for the `tayga-e2e-probe` service | one `make e2e` run plus one single-test re-run | single run, not a latency guarantee; the shipping failure fits the rarity of international orders (see the `make e2e` row and followups) |
| Story scenario times in that run: ad 85 s, payment 95 s, unreachable 65 s, catalog 25 s, shipping failed (40 s in the re-run) | same run | single run; earlier runs differed (see followups) |
| Performance, scale, or latency claims | none made beyond the single-run timings above | n/a |
| **Plan 5 (web app)** | | |
| Rows below checked 2026-10-05 on branch `feat/plan-5-ui` at `870bfaa`, against the stack from `make up` (5 tayga containers running; Grafana and Prometheus not running) | | |
| App served on 8090: `/`, `/map`, `/logs`, `/logs/alerts`, `/pipeline`, `/traces` return 200 `text/html`; an unknown path (`/nope`) also returns 200 `text/html` | live `curl -D -` | verified |
| `/api/v1/nope` and `/assets/nope.js` return 404 `application/json`; `/metrics` returns 200 | live `curl` | verified |
| Redirects are 308 with the query kept: `/groups/1` to `/?group=%221%22`, `/service-map?since=1h` to `/map?since=1h`, `/alerts` to `/logs/alerts` | live `curl -D -` (the other two redirects, `/templates` and `/templates/{id}`, are in `OLD_URL_REDIRECTS` in `crates/tayga-api/src/spa.rs`, and the Task 13 report lists them live as 308) | verified live (3), in code and in the Task 13 report (2) |
| `GET /api/v1/config` returns `{"jaeger_url":"http://localhost:8080/jaeger/ui","grafana_url":null}` on a plain `make up` | live `curl` | verified |
| New API routes `overview`, `stories/series`, `traces/search`, `search?q=`, `pipeline/series` return 200; `services` returns a list of names; `pipeline/lag` returns three groups (writer, assembler, logminer) | live `curl` (`pipeline/series?metric=up&kind=gauge&job=tayga-api&since=15m`; `services/{name}` not called) | verified live |
| Route list, parameters and limits (`limit` 1 to 500, default 100; `touched`, `errors` flags; `kind` required for `pipeline/series`) | `crates/tayga-api/src/routes_v2.rs`, `params.rs` | verified in code |
| Immutable cache on `/assets/*`, `no-cache` on `index.html` | `IMMUTABLE` and `NO_CACHE` in `spa.rs`; Task 13 report shows live response headers (`cache-control: public, max-age=31536000, immutable` on the asset, `no-cache` on `/`) | verified in code and in the Task 13 report; not re-fetched today |
| Pages, paths, shortcuts (`g` then `s t m l p`, `?`, `Cmd/Ctrl+K`), theme cycle light, dark, system, time ranges 15m/1h/24h/7d, live refresh 10 s paused while hidden, palette contents | `ui/src/router.tsx`, `components/shell/{Shortcuts,CommandPalette,ThemeSwitch,TimeRange,LiveToggle}.tsx`, `app/search.ts`, `theme/theme.ts` | verified in code; not clicked through in a browser today (the Playwright suite in the Task 14 report covers pages, theme switch and palette) |
| Recorder: every 15 s, 7-day TTL, targets in `deploy/tayga-api.toml` via `TAYGA_CONFIG`, not settable by `TAYGA__` env | `default_record_secs` in `crates/tayga-api/src/main.rs`; `TTL ... INTERVAL 7 DAY` in `0005_metric_samples.sql`; Task 13 report (config 0.15.27 rejected the env form with `invalid type: map, expected a sequence`) | verified in code; the env failure is cited from the Task 13 report, not re-run |
| Grafana and Prometheus are the compose profile `extras`; `make up-extras` starts them and sets the Grafana link; `make down` removes them | `Makefile`, `deploy/compose.tayga.yaml`, `deploy/compose.extras.yaml`; Task 13 report (live: `up-extras` gave `grafana_url` set, Prometheus ready, 4 dashboards provisioned; a second `make up` left them running) | verified in files; live results cited from the Task 13 report; `make up-extras` not run today |
| Node 24 or newer only for UI development; Docker builds with `node:24`; `make ui-dev`, `make ui-e2e` | `engines` in `ui/package.json`; first stage of `docker/Dockerfile`; `Makefile`; `node --version` here prints v24.18.0 | verified |
| Without `ui/dist`, `tayga-api` compiles and serves a placeholder | `spa.rs` module comment, `PLACEHOLDER`, `allow_missing`; Task 4 ledger ruling | verified in code; a build without `ui/dist` not run today |
| Dev server proxies to 8090 by default, `TAYGA_API` overrides | `ui/vite.config.ts` | verified in code |
| UI unit tests: 371 pass | `npm --prefix ui test` run today: `Tests 371 passed (371)` | verified |
| Playwright suite: 139 tests in 11 files across 4 projects (dark, light, reduced motion in each theme) plus the perf project | Full run on 2026-10-05 against the 8090 app rebuilt from the final branch code (`make up`, image built 08:50 UTC): 139 passed, 12 skipped, 0 failed in 1.3 min. The 12 skips are the motion specs, which run only in the reduced-motion projects | one full run |
| Budgets (spec section 10), final run on 2026-10-05, unthrottled on the development machine against the live stack, medians of 5 cold-cache contexts: initial JS 185.2 KB gzip (limit 350 KB); JS fetched by a cold home load 231.1 KB (350 KB); home first render with data 146 ms (1000 ms); waterfall of the largest live trace (105 spans) 25 ms and of 5,000 synthetic spans 32 ms (200 ms); map layout 143 ms live and 181 ms for 60 synthetic nodes (300 ms); live refresh gaps 10071 and 10048 ms; 0 requests in 13 s while the tab is hidden | Playwright `perf` project output of that run | measured once on one machine, not a guarantee |
| Last full `make e2e`, 2026-10-05, after the scenario fixes in `3d0d8a5`: 8 of 9 passed. `shipping_slowdown_produces_slow_story_blaming_shipping` stopped at its baseline pre-check: two shipping runs in the previous hour lifted checkout p99 to 1.26 s, and each checkout endpoint had 35 to 37 traces in the window, below the 50 the detector needs. Run alone about 20 minutes earlier, with a clean baseline, it passed in 160 s. The earlier Task 14 timeouts came from the demo checkout service running about 100 times slower after a Docker restart; restarting `checkout` and `load-generator` fixed it (see followups) | one full run plus single-test runs | the shipping scenario needs an hour without slowdown runs and enough checkout traffic |
| Performance, scale, or latency claims | none beyond the cited budgets and single runs above | n/a |
