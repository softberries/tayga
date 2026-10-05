/**
 * Query option factories, one per route. Keys start with the route name so related queries
 * can be invalidated together. Every queryFn forwards Query's AbortSignal to fetch.
 */
import { queryOptions } from '@tanstack/react-query'
import { getJson, postJson } from './client'
import type { Params } from './client'
import type {
  ClientConfig,
  ConsumerLag,
  GroupDetail,
  LogAlertView,
  LogTemplateDetail,
  LogTemplateListItem,
  Me,
  OverviewView,
  SearchView,
  SeriesView,
  ServiceMapView,
  ServiceView,
  StoriesSeries,
  StoryGroup,
  StoryView,
  TraceHit,
  TraceLogTemplate,
  TraceView,
} from './types'

function q<T>(key: string, path: string, params?: Params) {
  return queryOptions({
    queryKey: [key, path, params ?? {}] as const,
    queryFn: ({ signal }) => getJson<T>(path, params, signal),
  })
}

/** The API's window params for a range: `since`, and `until` for a custom range. */
type Win = { since: string; until?: string }

export const api = {
  /** Any API window, e.g. the doubled one for the previous-window delta. */
  overview: (w: Win) => q<OverviewView>('overview', '/overview', w),
  storyGroups: (p: Win & { kind?: string; service?: string }) => q<StoryGroup[]>('story-groups', '/story-groups', p),
  storyGroup: (fingerprint: string, w: Win) =>
    q<GroupDetail>('story-groups', `/story-groups/${encodeURIComponent(fingerprint)}`, w),
  storiesSeries: (p: Win & { kind?: string; service?: string }) => q<StoriesSeries>('stories-series', '/stories/series', p),
  story: (id: string) => q<StoryView>('story', `/stories/${encodeURIComponent(id)}`),
  trace: (id: string) => q<TraceView>('trace', `/traces/${encodeURIComponent(id)}`),
  traceLogTemplates: (id: string) =>
    q<TraceLogTemplate[]>('trace', `/traces/${encodeURIComponent(id)}/log-templates`),
  traceSearch: (
    p: Win & {
      service?: string
      touched?: 1
      endpoint?: string
      min_ms?: number
      max_ms?: number
      errors?: boolean
      limit?: number
    },
  ) => q<TraceHit[]>('trace-search', '/traces/search', p),
  serviceMap: (w: Win) => q<ServiceMapView>('service-map', '/service-map', w),
  services: () => q<string[]>('services', '/services'),
  service: (name: string, w: Win) => q<ServiceView>('service', `/services/${encodeURIComponent(name)}`, w),
  logAlerts: (p: Win & { kind?: string; service?: string }) => q<LogAlertView[]>('log-alerts', '/log-alerts', p),
  logTemplates: (p: Win & { service?: string; q?: string }) =>
    q<LogTemplateListItem[]>('log-templates', '/log-templates', p),
  logTemplate: (id: string, w: Win) =>
    q<LogTemplateDetail>('log-templates', `/log-templates/${encodeURIComponent(id)}`, w),
  search: (text: string) => q<SearchView>('search', '/search', { q: text }),
  pipelineSeries: (p: Win & { metric: string; kind: string; job?: string; labels?: string }) =>
    q<SeriesView>('pipeline-series', '/pipeline/series', p),
  pipelineLag: () => q<ConsumerLag[]>('pipeline-lag', '/pipeline/lag'),
  /** Read from the API's own config, not ClickHouse: kept out of the storage banner. */
  config: () => queryOptions({ ...q<ClientConfig>('config', '/config'), staleTime: Infinity, meta: { outage: false } }),
  /**
   * The signed-in user. Fetched by the shell's guard, which turns its 401 into the redirect
   * itself (`sessionProbe`: the session-lost handler skips it). Kept until sign-in or sign-out
   * clears the cache.
   */
  me: () => queryOptions({ ...q<Me>('auth-me', '/auth/me'), staleTime: Infinity, meta: { outage: false, sessionProbe: true } }),
}

export interface Credentials {
  username: string
  password: string
}

/** Mutation functions for the auth routes (the login page's `useMutation`, `signOut`). */
export const authApi = {
  login: (c: Credentials) => postJson('/auth/login', c),
  /** The API wants a JSON body here too (`{}`); logout is open, so an expired session still works. */
  logout: () => postJson('/auth/logout', {}),
}
