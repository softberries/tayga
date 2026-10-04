import { afterEach, describe, expect, it, vi } from 'vitest'
import { ApiError, apiUrl, getJson, isApiError } from './client'

function respond(status: number, body: unknown, contentType = 'application/json') {
  return vi.fn(async () =>
    new Response(typeof body === 'string' ? body : JSON.stringify(body), {
      status,
      headers: { 'content-type': contentType },
    }),
  )
}

afterEach(() => vi.unstubAllGlobals())

describe('apiUrl', () => {
  it('prefixes /api/v1 and drops empty params', () => {
    expect(apiUrl('/overview')).toBe('/api/v1/overview')
    expect(apiUrl('/story-groups', { since: '1h', kind: undefined, service: '', n: 0, errors: false, x: null })).toBe(
      '/api/v1/story-groups?since=1h&n=0&errors=false',
    )
    expect(apiUrl('/search', { q: 'a b&c' })).toBe('/api/v1/search?q=a+b%26c')
  })
})

describe('getJson', () => {
  it('returns parsed JSON and sends Accept', async () => {
    const fetch = respond(200, { ok: 1 })
    vi.stubGlobal('fetch', fetch)
    await expect(getJson('/config')).resolves.toEqual({ ok: 1 })
    const [url, init] = fetch.mock.calls[0] as unknown as [string, RequestInit]
    expect(url).toBe('/api/v1/config')
    expect(new Headers(init.headers).get('accept')).toBe('application/json')
  })

  it('throws ApiError with the API error text', async () => {
    vi.stubGlobal('fetch', respond(503, { error: 'storage unavailable' }))
    const err = await getJson('/overview').catch((e: unknown) => e)
    expect(isApiError(err)).toBe(true)
    expect(err).toMatchObject({ status: 503, message: 'storage unavailable' })
  })

  it('falls back to the status text for a non-JSON error body', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => new Response('<html>bad gateway</html>', { status: 502, statusText: 'Bad Gateway' })))
    await expect(getJson('/overview')).rejects.toMatchObject({ status: 502, message: 'Bad Gateway' })
  })

  it('maps a network failure to status 0', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => Promise.reject(new TypeError('Failed to fetch'))))
    const err = await getJson('/overview').catch((e: unknown) => e)
    expect(err).toBeInstanceOf(ApiError)
    expect(err).toMatchObject({ status: 0, message: 'Failed to fetch' })
  })

  it('passes the abort signal and rethrows the abort as-is', async () => {
    const fetch = vi.fn(
      (_url: string, init: RequestInit) =>
        new Promise<Response>((_, reject) => {
          init.signal?.addEventListener('abort', () => reject(new DOMException('Aborted', 'AbortError')))
        }),
    )
    vi.stubGlobal('fetch', fetch)
    const ctl = new AbortController()
    const p = getJson('/overview', undefined, ctl.signal)
    ctl.abort()
    const err = await p.catch((e: unknown) => e)
    expect(err).toBeInstanceOf(DOMException)
    expect((err as DOMException).name).toBe('AbortError')
    expect(isApiError(err)).toBe(false)
  })
})
