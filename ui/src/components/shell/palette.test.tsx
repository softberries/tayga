import { act, fireEvent, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import search from '../../api/__fixtures__/search.json'
import serviceMap from '../../api/__fixtures__/service-map.json'
import { clearOutage } from '../../app/apiStatus'
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
    expect(screen.getByRole('combobox')).toHaveFocus()
  })

  it('lists pages, actions and no search before typing', async () => {
    const user = userEvent.setup()
    await openPalette()
    await user.keyboard('{Meta>}k{/Meta}')
    const list = await screen.findByRole('listbox')
    expect(within(list).getByRole('option', { name: /Pipeline health/ })).toBeInTheDocument()
    expect(within(list).getByRole('option', { name: /Toggle theme/ })).toBeInTheDocument()
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

  it('the theme action cycles the theme and closes the palette', async () => {
    const user = userEvent.setup()
    await openPalette()
    expect(screen.getByRole('button', { name: /^Theme: system/ })).toBeInTheDocument()
    await user.keyboard('{Meta>}k{/Meta}')
    await user.click(await screen.findByRole('option', { name: /Toggle theme/ }))
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())
    expect(screen.getByRole('button', { name: /^Theme: light/ })).toBeInTheDocument()
  })

  it('the time range action sets since', async () => {
    const user = userEvent.setup()
    const { router } = await openPalette()
    await user.keyboard('{Meta>}k{/Meta}')
    await user.click(await screen.findByRole('option', { name: 'Time range: 24h' }))
    await waitFor(() => expect(router.state.location.search).toEqual({ since: '24h' }))
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

  it('? lists the shortcuts', async () => {
    await openPalette()
    act(() => void fireEvent.keyDown(document.body, { key: '?' }))
    const dlg = await screen.findByRole('dialog', { name: 'Keyboard shortcuts' })
    expect(within(dlg).getByText('Go to Pipeline health')).toBeInTheDocument()
  })
})
