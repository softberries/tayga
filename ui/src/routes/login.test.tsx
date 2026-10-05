/**
 * The login page and the session guard against a stubbed API: the redirect to /login with
 * `next`, sign-in, wrong credentials, the rate limit, auth off, a session lost mid-use and
 * sign-out from the user menu and the command palette.
 */
import { screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import serviceMap from '../api/__fixtures__/service-map.json'
import { clearOutage, getOutage } from '../app/apiStatus'
import { renderApp, stubApi } from '../test/renderApp'
import type { Routes } from '../test/renderApp'

const UNAUTHORIZED = { status: 401, body: { error: 'unauthorized' } }

function routes(opts: { auth: boolean; signedIn: boolean }): Routes {
  return {
    '/config': { body: { jaeger_url: null, grafana_url: null, auth_enabled: opts.auth, infra_services: ['flagd'] } },
    '/auth/me': opts.signedIn ? { body: { username: 'admin' } } : UNAUTHORIZED,
    '/auth/login': { status: 204, body: null },
    '/auth/logout': { status: 204, body: null },
    '/service-map': opts.signedIn || !opts.auth ? { body: serviceMap } : UNAUTHORIZED,
    '/search': { body: { trace_id: null, services: [], groups: [], templates: [] } },
  }
}

type Fetch = ReturnType<typeof stubApi>

function calls(fetch: Fetch, path: string) {
  return fetch.mock.calls.filter(([u]) => new URL(u, 'http://test').pathname === `/api/v1${path}`)
}

afterEach(() => {
  vi.unstubAllGlobals()
  clearOutage()
})

async function signInPage(r: Routes, url = '/map?since=24h') {
  const fetch = stubApi(r)
  const utils = renderApp(url)
  await screen.findByRole('heading', { name: 'Tayga' })
  return { fetch, ...utils }
}

describe('login', () => {
  it('redirects to /login with next when the session is missing', async () => {
    const { router } = await signInPage(routes({ auth: true, signedIn: false }))
    expect(router.state.location.href).toBe('/login?next=%2Fmap%3Fsince%3D24h')
    // No shell outside the login page.
    expect(screen.queryByRole('navigation', { name: 'Main' })).toBeNull()
    expect(screen.getByLabelText('Username')).toHaveAttribute('autocomplete', 'username')
    expect(screen.getByLabelText('Password')).toHaveAttribute('autocomplete', 'current-password')
    expect(getOutage()).toBeNull()
  })

  it('signs in with JSON, resets the caches and goes to next', async () => {
    const user = userEvent.setup()
    const r = routes({ auth: true, signedIn: false })
    const { fetch, router, queryClient } = await signInPage(r)
    const clear = vi.spyOn(queryClient, 'clear')
    Object.assign(r, routes({ auth: true, signedIn: true }))
    await user.type(screen.getByLabelText('Username'), 'admin')
    await user.type(screen.getByLabelText('Password'), 'secret')
    await user.click(screen.getByRole('button', { name: 'Sign in' }))
    await waitFor(() => expect(router.state.location.href).toBe('/map?since=24h'))
    const [, init] = calls(fetch, '/auth/login')[0]!
    expect(init?.method).toBe('POST')
    expect(new Headers(init?.headers).get('content-type')).toBe('application/json')
    expect(JSON.parse(String(init?.body))).toEqual({ username: 'admin', password: 'secret' })
    expect(clear).toHaveBeenCalled()
    expect(await screen.findByRole('navigation', { name: 'Main' })).toBeInTheDocument()
    expect(await screen.findByRole('button', { name: 'Signed in as admin' })).toBeInTheDocument()
  })

  it('wrong credentials show an alert, keep the username and clear the password', async () => {
    const user = userEvent.setup()
    const r = routes({ auth: true, signedIn: false })
    r['/auth/login'] = UNAUTHORIZED
    const { router } = await signInPage(r)
    await user.type(screen.getByLabelText('Username'), 'admin')
    await user.type(screen.getByLabelText('Password'), 'nope')
    await user.click(screen.getByRole('button', { name: 'Sign in' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('Wrong username or password.')
    expect(screen.getByLabelText('Username')).toHaveValue('admin')
    expect(screen.getByLabelText('Password')).toHaveValue('')
    expect(router.state.location.pathname).toBe('/login')
  })

  it('a 429 says when to try again from Retry-After', async () => {
    const user = userEvent.setup()
    const r = routes({ auth: true, signedIn: false })
    r['/auth/login'] = { status: 429, body: { error: 'too many requests' }, headers: { 'retry-after': '42' } }
    await signInPage(r)
    await user.type(screen.getByLabelText('Username'), 'admin')
    await user.type(screen.getByLabelText('Password'), 'x')
    await user.click(screen.getByRole('button', { name: 'Sign in' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('Too many attempts, try again in 42 s')
  })

  it('an unsafe next lands on /', async () => {
    const user = userEvent.setup()
    const r = routes({ auth: true, signedIn: false })
    const { router } = await signInPage(r, '/login?next=%2F%2Fevil.com')
    Object.assign(r, routes({ auth: true, signedIn: true }))
    await user.type(screen.getByLabelText('Username'), 'admin')
    await user.type(screen.getByLabelText('Password'), 'secret')
    await user.click(screen.getByRole('button', { name: 'Sign in' }))
    await waitFor(() => expect(router.state.location.pathname).toBe('/'))
  })

  it('redirects /login to / when auth is disabled, and never asks auth/me', async () => {
    const fetch = stubApi(routes({ auth: false, signedIn: false }))
    const { router } = renderApp('/login?next=%2Fmap')
    await waitFor(() => expect(router.state.location.pathname).toBe('/'))
    expect(screen.queryByRole('heading', { name: 'Tayga' })).toBeNull()
    await screen.findByRole('navigation', { name: 'Main' })
    expect(calls(fetch, '/auth/me')).toHaveLength(0)
  })

  it('sends a signed-in user on /login to a safe next', async () => {
    stubApi(routes({ auth: true, signedIn: true }))
    const { router } = renderApp('/login?next=%2Fmap%3Fsince%3D24h')
    await waitFor(() => expect(router.state.location.href).toBe('/map?since=24h'))
    expect(await screen.findByRole('button', { name: 'Signed in as admin' })).toBeInTheDocument()
  })

  it('sends a signed-in user on /login with an unsafe next to /', async () => {
    stubApi(routes({ auth: true, signedIn: true }))
    const { router } = renderApp('/login?next=%2F%2Fevil.com')
    await waitFor(() => expect(router.state.location.pathname).toBe('/'))
  })

  it('guards unknown paths too, before any page query', async () => {
    const fetch = stubApi(routes({ auth: true, signedIn: false }))
    const { router } = renderApp('/nope?since=24h')
    await screen.findByRole('heading', { name: 'Tayga' })
    expect(router.state.location.href).toBe('/login?next=%2Fnope%3Fsince%3D24h')
    expect(calls(fetch, '/service-map')).toHaveLength(0)
  })

  it('a slow auth/me shows the shell skeleton, then lets the page load after the timeout', async () => {
    const r = routes({ auth: true, signedIn: true })
    const fetch = stubApi(r)
    const answer = fetch.getMockImplementation()!
    fetch.mockImplementation((input: string, init?: RequestInit) =>
      new URL(input, 'http://test').pathname === '/api/v1/auth/me' ? new Promise<Response>(() => {}) : answer(input, init),
    )
    renderApp('/map')
    expect(await screen.findByRole('status', { name: 'Loading Tayga' }, { timeout: 2_500 })).toBeInTheDocument()
    expect(await screen.findByRole('navigation', { name: 'Main' }, { timeout: 4_000 })).toBeInTheDocument()
  }, 10_000)

  it('a 401 mid-session redirects to /login once and raises no outage banner', async () => {
    // The session expires right after the guard's check: auth/me answers once, then 401s.
    const r = routes({ auth: true, signedIn: true })
    r['/service-map'] = UNAUTHORIZED
    r['/services'] = UNAUTHORIZED
    const fetch = stubApi(r)
    const answer = fetch.getMockImplementation()!
    fetch.mockImplementation(async (input: string, init?: RequestInit) => {
      const res = await answer(input, init)
      if (new URL(input, 'http://test').pathname === '/api/v1/auth/me') r['/auth/me'] = UNAUTHORIZED
      return res
    })
    const { router } = renderApp('/map?since=7d')
    const navigate = vi.spyOn(router, 'navigate')
    await screen.findByRole('heading', { name: 'Tayga' })
    expect(router.state.location.href).toBe('/login?next=%2Fmap%3Fsince%3D7d')
    expect(navigate).toHaveBeenCalledTimes(1)
    expect(getOutage()).toBeNull()
    expect(screen.queryByText(/storage/i)).toBeNull()
  })

  it('signs out from the user menu', async () => {
    const user = userEvent.setup()
    const r = routes({ auth: true, signedIn: true })
    const fetch = stubApi(r)
    const { router, queryClient } = renderApp('/map')
    const clear = vi.spyOn(queryClient, 'clear')
    await user.click(await screen.findByRole('button', { name: 'Signed in as admin' }))
    Object.assign(r, routes({ auth: true, signedIn: false }))
    await user.click(await screen.findByRole('menuitem', { name: 'Sign out' }))
    await screen.findByRole('heading', { name: 'Tayga' })
    expect(router.state.location.pathname).toBe('/login')
    const [, init] = calls(fetch, '/auth/logout')[0]!
    expect(init?.method).toBe('POST')
    expect(new Headers(init?.headers).get('content-type')).toBe('application/json')
    expect(init?.body).toBe('{}')
    expect(clear).toHaveBeenCalled()
  })

  it.each([true, false])('the palette has Sign out only when auth is %s', async (auth) => {
    const user = userEvent.setup()
    const fetch = stubApi(routes({ auth, signedIn: true }))
    renderApp('/map')
    await screen.findByRole('navigation', { name: 'Main' })
    if (auth) await screen.findByRole('button', { name: 'Signed in as admin' })
    await user.keyboard('{Meta>}k{/Meta}')
    const list = await screen.findByRole('listbox')
    expect(within(list).queryByRole('option', { name: 'Sign out' }) !== null).toBe(auth)
    if (!auth) expect(calls(fetch, '/auth/me')).toHaveLength(0)
  })

  it('a failed logout from the user menu says so and stays signed in', async () => {
    const user = userEvent.setup()
    const r = routes({ auth: true, signedIn: true })
    r['/auth/logout'] = { status: 503, body: { error: 'unavailable' } }
    stubApi(r)
    const { router, queryClient } = renderApp('/map')
    const clear = vi.spyOn(queryClient, 'clear')
    await user.click(await screen.findByRole('button', { name: 'Signed in as admin' }))
    await user.click(await screen.findByRole('menuitem', { name: 'Sign out' }))
    expect(await screen.findByRole('alert')).toHaveTextContent("Couldn't sign out, try again")
    expect(router.state.location.pathname).toBe('/map')
    expect(clear).not.toHaveBeenCalled()
    await user.keyboard('{Escape}')
    expect(await screen.findByRole('button', { name: 'Signed in as admin' })).toBeInTheDocument()
  })

  it('a failed logout from the palette says so and stays signed in', async () => {
    const user = userEvent.setup()
    const r = routes({ auth: true, signedIn: true })
    r['/auth/logout'] = { status: 503, body: { error: 'unavailable' } }
    stubApi(r)
    const { router } = renderApp('/map')
    await screen.findByRole('button', { name: 'Signed in as admin' })
    await user.keyboard('{Meta>}k{/Meta}')
    await user.click(await screen.findByRole('option', { name: 'Sign out' }))
    const dialog = screen.getByRole('dialog', { name: 'Command palette' })
    expect(await within(dialog).findByRole('alert')).toHaveTextContent("Couldn't sign out, try again")
    expect(router.state.location.pathname).toBe('/map')
  })

  it('the palette Sign out signs out', async () => {
    const user = userEvent.setup()
    const r = routes({ auth: true, signedIn: true })
    const fetch = stubApi(r)
    const { router } = renderApp('/map')
    await screen.findByRole('button', { name: 'Signed in as admin' })
    Object.assign(r, routes({ auth: true, signedIn: false }))
    await user.keyboard('{Meta>}k{/Meta}')
    await user.click(await screen.findByRole('option', { name: 'Sign out' }))
    await waitFor(() => expect(router.state.location.pathname).toBe('/login'))
    expect(calls(fetch, '/auth/logout')).toHaveLength(1)
  })
})
