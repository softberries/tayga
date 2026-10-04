/**
 * Contract: every fixture captured from the live API (curl, see the Task 5 report) parses
 * with the strict schema its route returns. A Rust view change that adds, drops or retypes a
 * field fails here until schemas.ts (and so types.ts) follow.
 */
import { describe, expect, it } from 'vitest'
import type { ZodType } from 'zod'
import { z } from 'zod'
import clientConfig from './__fixtures__/config.json'
import error400 from './__fixtures__/error-400.json'
import logAlerts from './__fixtures__/log-alerts.json'
import logTemplate from './__fixtures__/log-template.json'
import logTemplates from './__fixtures__/log-templates.json'
import overview from './__fixtures__/overview.json'
import pipelineLag from './__fixtures__/pipeline-lag.json'
import pipelineSeriesGauge from './__fixtures__/pipeline-series-gauge.json'
import pipelineSeriesQ99 from './__fixtures__/pipeline-series-q99.json'
import pipelineSeries from './__fixtures__/pipeline-series.json'
import search from './__fixtures__/search.json'
import serviceMap from './__fixtures__/service-map.json'
import service from './__fixtures__/service.json'
import services from './__fixtures__/services.json'
import storiesSeries from './__fixtures__/stories-series.json'
import storyErrorPayment from './__fixtures__/story-error-payment.json'
import storyGroup from './__fixtures__/story-group.json'
import storyGroups from './__fixtures__/story-groups.json'
import storySlow from './__fixtures__/story-slow.json'
import story from './__fixtures__/story.json'
import traceErrorPayment from './__fixtures__/trace-error-payment.json'
import traceLogTemplates from './__fixtures__/trace-log-templates.json'
import trace from './__fixtures__/trace.json'
import tracesSearch from './__fixtures__/traces-search.json'
import * as S from './schemas'

const cases: Array<[string, ZodType, unknown]> = [
  ['story-groups', z.array(S.StoryGroupSchema), storyGroups],
  ['story-groups/{fp}', S.GroupDetailSchema, storyGroup],
  ['stories/{id} (error, small)', S.StoryViewSchema, story],
  ['stories/{id} (error, payment)', S.StoryViewSchema, storyErrorPayment],
  ['stories/{id} (slow, baseline diff)', S.StoryViewSchema, storySlow],
  ['traces/{id}', S.TraceViewSchema, trace],
  ['traces/{id} (140 spans)', S.TraceViewSchema, traceErrorPayment],
  ['traces/{id}/log-templates', z.array(S.TraceLogTemplateSchema), traceLogTemplates],
  ['service-map', S.ServiceMapSchema, serviceMap],
  ['log-alerts', z.array(S.LogAlertViewSchema), logAlerts],
  ['log-templates', z.array(S.LogTemplateViewSchema), logTemplates],
  ['log-templates/{id}', S.LogTemplateDetailSchema, logTemplate],
  ['overview', S.OverviewSchema, overview],
  ['stories/series', S.StoriesSeriesSchema, storiesSeries],
  ['traces/search', z.array(S.TraceHitSchema), tracesSearch],
  ['services', S.ServicesSchema, services],
  ['services/{name}', S.ServiceViewSchema, service],
  ['search?q=pay', S.SearchViewSchema, search],
  ['pipeline/series (rate)', S.SeriesViewSchema, pipelineSeries],
  ['pipeline/series (gauge)', S.SeriesViewSchema, pipelineSeriesGauge],
  ['pipeline/series (q99, empty)', S.SeriesViewSchema, pipelineSeriesQ99],
  ['pipeline/lag', z.array(S.ConsumerLagSchema), pipelineLag],
  ['config', S.ClientConfigSchema, clientConfig],
  ['error body (400)', S.ErrorBodySchema, error400],
]

describe('API contract (live fixtures)', () => {
  it.each(cases)('%s', (_name, schema, fixture) => {
    const r = schema.safeParse(fixture)
    expect(r.success ? [] : r.error.issues.slice(0, 5)).toEqual([])
  })

  it('fixtures are not trivially empty', () => {
    expect(storyGroups.length).toBeGreaterThan(0)
    expect(traceErrorPayment.spans.length).toBeGreaterThan(100)
    expect(serviceMap.nodes.length).toBeGreaterThan(0)
    expect(pipelineSeries.points.length).toBeGreaterThan(0)
    expect(storySlow.baseline_diff).not.toBeNull()
  })

  it('schemas are strict: an unknown field fails', () => {
    expect(S.ClientConfigSchema.safeParse({ ...clientConfig, extra: 1 }).success).toBe(false)
    expect(S.ClientConfigSchema.safeParse({ jaeger_url: null }).success).toBe(false)
  })
})
