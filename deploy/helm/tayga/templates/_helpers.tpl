{{/* Names */}}
{{- define "tayga.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "tayga.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{- define "tayga.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/* Common labels. Call with (dict "ctx" $ "component" "api"). */}}
{{- define "tayga.labels" -}}
helm.sh/chart: {{ include "tayga.chart" .ctx }}
{{ include "tayga.selectorLabels" . }}
app.kubernetes.io/version: {{ .ctx.Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .ctx.Release.Service }}
app.kubernetes.io/part-of: tayga
{{- end }}

{{- define "tayga.selectorLabels" -}}
app.kubernetes.io/name: {{ include "tayga.name" .ctx }}
app.kubernetes.io/instance: {{ .ctx.Release.Name }}
app.kubernetes.io/component: {{ .component }}
{{- end }}

{{- define "tayga.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "tayga.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{- define "tayga.image" -}}
{{- printf "%s:%s" .Values.image.repository (default .Chart.AppVersion .Values.image.tag) }}
{{- end }}

{{/*
The highest ClickHouse schema migration this version applies
(crates/tayga-store/migrations; ci.yml checks they agree). With the bundled
ClickHouse the services wait until the schema reaches it.
*/}}
{{- define "tayga.schemaVersion" -}}12{{- end }}

{{/* Backends */}}
{{- define "tayga.clickhouseHost" -}}
{{- printf "%s-clickhouse" (include "tayga.fullname" .) }}
{{- end }}

{{- define "tayga.clickhouseUrl" -}}
{{- if .Values.clickhouse.enabled }}
{{- printf "http://%s:8123" (include "tayga.clickhouseHost" .) }}
{{- else }}
{{- required "external.clickhouse.url is required when clickhouse.enabled is false" .Values.external.clickhouse.url }}
{{- end }}
{{- end }}

{{- define "tayga.kafkaBrokers" -}}
{{- if .Values.redpanda.enabled }}
{{- printf "%s-redpanda:9092" (include "tayga.fullname" .) }}
{{- else }}
{{- required "external.kafka.brokers is required when redpanda.enabled is false" .Values.external.kafka.brokers }}
{{- end }}
{{- end }}

{{/* Environment shared by every Tayga container. */}}
{{- define "tayga.env" -}}
- name: RUST_LOG
  value: {{ .Values.logLevel | quote }}
- name: TAYGA__KAFKA__BROKERS
  value: {{ include "tayga.kafkaBrokers" . | quote }}
- name: TAYGA__CLICKHOUSE__URL
  value: {{ include "tayga.clickhouseUrl" . | quote }}
- name: TAYGA__CLICKHOUSE__DATABASE
  value: {{ .Values.database | quote }}
{{- with .Values.extraEnv }}
{{ toYaml . }}
{{- end }}
{{- end }}

{{/*
Init container that waits for the first Kafka broker to accept connections
and, with the bundled ClickHouse, for the schema migration to reach
tayga.schemaVersion. The image has bash but no curl. Call with (dict "ctx" $ "schema" true).
*/}}
{{- define "tayga.waitInit" -}}
{{- $ctx := .ctx -}}
- name: wait-for-backends
  image: {{ include "tayga.image" $ctx }}
  imagePullPolicy: {{ $ctx.Values.image.pullPolicy }}
  securityContext:
    {{- toYaml $ctx.Values.securityContext | nindent 4 }}
  resources:
    requests: {cpu: 10m, memory: 16Mi}
    limits: {memory: 32Mi}
  env:
    - name: KAFKA_BROKER
      value: {{ include "tayga.kafkaBrokers" $ctx | splitList "," | first | trim | quote }}
    {{- if and .schema $ctx.Values.clickhouse.enabled }}
    - name: CH_HOST
      value: {{ include "tayga.clickhouseHost" $ctx | quote }}
    - name: CH_DB
      value: {{ $ctx.Values.database | quote }}
    - name: SCHEMA_VERSION
      value: {{ include "tayga.schemaVersion" $ctx | quote }}
    {{- end }}
  command:
    - bash
    - -c
    - |
      host="${KAFKA_BROKER%:*}"; port="${KAFKA_BROKER##*:}"
      until (exec 3<>"/dev/tcp/$host/$port") 2>/dev/null; do
        echo "waiting for Kafka at $KAFKA_BROKER"; sleep 2
      done
      [ -n "${CH_HOST:-}" ] || exit 0
      schema() {
        exec 3<>"/dev/tcp/$CH_HOST/8123" || return
        printf 'GET /?query=SELECT%%20max(version)%%20FROM%%20%s.schema_migrations HTTP/1.0\r\nHost: %s\r\n\r\n' "$CH_DB" "$CH_HOST" >&3
        tail -n 1 <&3
      }
      while :; do
        v="$(schema 2>/dev/null || true)"
        case "$v" in ''|*[!0-9]*) v=0 ;; esac
        [ "$v" -ge "$SCHEMA_VERSION" ] && break
        echo "waiting for ClickHouse schema version $SCHEMA_VERSION (at $v)"; sleep 3
      done
{{- end }}

{{/* Fields shared by every Tayga pod spec. */}}
{{- define "tayga.podCommon" -}}
serviceAccountName: {{ include "tayga.serviceAccountName" . }}
{{- with .Values.imagePullSecrets }}
imagePullSecrets:
  {{- toYaml . | nindent 2 }}
{{- end }}
securityContext:
  {{- toYaml .Values.podSecurityContext | nindent 2 }}
{{- with .Values.nodeSelector }}
nodeSelector:
  {{- toYaml . | nindent 2 }}
{{- end }}
{{- with .Values.tolerations }}
tolerations:
  {{- toYaml . | nindent 2 }}
{{- end }}
{{- with .Values.affinity }}
affinity:
  {{- toYaml . | nindent 2 }}
{{- end }}
{{- end }}
