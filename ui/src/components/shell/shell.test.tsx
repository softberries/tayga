import { act, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import serviceMap from '../../api/__fixtures__/service-map.json'
import { clearOutage } from '../../app/apiStatus'
import { renderApp, stubApi } from '../../test/renderApp'

const TRACE = '334c8a31ddeaa3304a4e4e7f219bebbc'
const node = serviceMap.nodes[0]!
/** The captured map is all healthy, so degrade two nodes for the badge tests. */
const degradedMap = {
  edges: serviceMap.edges,
  nodes: [
    { ...node, service: 'payment', health: 'error' },
    { ...node, service: 'shipping', health: 'slow' },
    { ...node, service: 'cart', health: 'ok' },
  ],
}

beforeEach(() => {
  stubApi({ '/service-map': { body: serviceMap } })
})
afterEach(() => {
  vi.unstubAllGlobals()
  clearOutage()
})

describe('rail', () => {
  it('links the five sections and marks the active one', async () => {
    renderApp('/traces')
    const nav = await screen.findByRole('navigation', { name: 'Main' })
    const links = within(nav).getAllByRole('link')
    expect(links.map((l) => l.getAttribute('aria-label'))).toEqual([
      'Stories',
      'Traces',
      'Service map',
      'Logs and templates',
      'Pipeline health',
    ])
    expect(within(nav).getByRole('link', { name: 'Traces' })).toHaveAttribute('aria-current', 'page')
    expect(within(nav).getByRole('link', { name: 'Stories' })).not.toHaveAttribute('aria-current')
  })

  it('section links carry only since, not page filters', async () => {
    renderApp('/traces?since=24h&service=checkout&errors=true')
    const nav = await screen.findByRole('navigation', { name: 'Main' })
    for (const link of within(nav).getAllByRole('link')) {
      expect(link.getAttribute('href')).toMatch(/^\/[a-z]*\?since=24h$/)
    }
  })

  it('section links have no query at the default range', async () => {
    renderApp('/traces?service=checkout')
    const nav = await screen.findByRole('navigation', { name: 'Main' })
    for (const link of within(nav).getAllByRole('link')) {
      expect(link.getAttribute('href')).not.toContain('?')
    }
  })

  it('stories stays active on a story page', async () => {
    renderApp(`/stories/${TRACE}`)
    const nav = await screen.findByRole('navigation', { name: 'Main' })
    expect(within(nav).getByRole('link', { name: 'Stories' })).toHaveAttribute('aria-current', 'page')
  })

  it('shows a tooltip on focus', async () => {
    renderApp('/')
    const link = await screen.findByRole('link', { name: 'Pipeline health' })
    act(() => link.focus())
    expect(await screen.findByRole('tooltip')).toHaveTextContent('Pipeline health')
  })
})

describe('time range', () => {
  it('binds to ?since and is kept across navigation', async () => {
    const user = userEvent.setup()
    const { router } = renderApp('/')
    const group = await screen.findByRole('radiogroup', { name: 'Time range' })
    expect(within(group).getByRole('radio', { name: '1h' })).toHaveAttribute('data-state', 'on')
    await user.click(within(group).getByRole('radio', { name: '24h' }))
    await waitFor(() => expect(router.state.location.search).toEqual({ since: '24h' }))
    await user.click(screen.getByRole('link', { name: 'Traces' }))
    await waitFor(() => expect(router.state.location.pathname).toBe('/traces'))
    expect(router.state.location.search).toEqual({ since: '24h' })
    expect(within(screen.getByRole('radiogroup', { name: 'Time range' })).getByRole('radio', { name: '24h' })).toHaveAttribute('data-state', 'on')
  })

  it('reads ?since from the URL', async () => {
    renderApp('/map?since=7d')
    const group = await screen.findByRole('radiogroup', { name: 'Time range' })
    expect(within(group).getByRole('radio', { name: '7d' })).toHaveAttribute('data-state', 'on')
  })

  it('ignores an invalid ?since', async () => {
    const fetch = stubApi({ '/service-map': { body: serviceMap } })
    renderApp('/map?since=bogus')
    const group = await screen.findByRole('radiogroup', { name: 'Time range' })
    expect(within(group).getByRole('radio', { name: '1h' })).toHaveAttribute('data-state', 'on')
    await waitFor(() => expect(fetch).toHaveBeenCalled())
    expect(String(fetch.mock.calls[0]?.[0])).toBe('/api/v1/service-map?since=1h')
  })

  it('passes since to the API', async () => {
    const fetch = stubApi({ '/service-map': { body: serviceMap } })
    renderApp('/?since=15m')
    await waitFor(() => expect(fetch).toHaveBeenCalled())
    expect(String(fetch.mock.calls[0]?.[0])).toBe('/api/v1/service-map?since=15m')
  })
})

describe('header', () => {
  it('breadcrumb ends with the page title as h1', async () => {
    renderApp(`/traces/${TRACE}`)
    expect(await screen.findByRole('heading', { level: 1 })).toHaveTextContent('Trace 334c…ebbc')
    expect(screen.getByRole('navigation', { name: 'Breadcrumb' })).toHaveTextContent('Traces')
  })

  it('shows the degraded-services badge from /service-map', async () => {
    stubApi({ '/service-map': { body: degradedMap } })
    renderApp('/?since=24h')
    const badge = await screen.findByText('2 services degraded')
    expect(badge).toHaveAttribute('data-kind', 'error')
    const link = screen.getByRole('link', { name: '2 services degraded: payment, shipping' })
    expect(link).toContainElement(badge)
    expect(link).toHaveAttribute('href', '/map?since=24h')
    expect(link).toHaveAttribute('title', 'payment, shipping')
  })

  it('a slow-only degradation uses the slow color', async () => {
    stubApi({ '/service-map': { body: { ...degradedMap, nodes: degradedMap.nodes.slice(1) } } })
    renderApp('/')
    expect(await screen.findByText('1 service degraded')).toHaveAttribute('data-kind', 'slow')
  })

  it('hides the badge when every service is ok', async () => {
    stubApi({ '/service-map': { body: { edges: [], nodes: [{ ...node, health: 'ok' }] } } })
    renderApp('/')
    await screen.findByRole('radiogroup')
    await waitFor(() => expect(screen.queryByText(/degraded/)).toBeNull())
  })

  it('⌘K opens the palette dialog', async () => {
    const user = userEvent.setup()
    renderApp('/')
    await screen.findByRole('radiogroup')
    await user.keyboard('{Meta>}k{/Meta}')
    expect(await screen.findByRole('dialog', { name: 'Command palette' })).toBeInTheDocument()
    await user.keyboard('{Escape}')
    await user.click(screen.getByRole('button', { name: /Jump to service/ }))
    expect(await screen.findByRole('dialog', { name: 'Command palette' })).toBeInTheDocument()
  })

  it('live toggle reports its state', async () => {
    const user = userEvent.setup()
    renderApp('/')
    const live = await screen.findByRole('button', { name: 'Live' })
    expect(live).toHaveAttribute('aria-pressed', 'true')
    await user.click(live)
    expect(live).toHaveAttribute('aria-pressed', 'false')
  })

  it('shows a banner when the API answers 503', async () => {
    stubApi({ '/service-map': { status: 503, body: { error: 'storage unavailable' } } })
    renderApp('/')
    const alert = await screen.findByRole('alert')
    expect(alert).toHaveTextContent('Storage unavailable.')
    expect(alert).toHaveTextContent('storage unavailable')
  })
})

describe('routes', () => {
  it.each([
    ['/', 'Stories'],
    ['/traces', 'Traces'],
    ['/map', 'Service map'],
    ['/logs/alerts', 'Alerts'],
    ['/logs/templates', 'Templates'],
    ['/logs/templates/11039615203255878215', 'Template 1103…8215'],
    ['/pipeline', 'Pipeline'],
  ])('%s renders with title %s', async (url, title) => {
    renderApp(url)
    expect(await screen.findByRole('heading', { level: 1 })).toHaveTextContent(title)
  })

  it('/logs redirects to /logs/alerts and keeps since', async () => {
    const { router } = renderApp('/logs?since=24h')
    await waitFor(() => expect(router.state.location.pathname).toBe('/logs/alerts'))
    expect(router.state.location.search).toEqual({ since: '24h' })
  })

  it.each(['/nope', '/traces/not-a-trace-id', '/stories/123', '/logs/templates/abc'])('%s is not found', async (url) => {
    renderApp(url)
    expect(await screen.findByText('Page not found')).toBeInTheDocument()
    expect(screen.getByRole('navigation', { name: 'Main' })).toBeInTheDocument()
  })
})
