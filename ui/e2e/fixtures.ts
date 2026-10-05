import { expect, test as base } from '@playwright/test'
import type { APIRequestContext } from '@playwright/test'

export type Theme = 'light' | 'dark'

interface Fixtures {
  /** The stored theme for the project (`dark` or `light`); applied before the app loads. */
  theme: Theme
  /** Console errors, page errors, failed requests and 4xx/5xx responses of the test; must stay empty. */
  problems: string[]
}

export const test = base.extend<Fixtures>({
  theme: ['dark', { option: true }],

  // The theme is written before any app script runs, but only when none is stored yet, so a
  // theme switched in the test survives its own reload.
  page: async ({ page, theme }, use) => {
    await page.addInitScript((t) => {
      try {
        if (window.localStorage.getItem('tayga-theme') === null) window.localStorage.setItem('tayga-theme', t)
      } catch {
        // storage blocked: the app falls back to the system theme
      }
    }, theme)
    await use(page)
  },

  problems: [
    async ({ page }, use) => {
      const problems: string[] = []
      page.on('console', (m) => {
        if (m.type() === 'error') problems.push(`console: ${m.text()}`)
      })
      page.on('pageerror', (e) => problems.push(`pageerror: ${e.message}`))
      page.on('requestfailed', (r) => {
        // Leaving a page cancels its in-flight fetches; that is not a failure.
        if (r.failure()?.errorText !== 'net::ERR_ABORTED') problems.push(`request failed: ${r.url()} ${r.failure()?.errorText}`)
      })
      page.on('response', (r) => {
        if (r.status() >= 400) problems.push(`HTTP ${r.status()}: ${r.url()}`)
      })
      await use(problems)
      expect(problems, 'console errors and failed requests').toEqual([])
    },
    { auto: true },
  ],
})

export { expect }

export async function getJson<T>(request: APIRequestContext, path: string): Promise<T> {
  const res = await request.get(`/api/v1${path}`)
  expect(res.ok(), `GET ${path}: ${res.status()}`).toBe(true)
  return (await res.json()) as T
}

export interface TraceHit {
  trace_id: string
  story_id: string | null
  span_count: number
}

/** Newest traces of the last 24 h, as the explorer lists them. */
export async function recentTraces(request: APIRequestContext): Promise<TraceHit[]> {
  return getJson<TraceHit[]>(request, '/traces/search?since=24h&limit=500')
}

export interface Ids {
  /** The widest recent trace (most spans). */
  traceId: string
  /** A trace that is part of a story. */
  storyId: string
  templateId: string
}

/** Real ids from the live stack for the detail pages. */
export async function liveIds(request: APIRequestContext): Promise<Ids> {
  const hits = await recentTraces(request)
  const groups = await getJson<{ sample_story_id: string }[]>(request, '/story-groups?since=24h')
  const widest = hits.reduce((a, b) => (b.span_count > a.span_count ? b : a))
  const templates = await getJson<{ template_id: string }[]>(request, '/log-templates?since=1h')
  expect(groups.length, 'a story group in the last 24 h').toBeGreaterThan(0)
  expect(templates.length, 'log templates in the last hour').toBeGreaterThan(0)
  return { traceId: widest.trace_id, storyId: groups[0]!.sample_story_id, templateId: templates[0]!.template_id }
}
