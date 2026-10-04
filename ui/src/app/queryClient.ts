import { QueryCache, QueryClient } from '@tanstack/react-query'
import { isApiError } from '../api/client'
import { clearOutage, reportOutage } from './apiStatus'

/** Retry once, but never a client error (4xx): repeating a bad request cannot help. */
export function shouldRetry(failureCount: number, error: unknown): boolean {
  if (isApiError(error) && error.status >= 400 && error.status < 500) return false
  return failureCount < 1
}

export function createQueryClient(): QueryClient {
  return new QueryClient({
    queryCache: new QueryCache({
      onError: (error) => {
        if (isApiError(error) && error.status === 503) reportOutage(error.message)
      },
      onSuccess: () => clearOutage(),
    }),
    defaultOptions: {
      queries: {
        staleTime: 5_000,
        retry: shouldRetry,
      },
    },
  })
}
