/**
 * Zod schemas mirroring the tayga-api response views (crates/tayga-api/src/model.rs,
 * lag.rs, routes_v2.rs and the story JSON from tayga-analysis). They are strict: an unknown
 * or missing field fails the contract test against the captured fixtures, so API drift is
 * caught. App code imports the inferred types from ./types only; do not import these values
 * outside tests, or zod lands in the bundle.
 *
 * Numbers: `*_ns` timestamps (~1.8e18) exceed 2^53, so JSON.parse rounds them to the nearest
 * 256 ns. That is below display precision; never use them as identifiers.
 * Ids that must stay exact (fingerprint, template_id, log_id) are strings in the API.
 */
import { z } from 'zod'

const int = z.number().int()
const num = z.number()
/** Nanoseconds (timestamps ~1.8e18 are beyond 2^53, so not checked as safe integers). */
const ns = z.number().nonnegative()
const str = z.string()
/** `[key, value]` attribute pair. */
const attr = z.tuple([str, str])
/** `(bucket start in unix seconds, count)`. */
const countBucket = z.tuple([int, int])

export const StoryKindSchema = z.enum(['error', 'slow'])
export const SpanKindSchema = z.enum(['unspecified', 'internal', 'server', 'client', 'producer', 'consumer'])
export const SpanStatusSchema = z.enum(['unset', 'ok', 'error'])
export const AlertKindSchema = z.enum(['new', 'spike'])
export const HealthSchema = z.enum(['ok', 'slow', 'error'])
export const SeriesKindSchema = z.enum(['rate', 'gauge', 'q50', 'q99'])

/** `GET /story-groups` element (GroupView: StoryGroupRow flattened + buckets). */
export const StoryGroupSchema = z.strictObject({
  fingerprint: str,
  kind: StoryKindSchema,
  summary: str,
  rc_service: str,
  rc_span_name: str,
  endpoint_service: str,
  endpoint_name: str,
  stories: int,
  first_seen_ns: ns,
  last_seen_ns: ns,
  sample_story_id: str,
  bucket_secs: int,
  buckets: z.array(countBucket),
})

export const StorySummarySchema = z.strictObject({
  story_id: str,
  ts_ns: ns,
  trace_id: str,
  duration_ns: ns,
  summary: str,
})

/** `GET /story-groups/{fingerprint}` */
export const GroupDetailSchema = z.strictObject({
  group: StoryGroupSchema,
  examples: z.array(StorySummarySchema),
})

export const RootCauseSchema = z.strictObject({
  service: str,
  span_name: str,
  span_kind: SpanKindSchema,
  span_id: str,
  message: str,
  exception_type: str,
})

export const SpanRefSchema = z.strictObject({
  span_id: str,
  service: str,
  name: str,
  kind: SpanKindSchema,
  status: SpanStatusSchema,
  start_ns: ns,
  duration_ns: ns,
})

export const PathSegmentSchema = z.strictObject({
  span_id: str,
  service: str,
  name: str,
  start_ns: ns,
  end_ns: ns,
})

export const ContributorSchema = z.strictObject({
  span_id: str,
  service: str,
  name: str,
  self_time_ns: ns,
})

export const CriticalPathSchema = z.strictObject({
  segments: z.array(PathSegmentSchema),
  top: z.array(ContributorSchema),
})

export const SlowerOpSchema = z.strictObject({
  op: str,
  duration_ns: ns,
  baseline_p95_ns: ns,
})

export const BaselineDiffSchema = z.strictObject({
  new_ops: z.array(str),
  missing_ops: z.array(str),
  slower_ops: z.array(SlowerOpSchema),
})

export const StoryLogSchema = z.strictObject({
  ts_ns: ns,
  service: str,
  span_id: str,
  severity_number: int,
  severity_text: str,
  body: str,
})

/** `GET /stories/{id}` */
export const StoryViewSchema = z.strictObject({
  story_id: str,
  fingerprint: str,
  kind: StoryKindSchema,
  ts_ns: ns,
  duration_ns: ns,
  trace_id: str,
  endpoint_service: str,
  endpoint_name: str,
  root_cause: RootCauseSchema,
  summary: str,
  path_services: z.array(str),
  path_spans: z.array(SpanRefSchema),
  critical_path: CriticalPathSchema,
  baseline_diff: z.nullable(BaselineDiffSchema),
  logs: z.array(StoryLogSchema),
  also_failed: z.array(SpanRefSchema),
  span_count: int,
  flags: z.array(str),
})

export const SpanEventSchema = z.strictObject({
  ts_ns: ns,
  name: str,
  attrs: z.array(attr),
})

export const TraceSpanSchema = z.strictObject({
  span_id: str,
  parent_span_id: str,
  service_name: str,
  span_name: str,
  kind: SpanKindSchema,
  start_ns: ns,
  duration_ns: ns,
  status: SpanStatusSchema,
  status_message: str,
  attrs: z.array(attr),
  resource: z.array(attr),
  events: z.array(SpanEventSchema),
  self_ns: ns,
})

export const TraceLogSchema = z.strictObject({
  log_id: str,
  ts_ns: ns,
  span_id: str,
  service_name: str,
  severity_number: int,
  severity_text: str,
  body: str,
})

/** `GET /traces/{id}` */
export const TraceViewSchema = z.strictObject({
  trace_id: str,
  spans: z.array(TraceSpanSchema),
  logs: z.array(TraceLogSchema),
  story_id: z.nullable(str),
})

/** `GET /traces/{id}/log-templates` element: the template each log of the trace matched. */
export const TraceLogTemplateSchema = z.strictObject({
  log_id: str,
  template_id: str,
  template: str,
  /** `new` or `spike` when the template had an active alert at the trace's time. */
  alert: z.nullable(AlertKindSchema),
})

export const EdgeViewSchema = z.strictObject({
  parent: str,
  child: str,
  calls: int,
  errors: int,
  error_rate: num,
  avg_duration_ns: ns,
})

export const NodeViewSchema = z.strictObject({
  service: str,
  calls: int,
  rate: num,
  error_ratio: num,
  p99_ns: num,
  baseline_p99_ns: num,
  health: HealthSchema,
})

/** `GET /service-map` */
export const ServiceMapSchema = z.strictObject({
  edges: z.array(EdgeViewSchema),
  nodes: z.array(NodeViewSchema),
})

export const ExampleTraceSchema = z.strictObject({
  trace_id: str,
  story_id: z.nullable(str),
})

/** `GET /log-alerts` element */
export const LogAlertViewSchema = z.strictObject({
  alert_id: str,
  kind: AlertKindSchema,
  template_id: str,
  service: str,
  template: str,
  started_at_ns: ns,
  last_at_ns: ns,
  window_count: int,
  peak_count: int,
  baseline_per_window: num,
  active: z.boolean(),
  example_traces: z.array(ExampleTraceSchema),
})

/** `GET /log-templates` element */
export const LogTemplateViewSchema = z.strictObject({
  template_id: str,
  service: str,
  template: str,
  count: int,
  first_seen_ns: ns,
  last_seen_ns: ns,
  max_severity: int,
  alerting: z.boolean(),
})

/** `GET /log-templates` element: the template plus its hits per bucket over the window. */
export const LogTemplateListItemSchema = LogTemplateViewSchema.extend({
  bucket_secs: int,
  buckets: z.array(countBucket),
})

export const TemplateHitSchema = z.strictObject({
  ts_ns: ns,
  trace_id: str,
  span_id: str,
  severity_number: int,
  story_id: z.nullable(str),
})

/** `GET /log-templates/{id}` */
export const LogTemplateDetailSchema = z.strictObject({
  template: LogTemplateViewSchema,
  sample: str,
  bucket_secs: int,
  buckets: z.array(countBucket),
  recent: z.array(TemplateHitSchema),
  alerts: z.array(LogAlertViewSchema),
})

export const StoriesSeriesSchema = z.strictObject({
  bucket_secs: int,
  error: z.array(countBucket),
  slow: z.array(countBucket),
})

/** `GET /overview` */
export const OverviewSchema = z.strictObject({
  bucket_secs: int,
  error_stories: int,
  slow_stories: int,
  active_alerts: int,
  spans_per_sec: num,
  data_lag_secs: z.nullable(num),
  stories: StoriesSeriesSchema,
  /** (bucket start in unix seconds, spans per second) */
  spans: z.array(z.tuple([int, num])),
})

/** `GET /traces/search` element */
export const TraceHitSchema = z.strictObject({
  trace_id: str,
  ts_ns: ns,
  endpoint_service: str,
  endpoint_name: str,
  duration_ns: ns,
  is_error: z.boolean(),
  span_count: int,
  story_id: z.nullable(str),
  /** The story's kind when `story_id` is set; an error story can sit on a non-error trace. */
  story_kind: z.nullable(StoryKindSchema),
})

export const RedPointSchema = z.strictObject({
  bucket: int,
  rate: num,
  error_ratio: num,
  p50_ns: num,
  p95_ns: num,
  p99_ns: num,
})

/** `GET /services/{name}` */
export const ServiceViewSchema = z.strictObject({
  service: str,
  bucket_secs: int,
  calls: int,
  errors: int,
  buckets: z.array(RedPointSchema),
})

/** `GET /services` */
export const ServicesSchema = z.array(str)

export const SearchTemplateSchema = z.strictObject({
  template_id: str,
  service: str,
  template: str,
})

export const SearchGroupSchema = z.strictObject({
  fingerprint: str,
  kind: StoryKindSchema,
  summary: str,
  stories: int,
})

/** `GET /search?q=` */
export const SearchViewSchema = z.strictObject({
  services: z.array(str),
  templates: z.array(SearchTemplateSchema),
  groups: z.array(SearchGroupSchema),
  trace_id: z.nullable(str),
})

/** `GET /pipeline/series` */
export const SeriesViewSchema = z.strictObject({
  metric: str,
  kind: SeriesKindSchema,
  bucket_secs: int,
  /** (bucket start in unix ms, value); null where a quantile has no data. */
  points: z.array(z.tuple([int, z.nullable(num)])),
})

/** `GET /pipeline/lag` element */
export const ConsumerLagSchema = z.strictObject({
  group: str,
  committed: int,
  end: int,
  lag: int,
})

/** `GET /config` */
export const ClientConfigSchema = z.strictObject({
  jaeger_url: z.nullable(str),
  grafana_url: z.nullable(str),
  auth_enabled: z.boolean(),
})

/** `GET /auth/me` */
export const MeSchema = z.strictObject({ username: str })

/** Every error response: `{"error": "..."}`. */
export const ErrorBodySchema = z.strictObject({ error: str })
