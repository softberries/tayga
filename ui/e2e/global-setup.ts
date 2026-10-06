import { request, type FullConfig } from '@playwright/test'

/** Longest wait for a stack started moments ago (`make up`) to have the data the specs read. */
const READY_TIMEOUT_MS = 5 * 60_000
const POLL_MS = 5_000
/** Per request, so one hung read cannot stretch the wait past READY_TIMEOUT_MS. */
const REQUEST_TIMEOUT_MS = 10_000

const nonEmpty = (b: unknown): boolean => Array.isArray(b) && b.length > 0

/** One read per page family: stories, log templates, the service map, live traffic. */
const CHECKS: { path: string; ready: (body: unknown) => boolean }[] = [
  { path: '/api/v1/story-groups?since=1h', ready: nonEmpty },
  { path: '/api/v1/log-templates?since=1h', ready: nonEmpty },
  { path: '/api/v1/service-map?since=1h', ready: (b) => nonEmpty((b as { edges?: unknown }).edges) },
  { path: '/api/v1/overview?since=15m', ready: (b) => ((b as { spans_per_sec?: number }).spans_per_sec ?? 0) > 0 },
]

export default async function globalSetup(config: FullConfig): Promise<void> {
  const baseURL = config.projects[0]?.use.baseURL ?? 'http://localhost:8090'
  const api = await request.newContext({ baseURL })
  const started = Date.now()
  try {
    for (;;) {
      const pending: string[] = []
      let unauthorized = 0
      for (const c of CHECKS) {
        const res = await api.get(c.path, { timeout: REQUEST_TIMEOUT_MS }).catch(() => null)
        if (res?.status() === 401) unauthorized += 1
        // A non-JSON 200 (an HTML error page, say) counts as not ready.
        const body: unknown = res !== null && res.ok() ? await res.json().catch(() => undefined) : undefined
        if (body === undefined || !c.ready(body)) pending.push(c.path)
      }
      if (unauthorized === CHECKS.length) {
        // Auth is on: these anonymous reads cannot see the data, so there is nothing to wait for.
        console.log(`[global-setup] ${baseURL} requires sign-in (401); not waiting for data`)
        return
      }
      if (pending.length === 0) {
        console.log(`[global-setup] ${baseURL} has data after ${Math.round((Date.now() - started) / 1000)} s`)
        return
      }
      if (Date.now() - started > READY_TIMEOUT_MS) {
        throw new Error(`${baseURL} has no data after ${READY_TIMEOUT_MS / 1000} s: ${pending.join(', ')}`)
      }
      await new Promise((r) => setTimeout(r, POLL_MS))
    }
  } finally {
    await api.dispose()
  }
}
