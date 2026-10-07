/** Pipeline health: status strip (with a job down), series wiring, empty history, lag and errors. */
import { screen, waitFor, within } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import lag from '../api/__fixtures__/pipeline-lag.json'
import { clearOutage } from '../app/apiStatus'
import { LIVE_INTERVAL_MS } from '../app/live'
import { formatUntil } from '../app/search'
import { ALERTS_LAG_SCALE_FLOOR, ALERTS_TOPIC, LAG_SCALE_FLOOR, lagScales } from '../features/pipeline/LagList'
import { renderApp } from '../test/renderApp'

// ECharts needs a canvas; jsdom has none. The chart is covered by the screenshots.
vi.mock('../components/charts/EChartImpl', () => ({ default: () => <div data-testid="echart" /> }))

const now = Date.now()
const minute = (ago: number) => Math.floor((now - ago) / 60_000) * 60_000
const series = (points: [number, number | null][]) => ({ metric: 'm', kind: 'rate', bucket_secs: 60, points })

interface Stub {
  up?: Record<string, [number, number][]>
  lag?: { status?: number; body: unknown }
  /** Points for every non-`up` series. */
  metrics?: [number, number | null][]
  /** Requests for which this returns true fail with a 500. */
  fail?: (url: URL) => boolean
}

function stub(opts: Stub) {
  const calls: URL[] = []
  const fetch = vi.fn(async (input: string) => {
    const url = new URL(input, 'http://test')
    calls.push(url)
    if (opts.fail?.(url)) return new Response(JSON.stringify({ error: 'boom' }), { status: 500, headers: { 'content-type': 'application/json' } })
    const json = (body: unknown, status = 200) =>
      new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } })
    if (url.pathname.endsWith('/pipeline/lag')) return json(opts.lag?.body ?? lag, opts.lag?.status)
    if (url.pathname.endsWith('/pipeline/series')) {
      const q = url.searchParams
      if (q.get('metric') === 'up') return json(series(opts.up?.[q.get('job') ?? ''] ?? []))
      return json(series(opts.metrics ?? []))
    }
    return json({ error: 'not found' }, 404)
  })
  vi.stubGlobal('fetch', fetch)
  return calls
}

afterEach(() => {
  vi.unstubAllGlobals()
  clearOutage()
})

const healthy = (v: number): [number, number][] => [[minute(120_000), v], [minute(60_000), v], [minute(0), v]]

describe('status strip', () => {
  it('shows a chip per job and flags the one that is down', async () => {
    stub({
      up: {
        'tayga-ingest': healthy(1),
        'tayga-writer': healthy(1),
        'tayga-assembler': [[minute(120_000), 1], [minute(60_000), 1], [minute(0), 0]],
        'tayga-logminer': healthy(1),
        'tayga-notifier': healthy(1),
        'tayga-api': healthy(1),
      },
      metrics: [[minute(120_000), 1], [minute(60_000), 3]],
    })
    renderApp('/pipeline')
    const strip = await screen.findByRole('list', { name: 'Job status' })
    const chips = within(strip).getAllByRole('listitem')
    expect(chips.map((c) => c.getAttribute('data-state'))).toEqual(['up', 'up', 'down', 'up', 'up', 'up'])
    const down = chips[2] as HTMLElement
    expect(down).toHaveTextContent('assembler')
    expect(down).toHaveTextContent('down')
    expect(within(down).getByText('down').className).toMatch(/shadow-glow-err/)
    expect(within(chips[0] as HTMLElement).getByText('up').className).not.toMatch(/glow/)
    expect(chips[0]).toHaveTextContent('scraped within the last minute')
  })

  it('treats a job with no recent sample as down', async () => {
    stub({
      up: { 'tayga-ingest': [[minute(30 * 60_000), 1]], 'tayga-writer': healthy(1), 'tayga-assembler': healthy(1), 'tayga-logminer': healthy(1), 'tayga-notifier': healthy(1), 'tayga-api': healthy(1) },
      metrics: [[minute(120_000), 1], [minute(60_000), 2]],
    })
    renderApp('/pipeline')
    const strip = await screen.findByRole('list', { name: 'Job status' })
    await waitFor(() => expect(within(strip).getAllByRole('listitem')[0]).toHaveAttribute('data-state', 'down'))
  })
})

describe('charts', () => {
  it('requests each series with the recorded metric names, kinds and k=v labels', async () => {
    const calls = stub({ up: { 'tayga-ingest': healthy(1) }, metrics: [[minute(120_000), 1], [minute(60_000), 3]] })
    renderApp('/pipeline?since=24h')
    await waitFor(() => expect(screen.getAllByTestId('echart').length).toBe(12))
    const q = calls.filter((u) => u.pathname.endsWith('/pipeline/series')).map((u) => Object.fromEntries(u.searchParams))
    expect(q).toContainEqual({ since: '24h', metric: 'tayga_ingest_records_published_total', kind: 'rate', job: 'tayga-ingest', labels: 'kind=traces' })
    expect(q).toContainEqual({ since: '24h', metric: 'tayga_writer_batch_seconds', kind: 'q99', job: 'tayga-writer' })
    expect(q).toContainEqual({ since: '24h', metric: 'tayga_assembler_buffered_bytes', kind: 'gauge', job: 'tayga-assembler' })
    // Status chips use their own 15 min window whatever the page range.
    expect(q).toContainEqual({ since: '15m', metric: 'up', kind: 'gauge', job: 'tayga-api' })
    for (const title of ['Ingest records', 'Writer rows', 'Assembler output', 'Logminer throughput', 'Open traces', 'Buffered bytes', 'Writer batch latency', 'Logminer data lag', 'Commit failures', 'Errors', 'Logminer fingerprint cache', 'Logminer batch time']) {
      expect(screen.getByRole('heading', { name: title })).toBeInTheDocument()
    }
  })

  it('in a custom range the charts follow it and stop refreshing; status and lag stay live', async () => {
    const until = formatUntil(minute(3_600_000))
    const calls = stub({ up: { 'tayga-ingest': healthy(1) }, metrics: [[minute(7_000_000), 1]] })
    const { queryClient } = renderApp(`/pipeline?since=2h&until=${until}`)
    await waitFor(() => expect(screen.getAllByTestId('echart').length).toBe(12))
    const q = calls.filter((u) => u.pathname.endsWith('/pipeline/series')).map((u) => Object.fromEntries(u.searchParams))
    expect(q).toContainEqual({ since: '2h', until, metric: 'tayga_assembler_buffered_bytes', kind: 'gauge', job: 'tayga-assembler' })
    expect(q).toContainEqual({ since: '15m', metric: 'up', kind: 'gauge', job: 'tayga-api' })
    const interval = (match: (p: Record<string, unknown>) => boolean) =>
      queryClient
        .getQueryCache()
        .findAll({ queryKey: ['pipeline-series'] })
        .filter((c) => match(c.queryKey[2] as Record<string, unknown>))
        .map((c) => c.observers[0]?.options.refetchInterval)
    expect(new Set(interval((p) => p.metric !== 'up'))).toEqual(new Set([false]))
    expect(new Set(interval((p) => p.metric === 'up'))).toEqual(new Set([LIVE_INTERVAL_MS]))
    const lagQuery = queryClient.getQueryCache().findAll({ queryKey: ['pipeline-lag'] })[0]
    expect(lagQuery?.observers[0]?.options.refetchInterval).toBe(LIVE_INTERVAL_MS)
  })

  it('says a chart is collecting when its series has no points yet', async () => {
    stub({ up: { 'tayga-ingest': healthy(1) }, metrics: [] })
    renderApp('/pipeline')
    await waitFor(() => expect(screen.getAllByText('Collecting… first points in 15 s').length).toBe(12))
    expect(screen.queryByTestId('echart')).toBeNull()
  })
})

describe('failures inside charts', () => {
  const metrics: [number, number | null][] = [[minute(120_000), 1], [minute(60_000), 3]]

  it('names a series that failed to load instead of letting it vanish', async () => {
    stub({ up: { 'tayga-ingest': healthy(1) }, metrics, fail: (u) => u.searchParams.get('metric') === 'tayga_writer_insert_failures_total' })
    renderApp('/pipeline')
    const note = await screen.findByText(/Could not load insert failures; it is missing from this chart, not zero\./)
    expect(note.closest('[role="alert"]')).toBeInTheDocument()
    // The other lines still draw.
    await waitFor(() => expect(screen.getAllByTestId('echart').length).toBe(12))
  })

  it('keeps old chart data and notes the failed refresh', async () => {
    let down = false
    stub({ up: { 'tayga-ingest': healthy(1) }, metrics, fail: (u) => down && u.searchParams.get('metric') === 'tayga_writer_rows_inserted_total' })
    const { queryClient } = renderApp('/pipeline')
    await waitFor(() => expect(screen.getAllByTestId('echart').length).toBe(12))
    down = true
    await queryClient.refetchQueries({ queryKey: ['pipeline-series'] })
    expect(await screen.findByText(/Refresh failed · showing data from/)).toBeInTheDocument()
    expect(screen.getAllByTestId('echart').length).toBe(12)
  })

  it('treats a series of only gaps as no data', async () => {
    stub({ up: { 'tayga-ingest': healthy(1) }, metrics: [[minute(120_000), null], [minute(60_000), null]] })
    renderApp('/pipeline')
    await waitFor(() => expect(screen.getAllByText('Collecting… first points in 15 s').length).toBe(12))
    expect(screen.queryByTestId('echart')).toBeNull()
  })
})

describe('empty history', () => {
  it('shows one collecting state and no charts before the recorder has written anything', async () => {
    stub({})
    renderApp('/pipeline')
    expect(await screen.findByText('Collecting… first points in 15 s')).toBeInTheDocument()
    expect(screen.getAllByText('Collecting… first points in 15 s')).toHaveLength(1)
    expect(screen.queryByRole('heading', { name: 'Ingest records' })).toBeNull()
    // Lag is live from Kafka, so it does not wait for history.
    expect(await screen.findByRole('list', { name: 'Consumer lag' })).toBeInTheDocument()
  })
})

describe('consumer lag', () => {
  it('lists each group with its lag', async () => {
    stub({ up: { 'tayga-ingest': healthy(1) }, metrics: [[minute(120_000), 1], [minute(60_000), 2]] })
    renderApp('/pipeline')
    const list = await screen.findByRole('list', { name: 'Consumer lag' })
    const rows = within(list).getAllByRole('listitem')
    expect(rows).toHaveLength(4)
    expect(rows[1]).toHaveTextContent('tayga-assembler')
    expect(rows[1]).toHaveTextContent('1.2k')
    expect(rows[1]).toHaveTextContent('committed 17,327,899 · end 17,329,052')
    expect(rows[1]).toHaveTextContent('tayga.signals · committed')
    expect(rows[3]).toHaveTextContent('tayga-notifier')
    expect(rows[3]).toHaveTextContent('tayga.alerts · committed 2,487 · end 2,487')
  })

  it('scales bars to at least 1000 messages and keeps an empty track at zero lag', async () => {
    const sig = 'tayga.signals'
    stub({
      up: { 'tayga-ingest': healthy(1) },
      metrics: [[minute(120_000), 1], [minute(60_000), 2]],
      lag: {
        body: [
          { group: 'a', topic: sig, committed: 1, end: 21, lag: 20 },
          { group: 'b', topic: sig, committed: 5, end: 5, lag: 0 },
          { group: 'c', topic: sig, committed: 0, end: 500, lag: 500 },
        ],
      },
    })
    renderApp('/pipeline')
    const rows = within(await screen.findByRole('list', { name: 'Consumer lag' })).getAllByRole('listitem')
    const width = (i: number) => (rows[i]?.querySelector('.h-full') as HTMLElement).style.width
    expect([width(0), width(1), width(2)]).toEqual(['2%', '0%', '50%'])
  })

  it('scales each topic on its own, so a stuck notifier shows next to a large signal lag', async () => {
    stub({
      up: { 'tayga-ingest': healthy(1) },
      metrics: [[minute(120_000), 1], [minute(60_000), 2]],
      lag: {
        body: [
          { group: 'tayga-writer', topic: 'tayga.signals', committed: 0, end: 40_000, lag: 40_000 },
          { group: 'tayga-assembler', topic: 'tayga.signals', committed: 30_000, end: 40_000, lag: 10_000 },
          { group: 'tayga-notifier', topic: 'tayga.alerts', committed: 100, end: 130, lag: 30 },
        ],
      },
    })
    renderApp('/pipeline')
    const rows = within(await screen.findByRole('list', { name: 'Consumer lag' })).getAllByRole('listitem')
    const width = (i: number) => (rows[i]?.querySelector('.h-full') as HTMLElement).style.width
    // 30 alerts against 40,000 signal messages would be 0.075%; on its own topic it fills the bar.
    expect([width(0), width(1), width(2)]).toEqual(['100%', '25%', '100%'])
    expect(rows[2]).toHaveTextContent('tayga.alerts')
    expect(rows[0]).toHaveTextContent('tayga.signals')
  })

  it('floors the alerts scale at 10 alerts, the signals scale at 1000 messages', () => {
    const lag = (group: string, topic: string, n: number) => ({ group, topic, committed: 0, end: n, lag: n })
    const scales = lagScales([lag('w', 'tayga.signals', 20), lag('n', ALERTS_TOPIC, 3)])
    expect(scales.get('tayga.signals')).toBe(LAG_SCALE_FLOOR)
    expect(scales.get(ALERTS_TOPIC)).toBe(ALERTS_LAG_SCALE_FLOOR)
    expect(lagScales([lag('n', ALERTS_TOPIC, 42), lag('m', ALERTS_TOPIC, 7)]).get(ALERTS_TOPIC)).toBe(42)
  })

  it('keeps the last lag and says so when a refresh fails', async () => {
    let down = false
    stub({ up: { 'tayga-ingest': healthy(1) }, metrics: [[minute(120_000), 1], [minute(60_000), 2]], fail: (u) => down && u.pathname.endsWith('/pipeline/lag') })
    const { queryClient } = renderApp('/pipeline')
    await screen.findByRole('list', { name: 'Consumer lag' })
    down = true
    await queryClient.refetchQueries({ queryKey: ['pipeline-lag'] })
    const note = await screen.findByText(/Refresh failed · showing data from \d\d:\d\d:\d\d/)
    expect(note.closest('[role="alert"]')).toHaveTextContent('Try again')
    expect(screen.getByRole('list', { name: 'Consumer lag' })).toBeInTheDocument()
    expect(screen.queryByText('Could not load consumer lag.')).toBeNull()
  })

  it('a Kafka outage fails only the lag section', async () => {
    stub({ up: { 'tayga-ingest': healthy(1) }, metrics: [[minute(120_000), 1], [minute(60_000), 2]], lag: { status: 503, body: { error: 'kafka unavailable' } } })
    renderApp('/pipeline')
    const alert = await screen.findByRole('alert')
    expect(alert).toHaveTextContent('Could not load consumer lag.')
    // Kafka being down is not a storage outage.
    expect(screen.queryByText('Storage unavailable.')).toBeNull()
    expect(await screen.findByRole('list', { name: 'Job status' })).toBeInTheDocument()
    await waitFor(() => expect(screen.getAllByTestId('echart').length).toBeGreaterThan(0))
  })
})
