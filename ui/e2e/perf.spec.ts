/**
 * Performance budgets (spec §10), measured against the stack at the base URL, in the `perf`
 * project, which runs alone (no parallel tests) before the others.
 *
 * Each timing is the median of several runs, each in a fresh browser context (cold HTTP cache).
 * All marks are made by scripts this test injects (`performance.mark` and observers); the app
 * carries no measurement code.
 *
 * Synthetic data: the 5,000-span trace and the 60-node map are served by `page.route` from
 * generated JSON, in the browser test only. Nothing is added to the app or its bundle.
 */
import type { APIRequestContext, Browser, Page } from '@playwright/test'
import { gzipSync } from 'node:zlib'
import { expect, getJson, recentTraces, test } from './fixtures'

const RUNS = 5
const median = (xs: number[]) => [...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)]!
const fmt = (xs: number[]) => `median ${median(xs).toFixed(0)} ms (runs: ${xs.map((x) => x.toFixed(0)).join(', ')})`

// Page-side instrumentation. Marks are relative to the navigation start (performance.now()).
const INSTRUMENT = `
(() => {
  const mark = (name) => { if (!performance.getEntriesByName(name).length) performance.mark(name) }
  const orig = window.fetch.bind(window)
  window.fetch = async (...args) => {
    const res = await orig(...args)
    const url = typeof args[0] === 'string' ? args[0] : args[0].url
    const body = (name) => res.clone().arrayBuffer().then(() => mark(name), () => {})
    if (/\\/api\\/v1\\/traces\\/[0-9a-f]{32}(\\?|$)/.test(url)) body('trace-body')
    if (/\\/api\\/v1\\/service-map/.test(url)) body('map-body')
    return res
  }
  const frame = (name) => requestAnimationFrame(() => requestAnimationFrame(() => mark(name)))
  new MutationObserver(() => {
    if (document.querySelector('[data-fingerprint]')) frame('home-data')
    if (document.querySelector('[role=treeitem]')) frame('waterfall')
    if (document.querySelector('[data-service]')) frame('map-nodes')
  }).observe(document, { childList: true, subtree: true })
})()
`

async function fresh(browser: Browser, theme: 'dark' | 'light' = 'dark'): Promise<Page> {
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 }, baseURL: test.info().project.use.baseURL })
  const page = await ctx.newPage()
  await page.addInitScript((t) => localStorage.setItem('tayga-theme', t), theme)
  await page.addInitScript(INSTRUMENT)
  return page
}

const markAt = (page: Page, name: string) =>
  page.evaluate((n) => performance.getEntriesByName(n)[0]?.startTime ?? -1, name)

async function jsTransferredGzip(page: Page, path: string): Promise<{ files: number; gzipBytes: number }> {
  const bodies: Promise<Buffer>[] = []
  page.on('response', (r) => {
    if (r.request().resourceType() === 'script' && r.ok()) bodies.push(r.body())
  })
  await page.goto(path)
  await page.waitForLoadState('networkidle')
  const all = await Promise.all(bodies)
  return { files: all.length, gzipBytes: all.reduce((n, b) => n + gzipSync(b).length, 0) }
}

test.describe.configure({ mode: 'serial' })

test('initial JS stays within 350 KB gzip', async ({ browser, request }) => {
  // (a) The entry script and the chunks the HTML preloads: what blocks first render.
  const html = await (await request.get('/')).text()
  const srcs = [...html.matchAll(/<(?:script[^>]*src|link[^>]*rel="modulepreload"[^>]*href)="([^"]+\.js)"/g)].map((m) => m[1]!)
  expect(srcs.length).toBeGreaterThan(0)
  let entry = 0
  for (const s of srcs) entry += gzipSync(Buffer.from(await (await request.get(s)).body())).length

  // (b) Every script a cold load of the home page fetches until the network is idle (the
  // home route's chunks, ECharts included, but no other page's).
  const page = await fresh(browser)
  const home = await jsTransferredGzip(page, '/')
  await page.context().close()

  const kb = (n: number) => `${(n / 1024).toFixed(1)} KB`
  test.info().annotations.push({ type: 'initial-js-entry', description: `${srcs.length} files, ${kb(entry)} gzip` })
  test.info().annotations.push({ type: 'initial-js-home', description: `${home.files} files, ${kb(home.gzipBytes)} gzip` })
  console.log(`PERF initial JS: entry+preloads ${srcs.length} files ${kb(entry)} gzip; home load ${home.files} files ${kb(home.gzipBytes)} gzip (budget 350 KB)`)
  expect(entry).toBeLessThanOrEqual(350 * 1024)
  expect(home.gzipBytes).toBeLessThanOrEqual(350 * 1024)
})

test('home renders its first data within 1 s', async ({ browser }) => {
  const runs: number[] = []
  for (let i = 0; i < RUNS; i++) {
    const page = await fresh(browser)
    await page.goto('/')
    await expect(page.locator('[data-fingerprint]').first()).toBeVisible()
    await page.waitForFunction(() => performance.getEntriesByName('home-data').length > 0)
    runs.push(await markAt(page, 'home-data'))
    await page.context().close()
  }
  console.log(`PERF home first data: ${fmt(runs)} (budget 1000 ms)`)
  expect(median(runs)).toBeLessThanOrEqual(1000)
})

/** Fetch to paint of the waterfall: JSON parse, validation, layout and the first rows. */
async function waterfallMs(
  browser: Browser,
  path: string,
  prepare?: (page: Page) => Promise<void>,
  /** The virtualized tree must be as tall as this many rows (28 px each), proving every span is laid out. */
  rows?: number,
): Promise<number> {
  const page = await fresh(browser)
  await prepare?.(page)
  await page.goto(path)
  await page.waitForFunction(() => performance.getEntriesByName('waterfall').length > 0)
  const body = await markAt(page, 'trace-body')
  const painted = await markAt(page, 'waterfall')
  if (rows) expect(await page.getByRole('tree').evaluate((e) => e.firstElementChild!.getBoundingClientRect().height)).toBeGreaterThanOrEqual(rows * 28)
  await page.context().close()
  expect(body).toBeGreaterThan(0)
  return painted - body
}

test('the largest live trace renders its waterfall within 200 ms', async ({ browser, request }) => {
  const hits = await recentTraces(request)
  const widest = hits.reduce((a, b) => (b.span_count > a.span_count ? b : a))
  const runs: number[] = []
  for (let i = 0; i < RUNS; i++) runs.push(await waterfallMs(browser, `/traces/${widest.trace_id}`))
  console.log(`PERF waterfall, live trace of ${widest.span_count} spans: ${fmt(runs)} (budget 200 ms)`)
  expect(median(runs)).toBeLessThanOrEqual(200)
})

/** A trace of `n` spans: a tree of depth up to 14 across 12 services, with a few errors. */
function syntheticTrace(traceId: string, n: number) {
  const services = Array.from({ length: 12 }, (_, i) => `service-${i}`)
  const base = 1_791_000_000_000_000_000
  const depth: number[] = []
  const spans = Array.from({ length: n }, (_, i) => {
    // Parent: one of the last five spans (a deep, bushy tree); past depth 14 it hangs off the root.
    let parent = i === 0 ? -1 : Math.max(0, i - 1 - ((i * 7919) % 5))
    if (parent > 0 && depth[parent]! >= 14) parent = 0
    depth[i] = parent < 0 ? 0 : depth[parent]! + 1
    const p = parent
    return {
      span_id: i.toString(16).padStart(16, '0'),
      parent_span_id: p < 0 ? '' : p.toString(16).padStart(16, '0'),
      service_name: services[i % services.length]!,
      span_name: `GET /api/resource/${i % 97}`,
      kind: i % 3 === 0 ? 'server' : 'client',
      start_ns: base + i * 40_000,
      duration_ns: 2_000_000 + ((i * 104729) % 9_000_000),
      status: i % 250 === 0 ? 'error' : 'unset',
      status_message: '',
      attrs: [
        ['http.method', 'GET'],
        ['http.route', `/api/resource/${i % 97}`],
        ['peer.service', services[(i + 1) % services.length]!],
      ],
      resource: [['service.name', services[i % services.length]!]],
      events: [],
      self_ns: 1_000_000,
    }
  })
  return { trace_id: traceId, spans, logs: [], story_id: null }
}

test('a synthetic 5,000-span trace renders its waterfall within 200 ms', async ({ browser }) => {
  const traceId = 'abcdef0123456789abcdef0123456789'
  const body = JSON.stringify(syntheticTrace(traceId, 5000))
  const runs: number[] = []
  for (let i = 0; i < RUNS; i++) {
    runs.push(
      await waterfallMs(
        browser,
        `/traces/${traceId}`,
        async (page) => {
          await page.route(`**/api/v1/traces/${traceId}`, (route) => route.fulfill({ contentType: 'application/json', body }))
        },
        5000,
      ),
    )
  }
  console.log(`PERF waterfall, synthetic 5000 spans (${(body.length / 1e6).toFixed(1)} MB JSON): ${fmt(runs)} (budget 200 ms)`)
  expect(median(runs)).toBeLessThanOrEqual(200)
})

/** Map data in, nodes on screen: ELK layout in its worker plus the first render. */
async function mapMs(browser: Browser, prepare?: (page: Page) => Promise<void>): Promise<number> {
  const page = await fresh(browser)
  await prepare?.(page)
  await page.goto('/map')
  await page.waitForFunction(() => performance.getEntriesByName('map-nodes').length > 0)
  const body = await markAt(page, 'map-body')
  const nodes = await markAt(page, 'map-nodes')
  await page.context().close()
  expect(body).toBeGreaterThan(0)
  return nodes - body
}

test('the live service map lays out within 300 ms', async ({ browser }) => {
  const runs: number[] = []
  for (let i = 0; i < RUNS; i++) runs.push(await mapMs(browser))
  console.log(`PERF map layout, live graph: ${fmt(runs)} (budget 300 ms)`)
  expect(median(runs)).toBeLessThanOrEqual(300)
})

/** The live map repeated to 60 nodes, chained so the layers stay realistic. */
async function sixtyNodeMap(request: APIRequestContext) {
  const live = await getJson<{ nodes: { service: string }[]; edges: { parent: string; child: string }[] }>(request, '/service-map?since=1h')
  const copies = Math.ceil(60 / live.nodes.length)
  const nodes: unknown[] = []
  const edges: unknown[] = []
  for (let c = 0; c < copies; c++) {
    const name = (s: string) => (c === 0 ? s : `${s}-${c}`)
    for (const n of live.nodes) if (nodes.length < 60) nodes.push({ ...n, service: name(n.service) })
    for (const e of live.edges) edges.push({ ...e, parent: name(e.parent), child: name(e.child) })
  }
  const names = new Set(nodes.map((n) => (n as { service: string }).service))
  return { nodes, edges: edges.filter((e) => names.has((e as { parent: string }).parent) && names.has((e as { child: string }).child)) }
}

test('a 60-node service map lays out within 300 ms', async ({ browser, request }) => {
  const body = JSON.stringify(await sixtyNodeMap(request))
  const runs: number[] = []
  for (let i = 0; i < RUNS; i++) {
    runs.push(
      await mapMs(browser, async (page) => {
        await page.route('**/api/v1/service-map*', (route) => route.fulfill({ contentType: 'application/json', body }))
      }),
    )
  }
  console.log(`PERF map layout, synthetic 60 nodes: ${fmt(runs)} (budget 300 ms)`)
  expect(median(runs)).toBeLessThanOrEqual(300)
})

test('live refresh runs every 10 s and pauses while the tab is hidden', async ({ browser }) => {
  const page = await fresh(browser)
  const times: number[] = []
  page.on('request', (r) => {
    if (r.url().includes('/api/v1/overview?since=1h')) times.push(Date.now())
  })
  await page.goto('/')
  await expect(page.locator('[data-fingerprint]').first()).toBeVisible()
  await expect.poll(() => times.length, { timeout: 35_000, intervals: [500] }).toBeGreaterThanOrEqual(3)
  const gaps = times.slice(1, 3).map((t, i) => t - times[i]!)
  console.log(`PERF live refresh gaps: ${gaps.join(', ')} ms (budget 10000 ms)`)
  for (const g of gaps) expect(Math.abs(g - 10_000)).toBeLessThan(1_000)

  // Hide the tab: the app listens to visibilitychange / document.visibilityState.
  await page.evaluate(() => {
    Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'hidden' })
    document.dispatchEvent(new Event('visibilitychange'))
  })
  const before = times.length
  await page.waitForTimeout(13_000)
  expect(times.length).toBe(before)
  await page.context().close()
})
