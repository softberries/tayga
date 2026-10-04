/**
 * Response types of the tayga-api `/api/v1` routes, inferred from the strict zod schemas in
 * ./schemas (which mirror the Rust views field for field). Type-only imports: zod is not
 * bundled. See ./schemas for notes on number precision.
 */
import type { z } from 'zod'
import type {
  AlertKindSchema,
  BaselineDiffSchema,
  ClientConfigSchema,
  ConsumerLagSchema,
  ContributorSchema,
  CriticalPathSchema,
  EdgeViewSchema,
  ExampleTraceSchema,
  GroupDetailSchema,
  HealthSchema,
  LogAlertViewSchema,
  LogTemplateDetailSchema,
  LogTemplateListItemSchema,
  LogTemplateViewSchema,
  NodeViewSchema,
  OverviewSchema,
  PathSegmentSchema,
  RedPointSchema,
  RootCauseSchema,
  SearchGroupSchema,
  SearchTemplateSchema,
  SearchViewSchema,
  SeriesKindSchema,
  SeriesViewSchema,
  ServiceMapSchema,
  ServiceViewSchema,
  SlowerOpSchema,
  SpanEventSchema,
  SpanKindSchema,
  SpanRefSchema,
  SpanStatusSchema,
  StoriesSeriesSchema,
  StoryGroupSchema,
  StoryKindSchema,
  StoryLogSchema,
  StorySummarySchema,
  StoryViewSchema,
  TemplateHitSchema,
  TraceHitSchema,
  TraceLogSchema,
  TraceLogTemplateSchema,
  TraceSpanSchema,
  TraceViewSchema,
} from './schemas'

export type StoryKind = z.infer<typeof StoryKindSchema>
export type SpanKind = z.infer<typeof SpanKindSchema>
export type SpanStatus = z.infer<typeof SpanStatusSchema>
export type AlertKind = z.infer<typeof AlertKindSchema>
export type Health = z.infer<typeof HealthSchema>
export type SeriesKind = z.infer<typeof SeriesKindSchema>

/** `[bucket start in unix seconds, count]` */
export type CountBucket = [number, number]

export type StoryGroup = z.infer<typeof StoryGroupSchema>
export type StorySummary = z.infer<typeof StorySummarySchema>
export type GroupDetail = z.infer<typeof GroupDetailSchema>
export type RootCause = z.infer<typeof RootCauseSchema>
export type SpanRef = z.infer<typeof SpanRefSchema>
export type PathSegment = z.infer<typeof PathSegmentSchema>
export type Contributor = z.infer<typeof ContributorSchema>
export type CriticalPath = z.infer<typeof CriticalPathSchema>
export type SlowerOp = z.infer<typeof SlowerOpSchema>
export type BaselineDiff = z.infer<typeof BaselineDiffSchema>
export type StoryLog = z.infer<typeof StoryLogSchema>
export type StoryView = z.infer<typeof StoryViewSchema>
export type SpanEvent = z.infer<typeof SpanEventSchema>
export type TraceSpan = z.infer<typeof TraceSpanSchema>
export type TraceLog = z.infer<typeof TraceLogSchema>
export type TraceLogTemplate = z.infer<typeof TraceLogTemplateSchema>
export type TraceView = z.infer<typeof TraceViewSchema>
export type EdgeView = z.infer<typeof EdgeViewSchema>
export type NodeView = z.infer<typeof NodeViewSchema>
export type ServiceMapView = z.infer<typeof ServiceMapSchema>
export type ExampleTrace = z.infer<typeof ExampleTraceSchema>
export type LogAlertView = z.infer<typeof LogAlertViewSchema>
export type LogTemplateListItem = z.infer<typeof LogTemplateListItemSchema>
export type LogTemplateView = z.infer<typeof LogTemplateViewSchema>
export type TemplateHit = z.infer<typeof TemplateHitSchema>
export type LogTemplateDetail = z.infer<typeof LogTemplateDetailSchema>
export type StoriesSeries = z.infer<typeof StoriesSeriesSchema>
export type OverviewView = z.infer<typeof OverviewSchema>
export type TraceHit = z.infer<typeof TraceHitSchema>
export type RedPoint = z.infer<typeof RedPointSchema>
export type ServiceView = z.infer<typeof ServiceViewSchema>
export type SearchTemplate = z.infer<typeof SearchTemplateSchema>
export type SearchGroup = z.infer<typeof SearchGroupSchema>
export type SearchView = z.infer<typeof SearchViewSchema>
export type SeriesView = z.infer<typeof SeriesViewSchema>
export type ConsumerLag = z.infer<typeof ConsumerLagSchema>
export type ClientConfig = z.infer<typeof ClientConfigSchema>
