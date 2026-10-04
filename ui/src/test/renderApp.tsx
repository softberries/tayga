import { QueryClientProvider } from '@tanstack/react-query'
import { RouterProvider, createMemoryHistory, createRouter } from '@tanstack/react-router'
import { render } from '@testing-library/react'
import { MotionConfig } from 'motion/react'
import { vi } from 'vitest'
import { LiveProvider } from '../app/live'
import { createQueryClient } from '../app/queryClient'
import { TooltipProvider } from '../components/ui/Tooltip'
import { routeTree } from '../router'
import { ThemeProvider } from '../theme/ThemeProvider'

/** Responses by API path (without /api/v1 and query); unknown paths return 404. */
export type Routes = Record<string, { status?: number; body: unknown }>

export function stubApi(routes: Routes) {
  const fetch = vi.fn(async (input: string) => {
    const url = new URL(input, 'http://test')
    const path = url.pathname.replace(/^\/api\/v1/, '')
    const r = routes[path] ?? { status: 404, body: { error: 'not found' } }
    return new Response(JSON.stringify(r.body), {
      status: r.status ?? 200,
      headers: { 'content-type': 'application/json' },
    })
  })
  vi.stubGlobal('fetch', fetch)
  return fetch
}

/** Renders the real route tree at `url` with the app providers. */
export function renderApp(url: string) {
  const queryClient = createQueryClient()
  queryClient.setDefaultOptions({ queries: { ...queryClient.getDefaultOptions().queries, retry: false } })
  const history = createMemoryHistory({ initialEntries: [url] })
  const router = createRouter({ routeTree, history, context: { queryClient } })
  const utils = render(
    <ThemeProvider>
      <MotionConfig reducedMotion="always">
        <QueryClientProvider client={queryClient}>
          <LiveProvider>
            <TooltipProvider delayDuration={0}>
              <RouterProvider router={router} />
            </TooltipProvider>
          </LiveProvider>
        </QueryClientProvider>
      </MotionConfig>
    </ThemeProvider>,
  )
  return { ...utils, router, history, queryClient }
}
