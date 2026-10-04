import { describe, expect, it } from 'vitest'
import { ApiError } from '../api/client'
import { getOutage } from './apiStatus'
import { createQueryClient, shouldRetry } from './queryClient'

describe('query client', () => {
  it('retries once, never on 4xx', () => {
    expect(shouldRetry(0, new ApiError(503, 'x'))).toBe(true)
    expect(shouldRetry(1, new ApiError(503, 'x'))).toBe(false)
    expect(shouldRetry(0, new ApiError(0, 'net'))).toBe(true)
    expect(shouldRetry(0, new ApiError(400, 'bad'))).toBe(false)
    expect(shouldRetry(0, new ApiError(404, 'nf'))).toBe(false)
  })

  it('has a 5 s stale time', () => {
    expect(createQueryClient().getDefaultOptions().queries?.staleTime).toBe(5_000)
  })

  it('a 503 raises the outage banner and a success clears it', async () => {
    const qc = createQueryClient()
    await qc
      .fetchQuery({ queryKey: ['a'], queryFn: () => Promise.reject(new ApiError(503, 'clickhouse down')), retry: false })
      .catch(() => {})
    expect(getOutage()?.message).toBe('clickhouse down')
    await qc.fetchQuery({ queryKey: ['b'], queryFn: () => Promise.resolve(1) })
    expect(getOutage()).toBeNull()
  })

  it('other errors do not raise the banner', async () => {
    const qc = createQueryClient()
    await qc
      .fetchQuery({ queryKey: ['c'], queryFn: () => Promise.reject(new ApiError(400, 'bad')), retry: false })
      .catch(() => {})
    expect(getOutage()).toBeNull()
  })

  it('a query with meta outage=false neither raises nor clears the banner', async () => {
    const qc = createQueryClient()
    await qc
      .fetchQuery({ queryKey: ['d'], queryFn: () => Promise.reject(new ApiError(503, 'kafka unavailable')), retry: false, meta: { outage: false } })
      .catch(() => {})
    expect(getOutage()).toBeNull()
    await qc.fetchQuery({ queryKey: ['e'], queryFn: () => Promise.reject(new ApiError(503, 'clickhouse down')), retry: false }).catch(() => {})
    await qc.fetchQuery({ queryKey: ['f'], queryFn: () => Promise.resolve(1), meta: { outage: false } })
    expect(getOutage()?.message).toBe('clickhouse down')
    await qc.fetchQuery({ queryKey: ['g'], queryFn: () => Promise.resolve(1) })
  })
})
