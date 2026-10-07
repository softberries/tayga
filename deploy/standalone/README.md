# Tayga standalone (Docker Compose)

Tayga, ClickHouse and Redpanda on one Docker host, without the OpenTelemetry demo. You send it OTLP traces and logs from your own services or collector.

| Service | What it does | Published on the host |
|---|---|---|
| `tayga-ingest` | OTLP receiver: gRPC on 4317, HTTP on 4318 (`/v1/traces`, `/v1/logs`) | `TAYGA_OTLP_GRPC_PORT` (4317), `TAYGA_OTLP_HTTP_PORT` (4318) |
| `tayga-api` | Web app and JSON API (`/healthz`, `/metrics`, `/api/v1/...`) | `TAYGA_HTTP_PORT` (8090) |
| `tayga-writer`, `tayga-assembler`, `tayga-logminer`, `tayga-notifier` | Raw storage, error stories, log templates and alerts, alert delivery | no |
| `tayga-migrate` | One-shot ClickHouse schema migration; the services above wait for it | no |
| `clickhouse` | `clickhouse/clickhouse-server:26.8`, volume `clickhouse-data` | no |
| `redpanda` | `redpandadata/redpanda:v26.2.3`, one broker in `dev-container` mode, volume `redpanda-data` | no |

Requirements: Docker with the Compose v2 plugin, and disk for ClickHouse (raw spans and logs are kept 3 days). Memory: Redpanda may use up to `REDPANDA_MEMORY` (1 GB by default). With a little test traffic the whole stack used about 1.2 GB (ClickHouse about 950 MB, Redpanda about 220 MB, each Tayga service under 10 MB; measured with `docker stats` on 2026-10-07, Docker Desktop on Apple silicon); real traffic needs more.

## Install

With the installer (it checks Docker and the ports, downloads this bundle for a release, starts the stack, waits for health and prints the URLs):

```sh
curl -fsSL https://raw.githubusercontent.com/softberries/tayga/master/scripts/install.sh | sh
# or pin a release and a directory:
curl -fsSL https://raw.githubusercontent.com/softberries/tayga/master/scripts/install.sh | sh -s -- --version 0.1.0 --dir ~/tayga
```

| Option | Meaning |
|---|---|
| `--version V` | Release to install (`0.1.0` or `v0.1.0`). Default: the latest release |
| `--dir DIR` | Install directory. Default: `$TAYGA_DIR`, else `~/tayga` |
| `--project NAME` | Compose project name (default `tayga`); prefixes the containers, network and volumes |
| `--ports N` | Adds `N` to every published port: `--ports 10000` gives 18090, 14317 and 14318 |
| `--bind ADDR` | Address the ports bind to (default `127.0.0.1`) |
| `--local` | From a checkout: use its `deploy/standalone` and a local image `tayga:local` (built from `docker/Dockerfile` when missing) |
| `--uninstall` | Remove the containers and network; keep the volumes and the directory |
| `--purge` | With `--uninstall`: also delete the volumes and the directory |

Re-running the installer is safe: it updates the settings it manages in `.env` (`TAYGA_VERSION`, the ports, the bind address, the project name) and runs `docker compose up -d` again. It copies itself to `<dir>/install.sh`, so `sh ~/tayga/install.sh --uninstall` works later. It replaces `compose.yaml`, but never your edited `notifier.toml` or `otel-collector.yaml` (the new copies go next to them as `*.new`).

Without the installer:

```sh
cp .env.example .env     # set TAYGA_VERSION to a release
docker compose up -d
docker compose ps        # tayga-api reads (healthy) once it serves
```

From a source checkout, with an image you build yourself:

```sh
docker build -f docker/Dockerfile -t tayga:local .
sh scripts/install.sh --local
```

Open the app at http://localhost:8090. It is empty until telemetry arrives.

## Send telemetry

Tayga reads OTLP traces and logs; it ignores metrics. Point an exporter at the ingest ports:

- SDKs on the same host: `OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318` (HTTP) or `http://localhost:4317` (gRPC).
- An OpenTelemetry Collector: add the exporter below and list it in your `traces` and `logs` pipelines. `otel-collector.yaml` in this directory is a complete config.

```yaml
exporters:
  otlp_grpc/tayga:          # `otlp` in collector releases before the exporter was renamed
    endpoint: localhost:4317   # tayga-ingest:4317 from a container on this compose network
    tls:
      insecure: true
    compression: gzip
    timeout: 30s
service:
  pipelines:
    traces:
      exporters: [otlp_grpc/tayga]   # keep your other exporters in the list
    logs:
      exporters: [otlp_grpc/tayga]
```

To run the sample collector next to the stack (project `tayga`), receiving on 127.0.0.1:5317 (gRPC) and 5318 (HTTP):

```sh
docker run -d --name tayga-otelcol --network tayga_default \
  -p 127.0.0.1:5317:4317 -p 127.0.0.1:5318:4318 \
  -v "$PWD/otel-collector.yaml:/etc/otelcol-contrib/config.yaml:ro" \
  otel/opentelemetry-collector-contrib:0.162.0
```

A quick test with [telemetrygen](https://github.com/open-telemetry/opentelemetry-collector-contrib/tree/main/cmd/telemetrygen) (traces with error spans, then logs):

```sh
docker run --rm --network host ghcr.io/open-telemetry/opentelemetry-collector-contrib/telemetrygen:latest \
  traces --otlp-endpoint localhost:4317 --otlp-insecure --traces 20 --child-spans 3 --status-code Error --service checkout
docker run --rm --network host ghcr.io/open-telemetry/opentelemetry-collector-contrib/telemetrygen:latest \
  logs --otlp-endpoint localhost:4317 --otlp-insecure --logs 50 --service checkout
```

`--network host` reaches `localhost` only on Linux. With Docker Desktop, drop `--network host` and use `--otlp-endpoint host.docker.internal:4317`, or use `--network tayga_default` and `--otlp-endpoint tayga-ingest:4317`. Error stories appear on the Stories page a few seconds after a trace ends (the assembler closes a trace 10 s after its last span); log templates appear under Logs within a few seconds.

## Configuration

Everything is set in `.env` (start from `.env.example`). After a change, run `docker compose up -d` (or the installer again).

| Variable | Default | Meaning |
|---|---|---|
| `COMPOSE_PROJECT_NAME` | `tayga` | Compose project name |
| `TAYGA_VERSION` | `latest` | Image tag of `ghcr.io/softberries/tayga`. Pin a release |
| `TAYGA_IMAGE` | unset | A whole image reference that replaces the GHCR image, for example `tayga:local` |
| `TAYGA_BIND` | `127.0.0.1` | Address the published ports bind to |
| `TAYGA_HTTP_PORT` | `8090` | Host port of the web app and API |
| `TAYGA_OTLP_GRPC_PORT`, `TAYGA_OTLP_HTTP_PORT` | `4317`, `4318` | Host ports of the OTLP receivers |
| `TAYGA_PUBLIC_URL` | `http://localhost:8090` | Base of the links in alert notifications |
| `LOGMINER_REPLICAS` | `1` | Logminer replicas (`tayga.logs` has 12 partitions; more replicas than that idle) |
| `LOGMINER_FINGERPRINTER` | `scalar` | Fingerprint cache in front of Drain: `scalar`, `parallel` or `off` |
| `REDPANDA_MEMORY` | `1G` | Redpanda's memory (Seastar `--memory`) |
| `RUST_LOG` | `info` | Log filter of the Tayga services |
| `TAYGA_AUTH_*` | off | Login; see [Authentication](#authentication) |

Any other Tayga setting can be added as a `TAYGA__SECTION__KEY` environment variable in a `compose.override.yaml` next to `compose.yaml` (compose loads it automatically). The full list of settings is in the configuration reference of the documentation.

## Exposing the ports

By default every published port binds to 127.0.0.1, so only this machine reaches Tayga. To reach it from other machines, set `TAYGA_BIND=0.0.0.0` (or one interface's address) in `.env` and run `docker compose up -d`, or install with `--bind 0.0.0.0`.

Before you do:

- Turn on [authentication](#authentication), or put an authenticating reverse proxy in front of port 8090. Without it anyone who reaches the port can read every trace and log.
- The OTLP ports accept data from anyone who reaches them, with no authentication and no TLS. Prefer a collector on the trusted side that forwards to Tayga, or a firewall that only admits your senders.
- Tayga serves plain HTTP. For HTTPS, terminate TLS in a reverse proxy and set `TAYGA_AUTH_SECURE_COOKIE=true`.

ClickHouse and Redpanda are never published. ClickHouse's `default` user has no password (Tayga does not support ClickHouse credentials yet); only containers on the compose network can reach it.

## Authentication

Off by default. To turn it on, put the settings in `.env` and run `docker compose up -d`:

```sh
TAYGA_AUTH_ENABLED=true
TAYGA_AUTH_USERNAME=admin
TAYGA_AUTH_PASSWORD_HASH='$argon2id$v=19$m=19456,t=2,p=1$...'
TAYGA_AUTH_SESSION_KEY=<output of: openssl rand -base64 32>
```

Keep the single quotes around the hash: it contains `$`. The hash is an Argon2id PHC string. Make it from a source checkout with `cargo run -q -p tayga-devtools -- hash-password`, or without Rust with the `argon2` tool from Debian (the API accepts any Argon2id PHC string):

```sh
printf '%s' 'my password' | docker run --rm -i debian:trixie-slim sh -c \
  'apt-get update -qq >/dev/null && apt-get install -qq -y argon2 >/dev/null && argon2 "$(head -c 16 /dev/urandom | base64)" -id -m 16 -t 3 -p 1 -e'
```

## Operations

```sh
docker compose ps                         # status; tayga-api and tayga-ingest have healthchecks
docker compose logs -f tayga-api          # logs of one service
docker compose restart tayga-notifier     # after editing notifier.toml
docker compose exec clickhouse clickhouse-client   # SQL on the tayga database
```

- **Alert delivery:** add webhook or Slack targets to `notifier.toml` and restart `tayga-notifier`. The file holds secrets (a Slack webhook URL is a credential).
- **Upgrade:** set `TAYGA_VERSION` to the new release (or re-run the installer with `--version`) and run `docker compose up -d`. `tayga-migrate` runs first and the other services wait for it. Reload open browser tabs afterwards.
- **Retention:** raw spans and logs keep 3 days, trace summaries 2 days, stories, service edges and alerts 7 days (ClickHouse TTLs). Redpanda topics keep 24 h.
- **Uninstall:** `docker compose down` keeps the volumes; `docker compose down -v` deletes them. The installer's `--uninstall` and `--uninstall --purge` do the same.
