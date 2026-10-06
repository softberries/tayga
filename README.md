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
                                     └─► Redpanda topic `tayga.alerts` ──► tayga-notifier ──► webhook and Slack targets
```

Workspace crates (`crates/`): `tayga-ingest`, `tayga-writer`, `tayga-assembler`, `tayga-logminer`, `tayga-notifier`, `tayga-api` (services); `tayga-analysis`, `tayga-drain` (Drain mining and alert rules, no I/O), `tayga-model`, `tayga-kafka`, `tayga-store`, `tayga-common` (libraries); `tayga-devtools` (flag, capture, verify-raw, emit-log, hash-password, remine CLI); `tayga-e2e` (end-to-end tests).

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
| Service map | `/map` | Services and their calls with health, rate, error ratio and p99; a node opens a drawer with RED charts and related stories. Infrastructure services are hidden unless "Show infrastructure" is on (see below) |
| Login | `/login` | Only when authentication is on (see [Authentication](#authentication)) |
| Logs | `/logs/alerts`, `/logs/templates`, `/logs/templates/{id}` | Log alerts, templates and a template's detail (`/logs` redirects to the alerts tab) |
| Pipeline | `/pipeline` | Component status, metric history charts and consumer lag, from the recorder below; needs no Prometheus |

Header and shortcuts:

- Service map: infrastructure services (`flagd` by default) are hidden unless the "Show infrastructure" switch beside the search box is on; the switch is stored in the URL as `infra=true`. A service that called a hidden one keeps a "+N infra" badge on its card, amber, or red when any of those calls is failing; its tooltip lists each hidden callee with calls per minute and error percentage. The summary line adds "· N infra hidden". A service whose calls all went to hidden services stays on the map with its badge (unless it is infrastructure itself), and `/map?service=flagd` still opens flagd's drawer while it is hidden. The mini map on the Stories page always hides them. The header's degraded-services badge still counts infrastructure services. The list is `[map] infra_services` in the config file (default `["flagd"]`); set it in the file, not with an environment variable: Tayga's settings loader does not turn on the config crate's list parsing for `TAYGA__*` variables, so a list cannot be given that way (same as `metric_targets`). With an empty list the switch is not shown.
- Time range: 15m, 1h, 24h or 7d (default 1h), each ending now, or "Custom…": a past window picked with "from" and "to" fields in local time, up to 7 days long and starting within the last 7 days. The custom range is kept in the URL as `since` and `until` (UTC) and shows in the header as, for example, "Oct 4 12:00 – 14:00"; links between pages keep it, and choosing a preset clears it.
- Live: refreshes every 10 s and pauses while the browser tab is hidden. It is off, and cannot be turned on, while a custom range is set. The degraded-services badge (always the last 15 minutes) and the Pipeline page's job status and consumer lag show the current state and keep refreshing in any range.
- A custom range in an old link may have aged past the 7 days of data: the pages then show the API's error with a "Show last 1h" button, and the URL is left alone until it is clicked.
- Theme: a switch that cycles light, dark and system (the default). The choice is stored in the browser.
- `Cmd+K` or `Ctrl+K` opens the command palette: jump to a page, a service, a trace id (32 hex characters), a story group or a template, or switch the theme and the time range (including "Custom range…", which opens the custom range fields).
- `g` then `s`, `t`, `m`, `l` or `p` goes to Stories, Traces, Service map, Log alerts or Pipeline. `?` lists the shortcuts. They are ignored while you type in a field or a dialog is open.
- "Open in Jaeger" (span drawer, trace page) links to the demo's Jaeger (`TAYGA__JAEGER_URL`, set in `deploy/compose.tayga.yaml`). "Open in Grafana" on the map appears only when `TAYGA__GRAFANA_URL` is set, which `make up-extras` does. Both are empty when unset.

Pipeline history comes from a recorder inside `tayga-api`: every 15 s (`record_secs`) it scrapes the `/metrics` of ingest, writer, assembler, logminer and notifier plus its own registry, and stores the samples in ClickHouse `metric_samples` (7-day TTL). The scrape targets are in `deploy/tayga-api.toml` (`TAYGA_CONFIG`); a list of targets cannot be set through `TAYGA__` environment variables. Consumer lag is read from Kafka on request, so `tayga-api` has Kafka settings.

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

## Authentication

Off by default: with no `[auth]` section the API and the app are open, as before. Turning it on adds a login page and protects every `/api/*` route except the ones listed below. The settings live in the `[auth]` section of the config file (`TAYGA_CONFIG`, for example `deploy/tayga-api.toml`) or in `TAYGA__AUTH__*` environment variables:

| Key (env var) | Meaning | Default |
|---|---|---|
| `enabled` (`TAYGA__AUTH__ENABLED`) | Turns authentication on. When it is on, the keys below are checked at startup and a bad value stops the API | `false` |
| `username` (`TAYGA__AUTH__USERNAME`) | The one account. It may not contain `\|` or `:` | none (required when enabled) |
| `password_hash` (`TAYGA__AUTH__PASSWORD_HASH`) | An Argon2id hash in PHC format (see below). Plain passwords are not accepted | none (required when enabled) |
| `session_ttl` (`TAYGA__AUTH__SESSION_TTL`) | Session length: `<n>s`, `<n>m`, `<n>h` or `<n>d`, at most `365d` | `12h` |
| `session_key` (`TAYGA__AUTH__SESSION_KEY`) | Base64 of at least 32 bytes, used to sign session cookies. When unset, a random key is generated per process, so every restart signs everyone out | unset |
| `secure_cookie` (`TAYGA__AUTH__SECURE_COOKIE`) | Adds `Secure` to the cookie; set it when the app is served over HTTPS | `false` |

Make the hash with the devtools command. On a terminal it asks for the password twice without echo; when its input is piped it reads the first line:

```sh
cargo run -q -p tayga-devtools -- hash-password
printf '%s' 'my password' | cargo run -q -p tayga-devtools -- hash-password
```

Put the output in the config file, in quotes. With the compose stack that file is `deploy/tayga-api.toml`, which `deploy/compose.tayga.yaml` mounts read-only at `/etc/tayga/api.toml` and points `TAYGA_CONFIG` at; add the section there and restart tayga-api:

```toml
[auth]
enabled = true
username = "admin"
password_hash = "$argon2id$v=19$..."
```

`deploy/tayga-api.toml` is tracked by git, so keep your edit out of commits. Avoid putting the hash in a compose `environment:` entry as written: compose interpolates `$` in its files, so `$argon2id$v=19$...` arrives mangled and the API refuses to start. If you do set `TAYGA__AUTH__PASSWORD_HASH` in a compose file, write every `$` in the hash as `$$`. A plain shell export (`TAYGA__AUTH__PASSWORD_HASH='$argon2id$...'`, single quotes) needs no escaping.

How it works:

- Signing in (`POST /api/v1/auth/login` with a JSON body `{"username", "password"}`, content type `application/json`) returns 204 and an `HttpOnly`, `SameSite=Strict` cookie `tayga_session`, signed with HMAC-SHA256. The session is not sliding: it ends `session_ttl` after sign-in, whatever the activity. The cookie is a signed token and the API keeps no session state: signing out clears the browser's cookie but does not revoke a copy of it, and a restart without `session_key`, or a new `session_key`, signs everyone out.
- Scripts can skip the cookie and send HTTP Basic credentials with each request, for example `curl -u admin:password http://localhost:8090/api/v1/story-groups`. Each Basic request costs one password check.
- Password checks are limited to 5 attempts per client IP in 5 minutes (IPv6 clients are keyed by their /64 prefix). Every attempt counts, and it is counted before the password is checked, so more than 5 parallel checks from one client can briefly get 429; a successful check clears the client's count, so in practice the limit is reached by failures. After that, both the login page and Basic requests return 429 with `Retry-After`, even for the correct password. The two share one count per IP: a script sending wrong Basic credentials more than 5 times in 5 minutes also locks out browser sign-ins from that IP until the window passes. The limiter uses the connection's address and ignores `X-Forwarded-For`, so behind a reverse proxy all users share the proxy's one limit.
- Signing out (`POST /api/v1/auth/logout`) clears the cookie. It needs no session, but it does need a JSON content type (`application/json`), so a cross-site form post cannot trigger it.
- Open without a session: `GET /healthz`, `GET /metrics`, `GET /api/v1/config`, `GET /api/v1/auth/me`, `POST /api/v1/auth/login`, `POST /api/v1/auth/logout`, and the app's own files (`/`, `/assets/*` and every client route). Everything else under `/api/` returns 401 `{"error":"unauthorized"}`, including unknown `/api/` paths. `/metrics` stays open, so scrapers need no credentials. `HEAD` is treated as `GET` for these open routes.
- `GET /api/v1/auth/me` answers from the session cookie only: it returns 401 for a Basic `Authorization` header without a cookie, even with the right credentials. Scripts using Basic can call the data routes directly and need not ask `auth/me`.
- `GET /api/v1/config` carries `auth_enabled`; the app reads it, and when it is `true` it asks `GET /api/v1/auth/me` and shows the login page on a 401 (`/login?next=…`, where `next` is a path inside the app). A later 401 from any request (an expired session) sends the user to the login page once, and back after signing in. The header shows a user menu with "Sign out". When authentication is off, `/api/v1/auth/*` returns 404 and `/login` redirects to `/`; a 401 then (from an authenticating reverse proxy, say) only shows the page's error state, with no redirect.
- HTTPS: tayga-api serves plain HTTP. For HTTPS, put a reverse proxy in front of it and set `secure_cookie = true`; the cookie is then only sent over HTTPS.
- It is one account, with no roles and no per-user data.

## Load generator and the missing agent service

The demo's load generator has a task, `ask_agent`, that posts to an `agent` service. That service is defined only in the demo's `compose.agent.yaml`, which Tayga's Makefile does not include, so every call failed and showed up as noise. `deploy/compose.tayga.yaml` therefore mounts `deploy/locust/tayga_locustfile.py` into the `load-generator` container and points `LOCUST_LOCUSTFILE` at it. That file imports the demo's locustfile unchanged (the vendored submodule is not edited) and removes the `ask_agent` task from `WebsiteUser`. Tayga itself keeps no ignore list.

`deploy/compose.tayga.yaml` also raises the memory limits of demo services that ran at their caps during long sessions: `checkout` and `product-catalog` 20M to 64M, `ad` and `fraud-detection` 300M to 512M, `accounting` 160M to 320M, `kafka` 620M to 1G, `opensearch` 1G to 1.5G, the demo's `grafana` 175M to 256M, and `load-generator` to 2G. On 2026-10-05 `checkout` sat at 95% of 20M and `kafka` near 620M while `publish orders` took about 90 s and every checkout failed for six hours.

## Log templates and alerts

`tayga-logminer` reads the logs from `tayga.signals` (its own consumer group) and groups them into templates with the Drain algorithm, one tree per service. A body is first masked (UUIDs, hex runs of 8 or more characters and numbers become `<*>`), so `Found 3 products from database` and `Found 12 products from database` are one template, `Found <*> products from database`. A template keeps its id when it generalizes. Other numbers are masked, but an HTTP status code in an access-log position is kept (see [Detection correctness](#detection-correctness-plan-7a)), so a burst of 500s in Envoy logs can be its own template.

Per log, one row in `log_template_hits` (3-day TTL); per template, one row in `log_templates` (30-day TTL); alerts in `log_alerts` (7-day TTL) and as JSON on the topic `tayga.alerts`. Counting uses `uniqExact(log_id)`, so replayed records are not counted twice. `tayga-notifier` delivers the alerts to webhook and Slack targets (see [Alert delivery](#alert-delivery-tayga-notifier)).

Every 60 seconds the logminer runs two rules over `log_template_hits`, plus the opt-in silence rule (see [Silence alerts](#silence-alerts)):

| Rule | Fires when | Default |
|---|---|---|
| New template | the template's first log falls since the previous detection tick, in log time; its service had a template at least `new_template_warmup_min` before that first log (so a fresh install or new service does not flood); the first log is at least `new_template_warmup_min` after the masking epoch start (see below); it is not a kept status code split out of a template that existed before the epoch (see below); and it is not `<overflow>`. One alert per template, ever | warmup 15 min |
| Rate spike | the count in the last `spike_window_min` is at least `spike_min_count` and at least `spike_factor` times the mean per-window count over the preceding `baseline_window_min` (floored at 1), and the template has enough baseline coverage (see below). Templates older than `new_template_recent_min` (10 min) can spike; the mean counts only minutes with data | window 5 min, baseline 60 min, factor 5, min count 10 |

"In log time" means the logminer keeps a data clock, the newest mined log `ts`, and checks templates first seen after the previous tick's data clock minus 60 s. A template that appeared while the logminer was down or behind is therefore still reported when it catches up. On a fresh install (no hits yet) the clock starts at the wall clock minus 10 minutes, so a replayed backlog does not report its history. Spike windows are wall-clock based: a spike during such a gap is not reported. `tayga_logminer_data_lag_seconds` (wall clock minus the data clock; charted on the app's Pipeline page, and on the Grafana Pipeline health dashboard with `make up-extras`) shows the lag, and the logminer logs a warning when it exceeds 10 minutes.

A spike alert stays active while it was last confirmed within `alert_active_min` (10) of now; a tick that still fires updates it, otherwise a new alert starts. Each alert carries up to 5 example trace ids (newest first) that link to the error story when one exists, else to Jaeger. The other values are `TAYGA__LOGMINER__*` environment variables (keys in `LogminerSettings` in `crates/tayga-logminer/src/main.rs`). Other defaults: similarity threshold 0.5, at most 5,000 clusters per service (beyond that, unmatched logs go to the `<overflow>` template), flush at 5,000 logs or 1 s.

The API decides whether an alert is "active" (the `active` field, the `alerting` flag and the story-page badges) with its own constant, `ALERT_ACTIVE_MIN = 10` in `crates/tayga-api/src/repo.rs`, not with the logminer setting. If you change `TAYGA__LOGMINER__ALERT_ACTIVE_MIN`, change that constant to match, or the UI and the logminer disagree on which alerts are active.

Where to look:

- `/logs/alerts`: alerts with kind badge, count against baseline and example traces.
- `/logs/templates` and `/logs/templates/{id}`: templates by count, search, sparkline, sample and recent hits.
- Story pages: the log table has a Template column, with a `new` or `spike` badge when the template was alerting at the story's time.
- With `make up-extras`: Grafana "Tayga · Logs" dashboard (`tayga-logs`) and logminer panels in "Tayga · Pipeline health". The app's Pipeline page charts the logminer metrics too.

Limits: one logminer replica only (Drain state is global per service while records are partitioned by trace); seasonal baselines are opt-in; a template that disappears raises an alert only when silence alerts are switched on for it; history is re-mined only by hand (see [Re-mining templates](#re-mining-templates-tayga-devtools-remine)). Open questions are in `docs/superpowers/followups.md`.

To probe the new-template rule by hand (the stack's ingest listens on 14318):

```sh
cargo run -p tayga-devtools -- emit-log --service tayga-e2e-probe --body "hello probe marker"
```

The command prints the trace id it used. An alert fires only if the service already had a template 15 minutes before. The e2e `new_template_from_probe` logs under its own service, `tayga-e2e-probe`, never a demo service. Every run emits a constant seed log, `tayga e2e probe seed`, and then a probe `{word} probe … probe marker` with a random 12-letter word and 4 to 58 tokens. The first run on a stack waits up to 16 minutes for the seed template to age past the warmup. Drain routes on the token count, then on the first word, so the probes spread over about 55 nodes of 100 children: about 4,000 runs (simulated: the first node fills at 4,000–4,650 runs) fit in the 30-day template TTL before probes start merging and the test fails.

## Detection correctness (plan 7a)

**Settings** (`LogminerSettings`; env vars as for the other logminer keys):

| Key (env var) | Meaning | Default |
|---|---|---|
| `baseline_mode` (`TAYGA__LOGMINER__BASELINE_MODE`) | `flat` or `seasonal`. Any other value stops the logminer at startup | `flat` |
| `keep_http_status` (`TAYGA__LOGMINER__KEEP_HTTP_STATUS`) | Keep HTTP status codes in access-log templates | `true` |

**Spike baseline coverage (flat mode).** Per detection pass the logminer fetches the minutes of the baseline window that have at least one log row from any template (at most 61 minute buckets). A template counts only the covered minutes since its first log. The baseline per spike window is the baseline total divided by `max(effective minutes / spike_window_min, 1)`, floored at 1 as before. When the covered minutes are fewer than `baseline_window_min` (a mature template whose window had an outage gap, as well as a young one), the baseline is scaled proportionally instead: `baseline_total × spike_window_min / effective minutes`. A template whose covered minutes are under half of the minutes it could have been seen in gets no spike judgement; each such candidate is counted in `tayga_logminer_spike_skipped_total{reason="coverage"}`. A logminer or ingest outage therefore no longer shrinks the baseline into a spike storm.

**Young templates.** A template older than 10 minutes and younger than 65 can spike. Its baseline uses only the minutes it has existed before the spike window (it needs at least `spike_window_min` of them); a shortened baseline is scaled proportionally.

**Seasonal mode** (`baseline_mode = "seasonal"`, opt-in). A template that passes the flat rule must also reach `spike_factor` times the count of the same spike window 1 day earlier and 1 week earlier (each floored at 1). A comparator is used only if that past window has at least one row for any template; with neither available, seasonal behaves like flat. Counts come from `log_template_minutes` (migration 0007), a per-minute aggregate filled by a materialized view from `log_template_hits`, with `uniqExact` so replays do not double count and an 8-day TTL. Migration 0009 backfills it once from `log_template_hits` (3 days), so the 1-day comparator works right after the upgrade; the 1-week comparator needs a week of history. If the comparator lookup fails, the pass falls back to the flat rule and counts `tayga_logminer_seasonal_failures_total`. Alerts carry optional `baseline_day` and `baseline_week` (migration 0008); the API passes them through and the app does not show them yet.

**HTTP status codes in templates.** A token of exactly 3 digits in 100 to 599, directly after an `HTTP/1.1`, `HTTP/1.0` or `HTTP/2` style token, stays literal (`"GET /api/cart HTTP/1.1" 503 UF` keeps `503`); other numbers are masked as before. Span fingerprints use a separate mask and are unchanged. A kept code matches only itself in Drain: a template's `<*>` does not match it, a merge never turns it into `<*>`, and a template and a line that differ at a kept code (or have a kept code on one side only) are not similar, so a `200` line and a `503` line never share a template. A routing token that is a kept code always gets its own branch, never the `<*>` overflow branch; these branches come on top of the 100-children limit (at most 500 more per node, one per code). The rule is positional only, so a non-access-log line with `HTTP/x NNN` (`upstream replied HTTP/1.1 503`) keeps its code too. Because masking turns every other digit run into `<*>`, a bare 3-digit token in 100 to 599 can only be a kept code; that is how a stored template string is recognised after a restart (no marker in the stored text). Existing templates with `<*>` in the status position stay as they are and keep matching other lines, but no longer absorb lines with a kept code: those start new templates.

**Persisted state and the masking epoch.** The table `logminer_state` (migration 0006, a `ReplacingMergeTree`) holds `new_template_watermark_ns` (saved after each detection pass that stored its alerts; a restart resumes from it, clamped to the wall clock), `masking_version` (3 with `keep_http_status`, 1 without; 2 was a retired variant whose kept codes could still be generalised) and `masking_epoch_start_ns`. When the stored version differs from the running one, the logminer starts a new epoch at "now"; an install that has templates but no stored version counts as version 1, so the upgrade starts an epoch. During `new_template_warmup_min` (15) after the epoch start no new-template alert fires for templates first seen in that time. After the warmup, a rarer status/shape combination still starts a new template the first time it appears; it raises no alert when, with its kept codes read as `<*>`, it would have matched a template of the same service first seen before the epoch (same length and first two tokens, similarity at least the threshold with that template's `<*>` matching anything). Each such case is counted in `tayga_logminer_new_suppressed_total{reason="pre_epoch_match"}`. Templates without a kept code are judged as before. Spike detection is unaffected. A save failure is logged and counted in `tayga_logminer_state_save_failures_total`; it does not fail the pass. The new-template watermark is a per-partition data clock: a partition that is behind (unconsumed records ahead and its newest record older than 60 s) holds the clock at its own newest time, so templates in a lagging partition are not missed. While the consumer has no partitions assigned (a rebalance or rejoin) or the assignment cannot be read, the clock holds at the stored watermark.

**Slow-request baselines (assembler).** Each baseline refresh excludes traces longer than the endpoint's previous limit, `max(p99 x slow_trace_factor, p99 + slow_trace_margin_ms)` (1.5 and 100 ms by default), as well as traces that already have a slow story. An endpoint with no trusted previous baseline (startup, new endpoint, fewer than `min_baseline_traces` traces) is capped at 10 x its window p50. One slow outlier therefore does not stretch the baseline. An endpoint whose traces are nearly all slow-storied or capped (fewer than `min_baseline_traces`, 50, kept) keeps its previous baseline for at most 2 baseline windows (120 minutes with the default 60); after that the new level is adopted, so a slowdown longer than 2 baseline windows stops being flagged. Carry state is in memory only: an assembler restart during a slowdown re-learns through the 10 x p50 bootstrap.

**New metrics.**

| Metric | Service | Meaning |
|---|---|---|
| `tayga_assembler_baseline_excluded_traces` | assembler | Traces dropped by caps at the last successful refresh (gauge) |
| `tayga_assembler_baseline_carried_endpoints` | assembler | Endpoints whose previous baseline was carried at the last successful refresh (gauge) |
| `tayga_logminer_spike_skipped_total{reason="coverage"}` | logminer | Spike candidates not judged for low coverage |
| `tayga_logminer_seasonal_failures_total` | logminer | Failed seasonal lookups (flat fallback used) |
| `tayga_logminer_state_save_failures_total` | logminer | Failed saves to `logminer_state` |
| `tayga_logminer_new_suppressed_total{reason="pre_epoch_match"}` | logminer | New-template candidates not alerted because a pre-epoch template would have matched them |

Known gaps: seasonal mode has not been run against a live stack (it needs a day of history); the per-partition clock's Kafka position and committed-offset paths are tested with fakes, not a real broker; partitions lagging under 60 s count as caught up.

## Silence alerts

Silence alerts are opt-in per template. Tayga raises a `silence` alert when a template has had no log for N minutes while its service still sends other logs.

**In the app.** The template page (`/logs/templates/{id}`) has a "Silence alert" card. It holds an "Alert when silent" switch and a minutes field, which defaults to 10, takes 1 to 1440, and is disabled while the switch is off.
- Save sends the PUT below. A 401 follows the session-lost flow, and other errors show inline.
- The templates table shows a bell on rows with silence on.
- In `/logs/alerts` and the other alert lists, a silence alert has its own `silence` badge, the kind filter has a "Silence" option, and the count column reads "silent N min".

**API.** `PUT /api/v1/log-templates/{id}/silence` with `Content-Type: application/json`:

```sh
curl -X PUT localhost:8090/api/v1/log-templates/<template_id>/silence \
  -H 'content-type: application/json' -d '{"enabled": true, "minutes": 10}'
```

- The answer is the stored setting.
- Errors:
  - 415 without the JSON content type;
  - 400 on a bad body, an id that is not a decimal u64, or `minutes` outside 1 to 1440;
  - 404 for an unknown template.
- With authentication on, it needs a session or Basic credentials, like the other API routes.
- Settings are stored in `log_template_silence` (migration 0010), where the newest row per template wins.
- `GET /api/v1/log-templates/{id}` returns the setting as `silence`, and list rows carry `silence_enabled`.

**The rule, in log time.** On every detection pass (60 s), for each template with silence on:
- `t_last` is the template's newest hit inside the 3-day hits TTL. Past that TTL it falls back to the template's `last_seen`, and without one to its `first_seen`.
- `s_last` is the newest hit of any template of the same service.
- The template is silent when `s_last − t_last ≥ minutes`.

Both values are log timestamps, not the wall clock. So a pipeline outage, where no logs arrive at all, does not make a template silent. A service with no hits in the last 3 days is not judged.

**Partition-clock guard.** `s_last` counts only up to the logminer's per-partition data clock, the same clock the new-template rule uses. That clock holds back while an assigned partition is behind, so logs still waiting in a lagging partition cannot make a template look silent.

**The alert:**
- `started_at` is the template's last hit, so `last_at − started_at` is how long it has been quiet.
- That quiet time ("silent N min" in the app, Slack and the webhook `summary`) mixes clocks: `last_at` is the logminer's wall clock, while `started_at` is a log timestamp. Pipeline lag or clock skew is counted in, so a logminer 10 minutes behind the logs reports a silence 10 minutes longer than it is in log time. Whether the template is silent at all is still judged in log time only.
- `last_at` is refreshed on every pass while the template stays silent.
- `alert_id` is a hash of the template id and that last hit, so one quiet period is one alert, also across a logminer restart.
- `window_count` is 0 and there are no example traces. `baseline_per_window` is 0: the spec marks it informational, and no query computes it yet.
- Silence alerts do not set a template's `alerting` flag, and they are not shown as badges on story logs.
- `tayga_logminer_silence_alerts` is a gauge of the templates that are currently silent.

**How a silence ends.** When the template gets a hit again, or silence is switched off for it, the alert is no longer refreshed. It turns inactive 10 minutes (`alert_active_min`) after its last `last_at`, like a spike. A later quiet period gets a new alert id.

## Alert delivery (tayga-notifier)

`tayga-notifier` consumes `tayga.alerts` as consumer group `tayga-notifier` and delivers `new`, `spike` and `silence` alerts to webhook and Slack targets. It always runs in compose. With no targets, which is the default, it logs `delivery disabled: no targets` once and keeps committing offsets.

**Config.** The settings live in `deploy/tayga-notifier.toml`, which is mounted as `TAYGA_CONFIG`.
- Scalar keys can be overridden with `TAYGA__NOTIFIER__*` environment variables.
- `targets` is a list of tables, so it can only be set in a file.
- Settings are read at startup, so run `docker restart tayga-notifier` after an edit.

| Key | Meaning | Default |
|---|---|---|
| `targets` | `{name, kind, url}` per target. `kind` is `"webhook"` or `"slack"`. `name` is 1 to 64 of `[A-Za-z0-9._-]` and unique. `url` is http(s) | none |
| `public_url` | Base of the links back into the app. Slack readers must be able to open it, so `localhost` only works on the machine running the stack | `http://localhost:8090` |
| `kinds` | The alert kinds to deliver | `["new", "spike", "silence"]` |
| `max_attempts` | Attempts per alert and target | 8 |
| `timeout_secs` | Timeout per request, 1 to 15 | 10 |
| `max_age_secs` | Alerts whose `last_at` is older are skipped and committed (`result="stale"`), so a first start with targets does not deliver the backlog retained on the topic | 3600 |
| `breaker_cooldown_secs` | How long a target's circuit breaker stays open after it gave up on a retryable error (see Retries). Must be positive | 300 |

```toml
[[notifier.targets]]
name = "ops-slack"
kind = "slack"
url = "https://hooks.slack.com/services/…"
```

**URL secrecy.** A webhook URL, and a Slack one in particular, is a credential.
- The notifier never logs a URL, never uses it as a metric label, and never stores it.
- Logs, metrics and `notifier_deliveries` name the target by its `name`.
- Debug output prints `<redacted>`, and every `http(s)://…` in an error text is replaced with `<redacted>` before it is logged or stored.
- `deploy/tayga-notifier.toml` is tracked by git, so do not commit a real URL. Either keep it as a local, uncommitted edit, or mount an untracked file through a compose override, the way `deploy/compose.notifier-e2e.yaml` does.

**Webhook payload.** The notifier sends a `POST` with `Content-Type: application/json`. This is the body from the live `make e2e-notifier` run, with the template shortened:

```json
{"alert_id":"fe0492738da0b683","kind":"silence","service":"tayga-e2e-probe",
 "template_id":"2340958801421686365","template":"xzwoppcincbw probe … probe marker",
 "started_at":"2026-10-06T05:40:40.946Z","last_at":"2026-10-06T05:43:20.029Z",
 "count":0,"baseline":0.0,
 "summary":"xzwoppcincbw probe … probe marker has been silent for 2 min in tayga-e2e-probe",
 "example_trace_ids":[],
 "links":{"template":"http://localhost:8090/logs/templates/2340958801421686365","traces":[]}}
```

- `template_id` is a decimal string.
- Times are RFC 3339 UTC with milliseconds. `last_at` is the alert's latest update.
- `count` is the alert's `window_count` (0 for a silence), and `baseline` is its `baseline_per_window`.
- `summary` is one unescaped line, the same text as Slack's fallback:
  - `New log template in <service>: <template>`;
  - `<template> spiked to <count> per window in <service> (baseline <b>)`;
  - `<template> has been silent for <N> min in <service>`.
- `example_trace_ids` holds every example trace id; `links.traces` links at most 3 of them.
- Redirects are not followed.

Each alert is delivered once, when the notifier first sees it. Later updates, such as a spike growing or a silence getting longer, are not sent.

**Slack.**
1. Create a Slack app with Incoming Webhooks turned on.
2. Add a webhook for the channel.
3. Put its URL in a `kind = "slack"` target.

The message is built from `blocks`:
- a header such as "Log spike in checkout", "Log silence in cart" or "New log template in ad";
- the template in a code block;
- fields for the count, the baseline and the start (for a silence, how long it has been silent and the last hit);
- buttons to the template and up to 3 traces.

Template text is escaped for Slack, so `<*>` is not read as a link. Tayga only posts; nothing is read back from Slack.

**Retries:**
- A 2xx is delivered.
- 429, 5xx, a network error or a timeout is retried.
- Any other status, including every other 4xx and a 3xx, fails at once. The status is recorded as `last_error`.
- The wait after the n-th failed attempt is the larger of 1 s × 2^(n−1) and the response's `Retry-After` (delta-seconds or an HTTP date), capped at 300 s.
- After `max_attempts` attempts the delivery is marked failed.

Records are handled one at a time, and each waits for all its targets. With the defaults and no `Retry-After`, a target that keeps failing takes about 2 minutes of backoff (1 + 2 + … + 64 s) plus up to 8 request timeouts before it gives up. The consumer's `max.poll.interval.ms` is raised to 40 minutes, so even waits at the `Retry-After` cap do not trigger a rebalance.

**Circuit breaker per target.** Paying that ladder on every alert would let the backlog grow past `max_age_secs` after roughly 17 to 28 alerts (3600 s at 127 to 207 s each); the live stack has seen over 100 alerts in an hour. From then on every record would be skipped as `stale` for every target, healthy ones included. A breaker per target prevents this:
- When a target gives up on a retryable error (429, 5xx, a network error or a timeout), its breaker opens for `breaker_cooldown_secs` (300 s).
- While it is open, each new alert gets exactly one attempt to that target, without backoff. A 2xx closes the breaker. A retryable failure marks that alert `failed` for the target, with its attempt recorded, and keeps the breaker open for another cooldown.
- A permanent error (a 4xx) does not open or close it.
- When the cooldown passes with no new alert, the breaker closes, and the next alert runs the full ladder again.
- The state is kept in memory, so a restart starts every target closed.

So, after its first ladder, a dead target costs each record one attempt: almost nothing for a fast 5xx, and at most `timeout_secs` (10 s) for a host that does not answer. Healthy targets keep pace and are not skipped as stale. The price is that alerts sent while a target's breaker is open are not retried to it: they are marked `failed` after one attempt and are not delivered there later. `max_age_secs` still applies per record, judged when the record is read, so a backlog that is old for another reason, such as a notifier that was down for over an hour, is skipped as before.

**Dedup and offsets.** Delivery state is kept per `(alert_id, target)` in `notifier_deliveries`:
- migration 0011, `ReplacingMergeTree(updated)`, 30-day TTL;
- columns `status` (`pending`, `delivered` or `failed`), `attempts` and `last_error`.

The logminer re-publishes an alert every time it updates it. Once the target has a `delivered` or `failed` row, a re-published or re-read alert sends nothing and is counted as `result="duplicate"`. A `pending` row keeps its attempt count across a restart. The Kafka offset is committed only once every target is delivered or failed.

**Stopping.** A stop (SIGTERM) does not cancel an attempt that is in flight. The attempt finishes within `timeout_secs`, and its row is written: each write times out after 5 s and is retried for up to 10 s more. Compose gives the container `stop_grace_period: 40s` for this.

**Accepted limits.** Receivers should deduplicate on `alert_id`, because an alert can be sent a second time in these cases:
- The process is hard-killed (SIGKILL, OOM, or a stop that outlasts the grace period) between a 2xx and the row write. The alert is sent again once after the restart.
- ClickHouse is down for the whole grace period, so the final row is lost. The offset is still committed, and the next re-publish of that alert is sent again.
- A `notifier_deliveries` row expires after its 30-day TTL. A re-publish of the same alert id after that, for example a silence lasting more than 30 days, is delivered again.

**Metrics** are served on `:9100` inside the network; the port is not published. The API's recorder scrapes them (`deploy/tayga-api.toml`).

| Metric | Meaning |
|---|---|
| `tayga_notifier_deliveries_total{target,result}` | `result` is `delivered`, `failed`, `retry`, `duplicate` (a re-publish of an alert already resolved, not a send), `stale` (older than `max_age_secs`) or `breaker` (an attempt made while the target's breaker was open; its outcome is also counted as `delivered` or `failed`) |
| `tayga_notifier_delivery_seconds` | Histogram of each HTTP attempt |
| `tayga_notifier_pending` | Deliveries started and not yet resolved |
| `tayga_notifier_breaker_open{target}` | 1 while the target's breaker is open, else 0. Updated when an alert is delivered to the target, so after an idle cooldown it can read 1 until the next alert |

The Pipeline page shows `tayga-notifier` as a job, and its consumer lag on `tayga.alerts` next to the other groups.

### Pointing the notifier at a mock

`make e2e-notifier` checks delivery against a live stack without any outside service:
1. It recreates `tayga-notifier` with `deploy/compose.notifier-e2e.yaml`. That override mounts `deploy/tayga-notifier.e2e.toml`, which has one webhook target, `e2e-mock`, at `http://host.docker.internal:18099/hook`.
2. It runs the `silence_alert_and_delivery` scenario with `TAYGA_E2E_NOTIFIER=1`. The scenario starts the e2e crate's mock webhook on the host's `0.0.0.0:18099` and waits for its silence alert. It then checks that the mock got exactly one delivery for that `alert_id`, runs `docker restart tayga-notifier`, watches for 150 s while the logminer keeps re-publishing the alert, and checks that there is still exactly one.
3. It always recreates the notifier with the default, target-less `deploy/tayga-notifier.toml` afterwards, whether the scenario passed or not.

Docker Desktop resolves `host.docker.internal` by itself. The override also maps it to `host-gateway` for Linux engines, which was not tested. The mock listens on all interfaces only while the scenario runs.

## Re-mining templates (`tayga-devtools remine`)

Use it after changing masking or a Drain setting (`sim_threshold`, `max_clusters_per_service`, `keep_http_status`). Without it, the templates reflect the new configuration only for logs mined since the change.

`remine` does the following:
1. It truncates `log_templates`, `log_template_hits` and `log_template_minutes`.
2. It reads `logs` from the last 3 days in `(ts, log_id)` order, 10,000 rows per page, and mines them with the logminer's own code and its current `[logminer]` settings, taken from the `TAYGA_CONFIG` file and the `TAYGA__LOGMINER__*` environment variables. The compose logminer is configured by environment variables only, so give the CLI the same ones.
3. After each page it writes the hits and the changed templates; `log_template_minutes` is filled through its materialized view. Template rows of page n carry version `now + n` ns, so a template written on several pages keeps the row of its last page (final `count` and `last_seen`) without depending on how ClickHouse breaks an equal-version tie.
4. It stores `new_template_watermark_ns` (the newest mined `ts`), `masking_epoch_start_ns = now` and the current `masking_version` in `logminer_state`.

The watermark means the restarted logminer does not report the rebuilt templates as new. The new masking epoch adds the 15-minute warmup: no `new` alert fires for templates first seen in the 15 minutes after the re-mine. Only `new` alerts are gated this way. Spikes are not: spike alerts are matched by template id, so a template that is spiking when the re-mine changes its id gets a new spike alert id, and the notifier may deliver it a second time. `log_alerts` and `log_template_silence` are kept. ClickHouse defaults to `http://localhost:18123`, database `tayga`; `--clickhouse` and `--database` override them.

```sh
# 1. Preview. Read-only, and allowed while the logminer runs.
cargo run --release -q -p tayga-devtools -- remine --dry-run
# 2. Stop the logminer. These are the Makefile's compose files, run from the repository root.
TAYGA_ROOT=$PWD docker compose --project-directory vendor/opentelemetry-demo \
  -f vendor/opentelemetry-demo/compose.yaml -f vendor/opentelemetry-demo/compose.full.yaml \
  -f vendor/opentelemetry-demo/compose.observability.yaml \
  -f deploy/compose.infra.yaml -f deploy/compose.tayga.yaml stop tayga-logminer
# 3. Wait until the heartbeat is 3 minutes old, then re-mine.
cargo run --release -q -p tayga-devtools -- remine
# 4. Start the logminer: the same compose command with `start tayga-logminer`.
```

- **Duration.** Build with `--release`: on 8.6 million logs a debug-build dry run took 501 s, and a release-build real run took 211 s. The logminer is down for that time plus the 3-minute heartbeat wait.
- **Dry run.** The dry run prints the same summary as a real run without writing anything:
  - templates per service, before and after;
  - template ids added, removed and unchanged;
  - silence settings that would be orphaned.
- **Heartbeat guard.** The logminer writes `logminer_heartbeat_ns` to `logminer_state` on every detection pass. A real run refuses while that heartbeat is under 3 minutes old, and the message says to stop the logminer first. This means you wait about 3 minutes after stopping it. `--force` skips the check; use it only when you know the logminer is down. `--dry-run` never checks.
- **Not atomic.** The tables are truncated before mining. If a run fails partway, run it again before you start the logminer.
- **3-day window.** Only the stored logs (3-day TTL) are mined. Templates whose logs are all older are dropped, even though `log_templates` itself keeps rows for 30 days.
- **Id churn and orphans.** A template id hashes the service and the cluster's first-seen template. Ids are stable only for the same logs, order and configuration, so a rebuild after a config change can change many of them.
  - Alerts keep their own template text, but their template links can point at ids that no longer exist.
  - Silence settings on vanished ids are reported as orphaned and left in place. Switch silence on again for the new ids.
- **Empty window.** With no logs in the last 3 days, the template tables end up empty and the watermark is left unchanged.

## Deploy order

Migrations must run before `tayga-api`, `tayga-logminer` or `tayga-notifier` restart on a new version:
- migration 0010 adds `log_template_silence` and the `silence` alert kind, which the API reads and the logminer writes;
- migration 0011 adds `notifier_deliveries`.

`make up` takes care of this. It rebuilds the image and recreates the stack, and every Tayga service except ingest waits for the one-shot `tayga-migrate` service (`tayga-writer migrate`) to finish successfully (`depends_on: condition: service_completed_successfully`). If you restart one service by hand after an upgrade, run the migration first.

Reload browser tabs opened before the upgrade. Their old bundle validates alert kinds against a strict enum that does not know `silence`, so its alert views show an error until the page is reloaded.

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
| `make e2e` | Resets flags, then runs the end-to-end tests against the live stack (`make up` first). Ten tests: the seven from before (flag-driven story scenarios for payment, payment unreachable, shipping, product catalog and ad, the service map, and raw span counts versus Jaeger) plus `log_spike_on_payment_failure`, `new_template_from_probe` and `silence_alert_and_delivery` (without its notifier part). The spike scenario fails up front if a payment spike alert is still active from an earlier run (wait about 10 minutes). The first run on a fresh stack waits up to 16 more minutes for the probe service warmup |
| `make e2e-notifier` | Runs `silence_alert_and_delivery` with the notifier check: points `tayga-notifier` at a mock webhook on the host, then puts the target-less config back (see [Pointing the notifier at a mock](#pointing-the-notifier-at-a-mock)) |
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
| `GET /api/v1/log-alerts` | `since` (default `24h`), `until`, `kind` (`new`, `spike` or `silence`), `service` | Up to 200 alerts that overlap the window (seen after its start, started by its end), newest `last_at` first; each has `active` and `example_traces` (`[{trace_id, story_id \| null}]`) |
| `GET /api/v1/log-templates` | `since` (default `1h`), `until`, `service`, `q` (substring, at most 200 chars) | Top 200 templates with hits in the window, by count; each has `alerting` (spike and new alerts only) and `silence_enabled`, plus `bucket_secs` (the window / 120, rounded up to whole minutes) and `buckets` (`[bucket_start_unix_s, hits]`, oldest first; buckets without hits are left out) |
| `GET /api/v1/log-templates/{id}` | `since` (default `24h`), `until` | One template with `buckets`, the 20 most recent hits up to the window's end, its alerts of the 7 days before that end, and `silence` (`{enabled, minutes}`, or null when never set) |
| `PUT /api/v1/log-templates/{id}/silence` | JSON body `{enabled, minutes}` (`minutes` 1 to 1440) | The stored setting. 415 without a JSON content type, 400 on a bad body or id, 404 when the template does not exist. The only route that changes stored data |
| `GET /api/v1/traces/{trace_id}/log-templates` | none | `[{log_id, template_id, template, alert}]` for the trace's logs |
| `GET /api/v1/overview` | `since`, `until` | KPI values and bucket series for the Stories page |
| `GET /api/v1/stories/series` | `since`, `until`, `kind`, `service` | Stories per bucket, for charts |
| `GET /api/v1/traces/search` | `since`, `until`, `service`, `touched` (0 or 1), `endpoint`, `min_ms`, `max_ms`, `errors` (0 or 1), `limit` (1 to 500, default 100) | Trace rows for the explorer, newest first, each with `story_id` and `story_kind` when a story exists |
| `GET /api/v1/services` | none | Service names |
| `GET /api/v1/services/{name}` | `since`, `until` | RED series (rate, error ratio, p50/p95/p99) for one service |
| `GET /api/v1/search` | `q` | Command palette: matching services, templates and story groups; a trace id when `q` is 32 hex characters |
| `GET /api/v1/pipeline/series` | `metric`, `kind` (required), `job`, `labels` (`k=v`), `since`, `until` | A rate, gauge or quantile series from the recorded metrics |
| `GET /api/v1/pipeline/lag` | none | Consumer lag per group (committed, end offset, lag) |
| `GET /api/v1/config` | none | `{jaeger_url, grafana_url, auth_enabled, infra_services}` (the two links are null when unset; `infra_services` is the `[map]` list, default `["flagd"]`) |
| `POST /api/v1/auth/login` | JSON body `{username, password}` | 204 and the session cookie; 401 on a wrong login, 429 when limited, 415 without a JSON content type. Only exists when authentication is on (404 otherwise) |
| `POST /api/v1/auth/logout` | JSON content type | 204 and a cookie that clears the session. Only when authentication is on |
| `GET /api/v1/auth/me` | none | `{username}` with a valid session, else 401. Only when authentication is on |
| `GET /healthz` | none | `ok` |
| `GET /metrics` | none | Prometheus metrics |

With authentication on, every `/api/*` route in this table except `config` and the three `auth` routes needs a session cookie or Basic credentials (see [Authentication](#authentication)). Errors on these routes are JSON `{"error": "..."}`. A ClickHouse failure returns 503. `GET /api/v1/traces/{trace_id}` also carries the extra fields the app uses (span attributes, resource, events, self time). `GET /api/v1/service-map` changed shape: it returns an object, not an array, so a client that read the old array must read `edges`.

App routes (client-side; every path below serves `index.html`, and the app renders the page):

| Route | Query | Page |
|---|---|---|
| `/` | `since`, `until`, `kind`, `service`, `group` | Stories |
| `/stories/{story_id}` | `since`, `until` | Story |
| `/traces`, `/traces/{trace_id}` | `since`, `until`, filters | Trace explorer, trace |
| `/map` | `since`, `until`, `service` (open drawer), `q` (search), `infra=true` (show infrastructure services) | Service map |
| `/login` | `next` (path to return to) | Login, only when authentication is on |
| `/logs/alerts`, `/logs/templates`, `/logs/templates/{id}` | `since`, `until`, filters | Logs |
| `/pipeline` | `since`, `until` | Pipeline health |

Old server-rendered URLs (`/service-map`, `/alerts`, `/templates`, `/groups/…`) are not redirected; they show the app's not-found page.

Static files: `/assets/*` is served with `Cache-Control: public, max-age=31536000, immutable`; `index.html` with `no-cache`. A GET to a path the app does not know serves `index.html` with status 200 (the app shows its own not-found page). A missing `/api/*` route or `/assets/*` file returns a JSON 404.

## Verified

Rows above the `Plan 4` row were checked 2026-10-03 on branch `feat/plan-3-api-ui-e2e`; rows from the `Plan 4` row on were checked 2026-10-04 on branch `feat/plan-4-log-templates`; rows from the `Plan 5` row on were checked 2026-10-05 on branch `feat/plan-5-ui`; rows from the `Plan 6` row on were checked 2026-10-05 on branch `feat/plan-6-owner-decisions`. The plan 7a and 7b rows carry their own dates. The stack was running for all of them. Rows about the removed server-rendered pages are kept as history and marked **superseded**.

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
| Old-URL redirects (`/groups/…`, `/service-map`, `/alerts`, `/templates`) | removed in plan 6 (`3dffb27`); `old_urls_are_plain_client_routes` in `crates/tayga-api/src/spa.rs` | **superseded**: no redirects; old URLs are client routes and show the not-found page |
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
| **Plan 6 (login, infrastructure toggle, no redirects, agent noise)** | | |
| Rows below checked 2026-10-05 on branch `feat/plan-6-owner-decisions` at `a6cf80a` (plus this docs commit). Auth rows were run against a host `tayga-api` debug build on 127.0.0.1:18090 (ClickHouse :18123, Kafka :19092), stopped afterwards; "Task N report" means the report in `.superpowers/sdd/2026-10-05-tayga-plan-6-owner-decisions/` | | |
| `[auth]` keys `enabled`, `username`, `password_hash`, `session_ttl` (default 12h), `session_key`, `secure_cookie`, and `TAYGA__AUTH__*` env forms | `AuthSettings` and its `Default` in `crates/tayga-api/src/auth.rs`; live: the host API started with only `TAYGA__AUTH__ENABLED`, `TAYGA__AUTH__USERNAME`, `TAYGA__AUTH__PASSWORD_HASH` set accepted logins and returned `Max-Age=43200` (12 h) | verified |
| `session_ttl` is capped at 365d; `username` may not contain `\|` or `:`; `session_key` must be at least 32 bytes base64 | live startup errors with the env forms: `auth.session_ttl must be <n>s, <n>m, <n>h or <n>d, above 0 and at most 365d` for `366d`; `auth.username must not contain '\|' or ':'` for `a\|b` and `a:b`; `auth.session_key must decode to at least 32 bytes` for 3 bytes. With `365d`, a 32-byte key and `secure_cookie=true` the login returned `Max-Age=31536000` and `Secure`, and the log had no "session_key is unset" warning | verified live |
| Without `session_key` a random key is used and a restart logs everyone out | startup log of the host API: WARN `auth.session_key is unset: a random key is used, so a restart signs everyone out`; key generation in `Auth::from_settings` | verified live (log line); that old cookies stop working after a restart is by construction (the key changes), not re-tried today |
| The session is not sliding | the cookie's `Max-Age` is set only at login (`crates/tayga-api/src/auth.rs`); no route re-issues it | verified in code; not tested by waiting out a session |
| `hash-password` prints an Argon2id hash from piped stdin; asks twice on a TTY | live: `printf 'secret' \| tayga-devtools hash-password` printed a string starting `$argon2id$v=19$m=19456,t=2,p=1`, and the API accepted it; `read_password` and `confirmed` in `crates/tayga-devtools/src/password.rs`; unit test `hash_of_known_password_verifies` | piped form verified live; the two-prompt TTY path verified in code only (no TTY here) |
| Login returns 204 with a `tayga_session` cookie `HttpOnly; SameSite=Strict; Path=/`; protected routes need it; Basic works for scripts | live on :18090: `GET /api/v1/story-groups` without credentials 401; `POST /api/v1/auth/login` with a JSON body 204 with `Set-Cookie: tayga_session=…; HttpOnly; SameSite=Strict; Path=/; Max-Age=43200`; `curl -u admin:secret …/story-groups?since=15m` 200 | verified live |
| Open routes: `/healthz`, `/metrics`, `GET /api/v1/config`, `GET /api/v1/auth/me` (401 without a session, so reachable), `POST` login and logout, and the app's files | live on :18090 with no credentials: `/healthz` 200, `/metrics` 200, `/api/v1/config` 200 `{"jaeger_url":null,"grafana_url":null,"auth_enabled":true,"infra_services":["flagd"]}`, `/` 200, `/api/v1/auth/me` 401; `OPEN_API` in `auth.rs`; tests `open_routes_match_method_and_exact_path` and `open_routes_stay_open` (cargo test `auth::`: 28 passed) | verified live and in tests |
| Logout needs a JSON content type and no session | live: `POST /api/v1/auth/logout` without a content type 415; with `application/json` and no cookie 204; `logout_requires_json_content_type` and `logout_needs_no_session` in `auth.rs` | verified live |
| Limiter: 5 failed attempts per client IP per 5 minutes, the 6th is refused even with the right password | live: 5 wrong logins all 401, then the right password 429 with `retry-after: 299` and `{"error":"too many attempts"}`; `WINDOW` and `MAX_FAILURES` in `auth.rs` | verified live (IPv4); IPv6 /64 keying verified in the unit test `ipv6_is_limited_per_64_prefix` and the Task 2 report, not live |
| `X-Forwarded-For` is ignored, so behind a proxy all users share one limit | `forwarded_for_does_not_dodge_the_limiter` in `auth.rs`; Task 2 report live run: 5 wrong logins with a different `X-Forwarded-For` each, the 6th was 429 | verified in a test and in the Task 2 report; not re-run today |
| Parallel attempts are counted before the password check, so more than 5 in flight can get 429 | design note in the Task 2 report (concern 2) and spec section 2.2 | cited, not re-measured |
| Login page, redirect to `/login?next=…`, wrong-password alert, sign-in, reload stays signed in, sign-out, an unsafe `next` lands on `/` | `ui/e2e/auth.spec.ts` run today against the Vite dev server (5174) proxied to the auth API (18090): `TAYGA_E2E_AUTH_USER=admin TAYGA_E2E_AUTH_PASS=… TAYGA_UI_URL=http://localhost:5174 npx playwright test auth.spec.ts --project=dark --project=light --no-deps` | verified: 8 passed (4 tests in dark and light) |
| HTTPS goes through a reverse proxy with `secure_cookie` | `secure_cookie` adds `; Secure` (live: login with `TAYGA__AUTH__SECURE_COOKIE=true` returned a cookie ending `Secure`); tayga-api binds plain HTTP (`http_addr`, no TLS code in `main.rs`) | cookie flag verified live; no reverse proxy was set up or tried |
| `[map] infra_services` defaults to `["flagd"]`, is in `/api/v1/config`, is settable in file config only | live: `/api/v1/config` on the rebuilt stack (`make up`) returned `"infra_services":["flagd"]`; with a TOML file `[map] infra_services=["flagd","otel-collector"]` (`TAYGA_CONFIG`) it returned both; with `TAYGA__MAP__INFRA_SERVICES=flagd,otel-collector` the API failed at startup with `invalid type: string "flagd,otel-collector", expected a sequence for key `map.infra_services`` | verified live |
| "Show infrastructure" toggle stored in the URL as `infra=true`; flagd hidden by default and drawn with it; `?service=flagd` opens the drawer while hidden | `ui/e2e/map.spec.ts` tests `infrastructure (flagd) is hidden by default and drawn with infra=true` and `/map?service=flagd opens flagd even while infrastructure is hidden`, in the full Playwright run below; `MapSearch.infra` in `ui/src/app/search.ts` | verified |
| Callers of hidden infrastructure keep a "+N infra" badge; the header's degraded count still includes infrastructure | `hideInfra` in `ui/src/features/map/model.ts`; unit tests in `model.test.ts` and `map.test.tsx` (the header count is 4 against the map's 3 in the fixture); Task 4 report | verified in unit tests; badge appearance reviewed from screenshots in the Task 4 report |
| Old-URL redirects removed | `old_urls_are_plain_client_routes` in `crates/tayga-api/src/spa.rs`; Task 1 commit `3dffb27` | verified in a test (see also the superseded rows above) |
| The load generator no longer calls the agent service: no `user_ask_agent` spans after the deploy while other spans flow | `docker exec load-generator env` shows `LOCUST_LOCUSTFILE=/usr/src/app/tayga_locustfile.py`. ClickHouse `tayga.spans`: Task 5 ledger, window 13:17:05-13:23:05 UTC, after the Docker disk was freed: `user_ask_agent` 0, other load-generator spans flowing (GET 184, POST 99, `user_browse_product` 65, checkout 16). Rechecked at 13:31:49 UTC, last 20 minutes: `user_ask_agent` 0, 521 spans named `user_%`, 105,313 spans in all. The last `user_ask_agent` span was 2026-10-05 11:41:59, before the 11:48 deploy | verified live; the 20-minute window is short and the demo's other tasks are random |
| Demo memory limits raised; all services below 65% after the change | `docker compose … config --format json` shows the new limits; `docker stats` 2026-10-06 after `make up`: checkout 23/64 MiB, product-catalog 29/64, ad 277/512, fraud-detection 253/512, accounting 190/320, kafka 596/1024, opensearch 997/1536, grafana 148/256, load-generator 660/2048; no unhealthy containers. Before: checkout 19/20 (95%), ad 272/300, load-generator 1.27/1.47 GiB | verified live; one snapshot |
| The agent service is not part of Tayga's stack | the `agent` service exists only in the demo's `compose.agent.yaml`; the Makefile's `COMPOSE` lists `compose.yaml`, `compose.full.yaml`, `compose.observability.yaml` and Tayga's two files | verified in files |
| `make up` on this branch builds an API with the new config fields | `make up` finished in 24 s; `curl localhost:8090/api/v1/config` returned `auth_enabled:false` and `infra_services:["flagd"]` | verified |
| Rust unit tests: 306 pass, 29 ignored | `cargo test --workspace` run today (sum of the `test result` lines) | verified |
| UI unit tests: 463 pass | `npm --prefix ui test` run today: `Tests 463 passed (463)`, 37 files | verified |
| Playwright on the 8090 app with auth disabled: 131 passed, 28 skipped, 0 failed | `npx playwright test` from `ui/` run today after `make up`: `131 passed (1.6m)`. The skips: 16 for `auth.spec.ts` (needs the auth env) and 12 motion specs (reduced-motion projects only) | one full run |
| `make e2e` on 2026-10-05 after `make up`: 8 of 9 passed in 497 s. Story times: ad 75 s, payment 55 s, unreachable 70 s, catalog 25 s; log spike 211 s; new-template probe 60 s after 0 s warmup (the probe service was already warm). `shipping_slowdown_produces_slow_story_blaming_shipping` stopped at its baseline pre-check: `checkout baselines cannot flag a 5 s trace as slow` with an empty endpoint table | `make e2e` output; `make flags-reset` run after | single run, not a latency guarantee. The test itself marked ad (75 s) and unreachable (70 s) as over its 60 s target; both passed |
| **Plan 7a (detection correctness)** | | |
| Rows below checked 2026-10-05 on branch `feat/plan-7a-detection` at `42d9dbb` (plus this docs commit), against the stack redeployed by `make up` at 16:10:20 UTC (28 s; migrations 0006 to 0008 applied). Docker disk 85% before the deploy | | |
| Migrations 0006, 0007, 0008 applied; the three `logminer_state` keys exist | live `SELECT key, value FROM logminer_state FINAL`: `masking_epoch_start_ns` 1791216620578814135 (16:10:20.58 UTC, the logminer's start), `masking_version` 2, `new_template_watermark_ns` 1791217400436115384 (16:23:20, advancing per pass); `show tables` lists `logminer_state`, `log_template_minutes`, `log_template_minutes_mv`; `describe log_alerts` shows `baseline_day`, `baseline_week` | verified live; **superseded** for `masking_version` (now 3, see the v3 rows below) |
| An upgrade over existing templates starts a masking epoch | logminer start log: `restored: 384`, `masking_version: 2`, `epoch_start` = 1791216620578814135; the DB had templates and no stored version | verified live (v2 deploy); **superseded**: the v3 deploy started a new epoch at 18:33:56, see below |
| No new-template alerts after the deploy during the epoch warmup | live `SELECT count() FROM log_alerts FINAL WHERE kind='new' AND started_at BETWEEN '16:10:20' AND '16:25:20'`: 0 (all services), 0 for `frontend-proxy`. Alerts in that window: one `spike` (`otelcol-contrib`, "Exporting failed. Will retry the request after interval."). The first `new` alert after the deploy came at 17:12 (`load-generator`, a currency-task timeout), outside the window | verified live (one window, the v2 deploy). After the v3 epoch a rarer combination alerted once past the warmup, see the 18:49:39 row below |
| `log_template_minutes` fills | live `SELECT count(), uniqExact(template_id) FROM log_template_minutes`: 847 rows at 16:24, 6091 rows over 68 templates at 17:54 | verified live |
| Existing `frontend-proxy` templates do not gain literal status codes on upgrade | live: 0 `frontend-proxy` templates matching a 3-digit 1xx-5xx token and none with `' 200 '`, 6 templates in total, none first seen after the deploy, while the stored log bodies contain `HTTP/1.1" 200`; templates still read `"GET <*> <*> <*> ...` | **superseded** (v2 behaviour). Since `ac8e775` a kept code never matches `<*>`, so access-log lines with a code start new templates even on existing services; no template wipe or re-mining is needed. See the v3 rows below |
| A new service's access-log template keeps the status code | live: `emit-log --service tayga-status-probe` with an Envoy-style body containing `HTTP/1.1" 503 UF` produced the template `<*> "GET /api/cart <*> 503 UF <*> <*> <*> - "-" "probe"` in `log_templates` | verified live (one body, one service; the probe service stays in the table until its 30-day TTL) |
| Status rule details (100 to 599, only after an `HTTP/x` token, `600` and non-HTTP numbers masked, per-token equals whole-string masking) | `is_status`, `is_http_version` in `crates/tayga-drain/src/preprocess.rs`; unit tests `keeps_status_after_http_version`, `other_numbers_stay_masked`, `per_token_masking_equals_whole_string_masking` | verified in code and unit tests |
| New metrics are exported | live: assembler `/metrics` showed `tayga_assembler_baseline_excluded_traces 2`, `tayga_assembler_baseline_carried_endpoints 0`, `tayga_assembler_baseline_endpoints 56`; logminer `/metrics` showed `tayga_logminer_spike_skipped_total{reason="coverage"} 0`, `tayga_logminer_seasonal_failures_total 0`, `tayga_logminer_state_save_failures_total 0` (scraped with `curlimages/curl` on the `opentelemetry-demo` network, because the Tayga images have no `wget` or `curl`) | verified live; the counters are 0, so no skip, failure or carry was observed live |
| Coverage rule, per-template coverage, young-template baseline, proportional shortening, the 50% gate | `detect.rs` and `spike_baseline`; unit tests and live store ITs (18) per the Task 4 report; spec section 2 | verified in code and tests; no live coverage skip occurred (counter 0) |
| Seasonal mode: 1-day and 7-day comparators, optional `baseline_day`/`baseline_week`, flat fallback on lookup error, MV dedup under replay | Task 5 report: store ITs against live ClickHouse (20), including `minutes_mv_does_not_double_count_replayed_hits`; `baseline_mode_setting_is_validated` in `crates/tayga-logminer/src/main.rs` | verified in unit and store tests; **not run live** (at the time the stack had under a day of `log_template_minutes`; since migration 0009 it holds minutes back to 2026-10-02 17:37; the deployed mode is `flat`) |
| Slow baselines exclude above-cap traces; endpoints are carried for at most 2 windows; no baseline from no traces | `crates/tayga-store/src/store.rs`, `crates/tayga-assembler/src/baselines.rs`; Task 3 report (unit tests and store ITs, including `one_outlier_does_not_raise_p99_with_previous_cap` and `bootstrap_cap_is_ten_times_p50`) | verified in code and tests; live: 2 excluded traces and 0 carried endpoints at one scrape, no sustained slowdown observed |
| Carry state is in memory only; a slowdown longer than 2 windows stops being flagged | spec section 3 and `baselines.rs` (no persistence) | code and spec only, not run live |
| Rust unit tests: 348 pass, 36 ignored | `cargo test --workspace` run today (sum of the `test result` lines) | **superseded**: 365 pass, 37 ignored after the final-review fixes, see below |
| `make e2e` on 2026-10-05 after the 7a deploy (about 2 h after `make up`): 7 of 9 passed in 647 s. Passed: log spike 175 s, new-template probe 60 s after 0 s warmup, payment 125 s, unreachable 80 s, catalog 25 s, raw span counts, service map. Failed: `ad_failure_blames_ad` (one story in 180 s; the test needs 3) and `shipping_slowdown_produces_slow_story_blaming_shipping` (stopped at its pre-check with an empty table) | `make e2e` output; `make flags-reset` run after | single run |
| The ad failure was not a regression; re-run alone it passed in 80 s | `cargo test -p tayga-e2e -- --ignored ad_failure_blames_ad`: `adFailure: first matching story after 80s`, 1 passed | single re-run; the first failure's cause is not established (the flag affects about one request in ten, so a slow draw is plausible) |
| The shipping pre-check stop is environmental | live `trace_summaries` for the last 60 min: `user_checkout_single` 73 of 73 and `user_checkout_multi` 59 of 59 traces had `is_error = 1`; the 10-minute counts show 100% checkout errors every interval since 13:10 UTC, hours before the deploy; `docker logs checkout` shows `panic: runtime error: invalid memory address or nil pointer dereference`. The pre-check needs non-error checkout traces, so no rows came back | error counts verified live. **Corrected**: the only panic in `docker logs -t checkout` is stamped 2026-10-04 18:00:22 UTC, a day earlier, so it is not the cause; the evidence points to the demo's Kafka (slow `publish orders`, recovery after its restart), see the checkout rows below |
| **Plan 7a final-review fixes (v3 masking)** | | |
| Rows below checked 2026-10-05 between 19:12 and 19:26 UTC on `feat/plan-7a-detection` with the fix commits after `ac8e775`, against the stack redeployed by `make up` at 19:16:29 to 19:16:55 UTC | | |
| `masking_version` 3, masking epoch at 2026-10-05 18:33:56 UTC | live `SELECT key, value FROM logminer_state FINAL`: `masking_epoch_start_ns` 1791225236728986679 (`fromUnixTimestamp64Nano`: 18:33:56.729), `masking_version` 3; the 19:16 restart logged `restored: 399`, `epoch_start: 1791225236728986679`, `masking_version: 3` (unchanged version, so no new epoch) | verified live |
| 7 new `frontend-proxy` templates carry a literal status, next to the 6 older `<*>` ones | live `log_templates FINAL` for `frontend-proxy`: 13 templates, 7 with a ` [1-5][0-9][0-9] ` token, codes 200 (4), 308, 503 and 504, first seen (log time) from 18:33:53.51 to 18:49:39.43; the other 6 were first seen 2026-10-02 05:37 to 2026-10-03 17:59. The first four are stamped up to 3.2 s before the epoch because lines logged before the logminer's start were mined after it | verified live |
| The 18:49:39 `new` alert for `<*> "GET <*> <*> 503 UC upstream_reset_before_response_started{connection_termination} ...` was a false positive | `log_alerts`: `kind='new'`, `frontend-proxy`, `started_at` 18:49:39.434. `logs` holds 14 `frontend-proxy` bodies containing ` 503 UC ` before the epoch (2026-10-02 16:16 to 2026-10-05 17:10; 13 of them still have hits, 6 in template 6470045815083748258 and 7 in 11039615203255878215, both with `<*>` at the status); 1 after it. The template is new only because the kept 503 no longer matches `<*>` | verified live. Fixed by the pre-epoch match (final review I1): replaying the rule over the live `log_templates` (status read as `<*>`, same length and first two tokens, similarity ≥ 0.5) matches 11039615203255878215 with similarity 1.0, so the alert would now be suppressed. Unit tests `a_status_split_out_of_a_pre_epoch_wildcard_template_would_have_matched`, `new_alerts_for_status_splits_of_pre_epoch_templates_are_suppressed` |
| A genuinely new status shape still alerts; templates without a kept code are judged as before | unit tests `a_genuinely_new_shape_with_a_status_would_not_have_matched`, `templates_without_a_protected_token_are_never_suppressed`, `pre_epoch_match_survives_a_restore_and_spares_non_http_templates` | verified in unit tests; not observed live |
| `tayga_logminer_new_suppressed_total{reason="pre_epoch_match"}` is exported | logminer `/metrics` after the 19:16 deploy (scraped with `curlimages/curl` on the `opentelemetry-demo` network): `tayga_logminer_new_suppressed_total{reason="pre_epoch_match"} 0` | verified live; 0, so no suppression observed live yet |
| Migration 0009 backfilled `log_template_minutes` without double counting | live: `schema_migrations` version 9 applied 19:16:52. Before the deploy the earliest minute was 2026-10-05 16:10:00 (the view's start, 187 minutes); after it 2026-10-02 17:37:00 (4,302 minutes). Sums of `uniqExactMerge(hits)` per (template, minute) equal sums of `uniqExact(log_id)` from `log_template_hits`: before 16:10, 8,433,847 = 8,433,847; the boundary minute 16:10, 2,243 = 2,243; 16:11 to 19:00, 359,986 = 359,986 | verified live; store IT `backfill_fills_minutes_before_the_view_without_double_counting` |
| Duration caps come only from trusted previous baselines; after a carry expires with `0 < kept < 50` the next refresh bootstraps and adopts the new level | `caps_from` in `crates/tayga-assembler/src/baselines.rs`; unit tests `caps_skip_untrusted_baselines`, `after_a_carry_expires_with_few_kept_the_next_refresh_adopts_the_new_level` | verified in unit tests (the 10 x p50 bootstrap itself is the store IT `bootstrap_cap_is_ten_times_p50`); not run live |
| An empty or failed consumer assignment holds the new-template clock at the watermark and keeps per-partition state | `assigned_partitions`, `pass_clock` in `crates/tayga-logminer/src/main.rs`; unit test `an_empty_or_failed_assignment_holds_at_the_watermark_and_keeps_partition_state` | verified in unit tests; no live rebalance tried |
| Rust unit tests: 365 pass, 37 ignored; store ITs: 21 pass | `cargo test --workspace` (sum of the `test result` lines); `TAYGA_IT_CLICKHOUSE=http://localhost:18123 cargo test -p tayga-store --test store_it -- --ignored` | verified |
| Demo checkout: every `user_checkout_*` trace errored from about 13:00 UTC until the controller restarted the demo's Kafka and `checkout` | `trace_summaries FINAL`, 15-minute buckets: 13:15 34/34 errors, 13:30 40/40, and every bucket from 17:15 to 18:45 100% (the table keeps 2 days; earlier, 11:00 to 12:00 had 89 of 109 errors and 12:00 to 13:00 only 2 traces). Proxy: 820 of 863 `POST /api/checkout` `frontend-proxy` bodies from 13:15 to 19:05 carry ` 504 ` (template `... "POST /api/checkout <*> 504 UT response_timeout ...`). `checkout`'s `publish orders` span, 13:00 to 19:05: 825 spans, median 91.5 s, max 589.4 s. Restart: `docker inspect` `StartedAt` kafka 19:07:16 UTC, checkout 19:07:57 UTC | verified live. The demo Kafka at 600.8 of its 620 MiB memory limit is the controller's `docker stats` reading before the restart and cannot be re-checked now (548.2 MiB / 620 MiB at 19:19) |
| Checkout recovered after the restart | `SELECT toStartOfFifteenMinutes(ts), count(), countIf(is_error=1) FROM tayga.trace_summaries FINAL WHERE endpoint_name LIKE 'user_checkout%' AND ts > now()-INTERVAL 2 HOUR GROUP BY 1 ORDER BY 1` at 19:25 UTC: 18:45 40/40 errors, 19:00 40/20, 19:15 25/0; in 5-minute buckets 19:05 9/5, then 19:10 16/0, 19:15 11/0, 19:20 14/0. `publish orders` after 19:08: 31 spans, median under 0.1 s | verified live (about 15 minutes after the restart) |
| Trap: span `status_code` is lowercase | `spans.status_code` is `Enum8('unset' = 0, 'ok' = 1, 'error' = 2)`; over `checkout` spans 13:15 to 19:00, `countIf(status_code='error')` = 86 while `countIf(status_code='ERROR')` = 0, with no error raised | verified live |
| **Plan 7b (silence alerts, notifier, remine)** | | |
| Rows below checked 2026-10-06 from 05:39 to 07:02 UTC on `feat/plan-7b-alerting` (`e8621ff` plus the Task 7 commits), against the stack rebuilt by `make up` at 05:39 UTC | | |
| A silence alert fires in log time, and `started_at` is the last hit | `make e2e-notifier`: templates A and B existed 5 s after the emit; silence enabled with `minutes = 2`; alert `fe0492738da0b683` 155 s after enabling, with `last_at − started_at` = 159 s. In `make e2e`: alert `f59f6d2b7fec6162` after 176 s, with 176 s. The test asserts `last_at − started_at ≥ minutes` | verified live (2 runs) |
| Silence UI: switch, minutes (default 10, 1 to 1440, disabled while off), bell, badge, "Silence" filter, "silent N min" | `ui/src/features/logs/SilenceCard.tsx` (`DEFAULT_MINUTES = 10`, `SILENCE_MAX = 1440`, `disabled={!cur.enabled}`), `TemplatesTable.tsx` (`Bell`), `routes/logs/alerts.tsx` (`{ value: 'silence', label: 'Silence' }`), `features/logs/model.ts` (`silent ${min} min`) | verified in code; the UI was not driven in a browser for these rows |
| PUT silence: 415, 400, 404, auth; `silence` and `silence_enabled` on the GETs | `put_silence` in `crates/tayga-api/src/routes.rs`; Task 3 router tests; `silence_put_needs_a_session_or_basic` in `auth.rs`; live PUTs (200) from both e2e runs | verified in code and tests; 200 path live |
| Partition-clock guard: `s_last` counts only up to the data clock | `is_silent` in `crates/tayga-drain/src/detect.rs` (`s_last.min(clock_ns)`); unit test `a_held_back_clock_suppresses_silence_alerts` | verified in unit tests; no lagging partition observed live |
| Silence alerts are not `alerting` and not badges on story logs | `kind != 'silence'` in `ALERTING_AT` and in the per-trace alert query, `crates/tayga-api/src/repo.rs` | verified in code |
| A silence ends when it is no longer refreshed; inactive 10 min after `last_at` | `active` = `last_at > end − ALERT_ACTIVE_MIN` (10) in `repo.rs`; live: after the scenario switched silence off, `fe0492738da0b683` showed `active: false` at 06:17 UTC with `last_at` 05:45:20 | verified live |
| `host.docker.internal` resolves inside the notifier on Docker Desktop for Mac | `docker exec tayga-notifier getent hosts host.docker.internal`: `192.168.65.254` | verified live; the `host-gateway` mapping for Linux is not tested |
| The override replaces the config mount | `docker compose … -f deploy/compose.notifier-e2e.yaml config tayga-notifier`: one bind of `deploy/tayga-notifier.e2e.toml` at `/etc/tayga/notifier.toml` | verified |
| The notifier delivers a fresh alert exactly once, also after `docker restart tayga-notifier` | `make e2e-notifier` (exit 0, 310.9 s): the mock got 1 delivery of `fe0492738da0b683`, 0 s after the alert showed in the API (4 requests in all, the others for other alerts). After the restart and a 150 s watch, still 1. `tayga.alerts` partition 2 holds the alert at offsets 760, 761 and 762 (`last_at` 05:43:20, 05:44:20, 05:45:20), so two re-publishes came after the restart. `notifier_deliveries`: one row, `e2e-mock`, `delivered`, `attempts` 1 | verified live (one run) |
| Webhook body fields, including `last_at` and `summary` | the delivered body above: `alert_id`, `kind`, `service`, `template_id`, `template`, `started_at`, `last_at`, `count` 0, `baseline` 0.0, `summary` "… has been silent for 2 min in tayga-e2e-probe", `example_trace_ids` [], `links`; content type `application/json` (asserted by the scenario) | verified live |
| The default config is target-less again after the check | `docker logs tayga-notifier` after `make e2e-notifier`: `delivery disabled: no targets`, and `tayga-notifier consuming` with `"targets":"[]"` | verified live |
| Retries, `Retry-After`, permanent 4xx/3xx, backoff, `max_age_secs`, the 40 s stop grace, URL redaction | `classify`, `retry_after_delay`, `backoff`, `retry_wait` in `crates/tayga-notifier/src/deliver.rs`; `route.rs`; `stop_grace_period: 40s` in `deploy/compose.tayga.yaml`; the Task 6 unit tests and the notifier IT | verified in code and tests; not exercised live with a failing target |
| Notifier metrics, the Pipeline job and the lag | Task 6 report (live): `/metrics` served the three series; `pipeline/series?metric=up&job=tayga-notifier` gave 1.0; `pipeline/lag` listed `tayga-notifier` with lag 0 | verified live on 2026-10-06 (Task 6), not re-checked here |
| Receivers should dedup on `alert_id`: hard-kill, lost-final-write and 30-day TTL resends | `deliver.rs` module docs; `TTL toDateTime(updated) + INTERVAL 30 DAY` in migration `0011_notifier_deliveries.sql` | verified in code; accepted, not reproduced |
| `remine --dry-run` on live data | 06:07:47 to 06:16:08 UTC, debug build: 8,655,171 logs read; templates 412 before, 402 after; 31 added, 41 removed, 371 unchanged by id; `otelcol-contrib` 35 → 21, `frontend-proxy` 19 → 20, `tayga-e2e-probe` 12 → 15; orphaned silence settings: none; duration 500.9 s (370 s user CPU) | verified live |
| The real run refuses while the heartbeat is fresh | after `docker compose … stop tayga-logminer` at 06:40:01: `Error: the logminer looks alive (heartbeat 43s ago, limit 180s). Stop it first with ...`, exit status 1 | verified live |
| Real re-mine | release build, 06:42:30 to 06:46:01 UTC: 8,647,835 logs read and hits written; templates 415 before, 402 after; 27 added, 40 removed, 375 unchanged; no orphaned silence; 211.1 s (33 s user CPU); it printed "Start the logminer again now." Before: 415 templates, 8,984,121 hits, 301,878 `log_template_minutes` rows. After: 402 templates, 8,647,205 hits, 259,642 minute rows | verified live |
| `logminer_state` after the re-mine | `masking_epoch_start_ns` 1791268950154855000 (06:42:30.15, was 1791225236728986679), `new_template_watermark_ns` 1791269158886828000 (06:45:58.89), `masking_version` 3; the heartbeat stayed at 06:39:21 until the restart. The logminer restart at 06:46:09 logged `restored: 402`, `epoch_start: 1791268950154855000`, `masking_version: 3` | verified live |
| No `new` alerts during the warmup after the re-mine | `SELECT toString(kind), count() FROM log_alerts FINAL WHERE started_at >= '2026-10-06 06:46:09' AND started_at < '2026-10-06 07:01:09' GROUP BY kind`: no rows, so no alert of any kind in the 15 minutes after the restart (also none up to 07:08). `log_templates` had 0 templates first seen after the epoch, and the logminer was live: heartbeat 07:08:11, 51,040 hits after 06:46:09 | verified live; weak test, because no new template appeared in the window |
| `make e2e`: 9 of 10 pass | 06:02:36 to 06:18:24 UTC, 943.9 s, exit 2. Passed: log spike 221 s, new-template probe 60 s (warm after 0 s), payment 111 s, unreachable 50 s, catalog 25 s, raw span counts, service map, shipping 115 s (its pre-check passed), silence 176 s. Failed: `ad_failure_blames_ad` (groups with 2 and 1 stories in 180 s; it needs 3). Run alone afterwards it passed in 85 s. `make flags-reset` run after both | verified live; the ad failure is the same intermittent miss as in plan 7a |
| Rust unit tests: 428 pass, 49 ignored | `cargo test --workspace` (sum of the `test result` lines); fmt and clippy `-D warnings` clean | verified |
| Migrations run before the services that need them under `make up` | `depends_on: tayga-migrate: condition: service_completed_successfully` on writer, assembler, logminer, notifier and api in `deploy/compose.tayga.yaml` | verified in config |
| Performance, scale, or latency claims | none beyond the single runs above | n/a |
