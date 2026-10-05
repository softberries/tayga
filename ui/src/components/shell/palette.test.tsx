import { act, fireEvent, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import search from '../../api/__fixtures__/search.json'
import serviceMap from '../../api/__fixtures__/service-map.json'
import { clearOutage } from '../../app/apiStatus'
import { fromLocalInput } from '../../app/range'
import { formatUntil } from '../../app/search'
import { RECENT_KEY, readRecent } from '../../lib/recent'
import { renderApp, stubApi } from '../../test/renderApp'

const TRACE = '334c8a31ddeaa3304a4e4e7f219bebbc'

let fetch: ReturnType<typeof stubApi>
beforeEach(() => {
  fetch = stubApi({ '/service-map': { body: serviceMap }, '/search': { body: search } })
})
afterEach(() => {
  vi.unstubAllGlobals()
  clearOutage()
})

async function openPalette(url = '/map') {
  const utils = renderApp(url)
  await screen.findByRole('navigation', { name: 'Main' })
  return utils
}

describe('command palette', () => {
  it('opens with Cmd+K and Ctrl+K and closes with Escape', async () => {
    const user = userEvent.setup()
    await openPalette()
    expect(screen.queryByRole('dialog')).toBeNull()
    await user.keyboard('{Meta>}k{/Meta}')
    expect(await screen.findByRole('dialog', { name: 'Command palette' })).toBeInTheDocument()
    await user.keyboard('{Escape}')
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())
    await user.keyboard('{Control>}k{/Control}')
    expect(await screen.findByRole('dialog', { name: 'Command palette' })).toBeInTheDocument()
    await waitFor(() => expect(screen.getByRole('combobox')).toHaveFocus())
  })

  it('lists pages, actions and no search before typing', async () => {
    const user = userEvent.setup()
    await openPalette()
    await user.keyboard('{Meta>}k{/Meta}')
    const list = await screen.findByRole('listbox')
    expect(within(list).getByRole('option', { name: /Pipeline health/ })).toBeInTheDocument()
    expect(within(list).getByRole('option', { name: 'Theme: Dark' })).toBeInTheDocument()
    expect(fetch.mock.calls.some(([u]) => String(u).includes('/search'))).toBe(false)
  })

  it('offers Open trace first for a 32-hex id and navigates to it', async () => {
    const user = userEvent.setup()
    const { router } = await openPalette()
    await user.keyboard('{Meta>}k{/Meta}')
    await user.type(await screen.findByRole('combobox'), TRACE)
    const options = await screen.findAllByRole('option')
    expect(options[0]).toHaveTextContent(`Open trace ${TRACE}`)
    await user.keyboard('{Enter}')
    await waitFor(() => expect(router.state.location.pathname).toBe(`/traces/${TRACE}`))
    expect(readRecent()[0]).toMatchObject({ kind: 'trace', id: TRACE })
  })

  it('shows search results (debounced) grouped and opens a template', async () => {
    const user = userEvent.setup()
    const { router } = await openPalette()
    await user.keyboard('{Meta>}k{/Meta}')
    await user.type(await screen.findByRole('combobox'), 'pay')
    const tpl = await screen.findByRole('option', { name: /Checkout payment declined/ })
    expect(screen.getByText('Services')).toBeInTheDocument()
    expect(screen.getByText('Story groups')).toBeInTheDocument()
    // Typing 3 characters fires one request, not three.
    expect(fetch.mock.calls.filter(([u]) => String(u).includes('/search'))).toHaveLength(1)
    await user.click(tpl)
    await waitFor(() => expect(router.state.location.pathname).toBe('/logs/templates/1066356062084821589'))
  })

  it('the theme actions set the theme and close the palette', async () => {
    const user = userEvent.setup()
    await openPalette()
    expect(screen.getByRole('button', { name: /^Theme: system/ })).toBeInTheDocument()
    await user.keyboard('{Meta>}k{/Meta}')
    await user.click(await screen.findByRole('option', { name: 'Theme: Dark' }))
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())
    expect(screen.getByRole('button', { name: /^Theme: dark/ })).toBeInTheDocument()
  })

  it('the time range action sets since', async () => {
    const user = userEvent.setup()
    const { router } = await openPalette()
    await user.keyboard('{Meta>}k{/Meta}')
    await user.click(await screen.findByRole('option', { name: 'Time range: 24h' }))
    await waitFor(() => expect(router.state.location.search).toEqual({ since: '24h' }))
  })

  it('the custom range action opens the header popover', async () => {
    const user = userEvent.setup()
    await openPalette('/map?since=24h')
    await user.keyboard('{Meta>}k{/Meta}')
    await user.type(await screen.findByRole('combobox'), 'custom')
    await user.click(await screen.findByRole('option', { name: 'Time range: Custom range…' }))
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Command palette' })).toBeNull())
    const form = await screen.findByRole('form', { name: 'Custom time range' })
    // It starts from the range on screen: the last 24h.
    const from = fromLocalInput((within(form).getByLabelText('From') as HTMLInputElement).value)
    const to = fromLocalInput((within(form).getByLabelText('To') as HTMLInputElement).value)
    expect(to - from).toBe(86_400_000)
  })

  it('a preset action clears a custom range', async () => {
    const user = userEvent.setup()
    const until = formatUntil(Math.floor(Date.now() / 60_000) * 60_000 - 3_600_000)
    const { router } = await openPalette(`/map?since=2h&until=${until}`)
    await user.keyboard('{Meta>}k{/Meta}')
    await user.click(await screen.findByRole('option', { name: 'Time range: 15m' }))
    await waitFor(() => expect(router.state.location.search).toEqual({ since: '15m' }))
  })

  it('recent items persist and show on the next open', async () => {
    const user = userEvent.setup()
    await openPalette()
    await user.keyboard('{Meta>}k{/Meta}')
    await user.click(await screen.findByRole('option', { name: /Pipeline health/ }))
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())
    expect(JSON.parse(window.localStorage.getItem(RECENT_KEY) ?? '[]')).toHaveLength(1)
    await user.keyboard('{Meta>}k{/Meta}')
    expect(await screen.findByText('Recent')).toBeInTheDocument()
    expect(screen.getAllByRole('option')[0]).toHaveTextContent('Pipeline health')
  })
})

/** Replaces fetch so each /search call waits on a promise the test resolves. */
function deferredSearch() {
  const calls: { q: string; resolve: (body: unknown, status?: number) => void }[] = []
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: string) => {
      const url = new URL(input, 'http://test')
      if (url.pathname.endsWith('/search')) {
        return new Promise<Response>((res) =>
          calls.push({
            q: url.searchParams.get('q') ?? '',
            resolve: (body, status = 200) =>
              res(new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } })),
          }),
        )
      }
      return new Response(JSON.stringify(serviceMap), { status: 200, headers: { 'content-type': 'application/json' } })
    }),
  )
  return calls
}
const view = (service: string) => ({ services: [service], templates: [], groups: [], trace_id: null })

describe('palette search states', () => {
  it('shows Searching while debouncing and never the previous query results', async () => {
    const user = userEvent.setup()
    const calls = deferredSearch()
    await openPalette()
    await user.keyboard('{Meta>}k{/Meta}')
    const input = await screen.findByRole('combobox')
    await user.type(input, 'ab')
    await waitFor(() => expect(calls.map((c) => c.q)).toEqual(['ab']))
    act(() => calls[0]!.resolve(view('alpha')))
    expect(await screen.findByRole('option', { name: /alpha/ })).toBeInTheDocument()
    await user.type(input, 'c')
    expect(screen.queryByRole('option', { name: /alpha/ })).toBeNull()
    expect(screen.getByRole('status')).toHaveTextContent('Searching…')
  })

  it('keeps a steady results area from the first keystroke, through loading and results', async () => {
    const user = userEvent.setup()
    const calls = deferredSearch()
    await openPalette()
    await user.keyboard('{Meta>}k{/Meta}')
    const input = await screen.findByRole('combobox')
    expect(screen.getByTestId('palette-server').className).not.toContain('min-h')
    await user.type(input, 'ab')
    expect(screen.getByTestId('palette-server').className).toContain('min-h-36')
    await waitFor(() => expect(calls.map((c) => c.q)).toEqual(['ab']))
    act(() => calls[0]!.resolve(view('alpha')))
    expect(await screen.findByRole('option', { name: /alpha/ })).toBeInTheDocument()
    expect(screen.getByTestId('palette-server').className).toContain('min-h-36')
  })

  it('clearing the box shows only Recent and pages, no old groups', async () => {
    const user = userEvent.setup()
    await openPalette()
    await user.keyboard('{Meta>}k{/Meta}')
    const input = await screen.findByRole('combobox')
    await user.type(input, 'pay')
    await screen.findByRole('option', { name: /Checkout payment declined/ })
    await user.clear(input)
    expect(screen.queryByText('Story groups')).toBeNull()
    expect(screen.queryByText('Templates')).toBeNull()
    expect(screen.queryByRole('option', { name: /Checkout payment declined/ })).toBeNull()
  })

  it('a slow response for an old query does not overwrite a newer one', async () => {
    const user = userEvent.setup()
    const calls = deferredSearch()
    await openPalette()
    await user.keyboard('{Meta>}k{/Meta}')
    const input = await screen.findByRole('combobox')
    await user.type(input, 'ab')
    await waitFor(() => expect(calls).toHaveLength(1))
    await user.type(input, 'c')
    await waitFor(() => expect(calls).toHaveLength(2))
    act(() => calls[1]!.resolve(view('new')))
    expect(await screen.findByRole('option', { name: /new/ })).toBeInTheDocument()
    act(() => calls[0]!.resolve(view('old')))
    await new Promise((r) => setTimeout(r, 30))
    expect(screen.queryByRole('option', { name: /old/ })).toBeNull()
    expect(screen.getByRole('option', { name: /new/ })).toBeInTheDocument()
  })

  it('shows an error row even when pages match', async () => {
    const user = userEvent.setup()
    const calls = deferredSearch()
    await openPalette()
    await user.keyboard('{Meta>}k{/Meta}')
    await user.type(await screen.findByRole('combobox'), 'st')
    await waitFor(() => expect(calls).toHaveLength(1))
    act(() => calls[0]!.resolve({ error: 'boom' }, 500))
    expect(await screen.findByRole('alert')).toHaveTextContent('Search is unavailable.')
    expect(screen.getByRole('option', { name: /Stories/ })).toBeInTheDocument()
  })

  it('shows an explicit empty row', async () => {
    const user = userEvent.setup()
    const calls = deferredSearch()
    await openPalette()
    await user.keyboard('{Meta>}k{/Meta}')
    await user.type(await screen.findByRole('combobox'), 'st')
    await waitFor(() => expect(calls).toHaveLength(1))
    act(() => calls[0]!.resolve({ services: [], templates: [], groups: [], trace_id: null }))
    expect(await screen.findByText('No matches for “st”.')).toBeInTheDocument()
  })
})

describe('palette keyboard and focus', () => {
  it('arrow keys move the selection and Enter opens it', async () => {
    const user = userEvent.setup()
    const { router } = await openPalette('/')
    await user.keyboard('{Meta>}k{/Meta}')
    await screen.findByRole('combobox')
    // Order without a query: Pages first (no Recent yet): Stories, Traces, ...
    await user.keyboard('{ArrowDown}{Enter}')
    await waitFor(() => expect(router.state.location.pathname).toBe('/traces'))
  })

  it('returns focus to the trigger on close', async () => {
    const user = userEvent.setup()
    await openPalette()
    const trigger = screen.getByRole('button', { name: /Jump to service/ })
    await user.click(trigger)
    await screen.findByRole('dialog')
    await user.keyboard('{Escape}')
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())
    await waitFor(() => expect(trigger).toHaveFocus())
  })

  it('Cmd+K does not open over another dialog', async () => {
    const user = userEvent.setup()
    await openPalette()
    fireEvent.keyDown(document.body, { key: '?' })
    await screen.findByRole('dialog', { name: 'Keyboard shortcuts' })
    await user.keyboard('{Meta>}k{/Meta}')
    expect(screen.queryByRole('dialog', { name: 'Command palette' })).toBeNull()
  })
})

describe('keyboard shortcuts', () => {
  it('g then t goes to Traces, keeping since', async () => {
    const { router } = await openPalette('/map?since=24h')
    fireEvent.keyDown(document.body, { key: 'g' })
    fireEvent.keyDown(document.body, { key: 't' })
    await waitFor(() => expect(router.state.location.pathname).toBe('/traces'))
    expect(router.state.location.search).toEqual({ since: '24h' })
  })

  it('ignores shortcuts typed in a field', async () => {
    const user = userEvent.setup()
    const { router } = await openPalette()
    await user.keyboard('{Meta>}k{/Meta}')
    await user.type(await screen.findByRole('combobox'), 'gp?')
    expect(router.state.location.pathname).toBe('/map')
    expect(screen.queryByRole('dialog', { name: 'Keyboard shortcuts' })).toBeNull()
  })

  it('the g chord times out after 1 s', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true })
    try {
      const { router } = await openPalette()
      fireEvent.keyDown(document.body, { key: 'g' })
      await act(async () => void (await vi.advanceTimersByTimeAsync(1100)))
      fireEvent.keyDown(document.body, { key: 't' })
      await act(async () => void (await vi.advanceTimersByTimeAsync(50)))
      expect(router.state.location.pathname).toBe('/map')
    } finally {
      vi.useRealTimers()
    }
  })

  it('g followed by an invalid key does nothing and disarms', async () => {
    const { router } = await openPalette()
    fireEvent.keyDown(document.body, { key: 'g' })
    fireEvent.keyDown(document.body, { key: 'x' })
    fireEvent.keyDown(document.body, { key: 't' })
    await new Promise((r) => setTimeout(r, 30))
    expect(router.state.location.pathname).toBe('/map')
  })

  it('? does not fire while a dialog is open', async () => {
    const user = userEvent.setup()
    await openPalette()
    await user.keyboard('{Meta>}k{/Meta}')
    await screen.findByRole('dialog', { name: 'Command palette' })
    fireEvent.keyDown(document.body, { key: '?' })
    expect(screen.queryByRole('dialog', { name: 'Keyboard shortcuts' })).toBeNull()
  })

  it('? lists the shortcuts', async () => {
    await openPalette()
    act(() => void fireEvent.keyDown(document.body, { key: '?' }))
    const dlg = await screen.findByRole('dialog', { name: 'Keyboard shortcuts' })
    expect(within(dlg).getByText('Go to Log alerts')).toBeInTheDocument()
  })
})
