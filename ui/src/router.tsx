import type { QueryClient } from '@tanstack/react-query'
import type { RouterHistory } from '@tanstack/react-router'
import {
  createRootRouteWithContext,
  createRoute,
  createRouter,
  lazyRouteComponent,
  notFound,
  redirect,
  retainSearchParams,
} from '@tanstack/react-router'
import { isApiError } from './api/client'
import { api } from './api/queries'
import { GUARD_TIMEOUT_MS, authEnabledInCache, safeNext, setSessionLostHandler, singleFlight, withTimeout } from './app/auth'
import { HEX32, U64, validateHomeSearch, validateLogAlertsSearch, validateLogTemplatesSearch, validateMapSearch, validateRootSearch, validateStorySearch, validateTraceSearch, validateTracesSearch } from './app/search'
import type { RootSearch } from './app/search'
import { AppShell } from './components/shell/AppShell'
import { AppPending } from './components/shell/AppPending'
import { NotFound } from './pages/NotFound'
import { RouteError } from './pages/RouteError'

export interface RouterContext {
  queryClient: QueryClient
}

declare module '@tanstack/react-router' {
  interface StaticDataRouteOption {
    /** Breadcrumb label for this route. */
    crumb?: string
    /** Path param appended (shortened) to the crumb, e.g. a trace id. */
    crumbParam?: string
  }
}

/** Not-found pages keep the rail and header. */
function ShellNotFound() {
  return (
    <AppShell>
      <NotFound />
    </AppShell>
  )
}

/** The API's config, or undefined when it fails or is slow: the guard then lets the page load. */
function guardConfig(queryClient: QueryClient) {
  return withTimeout(queryClient.ensureQueryData(api.config()), GUARD_TIMEOUT_MS).catch(() => undefined)
}

/**
 * Whether `auth/me` says there is a session. `unknown` (an error other than 401, or no answer
 * within the timeout) fails open: the API still guards its data, and the first query's 401
 * redirects then (createQueryClient).
 */
async function sessionState(queryClient: QueryClient): Promise<'signed-in' | 'signed-out' | 'unknown'> {
  try {
    await withTimeout(queryClient.ensureQueryData(api.me()), GUARD_TIMEOUT_MS)
    return 'signed-in'
  } catch (e) {
    return isApiError(e) && e.status === 401 ? 'signed-out' : 'unknown'
  }
}

/**
 * The root renders only its outlet. Under it sit the login page and `_shell`, a pathless
 * layout with the rail and header that owns every app page and the range search params (so
 * /login neither keeps nor shows `since`/`until`).
 *
 * The session guard lives here, so it covers every path but /login, unknown ones included:
 * with auth on and no session, it sends the user to /login with `next` set to where they were
 * going. While it waits (over a second), a skeleton of the shell shows instead of a blank page.
 */
const rootRoute = createRootRouteWithContext<RouterContext>()({
  beforeLoad: async ({ context: { queryClient }, location }) => {
    if (location.pathname === '/login') return
    const config = await guardConfig(queryClient)
    if (!config?.auth_enabled) return
    if ((await sessionState(queryClient)) === 'signed-out') throw redirect({ to: '/login', search: { next: location.href } })
  },
  pendingComponent: AppPending,
  notFoundComponent: ShellNotFound,
  errorComponent: RouteError,
})

const shellRoute = createRoute({
  getParentRoute: () => rootRoute,
  id: '_shell',
  validateSearch: (s: Record<string, unknown>): RootSearch => validateRootSearch(s),
  search: { middlewares: [retainSearchParams<RootSearch>(['since', 'until'])] },
  component: AppShell,
  // A not-found under the shell replaces the shell's own component, so it brings the shell.
  notFoundComponent: ShellNotFound,
  errorComponent: RouteError,
})

/**
 * Outside the shell: no rail, no header. When the loaded config says auth is off it only
 * redirects home; a user who is already signed in goes on to a safe `next`. With the config
 * unavailable it shows the form: redirecting home then could bounce between the two pages.
 */
const loginRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: 'login',
  validateSearch: (s: Record<string, unknown>): { next?: string } => (typeof s.next === 'string' ? { next: s.next } : {}),
  beforeLoad: async ({ context: { queryClient }, search }) => {
    const config = await guardConfig(queryClient)
    if (config?.auth_enabled === false) throw redirect({ to: '/', replace: true })
    if (config?.auth_enabled && (await sessionState(queryClient)) === 'signed-in') throw redirect({ href: safeNext(search.next), replace: true })
  },
  component: lazyRouteComponent(() => import('./routes/login'), 'LoginPage'),
})

const storiesRoute = createRoute({
  getParentRoute: () => shellRoute,
  path: '/',
  staticData: { crumb: 'Stories' },
  validateSearch: validateHomeSearch,
  component: lazyRouteComponent(() => import('./routes/stories/index'), 'StoriesHome'),
})

const storyRoute = createRoute({
  getParentRoute: () => shellRoute,
  path: 'stories/$storyId',
  staticData: { crumb: 'Story', crumbParam: 'storyId' },
  validateSearch: validateStorySearch,
  beforeLoad: ({ params }) => {
    if (!HEX32.test(params.storyId)) throw notFound()
  },
  component: lazyRouteComponent(() => import('./routes/stories/$id'), 'StoryPage'),
})

const tracesRoute = createRoute({
  getParentRoute: () => shellRoute,
  path: 'traces',
  staticData: { crumb: 'Traces' },
})

const tracesIndexRoute = createRoute({
  getParentRoute: () => tracesRoute,
  path: '/',
  validateSearch: validateTracesSearch,
  component: lazyRouteComponent(() => import('./routes/traces/index'), 'TracesExplorer'),
})

const traceRoute = createRoute({
  getParentRoute: () => tracesRoute,
  path: '$traceId',
  staticData: { crumb: 'Trace', crumbParam: 'traceId' },
  validateSearch: validateTraceSearch,
  beforeLoad: ({ params }) => {
    if (!HEX32.test(params.traceId)) throw notFound()
  },
  component: lazyRouteComponent(() => import('./routes/traces/$id'), 'TracePage'),
})

const mapRoute = createRoute({
  getParentRoute: () => shellRoute,
  path: 'map',
  staticData: { crumb: 'Service map' },
  validateSearch: validateMapSearch,
  component: lazyRouteComponent(() => import('./routes/map'), 'MapPage'),
})

const logsRoute = createRoute({
  getParentRoute: () => shellRoute,
  path: 'logs',
  staticData: { crumb: 'Logs' },
})

const logsIndexRoute = createRoute({
  getParentRoute: () => logsRoute,
  path: '/',
  beforeLoad: ({ search }) => {
    throw redirect({ to: '/logs/alerts', search, replace: true })
  },
})

const logAlertsRoute = createRoute({
  getParentRoute: () => logsRoute,
  path: 'alerts',
  staticData: { crumb: 'Alerts' },
  validateSearch: validateLogAlertsSearch,
  component: lazyRouteComponent(() => import('./routes/logs/alerts'), 'LogAlertsPage'),
})

const logTemplatesRoute = createRoute({
  getParentRoute: () => logsRoute,
  path: 'templates',
  staticData: { crumb: 'Templates' },
})

const logTemplatesIndexRoute = createRoute({
  getParentRoute: () => logTemplatesRoute,
  path: '/',
  validateSearch: validateLogTemplatesSearch,
  component: lazyRouteComponent(() => import('./routes/logs/templates'), 'LogTemplatesPage'),
})

const logTemplateRoute = createRoute({
  getParentRoute: () => logTemplatesRoute,
  path: '$templateId',
  staticData: { crumb: 'Template', crumbParam: 'templateId' },
  beforeLoad: ({ params }) => {
    if (!U64.test(params.templateId)) throw notFound()
  },
  component: lazyRouteComponent(() => import('./routes/logs/templates.$id'), 'LogTemplatePage'),
})

const pipelineRoute = createRoute({
  getParentRoute: () => shellRoute,
  path: 'pipeline',
  staticData: { crumb: 'Pipeline' },
  component: lazyRouteComponent(() => import('./routes/pipeline'), 'PipelinePage'),
})

export const routeTree = rootRoute.addChildren([
  loginRoute,
  shellRoute.addChildren([
    storiesRoute,
    storyRoute,
    tracesRoute.addChildren([tracesIndexRoute, traceRoute]),
    mapRoute,
    logsRoute.addChildren([
      logsIndexRoute,
      logAlertsRoute,
      logTemplatesRoute.addChildren([logTemplatesIndexRoute, logTemplateRoute]),
    ]),
    pipelineRoute,
  ]),
])

/** `history` defaults to the browser; tests pass a memory history. */
export function createAppRouter(queryClient: QueryClient, history?: RouterHistory) {
  const router = createRouter({
    routeTree,
    history,
    context: { queryClient },
    // Page errors render inside the shell's <Outlet>, so the rail and header stay usable.
    defaultErrorComponent: RouteError,
    defaultNotFoundComponent: NotFound,
    defaultPreload: 'intent',
    // Query owns caching; the router re-runs loaders whenever asked.
    defaultPreloadStaleTime: 0,
    scrollRestoration: true,
  })
  // A 401 mid-session with auth on (expired or revoked): forget the user, so the guard asks
  // again, and go to /login once, back to here after signing in.
  setSessionLostHandler(
    singleFlight(() => {
      const { pathname, href } = router.state.location
      // Only Tayga's own login can restore a session; without it /login redirects home.
      if (pathname === '/login' || !authEnabledInCache(queryClient)) return undefined
      queryClient.removeQueries({ queryKey: api.me().queryKey })
      return router.navigate({ to: '/login', search: { next: href } })
    }),
  )
  return router
}

declare module '@tanstack/react-router' {
  interface Register {
    router: ReturnType<typeof createAppRouter>
  }
}
