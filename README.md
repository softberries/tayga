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
```

Workspace crates (`crates/`): `tayga-ingest`, `tayga-writer`, `tayga-assembler`, `tayga-api` (services); `tayga-analysis`, `tayga-model`, `tayga-kafka`, `tayga-store`, `tayga-common` (libraries); `tayga-devtools` (flag, capture, verify-raw CLI); `tayga-e2e` (end-to-end tests).

## Quick start

Requires Docker with Compose, `make`, and a Rust toolchain (for the dev commands).

```sh
git clone --recurse-submodules <repo-url> tayga
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

## Developer commands

| Command | What it does |
|---|---|
| `cargo test --workspace` | Unit tests (integration and e2e tests are `#[ignore]`d) |
| `make it` | Starts Redpanda + ClickHouse standalone (compose project `tayga-it`) and runs ignored integration tests. Run it with the full stack down: both use the same host ports 19092 and 18123 |
| `make infra-down` | Stops the standalone infra and removes its volumes |
| `make e2e` | Resets flags, then runs the flag-driven end-to-end tests against the live stack (`make up` first) |
| `make verify-raw` | Compares per-trace span counts in ClickHouse with the demo's Jaeger |
| `make capture NAME=<n> ARGS="<args>"` | Captures a fixture to `fixtures/<n>.pb.gz` (see `cargo run -p tayga-devtools -- capture --help`) |

## Ports

All published ports are bound to 127.0.0.1, except the demo's 8080.

| Port | Service | Defined in |
|---|---|---|
| 8080 | OTel demo frontend proxy (shop, Jaeger UI under `/jaeger/ui`) | demo compose (not published by Tayga's compose files) |
| 8090 | tayga-api: web UI, JSON API, `/healthz`, `/metrics` | `deploy/compose.tayga.yaml` |
| 3001 | Grafana (container port 3000) | `deploy/compose.tayga.yaml` |
| 19090 | Prometheus (container port 9090) | `deploy/compose.tayga.yaml` |
| 19092 | Redpanda Kafka API (external listener) | `deploy/compose.infra.yaml` |
| 18123 | ClickHouse HTTP (container port 8123) | `deploy/compose.infra.yaml` |

## HTTP routes (tayga-api, port 8090)

`since` takes `<n>[smhd]`, from `1s` to `7d`. Invalid values return 400. Fingerprints are decimal u64 strings; story and trace ids are 32 hex characters.

JSON API:

| Route | Query | Returns |
|---|---|---|
| `GET /api/v1/story-groups` | `since` (default `1h`), `kind` (`error` or `slow`), `service` | Story groups with per-minute counts |
| `GET /api/v1/story-groups/{fingerprint}` | `since` (default `24h`) | One group with example stories; 404 if absent |
| `GET /api/v1/stories/{story_id}` | none | Full story; 404 if absent |
| `GET /api/v1/traces/{trace_id}` | none | Spans and logs from the raw tables; 404 if neither exists |
| `GET /api/v1/service-map` | `since` (default `1h`) | Service-to-service edges |
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

Unknown routes return axum's plain 404.

## Verified

Checked 2026-10-03 on branch `feat/plan-3-api-ui-e2e`, with the stack running.

| Claim | How verified | Result |
|---|---|---|
| Port 8090 = tayga-api | `ports` in `deploy/compose.tayga.yaml`; default `http_addr` in `crates/tayga-api/src/main.rs`; live `curl localhost:8090/healthz` returned `ok` | verified |
| Port 3001 = Grafana | `ports` in `deploy/compose.tayga.yaml`; live `curl localhost:3001/login` returned HTTP 200 | verified |
| Port 19090 = Prometheus | `ports` in `deploy/compose.tayga.yaml`; live `curl localhost:19090/-/ready` returned "Prometheus Server is Ready." | verified |
| Port 18123 = ClickHouse HTTP | `ports` in `deploy/compose.infra.yaml`; live `curl localhost:18123/ping` returned `Ok.` | verified |
| Port 19092 = Redpanda Kafka API | `ports` in `deploy/compose.infra.yaml`; `docker ps` shows `127.0.0.1:19092->19092/tcp` | verified (port mapping only; no Kafka client connection made) |
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
