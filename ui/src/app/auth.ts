/**
 * Optional login (spec §2.4): the `next` check, the session hook for the header and palette,
 * sign-out, and the hook the query cache calls when a request comes back 401.
 */
import { useQuery } from '@tanstack/react-query'
import type { QueryClient } from '@tanstack/react-query'
import type { UseNavigateResult } from '@tanstack/react-router'
import { authApi, api } from '../api/queries'

/** Any origin works: only whether `next` resolves to it matters. */
const BASE = 'http://tayga.invalid'

/**
 * `raw` when it is a same-origin path, else `/`. It must start with one `/` (not `//` or `/\`,
 * which browsers read as another host), also once percent-decoded, carry no whitespace or
 * control characters (browsers drop tabs and newlines, so `/\t/evil.com` becomes `//evil.com`)
 * and resolve to this origin.
 */
export function safeNext(raw: unknown): string {
  if (typeof raw !== 'string') return '/'
  // eslint-disable-next-line no-control-regex
  if (/[\s\u0000-\u001f\u007f]/.test(raw)) return '/'
  let decoded: string
  try {
    decoded = decodeURIComponent(raw)
  } catch {
    return '/'
  }
  for (const s of [raw, decoded]) {
    if (!s.startsWith('/') || s[1] === '/' || s[1] === '\\') return '/'
  }
  try {
    if (new URL(raw, BASE).origin !== BASE) return '/'
  } catch {
    return '/'
  }
  return raw
}

export interface Session {
  /** `config.auth_enabled`; false until the config has loaded. */
  authEnabled: boolean
  /** The signed-in user, once `auth/me` has answered. */
  username: string | undefined
}

export function useSession(): Session {
  const config = useQuery(api.config())
  const authEnabled = config.data?.auth_enabled === true
  const me = useQuery({ ...api.me(), enabled: authEnabled })
  return { authEnabled, username: me.data?.username }
}

/**
 * Logs out, then leaves the shell for /login and drops every cached query. The user is
 * forgotten first, so /login does not send a still-cached user straight back; the rest is
 * cleared after the shell has gone, so no mounted query refetches without a session. If the
 * logout request fails the user stays signed in and where they are; the error propagates for
 * the caller to show.
 */
export async function signOut(queryClient: QueryClient, navigate: UseNavigateResult<string>): Promise<void> {
  await authApi.logout()
  await queryClient.cancelQueries()
  queryClient.removeQueries({ queryKey: api.me().queryKey })
  await navigate({ to: '/login' })
  queryClient.clear()
}

/** What the user menu and the palette say when logout fails. */
export const SIGN_OUT_FAILED = "Couldn't sign out, try again"

/** How long the session guard waits for `/config` or `auth/me` before letting the page load. */
export const GUARD_TIMEOUT_MS = 3_000

/** `p`, or a rejection once `ms` have passed. */
export function withTimeout<T>(p: Promise<T>, ms: number): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error(`timed out after ${ms} ms`)), ms)
  })
  return Promise.race([p, timeout]).finally(() => clearTimeout(timer))
}

let sessionLostHandler: (() => void) | null = null

/** The router registers the redirect to /login here (see createAppRouter). */
export function setSessionLostHandler(fn: (() => void) | null): void {
  sessionLostHandler = fn
}

/** Called by the query cache for a 401 from any query but the guard's own `auth/me`. */
export function sessionLost(): void {
  sessionLostHandler?.()
}

/**
 * Wraps `redirect` so that a burst of 401s (every query on the page failing together) runs it
 * once. `redirect` returns undefined when there is nothing to do (already on /login).
 */
export function singleFlight(redirect: () => Promise<unknown> | undefined): () => void {
  let pending = false
  return () => {
    if (pending) return
    const p = redirect()
    if (!p) return
    pending = true
    void p.finally(() => {
      pending = false
    })
  }
}
