import { presetRange } from '../../app/range'
/**
 * Logs section against captured fixtures: the alerts timeline and table, the template list
 * with its debounced search, the template page, and the empty, error and loading states.
 */
import { compact } from '../../lib/format'
import { act, fireEvent, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import logAlerts from '../../api/__fixtures__/log-alerts.json'
import logTemplate from '../../api/__fixtures__/log-template.json'
import logTemplates from '../../api/__fixtures__/log-templates.json'
import services from '../../api/__fixtures__/services.json'
import type { LogAlertView, LogTemplateDetail, LogTemplateListItem } from '../../api/types'
import { clearOutage } from '../../app/apiStatus'
import type { EChartProps } from '../../components/charts/EChart'
import { countVsBaseline, exampleLink, sortAlerts, sortTemplates, timeline } from '../../features/logs/model'
import { renderApp, stubApi } from '../../test/renderApp'
import type { Routes } from '../../test/renderApp'
import { SEARCH_DEBOUNCE_MS } from './templates'

let chart: EChartProps | undefined
vi.mock('../../components/charts/EChartImpl', () => ({
  default: (p: EChartProps) => {
    chart = p
    return <div data-testid="chart" />
  },
}))

// The virtualizer renders rows only with a measured viewport.
const sizes = ['offsetHeight', 'offsetWidth', 'clientHeight', 'clientWidth'] as const
beforeAll(() => {
  for (const p of sizes) Object.defineProperty(HTMLElement.prototype, p, { configurable: true, get: () => 4000 })
})
afterAll(() => {
  for (const p of sizes) delete (HTMLElement.prototype as unknown as Record<string, unknown>)[p]
})
afterEach(() => {
  vi.unstubAllGlobals()
  clearOutage()
  chart = undefined
})

const alerts = logAlerts as LogAlertView[]
const templates = logTemplates as unknown as LogTemplateListItem[]
const detail = logTemplate as unknown as LogTemplateDetail
const TID = detail.template.template_id

const routes = (extra: Routes = {}): Routes => ({
  '/log-alerts': { body: alerts },
  '/log-templates': { body: templates },
  [`/log-templates/${TID}`]: { body: detail },
  '/services': { body: services },
  '/service-map': { body: { nodes: [], edges: [] } },
  ...extra,
})

const calls = (fetch: ReturnType<typeof stubApi>, prefix: string) =>
  fetch.mock.calls.map((c) => String(c[0])).filter((u) => u.startsWith(`/api/v1/${prefix}`))

describe('model', () => {
  it('links an example trace to its story when it has one, else to the trace', () => {
    expect(exampleLink({ trace_id: 'a', story_id: 'b' })).toEqual({ to: '/stories/$storyId', params: { storyId: 'b' } })
    expect(exampleLink({ trace_id: 'a', story_id: null })).toEqual({ to: '/traces/$traceId', params: { traceId: 'a' } })
  })

  it('counts alerts per bucket and kind over the whole window', () => {
    const now = Date.UTC(2026, 9, 4, 12, 30)
    const at = (min: number) => (now - min * 60_000) * 1e6
    const a = (kind: 'new' | 'spike', min: number) => ({ kind, started_at_ns: at(min) }) as LogAlertView
    const bars = timeline([a('new', 5), a('spike', 5), a('spike', 6), a('spike', 24 * 60 * 2)], presetRange('24h'), now)
    expect(bars).toHaveLength(25)
    expect(bars.reduce((n, b) => n + b.new, 0)).toBe(1)
    expect(bars.reduce((n, b) => n + b.spike, 0)).toBe(2) // the 2-day-old alert is outside
    expect(bars.at(-1)).toMatchObject({ new: 1, spike: 2 })
    expect(bars[1]!.t - bars[0]!.t).toBe(3_600_000)
  })

  it('describes the count against the baseline, and sorts templates', () => {
    expect(countVsBaseline({ kind: 'spike', peak_count: 35, baseline_per_window: 1.75 })).toBe('35 vs 1.8 / window')
    expect(countVsBaseline({ kind: 'new', peak_count: 0, baseline_per_window: 0 })).toBe('first seen')
    const mk = (id: string, active: boolean, last: number) => ({ alert_id: id, active, last_at_ns: last }) as LogAlertView
    expect(sortAlerts([mk('old-active', true, 1), mk('new-ended', false, 9), mk('new-active', true, 5), mk('old-ended', false, 2)]).map((a) => a.alert_id)).toEqual([
      'new-active',
      'old-active',
      'new-ended',
      'old-ended',
    ])
    const byCount = sortTemplates(templates, { key: 'count', desc: false })
    expect(byCount[0]!.count).toBe(Math.min(...templates.map((t) => t.count)))
  })
})

describe('log alerts', () => {
  it('lists every alert with its kind, service, template and example traces', async () => {
    const fetch = stubApi(routes())
    renderApp('/logs/alerts')
    const table = await screen.findByRole('table', { name: 'Log alerts' })
    const rows = within(table).getAllByRole('row').slice(1)
    expect(rows).toHaveLength(alerts.length)
    const spike = alerts.find((a) => a.kind === 'spike' && a.example_traces.some((e) => e.story_id))!
    const row = rows[alerts.indexOf(spike)] as HTMLElement
    expect(within(row).getByText('spike')).toHaveAttribute('data-kind', 'spike')
    expect(within(row).getByText(spike.service)).toBeInTheDocument()
    expect(within(row).getByRole('link', { name: spike.template })).toHaveAttribute('href', `/logs/templates/${spike.template_id}`)
    expect(within(row).getByText(`${spike.peak_count} vs ${spike.baseline_per_window.toFixed(1)} / window`)).toBeInTheDocument()
    // Story when there is one, else the trace.
    for (const e of spike.example_traces) {
      const link = within(row).getByRole('link', { name: `${e.story_id ? 'Story' : 'Trace'} ${e.trace_id}` })
      expect(link).toHaveAttribute('href', e.story_id ? `/stories/${e.story_id}` : `/traces/${e.trace_id}`)
    }
    const plain = alerts.find((a) => a.example_traces.some((e) => !e.story_id))!
    const plainRow = rows[alerts.indexOf(plain)] as HTMLElement
    const e = plain.example_traces.find((x) => !x.story_id)!
    expect(within(plainRow).getByRole('link', { name: `Trace ${e.trace_id}` })).toHaveAttribute('href', `/traces/${e.trace_id}`)
    expect(within(table).getAllByText('new')[0]).toHaveAttribute('data-kind', 'new')
    expect(calls(fetch, 'log-alerts')[0]).toBe('/api/v1/log-alerts?since=1h')
    expect(screen.getByText(`${alerts.length} alerts · 0 active`)).toBeInTheDocument()
  })

  it('draws the timeline as stacked bars per kind with a text summary', async () => {
    stubApi(routes())
    renderApp('/logs/alerts?since=7d')
    await screen.findByRole('table', { name: 'Log alerts' })
    await waitFor(() => expect(chart).toBeDefined())
    expect(screen.getByText(/^Log alerts started per hour over the last 7d: \d+ new, \d+ spike\.$/)).toBeInTheDocument()
    const series = (chart!.option.series as Array<{ name: string; type: string; stack: string; data: unknown[] }>)
    expect(series.map((s) => [s.name, s.type, s.stack])).toEqual([
      ['new', 'bar', 'total'],
      ['spike', 'bar', 'total'],
    ])
    expect(series[0]!.data).toHaveLength(169)
  })

  it('shows an active alert with a pulse', async () => {
    const live = alerts.map((a, i) => (i === 0 ? { ...a, active: true } : a))
    stubApi(routes({ '/log-alerts': { body: live } }))
    renderApp('/logs/alerts')
    const table = await screen.findByRole('table', { name: 'Log alerts' })
    expect(within(table).getAllByText('active')).toHaveLength(1)
    expect(screen.getByText(`${alerts.length} alerts · 1 active`)).toBeInTheDocument()
  })

  it('lists active alerts first, then the newest, inside a labelled scroll region', async () => {
    const live = alerts.map((a, i) => (i === alerts.length - 1 ? { ...a, active: true } : { ...a, active: false }))
    stubApi(routes({ '/log-alerts': { body: live } }))
    renderApp('/logs/alerts')
    const table = await screen.findByRole('table', { name: 'Log alerts' })
    const rows = within(table).getAllByRole('row').slice(1)
    const last = live[live.length - 1]!
    expect(rows[0]).toHaveTextContent(last.service)
    expect(within(rows[0] as HTMLElement).getByText('active')).toBeInTheDocument()
    // The rest by last seen, newest first.
    const order = live.filter((a) => !a.active).sort((a, b) => b.last_at_ns - a.last_at_ns)
    expect(rows[1]).toHaveTextContent(order[0]!.template.slice(0, 20))
    const region = screen.getByRole('region', { name: 'Log alerts (scrollable)' })
    expect(region).toHaveAttribute('tabindex', '0')
    expect(region.style.maxHeight).toContain('vh')
  })

  it('Active only goes into the URL and hides ended alerts', async () => {
    const user = userEvent.setup()
    const live = alerts.map((a, i) => ({ ...a, active: i < 2 }))
    stubApi(routes({ '/log-alerts': { body: live } }))
    const { router } = renderApp('/logs/alerts')
    await screen.findByRole('table', { name: 'Log alerts' })
    await user.click(screen.getByRole('button', { name: 'Active only' }))
    await waitFor(() => expect(router.state.location.search).toMatchObject({ active: true }))
    const table = await screen.findByRole('table', { name: 'Log alerts' })
    await waitFor(() => expect(within(table).getAllByRole('row').slice(1)).toHaveLength(2))
    expect(screen.getByRole('button', { name: 'Active only' })).toHaveAttribute('aria-pressed', 'true')
    expect(screen.getByText('2 alerts · 2 active')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Clear filters' }))
    await waitFor(() => expect(router.state.location.search).toEqual({}))
  })

  it('reads Active only from the URL', async () => {
    stubApi(routes())
    renderApp('/logs/alerts?active=true')
    // No alert in the fixture is active.
    expect(await screen.findByText('No alerts match these filters')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Active only' })).toHaveAttribute('aria-pressed', 'true')
  })

  it('stacks each alert as a card on a narrow screen', async () => {
    const original = window.matchMedia
    window.matchMedia = ((query: string) => ({ ...original(query), matches: query.includes('max-width: 639') })) as typeof window.matchMedia
    try {
      stubApi(routes())
      renderApp('/logs/alerts')
      const list = await screen.findByRole('list', { name: 'Log alerts' })
      const items = within(list).getAllByRole('listitem').filter((li) => li.parentElement === list)
      expect(items).toHaveLength(alerts.length)
      const first = items[0] as HTMLElement
      const top = [...alerts].sort((a, b) => b.last_at_ns - a.last_at_ns)[0]!
      expect(within(first).getByRole('link', { name: top.template })).toHaveAttribute('href', `/logs/templates/${top.template_id}`)
      expect(screen.queryByRole('table', { name: 'Log alerts' })).toBeNull()
    } finally {
      window.matchMedia = original
    }
  })

  it('captions the chart as alerts started in the window', async () => {
    stubApi(routes())
    renderApp('/logs/alerts')
    expect(await screen.findByText('alerts started in this window, by kind')).toBeInTheDocument()
  })

  it('shows the full template in a tooltip', async () => {
    const user = userEvent.setup()
    stubApi(routes())
    renderApp('/logs/alerts')
    const table = await screen.findByRole('table', { name: 'Log alerts' })
    const long = alerts.reduce((a, b) => (b.template.length > a.template.length ? b : a))
    // jsdom has no layout: report the text as cut off, so the tooltip may open.
    vi.spyOn(HTMLElement.prototype, 'scrollWidth', 'get').mockReturnValue(400)
    vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(100)
    await user.hover(within(table).getAllByRole('link', { name: long.template })[0] as HTMLElement)
    expect((await screen.findAllByRole('tooltip'))[0]).toHaveTextContent(long.template)
  })

  it('kind and service filters go into the URL and the API query', async () => {
    const user = userEvent.setup()
    const fetch = stubApi(routes())
    const { router } = renderApp('/logs/alerts')
    await screen.findByRole('table', { name: 'Log alerts' })
    await user.click(screen.getByRole('radio', { name: 'Spike' }))
    await waitFor(() => expect(router.state.location.search).toMatchObject({ kind: 'spike' }))
    await waitFor(() => expect(calls(fetch, 'log-alerts').at(-1)).toBe('/api/v1/log-alerts?since=1h&kind=spike'))

    await user.click(screen.getByRole('button', { name: 'Service: any' }))
    await user.click(await screen.findByRole('option', { name: 'payment' }))
    await waitFor(() => expect(router.state.location.search).toMatchObject({ kind: 'spike', service: 'payment' }))
    await waitFor(() => expect(calls(fetch, 'log-alerts').at(-1)).toBe('/api/v1/log-alerts?since=1h&kind=spike&service=payment'))

    await user.click(screen.getByRole('button', { name: 'Clear filters' }))
    await waitFor(() => expect(router.state.location.search).toEqual({}))
  })

  it('reads the filters from the URL', async () => {
    const fetch = stubApi(routes())
    renderApp('/logs/alerts?kind=new&service=checkout')
    await screen.findByRole('table', { name: 'Log alerts' })
    expect(calls(fetch, 'log-alerts')[0]).toBe('/api/v1/log-alerts?since=1h&kind=new&service=checkout')
    expect(screen.getByRole('radio', { name: 'New' })).toBeChecked()
  })

  it('shows the empty states, with and without filters', async () => {
    const user = userEvent.setup()
    stubApi(routes({ '/log-alerts': { body: [] } }))
    const { router } = renderApp('/logs/alerts')
    expect(await screen.findByText('No log alerts in this window')).toBeInTheDocument()
    expect(screen.queryByRole('table', { name: 'Log alerts' })).toBeNull()
    await act(async () => void router.navigate({ to: '/logs/alerts', search: { kind: 'new' } }))
    expect(await screen.findByText('No alerts match these filters')).toBeInTheDocument()
    await user.click(screen.getAllByRole('button', { name: 'Clear filters' }).at(-1)!)
    expect(await screen.findByText('No log alerts in this window')).toBeInTheDocument()
  })

  it('shows an error banner that retries, and skeletons while loading', async () => {
    const user = userEvent.setup()
    const fetch = stubApi(routes({ '/log-alerts': { status: 503, body: { error: 'clickhouse unavailable' } } }))
    renderApp('/logs/alerts')
    const banner = (await screen.findByText('Could not load log alerts.')).closest('[role="alert"]') as HTMLElement
    expect(banner).toHaveTextContent('503 · clickhouse unavailable')
    const before = calls(fetch, 'log-alerts').length
    await user.click(within(banner).getByRole('button', { name: 'Try again' }))
    await waitFor(() => expect(calls(fetch, 'log-alerts').length).toBeGreaterThan(before))
  })

  it('shows skeletons while loading', async () => {
    vi.stubGlobal('fetch', vi.fn(() => new Promise(() => {})))
    renderApp('/logs/alerts')
    expect(await screen.findByLabelText('Loading log alerts')).toBeInTheDocument()
  })
})

describe('log templates', () => {
  const bodyRows = async () => {
    const table = await screen.findByRole('table', { name: 'Templates' })
    return within(table).getAllByRole('row').slice(1)
  }

  it('lists every template with count, service and a trend', async () => {
    const fetch = stubApi(routes())
    renderApp('/logs/templates')
    const rows = await bodyRows()
    expect(rows).toHaveLength(templates.length)
    const first = rows[0] as HTMLElement
    const top = templates[0]!
    expect(within(first).getByRole('link', { name: top.template })).toHaveAttribute('href', `/logs/templates/${top.template_id}`)
    expect(within(first).getByText(compact(top.count))).toHaveAttribute('title', String(top.count))
    expect(within(first).getByText(top.service)).toBeInTheDocument()
    // The trend comes with the list: no request per row.
    expect(within(first).getByRole('img', { name: new RegExp(`^${top.count} hits over 1h, peak \\d+ per bucket$`) })).toBeInTheDocument()
    expect(calls(fetch, 'log-templates/')).toEqual([])
    expect(screen.getByText(`${templates.length} templates`)).toBeInTheDocument()
    expect(screen.getByRole('region', { name: 'Log templates' }).style.maxHeight).toContain('vh')
  })

  it('marks alerting templates', async () => {
    const alerting = templates.map((t, i) => ({ ...t, alerting: i === 1 }))
    stubApi(routes({ '/log-templates': { body: alerting } }))
    renderApp('/logs/templates')
    const rows = await bodyRows()
    expect(within(rows[1] as HTMLElement).getByText('alerting')).toHaveAttribute('data-kind', 'spike')
    expect(screen.getAllByText('alerting')).toHaveLength(1)
  })

  it('sorts by a column', async () => {
    const user = userEvent.setup()
    stubApi(routes())
    renderApp('/logs/templates')
    await bodyRows()
    await user.click(screen.getByRole('button', { name: 'Hits' }))
    const rows = await bodyRows()
    expect(rows[0]).toHaveTextContent(String(Math.min(...templates.map((t) => t.count))))
  })

  it('debounces the search and writes it to the URL and the API query', async () => {
    const user = userEvent.setup()
    const fetch = stubApi(routes())
    const { router } = renderApp('/logs/templates')
    await bodyRows()
    const box = screen.getByRole('searchbox', { name: 'Search templates' })
    await user.type(box, 'pay')
    // Typing is immediate in the field, not yet in the URL or the API.
    expect(box).toHaveValue('pay')
    expect(router.state.location.search).not.toHaveProperty('q')
    await waitFor(() => expect(router.state.location.search).toMatchObject({ q: 'pay' }), { timeout: SEARCH_DEBOUNCE_MS * 4 })
    await waitFor(() => expect(calls(fetch, 'log-templates?').at(-1)).toBe('/api/v1/log-templates?since=1h&q=pay'))
    // One request for the whole word, not one per keystroke.
    expect(calls(fetch, 'log-templates?').filter((u) => u.includes('q='))).toEqual(['/api/v1/log-templates?since=1h&q=pay'])
  })

  it('reads the search and service from the URL, and clears them', async () => {
    const user = userEvent.setup()
    const fetch = stubApi(routes())
    const { router } = renderApp('/logs/templates?q=failed&service=payment')
    await bodyRows()
    expect(screen.getByRole('searchbox', { name: 'Search templates' })).toHaveValue('failed')
    expect(calls(fetch, 'log-templates?')[0]).toBe('/api/v1/log-templates?since=1h&service=payment&q=failed')
    await user.click(screen.getByRole('button', { name: 'Clear filters' }))
    await waitFor(() => expect(router.state.location.search).toEqual({}))
    expect(screen.getByRole('searchbox', { name: 'Search templates' })).toHaveValue('')
  })

  it('picks a service into the URL', async () => {
    const user = userEvent.setup()
    const { router } = (stubApi(routes()), renderApp('/logs/templates'))
    await bodyRows()
    await user.click(screen.getByRole('button', { name: 'Service: any' }))
    await user.click(await screen.findByRole('option', { name: 'cart' }))
    await waitFor(() => expect(router.state.location.search).toMatchObject({ service: 'cart' }))
  })

  it('shows the empty states', async () => {
    stubApi(routes({ '/log-templates': { body: [] } }))
    const { router } = renderApp('/logs/templates')
    expect(await screen.findByText('No log templates in this window')).toBeInTheDocument()
    await act(async () => void router.navigate({ to: '/logs/templates', search: { q: 'zzz' } }))
    expect(await screen.findByText('No templates match these filters')).toBeInTheDocument()
  })

  it('shows an error banner that retries', async () => {
    const user = userEvent.setup()
    const fetch = stubApi(routes({ '/log-templates': { status: 503, body: { error: 'clickhouse unavailable' } } }))
    renderApp('/logs/templates')
    const banner = (await screen.findByText('Could not load log templates.')).closest('[role="alert"]') as HTMLElement
    const before = calls(fetch, 'log-templates?').length
    await user.click(within(banner).getByRole('button', { name: 'Try again' }))
    await waitFor(() => expect(calls(fetch, 'log-templates?').length).toBeGreaterThan(before))
  })

  it('shows skeletons while loading', async () => {
    vi.stubGlobal('fetch', vi.fn(() => new Promise(() => {})))
    renderApp('/logs/templates')
    expect(await screen.findByLabelText('Loading log templates')).toBeInTheDocument()
  })
})

describe('log template page', () => {
  const XSS = '<script>alert(1)</script><img src=x onerror=alert(2)>'

  it('shows the template, chart, sample, recent hits and alerts', async () => {
    const withAlert = { ...detail, alerts: [alerts[0]!, alerts[3]!] }
    stubApi(routes({ [`/log-templates/${TID}`]: { body: withAlert } }))
    renderApp(`/logs/templates/${TID}`)
    expect(await screen.findByRole('heading', { level: 2, name: detail.template.template })).toBeInTheDocument()
    expect(screen.getByText('Sample').closest('section, div')).toBeTruthy()
    expect(screen.getByText(detail.sample.trim())).toBeInTheDocument()
    await waitFor(() => expect(chart).toBeDefined())
    const series = chart!.option.series as Array<{ type: string; data: unknown[] }>
    expect(series[0]!.type).toBe('bar')
    // One bar per bucket of the 1h window, starting at its start (as the API buckets it).
    expect(series[0]!.data).toHaveLength(Math.ceil(3600 / detail.bucket_secs))
    const hits = within(screen.getByRole('table', { name: 'Recent hits' })).getAllByRole('row').slice(1)
    expect(hits).toHaveLength(detail.recent.length)
    const first = detail.recent[0]!
    expect(within(hits[0] as HTMLElement).getByRole('link')).toHaveAttribute('href', `/traces/${first.trace_id}`)
    const al = within(screen.getByRole('table', { name: 'Alerts of this template' })).getAllByRole('row').slice(1)
    expect(al).toHaveLength(2)
    expect(screen.queryByRole('columnheader', { name: 'Template' })).toBeNull()
  })

  it('links recent hits to their story when they have one', async () => {
    const sid = 'a'.repeat(32)
    const body = { ...detail, recent: detail.recent.map((h, i) => (i === 0 ? { ...h, story_id: sid } : h)) }
    stubApi(routes({ [`/log-templates/${TID}`]: { body } }))
    renderApp(`/logs/templates/${TID}`)
    const link = await screen.findByRole('link', { name: `Story of trace ${detail.recent[0]!.trace_id}` })
    expect(link).toHaveAttribute('href', `/stories/${sid}`)
  })

  it('renders hostile template and sample text as text', async () => {
    const body = { ...detail, sample: XSS, template: { ...detail.template, template: XSS } }
    stubApi(routes({ [`/log-templates/${TID}`]: { body } }))
    const { container } = renderApp(`/logs/templates/${TID}`)
    await screen.findByRole('heading', { level: 2, name: XSS })
    expect(screen.getByText(XSS, { selector: 'pre' })).toBeInTheDocument()
    expect(container.querySelector('script')).toBeNull()
    expect(container.querySelector('img[src="x"]')).toBeNull()
  })

  it('shows empty states for no hits, alerts and recent hits', async () => {
    const body = { ...detail, buckets: [], recent: [], alerts: [] }
    stubApi(routes({ [`/log-templates/${TID}`]: { body } }))
    renderApp(`/logs/templates/${TID}`)
    expect(await screen.findByText('No hits in this window')).toBeInTheDocument()
    expect(screen.getByText('No hits recorded')).toBeInTheDocument()
    expect(screen.getByText('No alerts for this template')).toBeInTheDocument()
  })

  it('shows not found for an unknown template, an error with retry, and a skeleton', async () => {
    stubApi(routes({ [`/log-templates/${TID}`]: { status: 404, body: { error: 'not found' } } }))
    const { unmount } = renderApp(`/logs/templates/${TID}`)
    expect(await screen.findByText('Template not found')).toBeInTheDocument()
    unmount()

    const fetch = stubApi(routes({ [`/log-templates/${TID}`]: { status: 500, body: { error: 'boom' } } }))
    renderApp(`/logs/templates/${TID}`)
    const retry = await screen.findByRole('button', { name: /try again/i })
    const before = calls(fetch, 'log-templates/').length
    fireEvent.click(retry)
    await waitFor(() => expect(calls(fetch, 'log-templates/').length).toBeGreaterThan(before))
  })

  it('a failed refresh keeps the page and notes it', async () => {
    stubApi(routes())
    const { queryClient } = renderApp(`/logs/templates/${TID}`)
    await screen.findByRole('heading', { level: 2, name: detail.template.template })
    stubApi(routes({ [`/log-templates/${TID}`]: { status: 503, body: { error: 'clickhouse unavailable' } } }))
    await act(() => queryClient.refetchQueries({ queryKey: ['log-templates'] }))
    expect(await screen.findByText(/^Refresh failed · showing data from \d\d:\d\d:\d\d$/)).toBeInTheDocument()
    expect(screen.getByRole('heading', { level: 2, name: detail.template.template })).toBeInTheDocument()
    expect(screen.getByRole('table', { name: 'Recent hits' })).toBeInTheDocument()
    expect(screen.queryByText('Storage is unavailable')).toBeNull()
  })

  it('shows a skeleton while loading, and 404s a malformed id', async () => {
    vi.stubGlobal('fetch', vi.fn(() => new Promise(() => {})))
    const { unmount } = renderApp(`/logs/templates/${TID}`)
    expect(await screen.findByLabelText('Loading log template')).toBeInTheDocument()
    unmount()
    stubApi(routes())
    renderApp('/logs/templates/not-a-number')
    expect(await screen.findByText(/not found/i)).toBeInTheDocument()
  })
})
