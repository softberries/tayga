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
const searchCalls = (fetch: ReturnType<typeof stubApi>) =>
  fetch.mock.calls.map((c) => String(c[0])).filter((u) => u.startsWith('/api/v1/traces/search'))

describe('traces explorer', () => {
  it('plots and lists every result, newest first, with links to traces and stories', async () => {
    const fetch = stubApi(routes())
    renderApp('/traces')
    expect(await bodyRows()).toHaveLength(rows.length)
    expect(searchCalls(fetch)[0]).toBe('/api/v1/traces/search?since=1h&limit=500')
    const withStory = rows.find((r) => r.story_id)!
    expect(screen.getByRole('link', { name: `Story of trace ${withStory.trace_id}` })).toHaveAttribute('href', `/stories/${withStory.story_id}`)
    const first = (await bodyRows())[0] as HTMLElement
    expect(first).toHaveAttribute('data-trace-id', rows[0]!.trace_id)
    expect(within(first).getByRole('link')).toHaveAttribute('href', `/traces/${rows[0]!.trace_id}`)
    expect(screen.getByText(`${rows.length} traces`)).toBeInTheDocument()
    // The chart's text summary.
    expect(screen.getByText(new RegExp(`^Duration over time of ${rows.length} traces in the last 1h: 1 errors`))).toBeInTheDocument()
  })

  it('filters build the URL and the API query string', async () => {
    const user = userEvent.setup()
    const fetch = stubApi(routes())
    const { router } = renderApp('/traces?since=15m')
    await bodyRows()

    await user.click(screen.getByRole('button', { name: 'Service: any' }))
    await user.type(await screen.findByPlaceholderText('Search service'), 'paym')
    await user.click(await screen.findByRole('option', { name: 'payment' }))
    await waitFor(() => expect(router.state.location.search).toMatchObject({ service: 'payment' }))

    await user.click(screen.getByRole('button', { name: 'Errors only' }))
    await user.type(screen.getByLabelText('Min duration (ms)'), '5')
    await user.type(screen.getByLabelText('Max duration (ms)'), '250{Enter}')
    await waitFor(() =>
      expect(router.state.location.search).toMatchObject({ service: 'payment', errors: true, min_ms: 5, max_ms: 250 }),
    )
    await waitFor(() =>
      expect(searchCalls(fetch).at(-1)).toBe('/api/v1/traces/search?since=15m&service=payment&min_ms=5&max_ms=250&errors=true&limit=500'),
    )
    expect(screen.getByRole('button', { name: 'Errors only' })).toHaveAttribute('aria-pressed', 'true')

    await user.click(screen.getByRole('button', { name: 'Clear service filter' }))
    await waitFor(() => expect(router.state.location.search).not.toHaveProperty('service'))
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

  it('a failed search shows the error with a retry', async () => {
    stubApi(routes({ '/traces/search': { status: 400, body: { error: 'min_ms must not exceed max_ms' } } }))
    renderApp('/traces')
    const alert = await screen.findByRole('alert')
    expect(alert).toHaveTextContent('Could not load traces.')
    expect(alert).toHaveTextContent('400 · min_ms must not exceed max_ms')
    expect(within(alert).getByRole('button', { name: 'Try again' })).toBeInTheDocument()
  })
})
