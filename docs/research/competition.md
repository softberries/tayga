# Competitive research

Deliverable D4 of the docs launch. Every fact below comes from a primary source: vendor docs, a vendor pricing page, a vendor blog, or a GitHub repository's license (read through the GitHub API). Every source was accessed on **2026-10-07**. Where a source does not state a fact, the cell says "not stated". Nothing is inferred from third-party reviews. Prices are quoted only where the vendor's own page states them, and they can change.

Tayga's own column is sourced to files in this repository.

## 1. Summary

- **Nobody else in this set builds per-request "error stories" from traces as they stream in, with a deterministic root cause.** The closest are Coroot, whose RCA runs a non-LLM analysis step and then uses an LLM to summarize it (Enterprise edition), and Dynatrace, which describes its RCA as "causal AI" over its topology model. Both are much broader products than Tayga.
- **Log template mining is common.** Grafana Loki (an optional pattern ingester, Drain), ClickStack (Drain3), Datadog, Dynatrace, New Relic, Coroot (on its node agent) and OpenObserve (Enterprise) all group logs into patterns. Most of them do it at query time, over a sample, without alerting on a new pattern. Loki's pattern ingester and Coroot's node agent mine as logs arrive. Tayga mines at ingest, keeps templates, and alerts on new, spiking and silent templates.
- **The commercial and the newer open-source products are moving to LLM agents** for investigation: Datadog's Bits AI, Dynatrace Intelligence agentic AI, New Relic Autopilot, Grafana Assistant, SigNoz Noz, OpenObserve's AI SRE Agent and ClickStack's AI Notebooks. Tayga uses no LLM. Its answers are rules over span data, so the same input gives the same output.
- **Every other product is broader than Tayga.** All of them cover metrics and dashboards at least, and several add eBPF auto-instrumentation (Coroot), profiling, RUM or session replay, and years of production use. Tayga needs OpenTelemetry traces and logs, has no general metrics store, and has only been tested against the OpenTelemetry demo.

## 2. Products

### Jaeger

- **What it is.** "A distributed tracing platform released as open source by Uber Technologies in 2016 and donated to Cloud Native Computing Foundation where it is a graduated project." [J1]
- **Analysis.** Query time. The UI shows trace search, a critical-path highlight in the trace view (`criticalPathEnabled`, default `true`) [J2], topology graphs, and Service Performance Monitoring (aggregated RED metrics) [J3].
- **Automatic RCA / error explanation.** Not stated. The docs list no automatic root-cause feature [J1][J3].
- **Log template mining.** Not stated (Jaeger is traces-only) [J1].
- **Self-hosting.** Yes. "Distributed as Docker images", with a "Kubernetes operator" and Helm chart support [J3].
- **License.** Apache-2.0 (GitHub `jaegertracing/jaeger`) [J4].
- **Pricing.** Free open source. No commercial offering stated [J1].
- **Instrumentation.** OpenTelemetry. Jaeger v2 is built on the OpenTelemetry Collector and accepts OTLP, Jaeger, Kafka and Zipkin [J5].
- **Where it beats Tayga.** A CNCF graduated project [J1]. Storage choice: Cassandra, Elasticsearch, OpenSearch, ClickHouse, Badger, in-memory [J3]. Adaptive sampling [J1]. Tayga's own UI links out to Jaeger for the raw trace view (README, "Open in Jaeger").

Sources:
- [J1] https://www.jaegertracing.io/docs/latest/ (2026-10-07)
- [J2] https://www.jaegertracing.io/docs/2.dev/deployment/frontend-ui/ (2026-10-07)
- [J3] https://www.jaegertracing.io/docs/1.76/features/ (2026-10-07)
- [J4] https://github.com/jaegertracing/jaeger, license field via GitHub API (2026-10-07)
- [J5] https://www.jaegertracing.io/docs/latest/architecture/ (2026-10-07)

### Grafana LGTM stack (Tempo, Loki, Grafana)

- **What it is.** Tempo is "an open-source, easy-to-use, and high-scale distributed tracing backend" that needs only object storage [G1]. Loki indexes "only ... metadata about your logs' labels" and stores compressed chunks in object storage [G2]. Grafana is the visualization layer.
- **Analysis.** Mostly query time (TraceQL [G1], LogQL [G2]). Two parts run at ingest: Tempo's metrics-generator produces service graphs and span metrics from traces [G1], and Loki's optional pattern ingester mines patterns as logs arrive [G3].
- **Automatic RCA / error explanation.**
  - Self-hosted OSS: not stated.
  - Grafana Cloud **Sift**: "a powerful, free diagnostic assistant included in Grafana Cloud" [G4]. Its analyses include Error Pattern Logs, HTTP Error Series, Kube Crashes, Noisy Neighbors, Recent Deployments and Resource Contention [G5]. Whether these are LLM-based is not stated [G4][G5].
  - **Grafana Assistant**: "a purpose-built LLM". It runs in Grafana Cloud, or in a self-managed Grafana connected to a Grafana Cloud Assistant backend [G6]. It runs multi-step investigations [G6].
- **Log template mining.** Yes. The Loki pattern ingester "uses a drain algorithm to identify related logs that share the same pattern". It is "disabled by default" [G3].
- **Self-hosting.** Yes: Tempo, Loki and Grafana are open source [G7]. Sift is Grafana Cloud only [G4]. The Assistant needs a Grafana Cloud backend [G6].
- **License.** AGPL-3.0 for `grafana/grafana`, `grafana/loki` and `grafana/tempo` [G7].
- **Pricing (Grafana Cloud).** Usage-based, plus per user [G8].
  - Free tier: 10k active metric series, and 50 GB of logs, traces and profiles per month.
  - Pro: a $19/month platform fee. Logs and traces are "$0.050/GB Process", "$0.400/GB Write" and "$0.100/GB Retain". Visualization starts at $8 per active user.
  - Assistant: "$20/ active AI user" plus "$2/ 1M tokens".
  - Enterprise: a "$25,000/ year spend commit" [G8].
- **Instrumentation.** OpenTelemetry, Jaeger and Zipkin for traces [G1].
- **Where it beats Tayga.** Metrics, dashboards and alerting across every signal. Long-term object-storage retention [G1][G2]. A large ecosystem. Managed cloud with an investigation assistant [G4][G6].

Sources:
- [G1] https://grafana.com/docs/tempo/latest/ (2026-10-07)
- [G2] https://grafana.com/docs/loki/latest/ (2026-10-07)
- [G3] https://grafana.com/docs/loki/latest/get-started/components/ (2026-10-07)
- [G4] https://grafana.com/docs/grafana-cloud/machine-learning/sift/ (2026-10-07)
- [G5] https://grafana.com/docs/grafana-cloud/machine-learning/sift/analyses/ (2026-10-07)
- [G6] https://grafana.com/docs/grafana-cloud/machine-learning/assistant/ (2026-10-07)
- [G7] https://github.com/grafana/grafana, https://github.com/grafana/loki and https://github.com/grafana/tempo, license fields via GitHub API (2026-10-07)
- [G8] https://grafana.com/pricing/ (2026-10-07)

### SigNoz

- **What it is.** "An open-source observability platform that helps you monitor your applications with distributed tracing, metrics, and logs", with exception tracking [S1]. It stores data in ClickHouse: "At the core of SigNoz's data storage is ClickHouse" [S2].
- **Analysis.** Query time (Query Builder, ClickHouse queries) [S2][S4]. Its own SigNoz OpenTelemetry Collector writes the data to ClickHouse [S2].
- **Automatic RCA / error explanation.** LLM-based: **Noz**, "SigNoz's AI teammate, built into the product". It "investigates across your telemetry" [S3]. Deterministic automatic RCA: not stated.
  - **Docs disagree on availability.** The AI overview says Noz is "Available to SigNoz Cloud users" [S3]. The AI use-cases page is tagged "SigNoz Cloud, Self-Host" [S5].
- **Log template mining.** Not in the current logs docs, which list Logs Explorer, Query Builder, Log Pipelines and Alerts [S4]. An undated SigNoz launch-week blog says: "Soon, you'll be able to detect patterns within logs" [S6]. Treated as not shipped.
- **Self-hosting.** Yes, as the Community Edition (Docker Compose, Linux, Kubernetes with Helm) [S1]. An Enterprise plan can be dedicated, BYOC or self-hosted [S7].
- **License.** MIT (Expat) outside `ee/` and `cmd/enterprise/`. Those directories fall under `ee/LICENSE` (repo `LICENSE` file) [S8].
- **Pricing.** Usage-based, with "no per-host or per-seat charges" [S7].
  - Teams: "$49/month, including $49 of usage". Logs and traces are "$0.3/GB ingested", and metrics are "$0.1/mn samples".
  - Enterprise: "starts at $4000/month".
  - Community Edition: free [S7].
- **Instrumentation.** OpenTelemetry. "Built on OpenTelemetry from day 1, with no proprietary agents" (SigNoz site, in search results for [S2]). The architecture page itself confirms only the OTel Collector ingest path [S2].
- **Where it beats Tayga.** One product covers metrics, traces, logs, dashboards and alerts. A hosted cloud option. A larger installed base (GitHub stars: about 32.3k on 2026-10-07 [S8]). Uses ClickHouse like Tayga, at broader scope.

Sources:
- [S1] https://signoz.io/docs/introduction/ (2026-10-07)
- [S2] https://signoz.io/docs/architecture/ (2026-10-07)
- [S3] https://signoz.io/docs/ai/overview/ (2026-10-07)
- [S4] https://signoz.io/docs/logs-management/overview/ (2026-10-07)
- [S5] https://signoz.io/docs/ai/use-cases/ (2026-10-07)
- [S6] https://signoz.io/blog/improvements-to-logs-search-and-filter/ (2026-10-07; the post shows no date)
- [S7] https://signoz.io/pricing/ (2026-10-07)
- [S8] https://github.com/SigNoz/signoz, `LICENSE` file and stars via GitHub API (2026-10-07)

### Coroot

- **What it is.** An observability platform built around `coroot-node-agent`, "an open-source observability agent powered by eBPF". The agent collects metrics, logs, traces and profiles from all containers on a node [C1]. Storage is Prometheus or ClickHouse for metrics, and ClickHouse for logs, traces and profiles [C2].
- **Analysis.** Mixed.
  - Log analysis runs "right on the node" as logs are collected [C3].
  - The RCA runs per investigation. It "follows the dependency graph starting from the affected service" [C4].
- **Automatic RCA / error explanation.** Yes: **AI-powered Root Cause Analysis**, a hybrid.
  - The first step "uses various machine learning techniques, but it doesn't involve LLMs".
  - Then "the LLM steps in ... it summarizes the findings and suggests possible fixes" [C4].
  - Supported LLM providers: Anthropic (recommended), OpenAI, and any OpenAI-compatible API [C5].
  - **Sources disagree on availability.** The editions page lists AI RCA as Enterprise only [C6]. The AI docs say it is "available in Coroot Enterprise ... or through Coroot Cloud integration for Community Edition users" [C7].
  - **Sources disagree on recommended models.** The launch blog recommends Claude 3.7 Sonnet and GPT-4o [C4]. The current configuration docs name Claude Opus 4.6 and GPT-5.2 [C5].
- **Log template mining.** Yes. The node agent identifies "message severities and recurring patterns". Records "that differ only in attribute values are grouped into the same pattern" [C3]. The logs docs do not state alerting on new patterns or pattern spikes [C3].
- **Self-hosting.** Yes, on Kubernetes (a DaemonSet), Docker, Ubuntu/Debian, RHEL/CentOS, Windows and OpenShift [C2]. The pricing page names no SaaS-only tier [C8].
- **License.** Apache-2.0 for `coroot/coroot` and `coroot/coroot-node-agent` [C9]. Enterprise is commercial [C6].
- **Pricing.** Per monitored CPU core: "$1 per monitored CPU core/month" for Enterprise. The Community Edition is "Free forever" [C6][C8].
- **Instrumentation.** eBPF with no code changes. It can "auto-instrument protocols including HTTP, Postgres, MySQL, Redis, MongoDB, and Memcached", but "eBPF-based spans may not provide complete traces" [C10]. It also accepts OTLP from OpenTelemetry SDKs [C1].
- **Where it beats Tayga.** eBPF auto-instrumentation with no code changes [C10]. Metrics, profiles and database monitoring [C1]. RCA that reaches infrastructure causes through the dependency graph [C4]. Proven on real fleets rather than one demo.

Sources:
- [C1] https://docs.coroot.com/configuration/coroot-node-agent/ (2026-10-07; quotes as returned by search for this page)
- [C2] https://docs.coroot.com/installation/architecture/ (2026-10-07)
- [C3] https://docs.coroot.com/logs/overview/ (2026-10-07)
- [C4] https://coroot.com/blog/we-built-ai-powered-root-cause-analysis-that-actually-works/ (2026-10-07)
- [C5] https://docs.coroot.com/ai/configuration/ (2026-10-07)
- [C6] https://coroot.com/editions (2026-10-07)
- [C7] https://docs.coroot.com/ai/ (2026-10-07)
- [C8] https://coroot.com/pricing/ (2026-10-07)
- [C9] https://github.com/coroot/coroot and https://github.com/coroot/coroot-node-agent, license via GitHub API and `LICENSE` file (2026-10-07)
- [C10] https://docs.coroot.com/tracing/ebpf-based-tracing/ (2026-10-07)

### OpenObserve

- **What it is.** "An open-source, unified, petabyte-scale observability platform for logs, metrics, and traces, built in Rust, with SQL and PromQL". It stores data on object storage in Parquet and also covers RUM [O1].
- **Analysis.** Query time (SQL, PromQL, full-text) [O1]. The AI SRE Agent starts "when an alert fires" [O3].
- **Automatic RCA / error explanation.** LLM-based: the **SRE Agent** gives "AI-powered root cause analysis with correlated logs, metrics, and traces" (Enterprise) [O2]. It "supports OpenAI, Anthropic Claude, Google Gemini, AWS Bedrock, DeepSeek, and OpenRouter" and "any OpenAI-compatible endpoint" [O3]. Deterministic RCA: not stated.
- **Log template mining.** Yes, as **Log Patterns**: "Automatic pattern extraction and anomaly identification", listed as an Enterprise feature [O2]. Whether patterns are mined at ingest or at query time is not stated [O2].
- **Self-hosting.** Yes, both the open-source edition and the Self-Hosted Enterprise. The Self-Hosted Enterprise is "free for up to 50 GB of ingestion per day" [O4].
- **License.** AGPL-3.0 (GitHub `openobserve/openobserve`) [O5]. Enterprise features are separate [O2].
- **Pricing (Cloud).** "$0.50/GB ingested" and "$0.01/GB queried", with unlimited users and "no per-seat charges" [O4]. The AI SRE Agent uses AI Credits at "$0.50 per AI Credit" [O3].
- **Instrumentation.** OpenTelemetry natively, plus 20+ other sources [O1].
- **Where it beats Tayga.** Logs, metrics, traces and RUM in one Rust binary with object storage [O1]. Patterns and an RCA agent in one product [O2]. Cost per GB stated openly [O4].

Sources:
- [O1] https://openobserve.ai/docs/ (2026-10-07)
- [O2] https://openobserve.ai/docs/enterprise-setup/enterprise-features/ (2026-10-07)
- [O3] https://openobserve.ai/ai-sre/ (2026-10-07)
- [O4] https://openobserve.ai/pricing/ (2026-10-07)
- [O5] https://github.com/openobserve/openobserve, license via GitHub API and `LICENSE` file (2026-10-07)

### ClickStack / HyperDX

- **What it is.** "A production-grade observability platform built on ClickHouse, unifying logs, traces, metrics and session" replays. Its parts are ClickHouse, the HyperDX UI and a custom OpenTelemetry Collector [K1].
- **Analysis.** Query time. Event patterns run Drain3 "at query time rather than at insert time ... Running clustering at ingest would allow pre-tagging of log patterns but would also introduce heavy overhead". They sample "10,000 events" [K2].
- **Automatic RCA / error explanation.** Not stated as an automatic feature.
  - **AI Notebooks** (beta, Managed ClickStack only) are "a persistent workspace for investigations" combining prompts, queries and findings [K3].
  - The **ClickStack MCP server** exposes trace analysis and event comparison to external agents [K3].
- **Log template mining.** Yes (event patterns, Drain3, query time, sampled) [K2]. Alerting on new patterns: not stated [K4].
- **Self-hosting.** Yes. "Open Source ClickStack" is self-managed and adds MongoDB for application state. "Managed ClickStack" runs in ClickHouse Cloud [K1].
- **License.** MIT for `hyperdxio/hyperdx` and Apache-2.0 for `ClickHouse/ClickHouse`. The `ClickHouse/ClickStack` repo has no license detected by GitHub [K5].
- **Pricing (Managed).** ClickHouse Cloud compute (per compute-unit-hour) and storage (per TB-month). For ClickStack, the configuration is "derived from your raw monthly ingest volume". Example list rates: AWS us-east-1 Enterprise compute "$0.39030" per unit-hour, storage "$25.30" per TB-month [K6].
- **Instrumentation.** OpenTelemetry [K1].
- **Where it beats Tayga.** Session replay and metrics [K1]. Ad hoc patterns over any subset or column [K2][K3]. ClickHouse at petabyte scale with a managed option [K6]. Backed by ClickHouse Inc.

Sources:
- [K1] https://clickhouse.com/docs/use-cases/observability/clickstack/overview (2026-10-07)
- [K2] https://clickhouse.com/blog/event-patterns-clickstack (2026-10-07)
- [K3] https://clickhouse.com/blog/whats-new-in-clickstack-may-2026 (2026-10-07)
- [K4] https://clickhouse.com/docs/use-cases/observability/clickstack/event_patterns (2026-10-07)
- [K5] https://github.com/hyperdxio/hyperdx, https://github.com/ClickHouse/ClickHouse and https://github.com/ClickHouse/ClickStack, license fields via GitHub API (2026-10-07)
- [K6] https://clickhouse.com/pricing (2026-10-07)

**Docs and blog disagree on detail.** The event-patterns docs page does not name the algorithm, or say whether patterns run at query or insert time [K4]. The vendor blog states both [K2].

### Datadog (brief)

- **What it is.** A SaaS observability platform: infrastructure, APM, logs and more [D4].
- **Automatic RCA.**
  - **Watchdog RCA** "requires the use of APM". It identifies "interdependencies between application performance anomalies and related components to draw causal relationships between symptoms". The method (statistical, ML or LLM) is not stated [D1].
  - **Bits AI** ("Bits Investigation", page at `bits_ai_sre`) is "an autonomous AI agent that investigates production issues end to end" [D2].
- **Log template mining.** Yes, **Log Patterns**: query time, "based on 10,000 log samples" [D3].
- **Self-hosting.** Not stated (no self-hosted edition is shown on the pricing page [D4]).
- **License.** Proprietary SaaS (no open-source license is stated for the platform) [D4].
- **Pricing.** Per host for Infrastructure ($15 per host per month billed annually) and APM ($31 per host per month billed annually). Per GB ingested for logs ($0.10) plus per million indexed events ($2.50 billed annually, 30-day retention) [D4]. Bits AI pricing: not stated.
- **Instrumentation.** The Datadog Agent with its DDOT Collector (recommended), an upstream OTel Collector, or direct OTLP [D5].
- **Where it beats Tayga.** Breadth and maturity. Hosted, with no operations burden. RCA across infrastructure and APM [D1].

Sources:
- [D1] https://docs.datadoghq.com/watchdog/rca/ (2026-10-07)
- [D2] https://docs.datadoghq.com/bits_ai/bits_ai_sre/ (2026-10-07)
- [D3] https://docs.datadoghq.com/logs/explorer/analytics/patterns/ (2026-10-07)
- [D4] https://www.datadoghq.com/pricing/list/ (2026-10-07)
- [D5] https://docs.datadoghq.com/opentelemetry/ (2026-10-07)

### Dynatrace (brief)

- **What it is.** A commercial observability platform with its own agent (OneAgent) and OTLP ingest [Y4].
- **Automatic RCA.**
  - **Dynatrace Intelligence causal AI root cause analysis** "automatically evaluates all captured and ingested information and highlights entities within the causal topology identified as the root cause" [Y1]. The RCA page does not mention LLMs [Y1]. Whether it is deterministic is not stated in those words.
  - Separately, **Dynatrace Intelligence agentic and generative AI** (Dynatrace Assist) translates prompts to DQL and analyzes environments [Y2].
- **Log template mining.** Yes, **Patterns (Preview)** in Logs. It is "a read-time view of the data", so it runs at query time [Y3].
- **Self-hosting.** Yes, as Dynatrace Managed: "all the monitoring capabilities on-premise within your own data center" [Y5].
- **License.** Proprietary (no open-source license stated) [Y6].
- **Pricing.** A commitment-based platform subscription [Y6]:
  - Full-Stack Monitoring: $58 per month per 8 GiB host ($0.01 per memory-GiB-hour).
  - Infrastructure Monitoring: $29 per host per month.
  - Logs: $0.20/GiB to ingest and process.
  - Traces: $0.20/GiB to ingest.
- **Instrumentation.** OneAgent, and OTLP directly, through a standard OTel Collector, or through the Dynatrace OTel Collector [Y4].
- **Where it beats Tayga.** Topology-wide causal RCA across infrastructure, processes and services [Y1]. Auto-instrumentation through OneAgent [Y4]. Enterprise maturity.

Sources:
- [Y1] https://docs.dynatrace.com/docs/dynatrace-intelligence/root-cause-analysis (2026-10-07)
- [Y2] https://docs.dynatrace.com/docs/dynatrace-intelligence/agentic-and-generative-ai/agentic-and-generative-ai-getting-started (2026-10-07; quotes as returned by search)
- [Y3] https://docs.dynatrace.com/docs/analyze-explore-automate/logs/lma-logs-app/patterns (2026-10-07)
- [Y4] https://docs.dynatrace.com/docs/ingest-from/opentelemetry (2026-10-07)
- [Y5] https://www.dynatrace.com/company/trust-center/sla/managed/ (2026-10-07)
- [Y6] https://www.dynatrace.com/pricing/ (2026-10-07)

### New Relic (brief)

- **What it is.** A commercial SaaS observability platform with APM agents and native OTLP ingest [N4].
- **Automatic RCA.** **New Relic Autopilot** (formerly SRE Agent) is "an AI-powered operations experience". It is "subject to the Generative AI Service Specific Terms". It is GA for Pro or Enterprise with Advanced Compute or Legacy Compute (CCU) [N1]. A deterministic RCA feature is not stated on that page.
- **Log template mining.** Yes, **log patterns**: "applies machine learning to normalize and group log messages". A model is built per account; this can take up to 24 hours after the feature is enabled. It "isn't available in all regions", and there is no separate price [N2].
- **Self-hosting.** Not stated (no self-hosted edition is shown on the pricing page [N3]).
- **License.** Proprietary SaaS (no open-source license stated) [N3].
- **Pricing.** Per GB ingested beyond a free 100 GB/month ("$0.40/GB", or "$0.60/GB" for Data Plus), plus per user (Core $49/user; Full platform users vary by edition), or compute-based (CCUs) [N3].
- **Instrumentation.** New Relic APM agents, or OTLP to `otlp.nr-data.net`, which it recommends as the preferred path for OTel data [N4].
- **Where it beats Tayga.** Breadth, hosted operation, a generous free ingest tier [N3], and log patterns with no extra charge [N2].

Sources:
- [N1] https://docs.newrelic.com/docs/agentic-ai/sre-agent/overview/ (2026-10-07)
- [N2] https://docs.newrelic.com/docs/logs/ui-data/find-unusual-logs-log-patterns/ (2026-10-07)
- [N3] https://newrelic.com/pricing (2026-10-07)
- [N4] https://docs.newrelic.com/docs/opentelemetry/best-practices/opentelemetry-otlp/ (2026-10-07; quotes as returned by search)

### Tayga (from this repo)

- **What it is.** Turns OpenTelemetry traces and logs into "error stories". For each failing or slow request it shows the root-cause span, the request path, the critical path, a diff against the endpoint's baseline, and related logs. Stories are grouped by fingerprint [T1].
- **Analysis.** Streaming. `tayga-assembler` closes per-trace session windows from a Kafka/Redpanda topic and writes stories. `tayga-logminer` mines templates per service as logs arrive, and runs detection every 60 s [T1].
- **Automatic RCA / error explanation.** Yes, deterministic: pure functions over the span tree [T2].
  - Root cause = the earliest-ending error leaf.
  - The explanation sentence is built from span attributes, for example "`<service>` could not reach `<peer>`".
  - For slow stories, the root cause is the top critical-path contributor [T2].
  - No LLM is used anywhere in the pipeline (crate list in [T1]).
- **Log template mining.** Yes, Drain per service at ingest, with masking [T1]. Templates persist (30-day TTL). Alerts fire on new templates, rate spikes and opt-in silence. They are delivered by webhook and Slack [T1].
- **Self-hosting.** Yes, Docker Compose (`make up`) [T1]. No Helm chart in the repo as of this commit.
- **License.** AGPL-3.0-only. An enterprise edition is available under a commercial license on request [T3].
- **Pricing.** The open-source core is free [T3]. Enterprise: "contact us", with no published prices [T4].
- **Instrumentation.** OpenTelemetry, OTLP gRPC and OTLP/HTTP [T1]. No eBPF, no own agent.
- **Known limits.** No general metrics store or dashboards of its own (Grafana and Prometheus are optional extras) [T1]. Tested against the OpenTelemetry demo 3.1.0 [T1]. Single ClickHouse store [T1].

Sources:
- [T1] `README.md` (architecture, Quick start, Web app, Log templates and alerts, ports table) at this commit
- [T2] `docs/superpowers/specs/2026-10-01-tayga-mvp-design.md` §9.2 Root cause, §9.3 Critical path, fingerprint rule
- [T3] `LICENSE`, `LICENSING.md`
- [T4] `docs/superpowers/specs/2026-10-07-tayga-docs-launch-design.md` (enterprise wording: "available on request", no prices)

## 3. Comparison matrix

"Q" = query time. "S" = streaming, at ingest.

| Product | Analysis timing | Automatic RCA / error explanation | LLM in RCA? | Log template mining | Self-host | License | Pricing model | Instrumentation |
|---|---|---|---|---|---|---|---|---|
| **Tayga** | S [^t1] | Yes, per request, error and slow stories [^t2] | No, deterministic rules [^t3] | Yes: Drain at ingest, plus new/spike/silence alerts [^t4] | Yes, Compose [^t5] | AGPL-3.0-only [^t6] | Free core; enterprise on request [^t7] | OTel (OTLP) [^t8] |
| Jaeger | Q [^j1] | Not stated (critical-path view only) [^j2] | n/a [^j3] | No (traces only) [^j4] | Yes [^j5] | Apache-2.0 [^j6] | Free OSS [^j7] | OTel, Jaeger, Zipkin [^j8] |
| Grafana LGTM | Q, plus S for Tempo metrics-generator and Loki patterns [^g1] | OSS: not stated. Cloud: Sift and Assistant [^g2] | Assistant is an LLM; Sift's method is not stated [^g3] | Yes: Loki pattern ingester (Drain, off by default) [^g4] | Yes for OSS; Sift and Assistant need Cloud [^g5] | AGPL-3.0 [^g6] | Cloud: per GB, per series, per user [^g7] | OTel, Jaeger, Zipkin [^g8] |
| SigNoz | Q [^s1] | Noz (AI teammate) [^s2] | Yes (AI teammate) [^s3] | Not in current docs ("soon" in a blog) [^s4] | Yes (Community) [^s5] | MIT, plus `ee/` license [^s6] | Per GB / per sample; no per-seat [^s7] | OTel [^s8] |
| Coroot | Logs S on node; RCA per investigation [^c1] | Yes: AI-powered RCA [^c2] | Hybrid: non-LLM analysis, then an LLM summary [^c3] | Yes, on the node agent [^c4] | Yes [^c5] | Apache-2.0 [^c6] | Per CPU core (Enterprise) [^c7] | eBPF plus OTel [^c8] |
| OpenObserve | Q [^o1] | SRE Agent (Enterprise) [^o2] | Yes (LLM providers) [^o3] | Yes, Log Patterns (Enterprise) [^o4] | Yes [^o5] | AGPL-3.0 [^o6] | Per GB ingested and queried [^o7] | OTel [^o8] |
| ClickStack / HyperDX | Q [^k1] | Not stated (AI Notebooks beta, MCP server) [^k2] | AI Notebooks (prompts) [^k3] | Yes: Drain3, query time, sampled [^k4] | Yes [^k5] | MIT (HyperDX), Apache-2.0 (ClickHouse) [^k6] | Managed: compute plus storage [^k7] | OTel [^k8] |
| Datadog | Not stated [^d1] | Watchdog RCA (needs APM); Bits AI [^d2] | Bits AI is an AI agent; Watchdog's method is not stated [^d3] | Yes, Q, 10k samples [^d4] | Not stated [^d5] | Proprietary [^d6] | Per host plus per GB plus per indexed event [^d7] | Agent or OTel [^d8] |
| Dynatrace | Not stated [^y1] | Causal AI RCA [^y2] | RCA page: no LLM mentioned; separate GenAI assistant [^y3] | Yes, Q (Preview) [^y4] | Yes (Managed) [^y5] | Proprietary [^y6] | Platform subscription; per host-hour, per GiB [^y7] | OneAgent or OTel [^y8] |
| New Relic | Not stated [^n1] | Autopilot [^n2] | Yes (GenAI terms) [^n3] | Yes, ML model per account [^n4] | Not stated [^n5] | Proprietary [^n6] | Per GB plus per user, or compute [^n7] | APM agents or OTel [^n8] |

Footnotes. All web sources were accessed 2026-10-07. Repo sources are files at this commit.

[^t1]: Tayga streaming: README architecture (assembler session windows; logminer detection every 60 s). [T1]
[^t2]: Tayga stories: README intro, "failing or slow request ... root-cause span". [T1]
[^t3]: Tayga deterministic: MVP spec §9 ("All functions are pure"), §9.2. [T2]
[^t4]: Tayga logs: README "Log templates and alerts". [T1]
[^t5]: Tayga self-host: README "Quick start" (`make up`). [T1]
[^t6]: Tayga license: `LICENSE`, `LICENSING.md`. [T3]
[^t7]: Tayga pricing: `LICENSING.md` (commercial license on request); launch design (no prices). [T3][T4]
[^t8]: Tayga instrumentation: README architecture and ports table (OTLP gRPC and HTTP). [T1]
[^t9]: Tayga limits: README Quick start (Grafana and Prometheus only with `make up-extras`; OTel demo 3.1.0). [T1]
[^j1]: Jaeger trace search and UI: https://www.jaegertracing.io/docs/latest/ [J1]
[^j2]: Jaeger critical path: https://www.jaegertracing.io/docs/2.dev/deployment/frontend-ui/ [J2]
[^j3]: Jaeger has no RCA feature listed: https://www.jaegertracing.io/docs/1.76/features/ [J3]
[^j4]: Jaeger is a tracing platform: https://www.jaegertracing.io/docs/latest/ [J1]
[^j5]: Jaeger Docker, operator and Helm: https://www.jaegertracing.io/docs/1.76/features/ [J3]
[^j6]: Jaeger license: https://github.com/jaegertracing/jaeger (GitHub API) [J4]
[^j7]: Jaeger, a CNCF open-source project: https://www.jaegertracing.io/docs/latest/ [J1]
[^j8]: Jaeger receivers: https://www.jaegertracing.io/docs/latest/architecture/ [J5]
[^g1]: TraceQL, LogQL, metrics-generator and pattern ingester: [G1][G2][G3]
[^g2]: Sift (Cloud) and Assistant: https://grafana.com/docs/grafana-cloud/machine-learning/sift/ and https://grafana.com/docs/grafana-cloud/machine-learning/assistant/ [G4][G6]
[^g3]: Assistant is "a purpose-built LLM" [G6]; Sift docs do not state a method [G4][G5]
[^g4]: Loki pattern ingester: https://grafana.com/docs/loki/latest/get-started/components/ [G3]
[^g5]: Sift is "included in Grafana Cloud" [G4]; Assistant needs a Cloud backend [G6]
[^g6]: Grafana, Loki and Tempo licenses: GitHub API [G7]
[^g7]: Grafana Cloud pricing: https://grafana.com/pricing/ [G8]
[^g8]: Tempo protocols: https://grafana.com/docs/tempo/latest/ [G1]
[^s1]: SigNoz query and storage: https://signoz.io/docs/architecture/ [S2]
[^s2]: Noz: https://signoz.io/docs/ai/overview/ [S3]
[^s3]: Noz, "AI teammate": [S3]
[^s4]: SigNoz logs docs and blog: https://signoz.io/docs/logs-management/overview/ and https://signoz.io/blog/improvements-to-logs-search-and-filter/ [S4][S6]
[^s5]: SigNoz self-host options: https://signoz.io/docs/introduction/ [S1]
[^s6]: SigNoz `LICENSE`: https://github.com/SigNoz/signoz [S8]
[^s7]: SigNoz pricing: https://signoz.io/pricing/ [S7]
[^s8]: SigNoz OTel Collector ingest: https://signoz.io/docs/architecture/ [S2]
[^c1]: Coroot on-node log analysis and RCA flow: https://docs.coroot.com/logs/overview/ and the RCA blog [C3][C4]
[^c2]: Coroot AI RCA: https://docs.coroot.com/ai/ [C7]
[^c3]: Coroot RCA steps: https://coroot.com/blog/we-built-ai-powered-root-cause-analysis-that-actually-works/ [C4]
[^c4]: Coroot log patterns: https://docs.coroot.com/logs/overview/ [C3]
[^c5]: Coroot install options: https://docs.coroot.com/installation/architecture/ [C2]
[^c6]: Coroot license: GitHub API and `LICENSE` file [C9]
[^c7]: Coroot pricing: https://coroot.com/pricing/ and https://coroot.com/editions [C8][C6]
[^c8]: Coroot eBPF and OTLP: https://docs.coroot.com/tracing/ebpf-based-tracing/ and the node-agent page [C10][C1]
[^o1]: OpenObserve SQL and PromQL: https://openobserve.ai/docs/ [O1]
[^o2]: OpenObserve SRE Agent: https://openobserve.ai/docs/enterprise-setup/enterprise-features/ [O2]
[^o3]: OpenObserve LLM providers: https://openobserve.ai/ai-sre/ [O3]
[^o4]: OpenObserve Log Patterns (Enterprise): [O2]
[^o5]: OpenObserve self-hosted plans: https://openobserve.ai/pricing/ [O4]
[^o6]: OpenObserve license: GitHub API and `LICENSE` file [O5]
[^o7]: OpenObserve pricing: https://openobserve.ai/pricing/ [O4]
[^o8]: OpenObserve OTel: https://openobserve.ai/docs/ [O1]
[^k1]: ClickStack query-time patterns: https://clickhouse.com/blog/event-patterns-clickstack [K2]
[^k2]: ClickStack AI Notebooks and MCP: https://clickhouse.com/blog/whats-new-in-clickstack-may-2026 [K3]
[^k3]: AI Notebooks "combine prompts, queries ...": [K3]
[^k4]: Drain3, query time, 10,000 events: [K2]
[^k5]: Open Source ClickStack, self-managed: https://clickhouse.com/docs/use-cases/observability/clickstack/overview [K1]
[^k6]: HyperDX, ClickHouse and ClickStack licenses: GitHub API [K5]
[^k7]: ClickStack managed pricing: https://clickhouse.com/pricing [K6]
[^k8]: ClickStack OTel Collector: [K1]
[^d1]: Datadog analysis timing: not stated in [D1]–[D5]
[^d2]: Watchdog RCA and Bits: https://docs.datadoghq.com/watchdog/rca/ and https://docs.datadoghq.com/bits_ai/bits_ai_sre/ [D1][D2]
[^d3]: Bits is "an autonomous AI agent" [D2]; Watchdog's method is not stated [D1]
[^d4]: Datadog Log Patterns: https://docs.datadoghq.com/logs/explorer/analytics/patterns/ [D3]
[^d5]: Datadog self-host: no such option on https://www.datadoghq.com/pricing/list/ [D4]
[^d6]: Datadog license: no open-source license stated [D4]
[^d7]: Datadog list prices: https://www.datadoghq.com/pricing/list/ [D4]
[^d8]: Datadog OTel options: https://docs.datadoghq.com/opentelemetry/ [D5]
[^y1]: Dynatrace analysis timing: not stated in [Y1]–[Y6]
[^y2]: Dynatrace RCA: https://docs.dynatrace.com/docs/dynatrace-intelligence/root-cause-analysis [Y1]
[^y3]: Dynatrace RCA page and the GenAI page: [Y1][Y2]
[^y4]: Dynatrace Patterns (Preview), "read-time view": https://docs.dynatrace.com/docs/analyze-explore-automate/logs/lma-logs-app/patterns [Y3]
[^y5]: Dynatrace Managed, on-premise: https://www.dynatrace.com/company/trust-center/sla/managed/ [Y5]
[^y6]: Dynatrace license: no open-source license stated [Y6]
[^y7]: Dynatrace pricing: https://www.dynatrace.com/pricing/ [Y6]
[^y8]: Dynatrace OTLP and OneAgent: https://docs.dynatrace.com/docs/ingest-from/opentelemetry [Y4]
[^n1]: New Relic analysis timing: not stated in [N1]–[N4]
[^n2]: New Relic Autopilot: https://docs.newrelic.com/docs/agentic-ai/sre-agent/overview/ [N1]
[^n3]: "Subject to the Generative AI Service Specific Terms": [N1]
[^n4]: New Relic log patterns: https://docs.newrelic.com/docs/logs/ui-data/find-unusual-logs-log-patterns/ [N2]
[^n5]: New Relic self-host: no such option on https://newrelic.com/pricing [N3]
[^n6]: New Relic license: no open-source license stated [N3]
[^n7]: New Relic pricing: https://newrelic.com/pricing [N3]
[^n8]: New Relic OTLP endpoint: https://docs.newrelic.com/docs/opentelemetry/best-practices/opentelemetry-otlp/ [N4]

## 4. Where Tayga wins / where others win

Only claims backed by the matrix above.

**Where Tayga wins**

- **Per-request error and slow stories from traces, built as they stream in.** Each story gets a root-cause span, a critical path and a baseline diff [^t1][^t2]. None of the other nine documents per-request stories built at ingest. Their RCA runs per investigation or alert (Coroot, OpenObserve, Datadog, New Relic), or the docs do not state when it runs [^c1][^o2][^d2][^n2].
- **Deterministic, no LLM.** The root cause comes from fixed rules over the span tree [^t3]. Coroot's RCA ends in an LLM summary [^c3]. Grafana Assistant, SigNoz Noz, OpenObserve's SRE Agent, Datadog's Bits AI and New Relic Autopilot are AI or LLM agents [^g3][^s3][^o3][^d3][^n3].
- **Log templates mined at ingest, with new/spike/silence alerts delivered to webhook and Slack** [^t4]. ClickStack, Datadog and Dynatrace mine patterns at query time over a sample or as a read-time view [^k4][^d4][^y4]. The Loki pattern ingester is off by default [^g4]. None of the sources fetched for this research describes an alert that fires when a new log pattern first appears.
- **Stories and log alerting are in the self-hosted AGPL core** [^t4][^t6]. OpenObserve's patterns and RCA agent are Enterprise [^o2][^o4]. Coroot's AI RCA is Enterprise, or the Community Edition through Coroot Cloud [^c2]. Grafana's Sift and Assistant need Grafana Cloud [^g5].

**Where others win**

- **Breadth.** Every product here except Jaeger covers general metrics and dashboards; Tayga has neither natively (Grafana and Prometheus are optional extras) [^t9]. SigNoz, OpenObserve and ClickStack each cover traces, metrics and logs in one product [^s1][^o1][^k5].
- **No-code instrumentation.** Coroot's eBPF agent traces services without OpenTelemetry SDKs [^c8]. Dynatrace has OneAgent [^y8]. Tayga needs OTel traces and logs [^t8].
- **RCA that reaches infrastructure.** Coroot walks the dependency graph across infrastructure signals [^c3]. Dynatrace evaluates its whole causal topology [^y2]. Datadog Watchdog RCA works across APM and infrastructure [^d2]. Tayga's root cause is a span.
- **Hosted options.** Grafana Cloud, SigNoz Cloud, OpenObserve Cloud, Managed ClickStack, Datadog, Dynatrace and New Relic all offer a hosted service [^g7][^s7][^o7][^k7][^d7][^y7][^n7]. Tayga has none.
- **Maturity and scale.** Jaeger is a CNCF graduated project [^j7]. The others support Kubernetes and Helm or managed scale [^j5][^s5][^c5]. Tayga ships Docker Compose only, and is tested against the OpenTelemetry demo [^t5][^t9].

## 5. What could not be verified

- Datadog, Dynatrace and New Relic: whether analysis runs at ingest or at query time, outside log patterns ("not stated").
- Datadog Watchdog RCA and Grafana Sift: whether they use an LLM ("not stated").
- Datadog: the price of Bits AI. The pricing page excerpt mixed it with LLM Observability spans, so it is left out.
- Datadog and New Relic: an explicit vendor statement that no self-hosted edition exists. Only the absence of one on the pricing pages was checked.
- SigNoz: whether Noz runs on self-hosted installs. The docs disagree [S3][S5].
- Coroot: whether the Community Edition gets AI RCA through Coroot Cloud. The docs and the editions page disagree [C6][C7].
- OpenObserve Log Patterns: ingest time or query time ("not stated").
- ClickStack: alerting on new event patterns ("not stated").
- Jaeger: trace comparison. Not found in the current docs, so not claimed.
