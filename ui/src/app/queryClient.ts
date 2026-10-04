import { QueryCache, QueryClient } from '@tanstack/react-query'
import type { Query } from '@tanstack/react-query'
import { isApiError } from '../api/client'
import { clearOutage, reportOutage } from './apiStatus'

/** Retry once, but never a client error (4xx): repeating a bad request cannot help. */
export function shouldRetry(failureCount: number, error: unknown): boolean {
  if (isApiError(error) && error.status >= 400 && error.status < 500) return false
  return failureCount < 1
}

/**
 * Whether a query speaks for the storage backend. A query that sets `meta: { outage: false }`
 * (consumer lag, which reads Kafka) neither raises nor clears the "storage unavailable" banner.
 */
function isOutageQuery(query: Pick<Query, 'meta'>): boolean {
  return query.meta?.outage !== false
}

export function createQueryClient(): QueryClient {
  return new QueryClient({
    queryCache: new QueryCache({
      onError: (error, query) => {
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
