/**
 * Trace and story pages against captured fixtures: loading, 404 and error states, the
 * waterfall opened at the root cause, and the span drawer rendering trace data as text.
 */
import { screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import logAlerts from '../api/__fixtures__/log-alerts.json'
import serviceMap from '../api/__fixtures__/service-map.json'
import storyGroup from '../api/__fixtures__/story-group.json'
import story from '../api/__fixtures__/story-error-payment.json'
import traceTemplates from '../api/__fixtures__/trace-log-templates.json'
import trace from '../api/__fixtures__/trace-error-payment.json'
import type { TraceView } from '../api/types'
import { clearOutage } from '../app/apiStatus'
import { renderApp, stubApi } from '../test/renderApp'
import type { Routes } from '../test/renderApp'

// ECharts needs a canvas; jsdom has none. The chart is covered by the screenshots.
vi.mock('../components/charts/EChartImpl', () => ({ default: () => <div data-testid="echart" /> }))

const ID = story.story_id
const RC = story.root_cause.span_id
const XSS = '<script>alert(1)</script><img src=x onerror=alert(2)>'

/** The fixture trace with hostile text in the root-cause span's attributes and logs. */
const hostile: TraceView = {
  ...(trace as TraceView),
  spans: trace.spans.map((s) =>
    s.span_id === RC ? { ...s, attrs: [...s.attrs, ['evil.attr', XSS] as [string, string]] } : s,
  ) as TraceView['spans'],
}

function routes(extra: Routes = {}): Routes {
  return {
    '/service-map': { body: serviceMap },
    '/config': { body: { jaeger_url: 'http://jaeger.local/ui/', grafana_url: null, auth_enabled: false } },
    [`/stories/${ID}`]: { body: story },
    [`/traces/${ID}`]: { body: hostile },
    [`/traces/${ID}/log-templates`]: { body: traceTemplates },
    [`/story-groups/${story.fingerprint}`]: { body: storyGroup },
    '/log-alerts': { body: logAlerts },
    ...extra,
  }
}

// The virtualizer sizes its window from offset and client sizes and clamps scrolling to
// scrollHeight, all of which jsdom reports as 0.
const sizes = ['offsetHeight', 'offsetWidth', 'clientHeight', 'clientWidth'] as const
// jsdom does not implement element scrolling (scrollTop stays 0); the virtualizer scrolls
// with scrollTo() and reads scrollTop back.
const scrollTo = Element.prototype.scrollTo
const scrollTops = new WeakMap<Element, number>()
const scrollTopDesc = Object.getOwnPropertyDescriptor(Element.prototype, 'scrollTop')
beforeAll(() => {
  Object.defineProperty(Element.prototype, 'scrollTop', {
    configurable: true,
    get(this: Element) {
      return scrollTops.get(this) ?? 0
    },
    set(this: Element, v: number) {
      scrollTops.set(this, v)
    },
  })
  for (const p of sizes) Object.defineProperty(HTMLElement.prototype, p, { configurable: true, get: () => 900 })
  Object.defineProperty(HTMLElement.prototype, 'scrollHeight', { configurable: true, get: () => 100_000 })
  Element.prototype.scrollTo = function (this: Element, opts?: ScrollToOptions | number) {
    if (typeof opts === 'object' && opts.top !== undefined) this.scrollTop = opts.top
    this.dispatchEvent(new Event('scroll'))
  } as typeof Element.prototype.scrollTo
})
afterAll(() => {
  for (const p of [...sizes, 'scrollHeight']) delete (HTMLElement.prototype as unknown as Record<string, unknown>)[p]
  Element.prototype.scrollTo = scrollTo
  if (scrollTopDesc) Object.defineProperty(Element.prototype, 'scrollTop', scrollTopDesc)
})
afterEach(() => {
  vi.unstubAllGlobals()
  clearOutage()
})

describe('story page', () => {
  it('opens the waterfall with the root cause selected and tinted', async () => {
    stubApi(routes())
    renderApp(`/stories/${ID}`)
    expect(await screen.findByRole('heading', { level: 2, name: story.summary })).toBeInTheDocument()
    const tree = await screen.findByRole('tree', { name: 'Trace waterfall' })
    const rc = await within(tree).findByRole('treeitem', { selected: true })
    expect(rc).toHaveAttribute('data-span-id', RC)
    expect(rc).toHaveAttribute('data-root-cause', 'true')
    expect(rc).toHaveTextContent('(root cause)')
    expect(tree).toHaveAttribute('aria-activedescendant', rc.id)
    // The span is selected, not opened: no drawer until the user asks.
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
  })

  it('scrolls the root-cause row into view', async () => {
    stubApi(routes())
    renderApp(`/stories/${ID}`)
    const tree = await screen.findByRole('tree', { name: 'Trace waterfall' })
    const rc = await within(tree).findByRole('treeitem', { selected: true })
    // Rows sit in absolutely placed wrappers moved by translateY(start px).
    const start = Number(/translateY\((\d+(?:\.\d+)?)px\)/.exec((rc.parentElement as HTMLElement).style.transform)?.[1])
    expect(start).toBeGreaterThan(900) // far below the first screen: scrolling was needed
    expect(tree.scrollTop).toBeGreaterThan(0)
    expect(start).toBeGreaterThanOrEqual(tree.scrollTop)
    expect(start + 28).toBeLessThanOrEqual(tree.scrollTop + 900)
    // Virtualized items state their position among visible siblings.
    expect(rc).toHaveAttribute('aria-posinset')
    expect(Number(rc.getAttribute('aria-setsize'))).toBeGreaterThanOrEqual(Number(rc.getAttribute('aria-posinset')))
  })

  it('labels the header duration as the root span and zooms to the critical path', async () => {
    stubApi(routes())
    renderApp(`/stories/${ID}`)
    expect(await screen.findByText('Root span')).toBeInTheDocument()
    expect(await screen.findByText(/Showing the critical path/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /Reset zoom/ })).toBeInTheDocument()
  })

  it('shows path, compared-with-normal, logs and related alerts', async () => {
    stubApi(routes())
    renderApp(`/stories/${ID}?since=24h`)
    const path = await screen.findByRole('list', { name: 'Request path' })
    expect(path).toHaveTextContent('payment')
    expect(screen.getByText('Compared with normal')).toBeInTheDocument()
    expect(await screen.findByRole('table', { name: 'Logs' })).toBeInTheDocument()
    expect(screen.getByRole('group', { name: 'Filter logs by service' })).toHaveTextContent(new RegExp(`all\\s*${trace.logs.length}`))
    expect(await screen.findByRole('link', { name: /Open in Jaeger/ })).toHaveAttribute(
      'href',
      `http://jaeger.local/ui/trace/${story.trace_id}`,
    )
    expect(await screen.findByTestId('echart')).toBeInTheDocument()
  })

  it('filters logs by service and severity through the URL', async () => {
    const user = userEvent.setup()
    stubApi(routes())
    const { router } = renderApp(`/stories/${ID}`)
    const chips = await screen.findByRole('group', { name: 'Filter logs by service' })
    await user.click(within(chips).getByRole('button', { name: /^payment/ }))
    await waitFor(() => expect(router.state.location.search).toMatchObject({ log_service: 'payment' }))
    await user.click(screen.getByRole('radio', { name: 'Error+' }))
    await waitFor(() => expect(router.state.location.search).toMatchObject({ log_service: 'payment', sev: 'error' }))
  })

  it('shows a 404 state for an unknown story', async () => {
    stubApi(routes())
    renderApp('/stories/0000000000000000000000000000abcd')
    expect(await screen.findByText('Story not found')).toBeInTheDocument()
    expect(screen.getByText(/There is no story 0000…abcd/)).toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'Go to stories' })).toBeInTheDocument()
  })

  it('shows an error state with retry when the API fails', async () => {
    stubApi(routes({ [`/stories/${ID}`]: { status: 503, body: { error: 'storage unavailable' } } }))
    renderApp(`/stories/${ID}`)
    expect(await screen.findByText('Storage is unavailable')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Try again' })).toBeInTheDocument()
  })

  it('shows a loading skeleton first', async () => {
    stubApi(routes())
    renderApp(`/stories/${ID}`)
    expect(await screen.findByLabelText('Loading story')).toBeInTheDocument()
  })
})

describe('trace page and span drawer', () => {
  it('shows the header, the story banner and the Jaeger link', async () => {
    stubApi(routes())
    renderApp(`/traces/${ID}`)
    const banner = await screen.findByRole('region', { name: 'Story for this trace' })
    expect(within(banner).getByRole('link', { name: /Open story/ })).toHaveAttribute('href', `/stories/${ID}`)
    expect(screen.getByText('Spans').nextSibling).toHaveTextContent(String(trace.spans.length))
    expect(await screen.findByRole('link', { name: /Open in Jaeger/ })).toBeInTheDocument()
  })

  it('hides the Jaeger link when the API has no Jaeger URL', async () => {
    stubApi(routes({ '/config': { body: { jaeger_url: null, grafana_url: null, auth_enabled: false } } }))
    renderApp(`/traces/${ID}`)
    await screen.findByRole('tree', { name: 'Trace waterfall' })
    expect(screen.queryByRole('link', { name: /Open in Jaeger/ })).not.toBeInTheDocument()
  })

  it('shows a 404 state for an unknown trace', async () => {
    stubApi(routes())
    renderApp('/traces/0000000000000000000000000000abcd')
    expect(await screen.findByText('Trace not found')).toBeInTheDocument()
  })

  it('renders attributes as literal text, never HTML', async () => {
    stubApi(routes())
    renderApp(`/traces/${ID}?span=${RC}`)
    const drawer = await screen.findByRole('dialog', { name: 'charge' })
    expect(within(drawer).getByText(XSS)).toBeInTheDocument()
    expect(document.querySelector('script')).toBeNull()
    expect(document.querySelector('img[src="x"]')).toBeNull()
    expect(within(drawer).getByText('root cause')).toBeInTheDocument()
    expect(within(drawer).getByRole('link', { name: 'payment' })).toHaveAttribute('href', '/map?service=payment')
  })

  it('searches attributes and formats exception events', async () => {
    const user = userEvent.setup()
    stubApi(routes())
    renderApp(`/traces/${ID}?span=${RC}`)
    const drawer = await screen.findByRole('dialog', { name: 'charge' })
    await user.type(within(drawer).getByRole('searchbox', { name: 'Search attributes' }), 'loyalty')
    const table = within(drawer).getByRole('table')
    expect(within(table).getAllByRole('row')).toHaveLength(1)
    expect(table).toHaveTextContent('demo.user_context.loyalty_level')
    await user.click(within(drawer).getByRole('tab', { name: /Events/ }))
    const stack = within(drawer).getByLabelText('Stack trace')
    expect(stack.tagName).toBe('PRE')
    expect(stack).toHaveTextContent('at module.exports.charge')
    await user.click(within(drawer).getByRole('tab', { name: /Timing/ }))
    expect(within(drawer).getByText('Self time')).toBeInTheDocument()
  })

  it('opens a span from the waterfall with Enter and closes it again', async () => {
    const user = userEvent.setup()
    stubApi(routes())
    const { router } = renderApp(`/traces/${ID}`)
    const tree = await screen.findByRole('tree', { name: 'Trace waterfall' })
    tree.focus()
    await user.keyboard('{Home}{ArrowDown}{Enter}')
    await waitFor(() => expect(router.state.location.search).toHaveProperty('span'))
    const drawer = await screen.findByRole('dialog')
    await user.click(within(drawer).getByRole('button', { name: 'Close panel' }))
    await waitFor(() => expect(router.state.location.search).not.toHaveProperty('span'))
  })

  it('returns focus to the waterfall when the drawer closes (Close and Escape)', async () => {
    const user = userEvent.setup()
    stubApi(routes())
    renderApp(`/traces/${ID}`)
    const tree = await screen.findByRole('tree', { name: 'Trace waterfall' })
    tree.focus()
    await user.keyboard('{Home}{Enter}')
    const drawer = await screen.findByRole('dialog')
    await user.click(within(drawer).getByRole('button', { name: 'Close panel' }))
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
    expect(tree).toHaveFocus()
    await user.keyboard('{Enter}')
    const again = await screen.findByRole('dialog')
    within(again).getByRole('button', { name: 'Close panel' }).focus()
    await user.keyboard('{Escape}')
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
    expect(tree).toHaveFocus()
  })

  it('selection follows the span in the URL on back and forward', async () => {
    const user = userEvent.setup()
    stubApi(routes())
    const { router } = renderApp(`/traces/${ID}?span=${RC}`)
    await screen.findByRole('dialog', { name: 'charge' })
    const tree = screen.getByRole('tree', { name: 'Trace waterfall' })
    expect(within(tree).getByRole('treeitem', { selected: true })).toHaveAttribute('data-span-id', RC)
    tree.focus()
    await user.keyboard('{Home}{Enter}')
    const first = trace.spans.find((x) => x.span_id === (router.state.location.search as { span?: string }).span)!
    await waitFor(() => expect(within(tree).getByRole('treeitem', { selected: true })).toHaveAttribute('data-span-id', first.span_id))
    router.history.back()
    await waitFor(() => expect(router.state.location.search).toMatchObject({ span: RC }))
    await waitFor(() => expect(within(tree).getByRole('treeitem', { selected: true })).toHaveAttribute('data-span-id', RC))
  })

  it('labels root span and trace window separately', async () => {
    stubApi(routes())
    renderApp(`/traces/${ID}`)
    expect(await screen.findByText('Root span')).toBeInTheDocument()
    expect(screen.getByText('Trace window')).toBeInTheDocument()
  })

  it('keeps the span filter in the URL', async () => {
    const user = userEvent.setup()
    stubApi(routes())
    const { router } = renderApp(`/traces/${ID}`)
    await screen.findByRole('tree', { name: 'Trace waterfall' })
    await user.click(screen.getByRole('radio', { name: 'Errors' }))
    await waitFor(() => expect(router.state.location.search).toMatchObject({ only: 'errors' }))
    const errors = trace.spans.filter((s) => s.status === 'error').length
    expect(screen.getByText(new RegExp(`^${errors} of ${trace.spans.length} spans$`))).toBeInTheDocument()
  })
})
