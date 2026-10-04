# Tayga

Tayga turns OpenTelemetry traces and logs into "error stories". For each failing or slow request it shows the root-cause span, the request path across services, the critical path, a diff against the endpoint's normal baseline, and the related logs. Stories are grouped by fingerprint, so one underlying problem shows up as one group rather than as hundreds of traces.

It runs next to the [OpenTelemetry demo](https://github.com/open-telemetry/opentelemetry-demo) (vendored as a git submodule, pinned to 3.1.0). The demo's collector forwards OTLP to Tayga; Tayga stores raw spans and logs in ClickHouse, assembles traces, and serves a web UI and JSON API.

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
                         tayga-api (JSON + web UI)                       Redpanda topic `tayga.stories`
                         tayga-grafana (dashboards)

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

`make up` initializes the submodule, builds the `tayga:dev` image, and starts the demo plus Tayga.

- Stories UI: http://localhost:8090
- Grafana: http://localhost:3001 (anonymous Viewer access is enabled; admin password is `admin`)
- Prometheus: http://localhost:19090
- Demo shop: http://localhost:8080

Trigger a failure with a demo feature flag, then watch a story appear:

```sh
make flag NAME=paymentFailure VARIANT=100%
make flags-reset        # restore the demo's default flags
```

Other targets: `make ps`, `make logs SERVICE=<name>`, `make down`.

## Log templates and alerts

`tayga-logminer` reads the logs from `tayga.signals` (its own consumer group) and groups them into templates with the Drain algorithm, one tree per service. A body is first masked (UUIDs, hex runs of 8 or more characters and numbers become `<*>`), so `Found 3 products from database` and `Found 12 products from database` are one template, `Found <*> products from database`. A template keeps its id when it generalizes. Because numbers are masked, an HTTP status code in an access log is a wildcard: a burst of 500s in Envoy logs is not separated from 200s (spans cover HTTP errors).

Per log, one row in `log_template_hits` (3-day TTL); per template, one row in `log_templates` (30-day TTL); alerts in `log_alerts` (7-day TTL) and as JSON on the topic `tayga.alerts`. Counting uses `uniqExact(log_id)`, so replayed records are not counted twice. Notification delivery (webhook, Slack) is not implemented.

Every 60 seconds the logminer runs two rules over `log_template_hits`:

| Rule | Fires when | Default |
|---|---|---|
| New template | the template's first log is within the last 10 minutes (fixed, not configurable), its service already has a template at least `new_template_warmup_min` old (so a fresh install or new service does not flood), and it is not `<overflow>`. One alert per template, ever | warmup 15 min |
| Rate spike | the count in the last `spike_window_min` is at least `spike_min_count` and at least `spike_factor` times the mean per-window count over the preceding `baseline_window_min` (floored at 1), and the template is older than window + baseline (65 min; younger ones are covered by the new-template rule) | window 5 min, baseline 60 min, factor 5, min count 10 |

A spike alert stays active while it was last confirmed within `alert_active_min` (10) of now; a tick that still fires updates it, otherwise a new alert starts. Each alert carries up to 5 example trace ids (newest first) that link to the error story when one exists, else to Jaeger. The other values are `TAYGA__LOGMINER__*` environment variables (keys in `LogminerSettings` in `crates/tayga-logminer/src/main.rs`). Other defaults: similarity threshold 0.5, at most 5,000 clusters per service (beyond that, unmatched logs go to the `<overflow>` template), flush at 5,000 logs or 1 s.

Where to look:

- `/alerts`: alerts with kind badge, count against baseline and example traces.
- `/templates` and `/templates/{id}`: templates by count, search, sparkline, sample and recent hits.
- Story pages: the log table has a Template column, with a `new` or `spike` badge when the template was alerting at the story's time.
- Grafana "Tayga · Logs" dashboard (`tayga-logs`) and logminer panels in "Tayga · Pipeline health".

Limits: one logminer replica only (Drain state is global per service while records are partitioned by trace); no seasonal baselines; no alert when a template disappears; history is not re-mined. Open questions are in `docs/superpowers/followups.md`.

To probe the new-template rule by hand (the stack's ingest listens on 14318):

```sh
cargo run -p tayga-devtools -- emit-log --service tayga-e2e-probe --body "hello probe marker"
```

The command prints the trace id it used. An alert fires only if the service already had a template 15 minutes before. The e2e `new_template_from_probe` logs under its own service, `tayga-e2e-probe`, never a demo service. Every run emits a constant seed log, `tayga e2e probe seed`, and then a probe `{word} probe … probe marker` with a random 12-letter word and 4 to 58 tokens. The first run on a stack waits up to 16 minutes for the seed template to age past the warmup. Drain routes on the token count, then on the first word, so the probes spread over about 55 nodes of 100 children: about 5,500 runs fit in the 30-day template TTL before probes start merging and the test fails.

## Developer commands

| Command | What it does |
|---|---|
| `cargo test --workspace` | Unit tests (integration and e2e tests are `#[ignore]`d) |
| `make it` | Starts Redpanda + ClickHouse standalone (compose project `tayga-it`) and runs the ignored integration tests (all crates except `tayga-e2e`; the ClickHouse tests each seed a uniquely named database). Run it with the full stack down: both use the same host ports 19092 and 18123 |
| `make infra-down` | Stops the standalone infra and removes its volumes |
| `make e2e` | Resets flags, then runs the end-to-end tests against the live stack (`make up` first). Nine tests: the seven from before (flag-driven story scenarios for payment, payment unreachable, shipping, product catalog and ad, the service map, and raw span counts versus Jaeger) plus `log_spike_on_payment_failure` and `new_template_from_probe`. The spike scenario fails up front if a payment spike alert is still active from an earlier run (wait about 10 minutes). The first run on a fresh stack waits up to 16 more minutes for the probe service warmup |
| `make verify-raw` | Compares per-trace span counts in ClickHouse with the demo's Jaeger |
| `make capture NAME=<n> ARGS="<args>"` | Captures a fixture to `fixtures/<n>.pb.gz` (see `cargo run -p tayga-devtools -- capture --help`) |

## Ports

Tayga's own published ports (8090, 3001, 19090, 19092, 18123, 14318) are bound to 127.0.0.1. The upstream OpenTelemetry demo is not: it publishes 8080 (frontend proxy), 9090 (the demo's Prometheus), 10000 (Envoy admin) and 26 other container ports (on ephemeral host ports, counted on demo 3.1.0) on all interfaces, so they are reachable from your network. Run the stack only on a trusted network, or firewall those ports.

| Port | Service | Defined in |
|---|---|---|
| 8080 | OTel demo frontend proxy (shop, Jaeger UI under `/jaeger/ui`) | demo compose (not published by Tayga's compose files) |
| 8090 | tayga-api: web UI, JSON API, `/healthz`, `/metrics` | `deploy/compose.tayga.yaml` |
| 3001 | Grafana (container port 3000) | `deploy/compose.tayga.yaml` |
| 19090 | Prometheus (container port 9090) | `deploy/compose.tayga.yaml` |
| 19092 | Redpanda Kafka API (external listener) | `deploy/compose.infra.yaml` |
| 18123 | ClickHouse HTTP (container port 8123) | `deploy/compose.infra.yaml` |
| 14318 | tayga-ingest OTLP/HTTP (container port 4318), used by `tayga-devtools emit-log` | `deploy/compose.tayga.yaml` |

## HTTP routes (tayga-api, port 8090)

`since` takes `<n>[smhd]`, from `1s` to `7d`. Invalid values return 400. Fingerprints are decimal u64 strings; story and trace ids are 32 hex characters.

JSON API:

| Route | Query | Returns |
|---|---|---|
| `GET /api/v1/story-groups` | `since` (default `1h`), `kind` (`error` or `slow`), `service` | Top 100 story groups, each with `buckets` (`[bucket_start_unix_s, stories]`, about 120 per window) and `bucket_secs` |
| `GET /api/v1/story-groups/{fingerprint}` | `since` (default `24h`) | One group with example stories; 404 if absent |
| `GET /api/v1/stories/{story_id}` | none | Full story; 404 if absent |
| `GET /api/v1/traces/{trace_id}` | none | Spans and logs from the raw tables; 404 if neither exists |
| `GET /api/v1/service-map` | `since` (default `1h`) | Service-to-service edges |
| `GET /api/v1/log-alerts` | `since` (default `24h`), `kind` (`new` or `spike`), `service` | Up to 200 alerts, newest `last_at` first; each has `active` and `example_traces` (`[{trace_id, story_id \| null}]`) |
| `GET /api/v1/log-templates` | `since` (default `1h`), `service`, `q` (substring, at most 200 chars) | Top 200 templates with hits in the window, by count; each has `alerting` |
| `GET /api/v1/log-templates/{id}` | `since` (default `24h`) | One template with `buckets`, the 20 most recent hits and its alerts |
| `GET /api/v1/traces/{trace_id}/log-templates` | none | `[{log_id, template_id, template, alert}]` for the trace's logs |
| `GET /healthz` | none | `ok` |
| `GET /metrics` | none | Prometheus metrics |

Errors on these routes are JSON `{"error": "..."}`. A ClickHouse failure returns 503.

HTML UI:

| Route | Query | Page |
|---|---|---|
| `GET /` | `since`, `kind`, `service` | Story groups |
| `GET /groups/{fingerprint}` | `since` | Group detail |
| `GET /stories/{story_id}` | none | Story detail |
| `GET /service-map` | `since` | Service map |
| `GET /alerts` | `since` (default `24h`), `kind`, `service` | Log alerts |
| `GET /templates` | `since` (default `1h`), `service`, `q` | Log templates |
| `GET /templates/{id}` | `since` (default `24h`) | Template detail |

Unknown routes return axum's plain 404.

## Verified

Rows above the `Plan 4` row were checked 2026-10-03 on branch `feat/plan-3-api-ui-e2e`; rows from the `Plan 4` row on were checked 2026-10-04 on branch `feat/plan-4-log-templates`. The stack was running for both.

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
| Grafana anonymous Viewer, admin password `admin` | `GF_AUTH_ANONYMOUS_*`, `GF_SECURITY_ADMIN_PASSWORD` in `deploy/compose.tayga.yaml` | verified in config; login not tried |
| API and UI routes and their query parameters | `crates/tayga-api/src/routes.rs`, `ui.rs`, `params.rs` | verified in code |
| `since` range 1s-7d, defaults 1h / 24h / 1h | `parse_since`, `group_filter`, `group`, `service_map` | verified in code; live `since=8d` returned 400, `since=1h&kind=error` returned 200 |
| `kind` accepts only `error` or `slow` | `group_filter` in `params.rs` | verified in code |
| `/metrics` on tayga-api | `tayga_common::metrics::router` merged in `main.rs`; live `curl localhost:8090/metrics` returned Prometheus text | verified |
| Unknown route returns plain 404 | live `curl localhost:8090/nope` returned 404 (body not inspected); no fallback in the routers | verified status; "plain" inferred from code |
| `/` UI returns 200 | live `curl localhost:8090/` | verified |
| `make up/down/ps/logs/flag/flags-reset/it/e2e/verify-raw/capture/infra-down` | `Makefile` | verified in Makefile; only `up`-state commands were observed, `make it`, `make e2e`, `make down`, `make infra-up` and flag changes were not run |
| `make it` conflicts with the full stack | both compose files publish 19092 and 18123 on the host (`compose.infra.yaml`) | inferred from port definitions, not run |
| Demo pinned to 3.1.0 | `.gitmodules`, `git submodule status` shows `(3.1.0)`; `DEMO_VERSION` in Makefile | verified |
| Architecture diagram | copied from spec section 3; services in `deploy/compose.tayga.yaml`; topic `tayga.signals` seen in the code and spec | topology matches compose services; topic names not checked against running Redpanda |
| Crate list | `crates/*/Cargo.toml` | verified |
| Performance, scale, or latency claims | none made | n/a |
| **Plan 4** | | |
| Port 14318 = tayga-ingest OTLP/HTTP | `"127.0.0.1:14318:4318"` in `deploy/compose.tayga.yaml`; `emit-log` default endpoint in `crates/tayga-devtools/src/main.rs`; the e2e probe sends through it | verified in config; the e2e probe's 60 s alert (see below) is evidence it works |
| `tayga-logminer` runs as one replica, metrics on 9100, scraped by Prometheus | service in `deploy/compose.tayga.yaml`; default `metrics_addr` in `crates/tayga-logminer/src/main.rs`; `deploy/prometheus/prometheus.yml`; live `docker ps` shows `tayga-logminer` Up; `/api/v1/targets` showed 6 jobs, all `up` (writer, ingest, logminer, assembler, api, redpanda) | verified |
| Rule defaults (5 min window, 60 min baseline, factor 5, min count 10, warmup 15 min, active 10 min, new within 10 min, 65 min = baseline + window) | `DetectConfig::default` and `min_age` in `crates/tayga-drain/src/detect.rs`; logminer settings take the same values (`LogminerSettings::default` and its test) | verified in code; the e2e scenarios exercise one new-template and one spike case, not every threshold |
| Drain defaults (similarity 0.5, depth 4, 100 children, 5,000 clusters per service, 64 tokens) | `DrainConfig::default` in `drain.rs`, `MAX_TOKENS` in `preprocess.rs` | verified in code |
| Flush at 5,000 logs or 1 s, detect every 60 s, topic `tayga.alerts` | `LogminerSettings::default` | verified in code |
| TTLs 3 d (hits) / 30 d (templates) / 7 d (alerts) | `TTL` lines in `crates/tayga-store/migrations/0004*` | verified in code |
| New routes and their defaults (`since` 24h / 1h / 24h, 200 alert and template limit, `q` at most 200 chars) | `routes.rs`, `ui.rs`, `params.rs`, `repo.rs` (limit 200 at `log_alerts` and `TEMPLATES_IN_WINDOW`) | verified in code |
| `/alerts` and `/templates` return 200; `/api/v1/log-alerts?since=24h` | live `curl`: 200, 200; 8 alerts in the last 24 h | verified live 2026-10-04 |
| About 60-120 templates | live `log_templates FINAL`: 293 rows in total (all ever mined, 30-day TTL), 72 with `last_seen` in the last hour; `/api/v1/log-templates?since=1h` returned 72 | verified live; the 60-120 range is the plan's estimate, the live hourly count (72) is inside it |
| Golden Drain test: 64 templates on the 5,000-line sample, `frontend-proxy` 5, bound is 120 and 10 | `cargo test -p tayga-drain --test '*' -- --nocapture` printed `templates: 64 {... "frontend-proxy": 5 ...}`, 2 passed | verified |
| Grafana `tayga-logs` ("Tayga · Logs") has 5 panels: Log alerts by kind, Recent alerts, New templates per service, Top templates, Logs mined/s | `/api/dashboards/uid/tayga-logs`, 2026-10-04 | verified live (rendering in a browser not checked) |
| `tayga-pipeline` has 12 panels, 4 of them logminer panels (logs mined/s, templates, cluster cap hits, detect p99) | `/api/dashboards/uid/tayga-pipeline`, 2026-10-04 | verified live |
| Settings come from `TAYGA__SECTION__KEY` environment variables | `load_settings` in `crates/tayga-common/src/lib.rs:20-30` | verified in code |
| The new-template 10-minute window is not configurable | `LogminerSettings` has no such key; `new_template_recent_min` comes from `DetectConfig::default` | verified in code |
| e2e: 9 tests, 2 of them new; timeouts 180 s default, 600 s shipping, 600 s spike, 180 s new template | `cargo test -p tayga-e2e -- --ignored --list` listed 9; `crates/tayga-e2e/src/lib.rs` | verified |
| Latest `make e2e` run, 2026-10-04: 9 of 9 passed. Alert times: log spike 316 s (245 s in a later single run), new-template probe 60 s | measured in one `make e2e` run on 2026-10-04 (controller-supervised); not re-run for this README | single run, not re-verified; not a latency guarantee |
| Story scenario times in that run: ad 80 s, payment 95 s, unreachable 100 s, catalog 25 s, shipping 206 s | same run | measured in one `make e2e` run on 2026-10-04 (controller-supervised); not re-verified; earlier runs differed (see followups) |
| Performance, scale, or latency claims | none made beyond the single-run timings above | n/a |
