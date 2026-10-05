/**
 * Traces explorer against fixtures: filters write the URL and the API query, a brush (the
 * ECharts brushEnd event, dispatched through the mocked chart's handlers) filters the table,
 * point clicks and the trace-id box open traces, and the empty and error states.
 */
import { act, fireEvent, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import services from '../../api/__fixtures__/services.json'
import fixture from '../../api/__fixtures__/traces-search.json'
import type { TraceHit } from '../../api/types'
import { clearOutage } from '../../app/apiStatus'
import { formatUntil } from '../../app/search'
import type { EChartProps } from '../../components/charts/EChart'
import { renderApp, stubApi } from '../../test/renderApp'
import type { Routes } from '../../test/renderApp'

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

const rows = fixture as TraceHit[]
const routes = (extra: Routes = {}): Routes => ({
  '/traces/search': { body: rows },
  '/services': { body: services },
  '/service-map': { body: { nodes: [], edges: [] } },
  ...extra,
})

const bodyRows = async () => {
  const table = await screen.findByRole('table', { name: 'Traces' })
  return within(table).getAllByRole('row').slice(1)
}
const escapeRe = (v: string) => v.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
const searchCalls = (fetch: ReturnType<typeof stubApi>) =>
  fetch.mock.calls.map((c) => String(c[0])).filter((u) => u.startsWith('/api/v1/traces/search'))

describe('traces explorer', () => {
  it('plots and lists every result, newest first, with links to traces and stories', async () => {
    const fetch = stubApi(routes())
    renderApp('/traces')
    expect(await bodyRows()).toHaveLength(rows.length)
    expect(searchCalls(fetch)[0]).toBe('/api/v1/traces/search?since=1h&limit=500')
    const withStory = rows.find((r) => r.story_id)!
    expect(screen.getByRole('link', { name: `${withStory.story_kind} story of trace ${withStory.trace_id}` })).toHaveAttribute(
      'href',
      `/stories/${withStory.story_id}`,
    )
    const first = (await bodyRows())[0] as HTMLElement
    expect(first).toHaveAttribute('data-trace-id', rows[0]!.trace_id)
    expect(within(first).getByRole('link')).toHaveAttribute('href', `/traces/${rows[0]!.trace_id}`)
    expect(screen.getByText(`${rows.length} traces`)).toBeInTheDocument()
    // The chart's text summary.
    expect(screen.getByText(new RegExp(`^Duration over time of ${rows.length} traces in the last 1h: 1 with errors or error stories, 0 with slow stories`))).toBeInTheDocument()
  })

  it('a custom range sends until, keys the query by it, spans the x-axis over it and links carry it', async () => {
    const end = Math.floor(Date.now() / 60_000) * 60_000 - 3_600_000
    const until = formatUntil(end)
    const fetch = stubApi(routes())
    const { queryClient } = renderApp(`/traces?since=2h&until=${until}&errors=1`)
    expect(await bodyRows()).toHaveLength(rows.length)
    expect(searchCalls(fetch)[0]).toBe(`/api/v1/traces/search?since=2h&until=${encodeURIComponent(until)}&errors=true&limit=500`)
    const keys = queryClient
      .getQueryCache()
      .findAll({ queryKey: ['trace-search'] })
      .map((q) => q.queryKey[2] as Record<string, unknown>)
    expect(keys).toEqual([expect.objectContaining({ since: '2h', until })])
    // The scatter spans the custom window, not the last 2h.
    await waitFor(() => expect(chart).toBeDefined())
    const x = (chart!.option as { xAxis: { min: number; max: number } }).xAxis
    expect([x.min, x.max]).toEqual([end - 7_200_000, end])
    // A row's trace link keeps the range.
    const link = within((await bodyRows())[0]!).getAllByRole('link')[0]!
    expect(link.getAttribute('href')).toContain(`until=${encodeURIComponent(until)}`)
    // No refetch timer for a past range.
    expect(queryClient.getQueryCache().findAll({ queryKey: ['trace-search'] })[0]!.observers[0]!.options.refetchInterval).toBe(false)
  })

  it('filters build the URL and the API query string', async () => {
    const user = userEvent.setup()
    const fetch = stubApi(routes())
    const { router } = renderApp('/traces?since=15m')
    await bodyRows()

    await user.click(screen.getByRole('button', { name: 'Service: any' }))
    await user.type(await screen.findByPlaceholderText('Search service'), 'paym')
    await user.click(await screen.findByRole('option', { name: 'payment' }))
    // A picked service matches any span of the trace by default.
    await waitFor(() => expect(router.state.location.search).toMatchObject({ service: 'payment', touched: true }))
    const scope = screen.getByRole('radiogroup', { name: 'Service match' })
    expect(within(scope).getByRole('radio', { name: 'Anywhere in trace' })).toHaveAttribute('data-state', 'on')

    await user.click(screen.getByRole('button', { name: 'Errors only' }))
    await user.type(screen.getByLabelText('Min duration (ms)'), '5')
    await user.type(screen.getByLabelText('Max duration (ms)'), '250{Enter}')
    await waitFor(() =>
      expect(router.state.location.search).toMatchObject({ service: 'payment', touched: true, errors: true, min_ms: 5, max_ms: 250 }),
    )
    await waitFor(() =>
      expect(searchCalls(fetch).at(-1)).toBe(
        '/api/v1/traces/search?since=15m&service=payment&touched=1&min_ms=5&max_ms=250&errors=true&limit=500',
      ),
    )
    expect(screen.getByRole('button', { name: 'Errors only' })).toHaveAttribute('aria-pressed', 'true')

    // As endpoint: only traces whose root is the service.
    await user.click(within(scope).getByRole('radio', { name: 'As endpoint' }))
    await waitFor(() => expect(router.state.location.search).not.toHaveProperty('touched'))
    await waitFor(() =>
      expect(searchCalls(fetch).at(-1)).toBe('/api/v1/traces/search?since=15m&service=payment&min_ms=5&max_ms=250&errors=true&limit=500'),
    )

    await user.click(screen.getByRole('button', { name: 'Clear service filter' }))
    await waitFor(() => expect(router.state.location.search).not.toHaveProperty('service'))
    expect(screen.queryByRole('radiogroup', { name: 'Service match' })).toBeNull()
  })

  it('rejects a min above max inline without querying', async () => {
    const user = userEvent.setup()
    const fetch = stubApi(routes())
    const { router } = renderApp('/traces')
    await bodyRows()
    const calls = searchCalls(fetch).length
    await user.type(screen.getByLabelText('Min duration (ms)'), '500')
    await user.type(screen.getByLabelText('Max duration (ms)'), '5{Enter}')
    expect(screen.getByRole('alert')).toHaveTextContent('Min must not exceed max.')
    expect(screen.getByLabelText('Max duration (ms)')).toHaveValue('5')
    // The valid min was committed on blur; the invalid max never reaches the URL or the API.
    await waitFor(() => expect(router.state.location.search).toMatchObject({ min_ms: 500 }))
    expect(router.state.location.search).not.toHaveProperty('max_ms')
    expect(searchCalls(fetch).slice(calls).every((u) => !u.includes('max_ms'))).toBe(true)
  })

  it('a brush selection filters the table; clear selection restores it', async () => {
    const user = userEvent.setup()
    stubApi(routes())
    const { router } = renderApp('/traces')
    await bodyRows()
    await waitFor(() => expect(chart?.onEvents?.brushEnd).toBeDefined())
    // Select the slowest third of the window: rows from 100 ms to 1 s.
    const inside = rows.filter((r) => r.duration_ns >= 100e6 && r.duration_ns <= 1000e6)
    expect(inside.length).toBeGreaterThan(0)
    const ts = rows.map((r) => r.ts_ns / 1e6)
    act(() =>
      chart!.onEvents!.brushEnd!({
        type: 'brushEnd',
        areas: [{ brushType: 'rect', coordRange: [[Math.min(...ts) - 1, Math.max(...ts) + 1], [100, 1000]] }],
      }),
    )
    await waitFor(() => expect(router.state.location.search).toHaveProperty('sel'))
    await waitFor(async () => expect(await bodyRows()).toHaveLength(inside.length))
    expect(screen.getByText(`${inside.length} of ${rows.length} selected`)).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Clear selection' }))
    await waitFor(() => expect(router.state.location.search).not.toHaveProperty('sel'))
    expect(await bodyRows()).toHaveLength(rows.length)
  })

  it('a brush below the axis starts at 0 ms; changing a filter clears the selection', async () => {
    const user = userEvent.setup()
    stubApi(routes())
    const { router } = renderApp('/traces')
    await bodyRows()
    await waitFor(() => expect(chart?.onEvents?.brushEnd).toBeDefined())
    act(() => chart!.onEvents!.brushEnd!({ type: 'brushEnd', areas: [{ brushType: 'rect', coordRange: [[1, 2], [-40, 60]] }] }))
    await waitFor(() => expect(router.state.location.search).toMatchObject({ sel: '1_2_0_60' }))
    await user.click(screen.getByRole('button', { name: 'Errors only' }))
    await waitFor(() => expect(router.state.location.search).toEqual({ errors: true }))
  })

  it("with an endpoint picked, the picker still offers the service's other endpoints", async () => {
    const user = userEvent.setup()
    stubApi(routes())
    const { router } = renderApp('/traces')
    await bodyRows()
    const names = [...new Set(rows.map((r) => r.endpoint_name))]
    expect(names.length).toBeGreaterThan(2)
    const option = (name: string) => screen.getByRole('option', { name: new RegExp(`^${escapeRe(name)}\\d+$`) })
    // From now on the API answers with the first endpoint's rows only.
    stubApi(routes({ '/traces/search': { body: rows.filter((r) => r.endpoint_name === names[0]) } }))
    await user.click(screen.getByRole('button', { name: 'Endpoint: any' }))
    await user.click(await waitFor(() => option(names[0]!)))
    await waitFor(() => expect(router.state.location.search).toMatchObject({ endpoint: names[0] }))
    await waitFor(async () => expect(await bodyRows()).toHaveLength(rows.filter((r) => r.endpoint_name === names[0]).length))
    await waitFor(() => expect(screen.getByRole('button', { name: `Endpoint: ${names[0]}` })).toBeInTheDocument())
    await user.click(screen.getByRole('button', { name: `Endpoint: ${names[0]}` }))
    const options = await screen.findAllByRole('option')
    expect(options.length).toBe(names.length + 1)
    await user.click(option(names[1]!))
    await waitFor(() => expect(router.state.location.search).toMatchObject({ endpoint: names[1] }))
  })

  it('the results scroller is keyboard reachable and End mounts the last row', async () => {
    const user = userEvent.setup()
    // jsdom has no element scrolling: emulate scrollTo the way browsers do.
    const proto = Element.prototype as unknown as { scrollTo?: unknown }
    const had = proto.scrollTo
    proto.scrollTo = function (this: HTMLElement, o: { top?: number }) {
      this.scrollTop = o.top ?? 0
      this.dispatchEvent(new Event('scroll'))
    }
    try {
      const many: TraceHit[] = Array.from({ length: 300 }, (_, i) => ({
        ...rows[0]!,
        trace_id: i.toString(16).padStart(32, '0'),
        ts_ns: rows[0]!.ts_ns - i * 1e9,
      }))
      stubApi(routes({ '/traces/search': { body: many } }))
      renderApp('/traces')
      const region = await screen.findByRole('region', { name: 'Trace results' })
      expect(region).toHaveAttribute('tabindex', '0')
      let top = 0
      Object.defineProperty(region, 'scrollTop', { configurable: true, get: () => top, set: (v: number) => (top = v) })
      // The virtualizer clamps scrolling to scrollHeight - clientHeight (4000, stubbed above).
      Object.defineProperty(region, 'scrollHeight', { configurable: true, get: () => 34 + many.length * 38 })
      const last = many.at(-1)!.trace_id
      expect(document.querySelector(`[data-trace-id="${last}"]`)).toBeNull()
      // Tab order reaches the scroller (after the header, filters and chart controls).
      for (let i = 0; i < 40 && document.activeElement !== region; i++) await user.tab()
      expect(region).toHaveFocus()
      await user.keyboard('{End}')
      const row = await waitFor(() => {
        const el = document.querySelector<HTMLElement>(`[data-trace-id="${last}"]`)
        expect(el).not.toBeNull()
        return el!
      })
      const link = within(row).getByRole('link')
      expect(link).toHaveAttribute('href', `/traces/${last}`)
      act(() => link.focus())
      expect(link).toHaveFocus()
      await user.keyboard('{Home}')
      await waitFor(() => expect(document.querySelector(`[data-trace-id="${many[0]!.trace_id}"]`)).not.toBeNull())
      await user.keyboard('{PageDown}')
      await waitFor(() => expect(region.scrollTop).toBeGreaterThan(0))
    } finally {
      proto.scrollTo = had
    }
  })

  it('a selection with no points says so', async () => {
    stubApi(routes())
    renderApp('/traces?sel=1_2_3_4')
    expect(await screen.findByText('No traces in the selection')).toBeInTheDocument()
  })

  it('clicking a point opens its trace', async () => {
    stubApi(routes())
    const { router } = renderApp('/traces?since=24h')
    await bodyRows()
    await waitFor(() => expect(chart?.onEvents?.click).toBeDefined())
    act(() => chart!.onEvents!.click!({ componentType: 'series', data: { value: [0, 0], id: rows[3]!.trace_id } }))
    await waitFor(() => expect(router.state.location.pathname).toBe(`/traces/${rows[3]!.trace_id}`))
    expect(router.state.location.search).toEqual({ since: '24h' })
  })

  it('an invalid trace id shows an inline error; a valid one opens the trace', async () => {
    const user = userEvent.setup()
    stubApi(routes())
    const { router } = renderApp('/traces')
    const box = await screen.findByLabelText('Trace id')
    await user.type(box, 'abc123{Enter}')
    expect(screen.getByRole('alert')).toHaveTextContent('Not a trace id: expected 32 hex characters.')
    expect(box).toHaveAttribute('aria-invalid', 'true')
    expect(router.state.location.pathname).toBe('/traces')

    await user.clear(box)
    expect(screen.queryByRole('alert')).toBeNull()
    await user.type(box, ` ${'AB'.repeat(16)} {Enter}`)
    await waitFor(() => expect(router.state.location.pathname).toBe(`/traces/${'ab'.repeat(16)}`))
  })

  it('pasting a valid trace id jumps straight to it', async () => {
    stubApi(routes())
    const { router } = renderApp('/traces')
    const box = await screen.findByLabelText('Trace id')
    fireEvent.paste(box, { clipboardData: { getData: () => 'cd'.repeat(16) } })
    await waitFor(() => expect(router.state.location.pathname).toBe(`/traces/${'cd'.repeat(16)}`))
  })

  it('empty results offer to clear the filters', async () => {
    const user = userEvent.setup()
    stubApi(routes({ '/traces/search': { body: [] } }))
    const { router } = renderApp('/traces?service=email&errors=1')
    expect(await screen.findByText('No traces match')).toBeInTheDocument()
    const clear = screen.getAllByRole('button', { name: 'Clear filters' })
    await user.click(clear.at(-1)!)
    await waitFor(() => expect(router.state.location.search).toEqual({}))
  })

  it('with only the endpoint service matched, an empty result offers to match it anywhere', async () => {
    const user = userEvent.setup()
    stubApi(routes({ '/traces/search': { body: [] } }))
    const { router } = renderApp('/traces?service=payment')
    expect(await screen.findByText(/No trace in the last 1h starts at payment with these filters/)).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Match payment anywhere in the trace' }))
    await waitFor(() => expect(router.state.location.search).toMatchObject({ service: 'payment', touched: true }))
  })

  it('a failed search shows the error with a retry', async () => {
    stubApi(routes({ '/traces/search': { status: 400, body: { error: 'min_ms must not exceed max_ms' } } }))
    renderApp('/traces')
    const alert = await screen.findByRole('alert')
    expect(alert).toHaveTextContent('Could not load traces.')
    expect(alert).toHaveTextContent('400 · min_ms must not exceed max_ms')
    expect(within(alert).getByRole('button', { name: 'Try again' })).toBeInTheDocument()
  })
})
