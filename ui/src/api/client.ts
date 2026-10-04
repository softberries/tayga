/** Typed JSON client for tayga-api `/api/v1`. */

export const API_BASE = '/api/v1'

/** A non-2xx response (or a network failure, status 0). `message` is the API's `error` text. */
export class ApiError extends Error {
  readonly status: number

  constructor(status: number, message: string) {
    super(message)
    this.name = 'ApiError'
    this.status = status
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
