import { afterEach, describe, expect, it, vi } from 'vitest'
import { ApiError } from '../api/client'
import { api } from '../api/queries'
import { getOutage } from './apiStatus'
import { setSessionLostHandler, singleFlight } from './auth'
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

  it('a /config success does not clear the banner: it never touches ClickHouse', async () => {
    const qc = createQueryClient()
    await qc.fetchQuery({ queryKey: ['h'], queryFn: () => Promise.reject(new ApiError(503, 'clickhouse down')), retry: false }).catch(() => {})
    await qc.fetchQuery({ ...api.config(), queryFn: () => Promise.resolve({ jaeger_url: null, grafana_url: null, auth_enabled: false, infra_services: ['flagd'] }) })
    expect(getOutage()?.message).toBe('clickhouse down')
    await qc.fetchQuery({ queryKey: ['i'], queryFn: () => Promise.resolve(1) })
    expect(getOutage()).toBeNull()
  })

  describe('a 401', () => {
    afterEach(() => setSessionLostHandler(null))

    const fail401 = () => Promise.reject(new ApiError(401, 'unauthorized'))

    it('with auth off or the config unknown, is only a failed query', async () => {
      const lost = vi.fn()
      setSessionLostHandler(lost)
      const qc = createQueryClient()
      await qc.fetchQuery({ queryKey: ['k'], queryFn: fail401, retry: false }).catch(() => {})
      qc.setQueryData(api.config().queryKey, { jaeger_url: null, grafana_url: null, auth_enabled: false, infra_services: [] })
      await qc.fetchQuery({ queryKey: ['l'], queryFn: fail401, retry: false }).catch(() => {})
      expect(lost).not.toHaveBeenCalled()
      expect(getOutage()).toBeNull()
    })

    it('with auth on, reports a lost session, not an outage, and the auth/me probe does not', async () => {
      const lost = vi.fn()
      setSessionLostHandler(lost)
      const qc = createQueryClient()
      qc.setQueryData(api.config().queryKey, { jaeger_url: null, grafana_url: null, auth_enabled: true, infra_services: [] })
      await qc.fetchQuery({ queryKey: ['j'], queryFn: () => Promise.reject(new ApiError(401, 'unauthorized')), retry: false }).catch(() => {})
      expect(lost).toHaveBeenCalledTimes(1)
      expect(getOutage()).toBeNull()
      await qc.fetchQuery({ ...api.me(), queryFn: () => Promise.reject(new ApiError(401, 'unauthorized')), retry: false }).catch(() => {})
      expect(lost).toHaveBeenCalledTimes(1)
    })

    it('a mutation that opts in reports a lost session; one that does not (the login) is only an error', async () => {
      const lost = vi.fn()
      setSessionLostHandler(lost)
      const qc = createQueryClient()
      qc.setQueryData(api.config().queryKey, { jaeger_url: null, grafana_url: null, auth_enabled: true, infra_services: [] })
      const run = (meta?: Record<string, unknown>) =>
        qc.getMutationCache().build(qc, { mutationFn: fail401, meta, retry: false }).execute(undefined).catch(() => {})
      await run()
      expect(lost).not.toHaveBeenCalled()
      await run({ sessionAware: true })
      expect(lost).toHaveBeenCalledTimes(1)
      qc.setQueryData(api.config().queryKey, { jaeger_url: null, grafana_url: null, auth_enabled: false, infra_services: [] })
      await run({ sessionAware: true })
      expect(lost).toHaveBeenCalledTimes(1)
    })

    it('a burst of 401s redirects once while the first redirect is pending', async () => {
      let finish = () => {}
      const redirect = vi.fn(() => new Promise<void>((r) => (finish = r)))
      const handler = singleFlight(redirect)
      handler()
      handler()
      handler()
      expect(redirect).toHaveBeenCalledTimes(1)
      finish()
      await Promise.resolve()
      await Promise.resolve()
      handler()
      expect(redirect).toHaveBeenCalledTimes(2)
    })
  })
})
