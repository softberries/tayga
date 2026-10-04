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
import { HEX32, U64, validateHomeSearch, validateMapSearch, validateRootSearch, validateStorySearch, validateTraceSearch } from './app/search'
import type { RootSearch } from './app/search'
import { AppShell } from './components/shell/AppShell'
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

type Pages = typeof import('./pages/placeholders')

/** A route component in its own chunk, loaded on first visit. */
const page = (name: keyof Pages) => lazyRouteComponent(() => import('./pages/placeholders'), name)

const rootRoute = createRootRouteWithContext<RouterContext>()({
  validateSearch: (s: Record<string, unknown>): RootSearch => validateRootSearch(s),
  search: { middlewares: [retainSearchParams<RootSearch>(['since'])] },
  component: AppShell,
  notFoundComponent: NotFound,
  errorComponent: RouteError,
})

const storiesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/',
  staticData: { crumb: 'Stories' },
  validateSearch: validateHomeSearch,
  component: lazyRouteComponent(() => import('./routes/stories/index'), 'StoriesHome'),
})

const storyRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: 'stories/$storyId',
  staticData: { crumb: 'Story', crumbParam: 'storyId' },
  validateSearch: validateStorySearch,
  beforeLoad: ({ params }) => {
    if (!HEX32.test(params.storyId)) throw notFound()
  },
  component: lazyRouteComponent(() => import('./routes/stories/$id'), 'StoryPage'),
})

const tracesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: 'traces',
  staticData: { crumb: 'Traces' },
})

const tracesIndexRoute = createRoute({
  getParentRoute: () => tracesRoute,
  path: '/',
  component: page('TracesPage'),
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
  getParentRoute: () => rootRoute,
  path: 'map',
  staticData: { crumb: 'Service map' },
  validateSearch: validateMapSearch,
  component: page('MapPage'),
})

const logsRoute = createRoute({
  getParentRoute: () => rootRoute,
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
  component: page('LogAlertsPage'),
})

const logTemplatesRoute = createRoute({
  getParentRoute: () => logsRoute,
  path: 'templates',
  staticData: { crumb: 'Templates' },
})

const logTemplatesIndexRoute = createRoute({
  getParentRoute: () => logTemplatesRoute,
  path: '/',
  component: page('LogTemplatesPage'),
})

const logTemplateRoute = createRoute({
  getParentRoute: () => logTemplatesRoute,
  path: '$templateId',
  staticData: { crumb: 'Template', crumbParam: 'templateId' },
  beforeLoad: ({ params }) => {
    if (!U64.test(params.templateId)) throw notFound()
  },
  component: page('LogTemplatePage'),
})

const pipelineRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: 'pipeline',
  staticData: { crumb: 'Pipeline' },
  component: page('PipelinePage'),
})

export const routeTree = rootRoute.addChildren([
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
])

/** `history` defaults to the browser; tests pass a memory history. */
export function createAppRouter(queryClient: QueryClient, history?: RouterHistory) {
  return createRouter({
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
}

declare module '@tanstack/react-router' {
  interface Register {
    router: ReturnType<typeof createAppRouter>
  }
}
