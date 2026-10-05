/**
 * `/map` against fixtures (ELK on the main thread, as jsdom has no Worker): health on the
 * nodes, failing edges, the drawer opened from the URL and from a node, search, the Grafana
 * link, and the empty and error states.
 */
import { act, configure, fireEvent, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import logAlerts from '../api/__fixtures__/log-alerts.json'
import logTemplates from '../api/__fixtures__/log-templates.json'
import service from '../api/__fixtures__/service.json'
import serviceMap from '../api/__fixtures__/service-map.json'
import groups from '../api/__fixtures__/story-groups.json'
import type { ServiceMapView } from '../api/types'
import { clearOutage } from '../app/apiStatus'
import { logSignals } from '../features/map/ServiceDrawer'
import { layoutGraph } from '../features/map/layout'
import { setReducedMotion } from '../test/setup'
import { renderApp, stubApi } from '../test/renderApp'
import type { Routes } from '../test/renderApp'

vi.mock('../components/charts/EChartImpl', () => ({ default: () => <div data-testid="echart" /> }))
// The real layout, counted.
vi.mock('../features/map/layout', async (importOriginal) => {
  const real = await importOriginal<typeof import('../features/map/layout')>()
  return { ...real, layoutGraph: vi.fn(real.layoutGraph) }
})

/** The fixture with payment and checkout failing and shipping slow, as in the demo. */
function degraded(): ServiceMapView {
  const m = structuredClone(serviceMap) as ServiceMapView
  for (const n of m.nodes) {
    if (n.service === 'payment') Object.assign(n, { error_ratio: 0.38, health: 'error' })
    if (n.service === 'checkout') Object.assign(n, { error_ratio: 0.12, health: 'error' })
    if (n.service === 'shipping') Object.assign(n, { p99_ns: 5.1e9, health: 'slow' })
  }
  for (const e of m.edges) {
    if (e.parent === 'checkout' && e.child === 'payment') Object.assign(e, { errors: 64, error_rate: 0.38 })
  }
  return m
}

const paymentGroup = { ...groups[0], fingerprint: '42', kind: 'error', rc_service: 'payment', summary: 'payment charge failed: Invalid token', stories: 31 }

function routes(extra: Routes = {}): Routes {
  return {
    '/service-map': { body: degraded() },
    '/config': { body: { jaeger_url: null, grafana_url: null, auth_enabled: false, infra_services: ['flagd'] } },
    '/services/payment': { body: { ...service, service: 'payment' } },
    '/story-groups': { body: [paymentGroup] },
    '/log-alerts': { body: [{ ...logAlerts[0], service: 'payment', template: 'Payment request failed. Invalid token.' }] },
    '/log-templates': { body: logTemplates.slice(0, 2).map((t) => ({ ...t, service: 'payment' })) },
    ...extra,
  }
}

const node = (name: string) => screen.findByRole('button', { name: new RegExp(`^${name}[,:]`) })

/** jsdom never measures: report each observed element at once, so React Flow draws edges. */
class MeasuringResizeObserver {
  private cb: ResizeObserverCallback
  constructor(cb: ResizeObserverCallback) {
    this.cb = cb
  }
  observe(target: Element) {
    const contentRect = { width: target instanceof HTMLElement ? target.offsetWidth : 0, height: 600 }
    this.cb([{ target, contentRect } as unknown as ResizeObserverEntry], this as unknown as ResizeObserver)
  }
  unobserve() {}
  disconnect() {}
}

/** React Flow reads the zoom from the viewport's CSS transform; jsdom lacks DOMMatrix. */
class DOMMatrixStub {
  m22: number
  constructor(transform?: string) {
    const scale = /scale\(([\d.]+)\)/.exec(transform ?? '')?.[1]
    this.m22 = scale ? Number(scale) : 1
  }
}

beforeEach(() => {
  vi.stubGlobal('ResizeObserver', MeasuringResizeObserver)
  vi.stubGlobal('DOMMatrixReadOnly', DOMMatrixStub)
  // A 1000×600 canvas, so React Flow can fit the view.
  vi.spyOn(HTMLElement.prototype, 'offsetWidth', 'get').mockReturnValue(1000)
  vi.spyOn(HTMLElement.prototype, 'offsetHeight', 'get').mockReturnValue(600)
})

afterEach(() => {
  vi.unstubAllGlobals()
  clearOutage()
})

/**
 * The first test used to pay for the cold start inside its `findBy` wait: the lazy route chunk
 * (React Flow, the drawer and their transforms, ~0.7 s alone), ELK's 1.4 MB bundle and its first
 * layout (~60 ms; ~15 ms warm). That fits in the default 1 s on an idle machine and not while
 * other test files compete for the CPU. Load the modules up front (own timeout), and give waits
 * on this heavy page 5 s: a load spike then slows a test instead of failing it.
 */
beforeAll(async () => {
  await import('./map')
  await layoutGraph({ services: ['a', 'b'], links: [['a', 'b']] })
  configure({ asyncUtilTimeout: 5000 })
}, 30_000)

afterAll(() => configure({ asyncUtilTimeout: 1000 }))

describe('service map', () => {
  it('renders every service with its health, and failing calls as flowing edges', async () => {
    stubApi(routes())
    const { container } = renderApp('/map')
    const payment = await node('payment')
    expect(payment).toHaveAttribute('data-health', 'error')
    expect(payment).toHaveAccessibleName(/^payment, errors: .* 38 % errors, p99 /)
    expect(await node('shipping')).toHaveAttribute('data-health', 'slow')
    expect(await node('cart')).toHaveAttribute('data-health', 'ok')
    // Callers without server spans of their own are still on the map.
    expect(await node('load-generator')).toHaveAccessibleName(/no server spans/)
    expect(screen.getAllByRole('button', { name: /Open details\.$/ })).toHaveLength(18)
    // flagd is infrastructure: hidden by default.
    expect(screen.getByText('18 services · 3 degraded · 1 failing call · 1 infra hidden')).toBeInTheDocument()
    expect(screen.getByText(/^Service map: 18 services; degraded: checkout \(error\), payment \(error\), shipping \(slow\); callers without spans of their own: frontend-web, load-generator;/)).toBeInTheDocument()
    expect(screen.getByRole('list', { name: 'Legend' })).toHaveTextContent('failing calls')
    await waitFor(() => expect(container.querySelectorAll('.react-flow__edge').length).toBeGreaterThan(0))
    expect(container.querySelectorAll('[data-tone="err"] path.tg-flow')).toHaveLength(1)
    // No Grafana URL configured: no link.
    expect(screen.queryByRole('link', { name: /Open in Grafana/ })).toBeNull()
  })

  it('hides infrastructure by default; callers carry a badge, red when a call into it fails', async () => {
    stubApi(routes())
    renderApp('/map')
    await node('payment')
    expect(screen.queryByRole('button', { name: /^flagd[,:]/ })).toBeNull()
    // cart -> flagd is healthy: a slow-toned badge. ad -> flagd is failing: red.
    const cart = await node('cart')
    expect(cart.querySelector('[data-infra-badge]')).toHaveTextContent('+1')
    expect(cart.querySelector('[data-infra-badge]')).toHaveAttribute('data-infra-badge', 'slow')
    expect(cart).toHaveAccessibleName(/Calls hidden infrastructure: flagd\. Open details\.$/)
    const ad = await node('ad')
    expect(ad.querySelector('[data-infra-badge]')).toHaveAttribute('data-infra-badge', 'err')
    expect(ad).toHaveAccessibleName(/Calls hidden infrastructure: flagd, failing\./)
    // No hidden callee, no badge.
    expect((await node('currency')).querySelector('[data-infra-badge]')).toBeNull()
    expect(document.querySelectorAll('.react-flow__node')).toHaveLength(18)
  })

  it('a caller whose only calls go to hidden infra stays on the map with its badge', async () => {
    const m = degraded()
    m.edges.push({ parent: 'batch', child: 'flagd', calls: 60, errors: 0, error_rate: 0, avg_duration_ns: 1 })
    stubApi(routes({ '/service-map': { body: m } }))
    const { router } = renderApp('/map')
    const batch = await node('batch')
    expect(batch).toHaveAccessibleName(/no server spans/)
    expect(batch.querySelector('[data-infra-badge]')).toHaveTextContent('+1')
    expect(screen.getAllByRole('button', { name: /Open details\.$/ })).toHaveLength(19)
    expect(screen.getByText('19 services · 3 degraded · 1 failing call · 1 infra hidden')).toBeInTheDocument()
    expect(screen.getByText(/callers without spans of their own: batch, frontend-web, load-generator;/)).toBeInTheDocument()
    // With infrastructure shown it is an ordinary caller of flagd.
    await userEvent.setup().click(screen.getByRole('switch', { name: 'Show infrastructure' }))
    await waitFor(() => expect(router.state.location.search).toMatchObject({ infra: true }))
    expect(await node('flagd')).toBeInTheDocument()
    expect(screen.getAllByRole('button', { name: /Open details\.$/ })).toHaveLength(20)
  })

  it('the badge tooltip opens with the card keyboard focus and lists the hidden callees', async () => {
    stubApi(routes())
    renderApp('/map')
    const ad = await node('ad')
    expect(screen.queryByText(/flagd · /)).toBeNull()
    // Keyboard modality, as Tab sets it (a test that used the pointer before would otherwise
    // focus without :focus-visible).
    fireEvent.keyDown(document.body, { key: 'Tab' })
    act(() => ad.focus())
    // 5 calls in the 1h window.
    const tip = await screen.findByRole('tooltip')
    expect(tip).toHaveTextContent('flagd · 0.1/min · 100 % err')
    act(() => ad.blur())
    await waitFor(() => expect(screen.queryByRole('tooltip')).toBeNull())
  })

  it('a search that only finds hidden infrastructure offers to show it instead of "No match"', async () => {
    const user = userEvent.setup()
    stubApi(routes())
    const { router } = renderApp('/map')
    await node('payment')
    await user.type(screen.getByRole('searchbox', { name: 'Find a service' }), 'flag')
    const status = document.getElementById('map-search-status')!
    await waitFor(() => expect(status).toHaveTextContent('flagd is hidden · Show infrastructure'))
    expect(status).not.toHaveClass('text-err')
    expect(screen.queryByText('No match')).toBeNull()
    await user.click(within(status).getByRole('button', { name: 'Show infrastructure' }))
    await waitFor(() => expect(router.state.location.search).toMatchObject({ infra: true, q: 'flag' }))
    expect(await node('flagd')).toBeInTheDocument()
    await waitFor(() => expect(status).toHaveTextContent('1 match · Enter to zoom'))
  })

  it('"Show infrastructure" draws flagd and its calls, and is kept in the URL', async () => {
    const user = userEvent.setup()
    const lay = vi.mocked(layoutGraph)
    lay.mockClear()
    stubApi(routes())
    const { router } = renderApp('/map')
    await node('payment')
    expect(lay).toHaveBeenCalledTimes(1)
    const toggle = screen.getByRole('switch', { name: 'Show infrastructure' })
    expect(toggle).not.toBeChecked()
    await user.click(toggle)
    await waitFor(() => expect(router.state.location.search).toMatchObject({ infra: true }))
    expect(router.state.location.href).toContain('infra=true')
    expect(await node('flagd')).toBeInTheDocument()
    // The topology changed, so ELK ran again.
    expect(lay).toHaveBeenCalledTimes(2)
    expect(screen.getAllByRole('button', { name: /Open details\.$/ })).toHaveLength(19)
    expect(screen.getByText('19 services · 3 degraded · 4 failing calls')).toBeInTheDocument()
    expect(document.querySelector('[data-infra-badge]')).toBeNull()
    await user.click(screen.getByRole('switch', { name: 'Show infrastructure' }))
    await waitFor(() => expect(router.state.location.search).not.toHaveProperty('infra'))
    await waitFor(() => expect(screen.queryByRole('button', { name: /^flagd[,:]/ })).toBeNull())
  })

  it('/map?service=flagd opens the flagd drawer with infra hidden, and keeps its card', async () => {
    stubApi(routes({ '/services/flagd': { body: { ...service, service: 'flagd' } } }))
    renderApp('/map?service=flagd')
    const drawer = await screen.findByRole('dialog', { name: 'flagd' })
    expect(drawer).toBeInTheDocument()
    expect(await node('flagd')).toHaveAttribute('aria-current', 'true')
    expect(screen.getByRole('switch', { name: 'Show infrastructure' })).not.toBeChecked()
  })

  it('a degraded infra service still counts in the header badge', async () => {
    const m = degraded()
    for (const n of m.nodes) if (n.service === 'flagd') Object.assign(n, { health: 'error' })
    stubApi(routes({ '/service-map': { body: m } }))
    renderApp('/map')
    await node('payment')
    // The map's own count leaves flagd out; the header counts it.
    expect(screen.getByText('18 services · 3 degraded · 1 failing call · 1 infra hidden')).toBeInTheDocument()
    expect(await screen.findByText('4 services degraded')).toBeInTheDocument()
  })

  it('without infra_services in /config (an older API) flagd is hidden', async () => {
    stubApi(routes({ '/config': { body: { jaeger_url: null, grafana_url: null, auth_enabled: false } } }))
    renderApp('/map')
    await node('payment')
    expect(screen.queryByRole('button', { name: /^flagd[,:]/ })).toBeNull()
  })

  it('hides whatever the API lists, and hides the toggle when the list is empty', async () => {
    stubApi(routes({ '/config': { body: { jaeger_url: null, grafana_url: null, auth_enabled: false, infra_services: ['cart'] } } }))
    const { unmount } = renderApp('/map')
    await node('payment')
    expect(screen.queryByRole('button', { name: /^cart[,:]/ })).toBeNull()
    expect(await node('flagd')).toBeInTheDocument()
    unmount()
    stubApi(routes({ '/config': { body: { jaeger_url: null, grafana_url: null, auth_enabled: false, infra_services: [] } } }))
    renderApp('/map')
    expect(await node('cart')).toBeInTheDocument()
    expect(screen.queryByRole('switch', { name: 'Show infrastructure' })).toBeNull()
  })

  it('opens the drawer from ?service= with RED charts, stories, log signals and callers', async () => {
    stubApi(routes())
    renderApp('/map?service=payment')
    const drawer = await screen.findByRole('dialog', { name: 'payment' })
    expect(await within(drawer).findByText('Failing: 38 % of calls failed')).toBeInTheDocument()
    const red = await within(drawer).findByRole('region', { name: 'RED · last 1h' })
    expect(within(red).getAllByRole('figure')).toHaveLength(3)
    expect(within(red).getByText('38 %')).toBeInTheDocument()
    const stories = within(drawer).getByRole('region', { name: 'Stories with root cause here' })
    const story = await within(stories).findByRole('link', { name: /payment charge failed/ })
    // The router quotes number-like strings, as it does for every fingerprint.
    expect(decodeURIComponent(story.getAttribute('href') ?? '')).toBe('/?service=payment&group="42"')
    const logs = within(drawer).getByRole('region', { name: 'Log signals' })
    expect(await within(logs).findByText('Payment request failed. Invalid token.')).toBeInTheDocument()
    const callers = within(drawer).getByRole('region', { name: 'Callers' })
    expect(within(callers).getByRole('link', { name: /checkout →.*38 % err/ })).toHaveAttribute('href', '/map?service=checkout')
    const callees = within(drawer).getByRole('region', { name: 'Callees' })
    expect(within(callees).getByRole('link', { name: /→ flagd/ })).toBeInTheDocument()
    // Any span, not only the endpoint: payment never serves a trace's root.
    expect(within(drawer).getByRole('link', { name: /Open payment traces/ })).toHaveAttribute('href', '/traces?service=payment&touched=true')
  })

  it('a failed refresh keeps the drawer sections and notes it', async () => {
    stubApi(routes())
    const { queryClient } = renderApp('/map?service=payment')
    const drawer = await screen.findByRole('dialog', { name: 'payment' })
    const red = await within(drawer).findByRole('region', { name: 'RED · last 1h' })
    await within(red).findByText('38 %')
    const stories = within(drawer).getByRole('region', { name: 'Stories with root cause here' })
    await within(stories).findByRole('link', { name: /payment charge failed/ })
    const logs = within(drawer).getByRole('region', { name: 'Log signals' })
    await within(logs).findByText('Payment request failed. Invalid token.')
    const down = { status: 503, body: { error: 'clickhouse unavailable' } }
    stubApi(routes({ '/services/payment': down, '/story-groups': down, '/log-alerts': down, '/log-templates': down }))
    await act(() => queryClient.refetchQueries({ type: 'active', predicate: (q) => q.queryKey[0] !== 'service-map' }))
    const note = /^Refresh failed · showing data from \d\d:\d\d:\d\d$/
    expect(await within(red).findByText(note)).toBeInTheDocument()
    expect(within(stories).getByText(note)).toBeInTheDocument()
    expect(within(logs).getByText(note)).toBeInTheDocument()
    expect(within(red).getAllByRole('figure')).toHaveLength(3)
    expect(within(stories).getByRole('link', { name: /payment charge failed/ })).toBeInTheDocument()
    expect(within(logs).getByText('Payment request failed. Invalid token.')).toBeInTheDocument()
    expect(within(drawer).queryByText('Storage is unavailable')).toBeNull()
  })

  it('a service with no spans in the window shows an empty state, not an error', async () => {
    stubApi(routes({ '/services/payment': { status: 404, body: { error: 'not found' } } }))
    renderApp('/map?service=payment&since=15m')
    const drawer = await screen.findByRole('dialog', { name: 'payment' })
    const red = await within(drawer).findByRole('region', { name: 'RED · last 15m' })
    expect(await within(red).findByText('No spans for payment in the last 15m.')).toBeInTheDocument()
    expect(within(red).queryByRole('alert')).toBeNull()
  })

  it('a node opens its drawer from the keyboard and closing returns focus to it', async () => {
    const user = userEvent.setup()
    stubApi(routes({ '/services/checkout': { body: service } }))
    const { router } = renderApp('/map')
    await node('checkout')
    // Keyboard, not a pointer: d3-zoom's mousedown handler needs a real window on the event.
    act(() => screen.getByRole('button', { name: /^checkout,/ }).focus())
    await user.keyboard('{Enter}')
    await waitFor(() => expect(router.state.location.search).toMatchObject({ service: 'checkout' }))
    const drawer = await screen.findByRole('dialog', { name: 'checkout' })
    await user.click(within(drawer).getByRole('button', { name: 'Close panel' }))
    await waitFor(() => expect(router.state.location.search).not.toHaveProperty('service'))
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())
    expect(await node('checkout')).toHaveFocus()
  })

  it('keyboard focus on a card outside the canvas pans it into view', async () => {
    setReducedMotion(true)
    stubApi(routes())
    renderApp('/map')
    const quote = await node('quote')
    const viewport = document.querySelector<HTMLElement>('.react-flow__viewport')!
    await waitFor(() => expect(viewport.style.transform).toMatch(/translate/))
    const before = viewport.style.transform
    const rect = (x: number, y: number, w: number, h: number) =>
      ({ x, y, left: x, top: y, width: w, height: h, right: x + w, bottom: y + h, toJSON: () => ({}) }) as DOMRect
    vi.spyOn(Element.prototype, 'getBoundingClientRect').mockImplementation(function (this: Element) {
      if (this.classList.contains('react-flow')) return rect(0, 0, 1000, 600)
      // The quote card sits past the canvas's right edge.
      if (this === quote) return rect(1400, 200, 190, 76)
      return rect(0, 0, 0, 0)
    })
    act(() => quote.focus())
    await waitFor(() => expect(viewport.style.transform).not.toBe(before))
    // Its centre moved by (500 - 1495, 300 - 238).
    const x = (t: string) => Number(/translate\(([-\d.]+)px/.exec(t)?.[1])
    expect(x(viewport.style.transform) - x(before)).toBeCloseTo(-995, 0)
  })

  it('a refresh with the same topology keeps the layout; a new call relays it out', async () => {
    const lay = vi.mocked(layoutGraph)
    lay.mockClear()
    const fetch = stubApi(routes())
    const { queryClient } = renderApp('/map')
    await node('payment')
    expect(lay).toHaveBeenCalledTimes(1)

    // Same services and calls, new numbers.
    const busier = degraded()
    for (const n of busier.nodes) if (n.service === 'payment') n.error_ratio = 0.5
    stubApi(routes({ '/service-map': { body: busier } }))
    await act(() => queryClient.invalidateQueries({ queryKey: ['service-map'] }))
    await waitFor(() => expect(screen.getByRole('button', { name: /^payment, errors: .* 50 % errors/ })).toBeInTheDocument())
    expect(lay).toHaveBeenCalledTimes(1)

    // A call that was not there before.
    const wider = degraded()
    wider.edges.push({ parent: 'quote', child: 'email', calls: 1, errors: 0, error_rate: 0, avg_duration_ns: 1 })
    stubApi(routes({ '/service-map': { body: wider } }))
    await act(() => queryClient.invalidateQueries({ queryKey: ['service-map'] }))
    await waitFor(() => expect(lay).toHaveBeenCalledTimes(2))
    expect(fetch).toHaveBeenCalled()
  })

  it('search highlights matching services and dims the rest', async () => {
    const user = userEvent.setup()
    stubApi(routes())
    const { router } = renderApp('/map')
    await node('payment')
    await user.type(screen.getByRole('searchbox', { name: 'Find a service' }), 'pay')
    await waitFor(() => expect(router.state.location.search).toMatchObject({ q: 'pay' }))
    expect(screen.getByText('1 match · Enter to zoom')).toBeInTheDocument()
    expect(await node('payment')).toHaveClass('tg-map-node-match')
    expect(await node('cart')).toHaveClass('opacity-35')
    await user.clear(screen.getByRole('searchbox', { name: 'Find a service' }))
    await user.type(screen.getByRole('searchbox', { name: 'Find a service' }), 'zzz')
    expect(await screen.findByText('No match')).toBeInTheDocument()
  })

  it('shows "Open in Grafana" when the API reports a Grafana URL', async () => {
    stubApi(routes({ '/config': { body: { jaeger_url: null, grafana_url: 'http://localhost:3001/', auth_enabled: false, infra_services: ['flagd'] } } }))
    renderApp('/map')
    expect(await screen.findByRole('link', { name: /Open in Grafana/ })).toHaveAttribute(
      'href',
      'http://localhost:3001/d/tayga-service-map',
    )
  })

  it('shows the empty state when no service calls are in the window', async () => {
    stubApi(routes({ '/service-map': { body: { edges: [], nodes: [] } } }))
    renderApp('/map?since=15m')
    expect(await screen.findByText('No service calls in this window')).toBeInTheDocument()
    expect(screen.getByText('Tayga has seen no spans in the last 15m. A longer time range may show older calls.')).toBeInTheDocument()
  })

  it('shows an error with a retry when the map cannot load', async () => {
    stubApi(routes({ '/service-map': { status: 500, body: { error: 'query failed' } } }))
    renderApp('/map')
    const alert = await screen.findByRole('alert')
    expect(alert).toHaveTextContent('query failed')
    expect(within(alert).getByRole('button', { name: 'Try again' })).toBeInTheDocument()
  })
})

describe('logSignals', () => {
  it('keeps one signal per template, active alerts first, then alerting templates', () => {
    const a = logAlerts[0] as (typeof logAlerts)[number]
    const alerts = [
      { ...a, alert_id: '1', template_id: 't1', active: false, last_at_ns: 3 },
      { ...a, alert_id: '2', template_id: 't1', active: false, last_at_ns: 1 },
      { ...a, alert_id: '3', template_id: 't2', active: true, last_at_ns: 2 },
    ] as Parameters<typeof logSignals>[0]
    const t = logTemplates[0] as (typeof logTemplates)[number]
    const templates = [
      { ...t, template_id: 't2', alerting: true },
      { ...t, template_id: 't3', alerting: true },
      { ...t, template_id: 't4', alerting: false },
    ]
    expect(logSignals(alerts, templates).map((s) => [s.templateId, s.kind])).toEqual([
      ['t2', a.kind],
      ['t1', a.kind],
      ['t3', 'alerting'],
    ])
  })
})
