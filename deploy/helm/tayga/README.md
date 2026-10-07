# Tayga Helm chart

Installs Tayga on Kubernetes: the six Tayga services, the ClickHouse schema migration, and (for evaluation) a bundled single-node ClickHouse and Redpanda.

| Workload | Kind | Notes |
|---|---|---|
| `<release>-ingest` | Deployment, `ingest.replicas` | OTLP receiver. Service ports 4317 (gRPC) and 4318 (HTTP) |
| `<release>-writer` | Deployment, 1 replica | Raw spans and logs to ClickHouse |
| `<release>-assembler` | Deployment, 1 replica | Error stories, trace summaries, service edges |
| `<release>-logminer` | Deployment, `logminer.replicas` | Log templates and alerts; `tayga.logs` has 12 partitions, so replicas beyond 12 idle. Its Service is headless, so the API's recorder scrapes every replica |
| `<release>-api` | Deployment, `api.replicas` | Web app and JSON API on 8090; optional Ingress |
| `<release>-notifier` | Deployment, 1 replica, `notifier.enabled` | Delivers log alerts to webhook and Slack targets |
| `<release>-migrate[-<revision>]` | Job | `tayga-writer migrate`; see [Schema migration](#schema-migration) |
| `<release>-clickhouse` | StatefulSet, `clickhouse.enabled` | `clickhouse/clickhouse-server:26.8.15.10`, one node |
| `<release>-redpanda` | StatefulSet, `redpanda.enabled` | `redpandadata/redpanda:v26.2.3`, one broker in `dev-container` mode |

The release name `tayga` gives the shortest names (`tayga-api`, `tayga-ingest`, ...). Every Tayga pod runs as uid 10001 with a read-only root filesystem, no capabilities and the `RuntimeDefault` seccomp profile, and waits in an init container until Kafka accepts connections.

## Install

The chart is published as an OCI artifact:

```sh
helm install tayga oci://ghcr.io/softberries/charts/tayga --version 0.1.0 \
  --namespace tayga --create-namespace --wait
kubectl --namespace tayga port-forward svc/tayga-api 8090:8090   # then http://localhost:8090
```

From a checkout, with an image you built and loaded yourself (this is how the chart was tested, on kind):

```sh
docker build -f docker/Dockerfile -t tayga:local .
kind create cluster --name tayga-test
kind load docker-image tayga:local --name tayga-test
helm install tayga deploy/helm/tayga --namespace tayga --create-namespace \
  --set image.repository=tayga --set image.tag=local --set image.pullPolicy=Never --wait
```

Send OTLP traces and logs to `tayga-ingest.<namespace>.svc:4317` (gRPC) or `http://tayga-ingest.<namespace>.svc:4318` (HTTP). An OpenTelemetry Collector exporter:

```yaml
exporters:
  otlp_grpc/tayga:
    endpoint: tayga-ingest.tayga.svc:4317
    tls:
      insecure: true
    compression: gzip
    timeout: 30s
```

`deploy/standalone/otel-collector.yaml` is a complete collector config.

## Production: external ClickHouse and Kafka

The bundled ClickHouse and Redpanda are single instances meant for evaluation. For production, run them yourself (or as managed services) and point Tayga at them:

```yaml
clickhouse:
  enabled: false
redpanda:
  enabled: false
external:
  clickhouse:
    url: http://clickhouse.data.svc:8123
  kafka:
    brokers: kafka-0.kafka.data.svc:9092,kafka-1.kafka.data.svc:9092
```

Limits of what Tayga supports today: ClickHouse over HTTP as the `default` user without a password, and Kafka over PLAINTEXT without SASL. Keep both reachable only from Tayga's namespace (for example with NetworkPolicies). Tayga creates its topics (`tayga.signals`, `tayga.logs`, `tayga.stories`, `tayga.alerts`) and the ClickHouse database `database` (default `tayga`) itself.

## Schema migration

- **External ClickHouse:** the migration Job is a `pre-install` and `pre-upgrade` hook, so the schema is current before any Tayga pod starts or is replaced.
- **Bundled ClickHouse:** the database belongs to the same release and does not exist yet when pre-install hooks run. The Job is then a normal resource, named per revision (`<release>-migrate-<revision>`), and every Tayga pod except ingest waits in its init container until the schema reaches the version this chart expects. On an upgrade, the new pods wait for the new schema while the old ones keep running.

## Values

| Key | Default | Meaning |
|---|---|---|
| `image.repository`, `image.tag`, `image.pullPolicy` | `ghcr.io/softberries/tayga`, the chart's `appVersion`, `IfNotPresent` | The one image with every Tayga binary |
| `imagePullSecrets`, `nameOverride`, `fullnameOverride` | empty | Standard |
| `serviceAccount.create`, `.name`, `.annotations` | `true` | Tayga calls no Kubernetes API; the token is not mounted |
| `podSecurityContext`, `securityContext` | uid 10001, read-only root, drop ALL | Applied to Tayga pods only |
| `podAnnotations`, `podLabels`, `nodeSelector`, `tolerations`, `affinity` | empty | Applied to Tayga pods |
| `logLevel` | `info` | `RUST_LOG` of every Tayga service |
| `extraEnv` | `[]` | Extra env for every Tayga container, e.g. `TAYGA__KAFKA__RETENTION_MS` |
| `ingest.replicas`, `.service.type`, `.service.grpcPort`, `.service.httpPort` | `1`, `ClusterIP`, `4317`, `4318` | OTLP receiver |
| `logminer.replicas`, `logminer.fingerprinter` | `1`, `scalar` | Fingerprint cache in front of Drain: `scalar`, `parallel` or `off` |
| `api.replicas`, `api.service.type`, `api.service.port` | `1`, `ClusterIP`, `8090` | More than one replica needs `api.auth.sessionKey` when auth is on |
| `api.jaegerUrl`, `api.grafanaUrl` | empty | Optional links in the app |
| `api.infraServices` | `["flagd"]` | Services the map hides unless "Show infrastructure" is on |
| `api.auth.enabled`, `.username`, `.passwordHash`, `.sessionKey`, `.secureCookie` | off | Login. The hash is an Argon2id PHC string. Stored in a Secret |
| `api.auth.existingSecret` | empty | A Secret with `TAYGA__AUTH__USERNAME`, `TAYGA__AUTH__PASSWORD_HASH` and optionally `TAYGA__AUTH__SESSION_KEY` |
| `notifier.enabled`, `.publicUrl`, `.kinds` | `true`, `http://localhost:8090`, all kinds | Alert delivery; `publicUrl` is the base of the links in notifications |
| `notifier.targets` | `[]` | `[{name, kind: webhook\|slack, url}]`, rendered into a Secret. The URLs also stay in the release's values (`helm get values`); for real webhooks use `notifier.existingSecret` |
| `notifier.existingSecret` | empty | A Secret with a complete `notifier.toml` |
| `<service>.resources`, `<service>.extraEnv` | see `values.yaml` | Per service: `ingest`, `writer`, `assembler`, `logminer`, `api`, `notifier` |
| `migrate.backoffLimit`, `.activeDeadlineSeconds`, `.resources` | `6`, `900` | The migration Job |
| `ingress.enabled`, `.className`, `.annotations`, `.hosts`, `.tls` | off | Ingress to the API Service |
| `clickhouse.enabled`, `.image`, `.persistence.{enabled,size,storageClass}`, `.resources` | `true`, `26.8.15.10`, 20Gi | Bundled ClickHouse |
| `redpanda.enabled`, `.image`, `.memory`, `.persistence`, `.resources` | `true`, `v26.2.3`, `1G`, 10Gi | Bundled Redpanda; keep `resources.limits.memory` above `memory` |
| `external.clickhouse.url`, `external.kafka.brokers` | empty | Required when the bundled services are disabled |
| `database` | `tayga` | ClickHouse database |

`values.schema.json` validates every value; unknown keys are rejected.

Other Tayga settings (`TAYGA__SECTION__KEY`) go in `extraEnv` or a service's `extraEnv`; the configuration reference in the documentation lists them.

## Upgrade

```sh
helm upgrade tayga oci://ghcr.io/softberries/charts/tayga --version <new version> \
  --namespace tayga --reset-then-reuse-values --wait
```

`--reset-then-reuse-values` keeps your settings and takes the new chart's defaults for keys it adds (plain `--reuse-values` does not). The schema migration runs first; see [Schema migration](#schema-migration).

## Authentication

The image ships `tayga-devtools`; make the Argon2id hash with it (it asks for the password twice without echo):

```sh
kubectl --namespace tayga exec -it deploy/tayga-api -- tayga-devtools hash-password
```

Then turn login on, keeping the installed chart version:

```sh
helm upgrade tayga oci://ghcr.io/softberries/charts/tayga --version 0.1.0 --namespace tayga --reuse-values \
  --set api.auth.enabled=true --set api.auth.username=admin \
  --set-string api.auth.passwordHash='$argon2id$v=19$...' \
  --set-string api.auth.sessionKey="$(openssl rand -base64 32)"
```

Behind an HTTPS Ingress, also set `api.auth.secureCookie=true`. To keep the hash and key out of the release's values, put them in a Secret and set `api.auth.existingSecret`.

## Alert delivery

Webhook and Slack URLs are credentials. `notifier.targets` renders them into a Secret, but they also stay in the release's values. For real targets, create the Secret yourself and point the chart at it:

```sh
kubectl --namespace tayga create secret generic tayga-notifier-config --from-file=notifier.toml
helm upgrade tayga oci://ghcr.io/softberries/charts/tayga --version 0.1.0 --namespace tayga --reuse-values \
  --set notifier.existingSecret=tayga-notifier-config
```

The file has the format of `deploy/standalone/notifier.toml`; set `public_url` in its `[notifier]` table. The notifier redacts target URLs in its logs.

## Re-mining log templates

After a change to the Drain or masking settings, `tayga-devtools remine` rebuilds the templates (see the documentation's "Re-mining templates" page). A dry run is read-only:

```sh
kubectl --namespace tayga exec deploy/tayga-logminer -- tayga-devtools remine --dry-run
```

A real run needs every logminer replica stopped for 3 minutes; run it from the API pod, which has the same ClickHouse settings. Pass any `TAYGA__LOGMINER__*` variables you set in `logminer.extraEnv` through `env`:

```sh
kubectl --namespace tayga scale deploy/tayga-logminer --replicas 0
# wait 3 minutes
kubectl --namespace tayga exec deploy/tayga-api -- tayga-devtools remine
kubectl --namespace tayga scale deploy/tayga-logminer --replicas 1   # or your logminer.replicas
```

## Uninstall

```sh
helm uninstall tayga --namespace tayga
```

The bundled StatefulSets' PersistentVolumeClaims are kept, as Kubernetes does for every StatefulSet. Delete them to delete the data (names for the release `tayga`):

```sh
kubectl --namespace tayga delete pvc data-tayga-clickhouse-0 data-tayga-redpanda-0
```
