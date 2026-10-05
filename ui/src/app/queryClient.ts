import { QueryCache, QueryClient } from '@tanstack/react-query'
import type { Query } from '@tanstack/react-query'
import { isApiError } from '../api/client'
import { clearOutage, reportOutage } from './apiStatus'
import { sessionLost } from './auth'

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
  return new QueryClient({
    queryCache: new QueryCache({
      onError: (error, query) => {
        // A lost session is not an outage: send the user to /login (once) instead. The guard's
        // own `auth/me` probe redirects by itself.
        if (isApiError(error) && error.status === 401) {
          if (query.meta?.sessionProbe !== true) sessionLost()
          return
        }
        if (!isOutageQuery(query)) return
        if (isApiError(error) && error.status === 503) reportOutage(error.message)
      },
      onSuccess: (_data, query) => {
        if (isOutageQuery(query)) clearOutage()
      },
    }),
    defaultOptions: {
      queries: {
        staleTime: 5_000,
        retry: shouldRetry,
      },
    },
  })
}
