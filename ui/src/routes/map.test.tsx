/**
 * `/map` against fixtures (ELK on the main thread, as jsdom has no Worker): health on the
 * nodes, failing edges, the drawer opened from the URL and from a node, search, the Grafana
 * link, and the empty and error states.
 */
import { act, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import logAlerts from '../api/__fixtures__/log-alerts.json'
import logTemplates from '../api/__fixtures__/log-templates.json'
import service from '../api/__fixtures__/service.json'
import serviceMap from '../api/__fixtures__/service-map.json'
import groups from '../api/__fixtures__/story-groups.json'
import type { ServiceMapView } from '../api/types'
import { clearOutage } from '../app/apiStatus'
import { logSignals } from '../features/map/ServiceDrawer'
import { renderApp, stubApi } from '../test/renderApp'
import type { Routes } from '../test/renderApp'

vi.mock('../components/charts/EChartImpl', () => ({ default: () => <div data-testid="echart" /> }))

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
    '/config': { body: { jaeger_url: null, grafana_url: null } },
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
    expect(screen.getAllByRole('button', { name: /Open details\.$/ })).toHaveLength(19)
    expect(screen.getByText('19 services · 3 degraded · 4 failing calls')).toBeInTheDocument()
    expect(screen.getByText(/^Service map: 17 services; degraded: checkout \(error\), payment \(error\), shipping \(slow\)/)).toBeInTheDocument()
    expect(screen.getByRole('list', { name: 'Legend' })).toHaveTextContent('failing calls')
    await waitFor(() => expect(container.querySelectorAll('.react-flow__edge').length).toBeGreaterThan(0))
    expect(container.querySelectorAll('[data-tone="err"] path.tg-flow')).toHaveLength(4)
    // No Grafana URL configured: no link.
    expect(screen.queryByRole('link', { name: /Open in Grafana/ })).toBeNull()
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
    expect(within(drawer).getByRole('link', { name: /Open payment traces/ })).toHaveAttribute('href', '/traces?service=payment')
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
    stubApi(routes({ '/config': { body: { jaeger_url: null, grafana_url: 'http://localhost:3001/' } } }))
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
