/**
 * Stories home against captured fixtures: the groups table, URL-synced filters, keyboard
 * selection, the inspector's root-cause chip, and the empty, error and loading states.
 */
import { act, fireEvent, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import logAlerts from '../../api/__fixtures__/log-alerts.json'
import overview from '../../api/__fixtures__/overview.json'
import serviceMap from '../../api/__fixtures__/service-map.json'
import groups from '../../api/__fixtures__/story-groups.json'
import story from '../../api/__fixtures__/story.json'
import trace from '../../api/__fixtures__/trace.json'
import { clearOutage } from '../../app/apiStatus'
import { endpointOf, splitSummary } from '../../features/stories/model'
import { renderApp, stubApi, stubPendingApi } from '../../test/renderApp'
import type { Routes } from '../../test/renderApp'

const top = groups[0] as (typeof groups)[number]
const second = groups[1] as (typeof groups)[number]

function routes(extra: Routes = {}): Routes {
  return {
    '/overview': { body: overview },
    '/story-groups': { body: groups },
    '/service-map': { body: serviceMap },
    '/log-alerts': { body: logAlerts },
    '/config': { body: { jaeger_url: null, grafana_url: null } },
    [`/stories/${story.story_id}`]: { body: story },
    [`/traces/${story.trace_id}`]: { body: trace },
    ...extra,
  }
}

const bodyRows = async () => {
  const grid = await screen.findByRole('grid', { name: 'Story groups' })
  return within(grid).getAllByRole('row').slice(1)
}

afterEach(() => {
  vi.unstubAllGlobals()
  clearOutage()
})

describe('stories home', () => {
  it('renders every fixture group with its title, count and sparkline', async () => {
    stubApi(routes())
    renderApp('/')
    const rows = await bodyRows()
    expect(rows).toHaveLength(groups.length)
    const first = rows[0] as HTMLElement
    expect(first).toHaveTextContent(splitSummary(top.summary).title)
    expect(within(first).getByText(String(top.stories))).toBeInTheDocument()
    expect(within(first).getByRole('img', { name: /^Stories per minute over 1h, peak \d+$/ })).toBeInTheDocument()
    expect(screen.getByText(`${groups.length} groups · sorted by stories`)).toBeInTheDocument()
    // KPI tiles from /overview, the map preview and the 5 newest alerts.
    expect(screen.getByRole('list', { name: 'Summary' })).toHaveTextContent(String(overview.error_stories))
    expect(screen.getByRole('link', { name: /^Service map: 19 services/ })).toHaveAttribute('href', '/map')
    expect(screen.getAllByRole('link', { name: /spike|new/ }).filter((a) => a.getAttribute('href')?.startsWith('/logs/templates/'))).toHaveLength(5)
  })

  it('filter chips and the search update the URL and the rows', async () => {
    const user = userEvent.setup()
    stubApi(routes())
    const { router } = renderApp('/')
    await bodyRows()
    act(() => screen.getByRole('button', { name: '+ service' }).focus())
    await user.keyboard('{Enter}')
    await user.click(await screen.findByRole('menuitem', { name: /^recommendation/ }))
    await waitFor(() => expect(router.state.location.search).toMatchObject({ service: 'recommendation' }))
    expect(await bodyRows()).toHaveLength(1)
    expect(screen.getByText('1 of 23 groups · sorted by stories')).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Remove Service filter' }))
    await waitFor(() => expect(router.state.location.search).not.toHaveProperty('service'))

    act(() => screen.getByRole('button', { name: /^Kind filter/ }).focus())
    await user.keyboard('{Enter}')
    await user.click(await screen.findByRole('menuitem', { name: /^Errors/ }))
    await waitFor(() => expect(router.state.location.search).toMatchObject({ kind: 'error' }))
    expect(await bodyRows()).toHaveLength(groups.filter((g) => g.kind === 'error').length)

    await user.type(screen.getByRole('searchbox', { name: 'Search story groups' }), 'agent')
    await waitFor(() => expect(router.state.location.search).toMatchObject({ kind: 'error', q: 'agent' }))
    expect(await bodyRows()).toHaveLength(1)
  })

  it('reads filters from the URL and offers to clear them when nothing matches', async () => {
    const user = userEvent.setup()
    stubApi(routes())
    const { router } = renderApp('/?kind=slow&q=nothing-matches-this')
    expect(await screen.findByText('No groups match these filters')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Clear filters' }))
    expect(await bodyRows()).toHaveLength(groups.length)
    expect(router.state.location.search).toEqual({})
  })

  it('moves the selection with the arrow keys and opens the story with Enter', async () => {
    stubApi(routes())
    const { router } = renderApp('/')
    const rows = await bodyRows()
    const first = rows[0] as HTMLElement
    expect(first).toHaveAttribute('aria-selected', 'true')
    expect(first).toHaveAttribute('tabindex', '0')
    act(() => first.focus())
    fireEvent.keyDown(first, { key: 'ArrowDown' })
    await waitFor(() => expect(router.state.location.search).toMatchObject({ group: second.fingerprint }))
    const secondRow = (await bodyRows())[1] as HTMLElement
    expect(secondRow).toHaveAttribute('aria-selected', 'true')
    expect(secondRow).toHaveFocus()
    fireEvent.keyDown(secondRow, { key: 'ArrowUp' })
    await waitFor(() => expect(router.state.location.search).toMatchObject({ group: top.fingerprint }))
    fireEvent.keyDown((await bodyRows())[0] as HTMLElement, { key: 'Enter' })
    await waitFor(() => expect(router.state.location.pathname).toBe(`/stories/${top.sample_story_id}`))
  })

  it('selects the top group and shows its root-cause chip, waterfall and links', async () => {
    stubApi(routes())
    renderApp('/')
    const aside = await screen.findByRole('complementary', { name: 'Selected story' })
    const path = await within(aside).findByRole('list', { name: 'Request path' })
    const rc = within(path).getByText(story.root_cause.service, { selector: 'li' })
    expect(rc).toHaveAttribute('aria-current', 'step')
    expect(rc).toHaveClass('shadow-glow-err')
    expect(await within(aside).findByRole('group', { name: 'Trace waterfall (summary)' })).toBeInTheDocument()
    expect(within(aside).getByRole('region', { name: 'Compared with normal' })).toBeInTheDocument()
    expect(within(aside).getByRole('link', { name: /Open story/ })).toHaveAttribute('href', `/stories/${story.story_id}`)
  })

  it('keeps every endpoint visible, so groups with the same title stay apart', async () => {
    stubApi(routes())
    renderApp('/')
    const rows = await bodyRows()
    groups.forEach((g, i) => {
      const ep = within(rows[i] as HTMLElement).getByText(endpointOf(g))
      expect(ep).toHaveClass('shrink-0')
    })
  })

  it('drops a selected group that is not in the table from the URL', async () => {
    stubApi(routes())
    const { router } = renderApp('/?group=123')
    expect((await bodyRows())[0]).toHaveAttribute('aria-selected', 'true')
    await waitFor(() => expect(router.state.location.search).not.toHaveProperty('group'))
  })

  it('loads only the group the selection rests on while arrows are held', async () => {
    const fetch = stubApi(routes())
    renderApp('/')
    const first = (await bodyRows())[0] as HTMLElement
    await screen.findByRole('list', { name: 'Request path' })
    act(() => first.focus())
    // Like a held key: each step lands, then a repeat gap, before the next.
    for (let i = 1; i <= 3; i++) {
      fireEvent.keyDown(document.activeElement as HTMLElement, { key: 'ArrowDown' })
      await waitFor(() => expect((screen.getAllByRole('row') as HTMLElement[])[i + 1]).toHaveFocus(), { interval: 5 })
      // A key-repeat gap: shorter than the settle time, long enough for a zero-delay timer.
      await act(() => new Promise((r) => setTimeout(r, 40)))
    }
    const storyFetches = (id: string) => fetch.mock.calls.filter(([u]) => String(u).includes(`/stories/${id}`)).length
    const landed = groups[3] as (typeof groups)[number]
    await waitFor(() => expect(storyFetches(landed.sample_story_id)).toBe(1))
    expect(storyFetches((groups[1] as (typeof groups)[number]).sample_story_id)).toBe(0)
    expect(storyFetches((groups[2] as (typeof groups)[number]).sample_story_id)).toBe(0)
  })

  it('shows an explained dash when the previous window cannot be loaded', async () => {
    const fetch = stubApi(routes())
    const inner = fetch.getMockImplementation()!
    fetch.mockImplementation(async (input: string) =>
      input.includes('/overview') && input.includes('since=2h')
        ? new Response(JSON.stringify({ error: 'boom' }), { status: 500 })
        : inner(input),
    )
    renderApp('/')
    const dashes = await screen.findAllByLabelText('Change unknown: The previous window could not be loaded.')
    expect(dashes).toHaveLength(2)
    expect(dashes[0]).toHaveTextContent('—')
  })

  it('shows the empty state when the window has no stories', async () => {
    stubApi(routes({ '/story-groups': { body: [] } }))
    renderApp('/')
    expect(await screen.findByText('No stories in this window')).toBeInTheDocument()
    expect(screen.queryByRole('complementary', { name: 'Selected story' })).toBeNull()
  })

  it('shows an error banner that retries', async () => {
    const user = userEvent.setup()
    const fetch = stubApi(routes({ '/story-groups': { status: 503, body: { error: 'clickhouse unavailable' } } }))
    renderApp('/')
    const banner = (await screen.findByText('Could not load story groups.')).closest('[role="alert"]') as HTMLElement
    expect(banner).toHaveTextContent('503 · clickhouse unavailable')
    const calls = () => fetch.mock.calls.filter(([u]) => String(u).includes('/story-groups')).length
    const before = calls()
    await user.click(within(banner).getByRole('button', { name: 'Try again' }))
    await waitFor(() => expect(calls()).toBe(before + 1))
  })

  it('a failed refresh keeps the tiles, table, inspector, map and alerts, and notes it', async () => {
    stubApi(routes())
    const { queryClient } = renderApp('/')
    await bodyRows()
    await screen.findByRole('complementary', { name: 'Selected story' })
    const down = { status: 503, body: { error: 'clickhouse unavailable' } }
    stubApi(routes({ '/overview': down, '/story-groups': down, '/service-map': down, '/log-alerts': down }))
    await act(() => queryClient.refetchQueries({ type: 'active' }))
    await waitFor(() => expect(screen.getAllByText(/^Refresh failed · showing data from \d\d:\d\d:\d\d$/)).toHaveLength(4))
    expect(screen.queryByText(/^Could not load/)).toBeNull()
    expect(await bodyRows()).toHaveLength(groups.length)
    expect(screen.getByRole('list', { name: 'Summary' })).toHaveTextContent(String(overview.error_stories))
    expect(screen.getByRole('complementary', { name: 'Selected story' })).toBeInTheDocument()
    expect(screen.getByRole('link', { name: /^Service map: 19 services/ })).toBeInTheDocument()
  })

  it('shows skeletons while loading', async () => {
    stubApi(routes())
    stubPendingApi()
    renderApp('/')
    expect(await screen.findByLabelText('Loading story groups')).toBeInTheDocument()
    expect(screen.getByLabelText('Loading summary')).toBeInTheDocument()
  })
})
