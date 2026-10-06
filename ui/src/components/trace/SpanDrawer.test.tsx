import { RouterProvider, createMemoryHistory, createRootRoute, createRouter } from '@tanstack/react-router'
import { render, screen } from '@testing-library/react'
import { LazyMotion, domAnimation } from 'motion/react'
import type { ReactNode } from 'react'
import { describe, expect, it } from 'vitest'
import trace from '../../api/__fixtures__/trace-error-payment.json'
import type { TraceSpan } from '../../api/types'
import { SpanDrawer } from './SpanDrawer'

function renderInRouter(ui: ReactNode) {
  const router = createRouter({
    routeTree: createRootRoute({ component: () => <LazyMotion features={domAnimation}>{ui}</LazyMotion> }),
    history: createMemoryHistory({ initialEntries: ['/'] }),
  })
  return render(<RouterProvider router={router} />)
}

describe('SpanDrawer', () => {
  it('shows the empty state and a zero count for a span without attributes', async () => {
    const span: TraceSpan = { ...(trace.spans[0] as TraceSpan), attrs: [] }
    renderInRouter(
      <SpanDrawer span={span} onClose={() => {}} traceStartNs={0} traceTotalNs={1_000_000} logs={[]} />,
    )
    const tab = await screen.findByRole('tab', { name: 'Attributes 0' })
    expect(tab).toHaveAttribute('aria-selected', 'true')
    expect(screen.getByText('No attributes on this span')).toBeInTheDocument()
  })
})
