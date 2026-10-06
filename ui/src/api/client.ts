/** Typed JSON client for tayga-api `/api/v1`. */

export const API_BASE = '/api/v1'

/**
 * A non-2xx response (or a network failure, status 0). `message` is the API's `error` text;
 * `retryAfter` is a 429's `Retry-After` in seconds.
 */
export class ApiError extends Error {
  readonly status: number
  readonly retryAfter: number | undefined

  constructor(status: number, message: string, retryAfter?: number) {
    super(message)
    this.name = 'ApiError'
    this.status = status
    this.retryAfter = retryAfter
  }
}

export function isApiError(e: unknown): e is ApiError {
  return e instanceof ApiError
}

/** Query values; `undefined`, `null` and `''` are left out. */
export type Params = Record<string, string | number | boolean | null | undefined>

export function apiUrl(path: string, params?: Params): string {
  const qs = new URLSearchParams()
  for (const [k, v] of Object.entries(params ?? {})) {
    if (v === undefined || v === null || v === '') continue
    qs.set(k, String(v))
  }
  const q = qs.toString()
  return `${API_BASE}${path}${q ? `?${q}` : ''}`
}

async function errorMessage(res: Response): Promise<string> {
  try {
    const body: unknown = await res.json()
    if (body && typeof body === 'object' && 'error' in body && typeof body.error === 'string') {
      return body.error
    }
  } catch {
    // Not JSON (proxy error page): fall through to the status text.
  }
  return res.statusText || `HTTP ${res.status}`
}

/**
 * GET `/api/v1{path}` and parse JSON as `T`. Pass TanStack Query's `signal` so the request
 * is aborted when the query is cancelled (component unmounted, key changed). An abort
 * rejects with the platform's AbortError, which Query treats as a cancellation.
 */
export async function getJson<T>(path: string, params?: Params, signal?: AbortSignal): Promise<T> {
  let res: Response
  try {
    res = await fetch(apiUrl(path, params), { headers: { Accept: 'application/json' }, signal })
  } catch (e) {
    if (signal?.aborted) throw e
    throw new ApiError(0, e instanceof Error ? e.message : 'network error')
  }
  if (!res.ok) throw new ApiError(res.status, await errorMessage(res))
  return (await res.json()) as T
}

/** `Retry-After` as whole seconds; the API sends delta-seconds, never an HTTP date. */
function retryAfter(res: Response): number | undefined {
  const v = Number(res.headers.get('retry-after'))
  return Number.isFinite(v) && v > 0 ? Math.ceil(v) : undefined
}

/** POST JSON to `/api/v1{path}`; resolves on any 2xx (the auth routes answer 204, no body). */
export async function postJson(path: string, body: unknown): Promise<void> {
  let res: Response
  try {
    res = await fetch(apiUrl(path), {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
      body: JSON.stringify(body),
    })
  } catch (e) {
    throw new ApiError(0, e instanceof Error ? e.message : 'network error')
  }
  if (!res.ok) throw new ApiError(res.status, await errorMessage(res), res.status === 429 ? retryAfter(res) : undefined)
}

/**
 * PUT JSON to `/api/v1{path}` and parse the JSON answer as `T`. Non-2xx and network failures
 * throw an `ApiError` like `postJson`.
 */
export async function putJson<T>(path: string, body: unknown): Promise<T> {
  let res: Response
  try {
    res = await fetch(apiUrl(path), {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
      body: JSON.stringify(body),
    })
  } catch (e) {
    throw new ApiError(0, e instanceof Error ? e.message : 'network error')
  }
  if (!res.ok) throw new ApiError(res.status, await errorMessage(res), res.status === 429 ? retryAfter(res) : undefined)
  return (await res.json()) as T
}
