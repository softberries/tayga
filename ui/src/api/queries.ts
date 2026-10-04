/**
 * Query option factories, one per route. Keys start with the route name so related queries
 * can be invalidated together. Every queryFn forwards Query's AbortSignal to fetch.
 */
import { queryOptions } from '@tanstack/react-query'
import type { Since } from '../app/search'
import { getJson } from './client'
import type { Params } from './client'
import type {
  ClientConfig,
  ConsumerLag,
  GroupDetail,
  LogAlertView,
  LogTemplateDetail,
  LogTemplateView,
  OverviewView,
  SearchView,
  SeriesView,
  ServiceMapView,
  ServiceView,
  StoriesSeries,
  StoryGroup,
  StoryView,
  TraceHit,
  TraceView,
} from './types'

function q<T>(key: string, path: string, params?: Params) {
  return queryOptions({
    queryKey: [key, path, params ?? {}] as const,
    queryFn: ({ signal }) => getJson<T>(path, params, signal),
  })
}

export const api = {
  overview: (since: Since) => q<OverviewView>('overview', '/overview', { since }),
  storyGroups: (p: { since: Since; kind?: string; service?: string }) =>
    q<StoryGroup[]>('story-groups', '/story-groups', p),
  storyGroup: (fingerprint: string, since: Since) =>
    q<GroupDetail>('story-groups', `/story-groups/${encodeURIComponent(fingerprint)}`, { since }),
  storiesSeries: (p: { since: Since; kind?: string; service?: string }) =>
    q<StoriesSeries>('stories-series', '/stories/series', p),
  story: (id: string) => q<StoryView>('story', `/stories/${encodeURIComponent(id)}`),
  trace: (id: string) => q<TraceView>('trace', `/traces/${encodeURIComponent(id)}`),
  traceSearch: (p: {
    since: Since
    service?: string
    endpoint?: string
    min_ms?: number
    max_ms?: number
    errors?: boolean
    limit?: number
  }) => q<TraceHit[]>('trace-search', '/traces/search', p),
  serviceMap: (since: Since) => q<ServiceMapView>('service-map', '/service-map', { since }),
  services: () => q<string[]>('services', '/services'),
  service: (name: string, since: Since) =>
    q<ServiceView>('service', `/services/${encodeURIComponent(name)}`, { since }),
  logAlerts: (p: { since: Since; kind?: string; service?: string }) =>
    q<LogAlertView[]>('log-alerts', '/log-alerts', p),
  logTemplates: (p: { since: Since; service?: string; q?: string }) =>
    q<LogTemplateView[]>('log-templates', '/log-templates', p),
  logTemplate: (id: string, since: Since) =>
    q<LogTemplateDetail>('log-templates', `/log-templates/${encodeURIComponent(id)}`, { since }),
  search: (text: string) => q<SearchView>('search', '/search', { q: text }),
  pipelineSeries: (p: { since: Since; metric: string; kind: string; job?: string; labels?: string }) =>
    q<SeriesView>('pipeline-series', '/pipeline/series', p),
  pipelineLag: () => q<ConsumerLag[]>('pipeline-lag', '/pipeline/lag'),
  config: () => queryOptions({ ...q<ClientConfig>('config', '/config'), staleTime: Infinity }),
}
