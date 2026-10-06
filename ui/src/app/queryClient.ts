import { MutationCache, QueryCache, QueryClient } from '@tanstack/react-query'
import type { Query } from '@tanstack/react-query'
import { isApiError } from '../api/client'
import { clearOutage, reportOutage } from './apiStatus'
import { authEnabledInCache, sessionLost } from './auth'

/** Retry once, but never a client error (4xx): repeating a bad request cannot help. */
export function shouldRetry(failureCount: number, error: unknown): boolean {
  if (isApiError(error) && error.status >= 400 && error.status < 500) return false
  return failureCount < 1
}

/**
 * Whether a query speaks for the storage backend (ClickHouse). A query that sets
 * `meta: { outage: false }` (consumer lag, which reads Kafka; `/config`, which reads no storage)
 * neither raises nor clears the "storage unavailable" banner.
 */
function isOutageQuery(query: Pick<Query, 'meta'>): boolean {
  return query.meta?.outage !== false
}

export function createQueryClient(): QueryClient {
  const client: QueryClient = new QueryClient({
    queryCache: new QueryCache({
      onError: (error, query) => {
        // A 401 is never an outage. With Tayga's own login on (the loaded config says so), it
        // means the session is gone: send the user to /login, once; the guard's own `auth/me`
        // probe redirects by itself. With auth off or the config unknown (a 401 from an
        // authenticating proxy, say), /login would only bounce back, so the query just fails
        // and the page shows its error state.
        if (isApiError(error) && error.status === 401) {
          if (query.meta?.sessionProbe !== true && authEnabledInCache(client)) sessionLost()
          return
        }
        if (!isOutageQuery(query)) return
        if (isApiError(error) && error.status === 503) reportOutage(error.message)
      },
      onSuccess: (_data, query) => {
        if (isOutageQuery(query)) clearOutage()
      },
    }),
    // A write that comes back 401 means the same as a read doing so, but only mutations that opt
    // in (`meta: { sessionAware: true }`) say so: a wrong password on /login is a 401 too.
    mutationCache: new MutationCache({
      onError: (error, _vars, _ctx, mutation) => {
        if (isApiError(error) && error.status === 401 && mutation.meta?.sessionAware === true && authEnabledInCache(client)) {
          sessionLost()
        }
      },
    }),
    defaultOptions: {
      queries: {
        staleTime: 5_000,
        retry: shouldRetry,
      },
    },
  })
  return client
}
