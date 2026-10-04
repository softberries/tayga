import { RouterProvider, createMemoryHistory, createRootRoute, createRouter } from '@tanstack/react-router'
import { act, render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { LazyMotion, domAnimation } from 'motion/react'
import type { ReactNode } from 'react'
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest'
import trace from '../../api/__fixtures__/trace-error-payment.json'
import story from '../../api/__fixtures__/story-error-payment.json'
import type { TraceSpan } from '../../api/types'
import { Waterfall } from './Waterfall'

const spans = trace.spans as TraceSpan[]
const RC = story.root_cause.span_id
const critical = story.critical_path.segments.map((s) => s.span_id)

/** Waterfall links need a router; a single root route renders the subject. */
function renderInRouter(ui: ReactNode) {
  const router = createRouter({
    routeTree: createRootRoute({ component: () => <LazyMotion features={domAnimation}>{ui}</LazyMotion> }),
    history: createMemoryHistory({ initialEntries: ['/'] }),
  })
  return render(<RouterProvider router={router} />)
}

const sizes = ['offsetHeight', 'offsetWidth', 'clientHeight', 'clientWidth'] as const
beforeAll(() => {
  for (const p of sizes) Object.defineProperty(HTMLElement.prototype, p, { configurable: true, get: () => 4000 })
})
afterAll(() => {
  for (const p of sizes) delete (HTMLElement.prototype as unknown as Record<string, unknown>)[p]
})

const small: TraceSpan[] = [
  { ...spans[0]!, span_id: 'a', parent_span_id: '', span_name: 'root', start_ns: 0, duration_ns: 100, status: 'unset' },
  { ...spans[0]!, span_id: 'b', parent_span_id: 'a', span_name: 'child-b', start_ns: 10, duration_ns: 50, status: 'unset' },
  { ...spans[0]!, span_id: 'c', parent_span_id: 'b', span_name: 'leaf-c', start_ns: 20, duration_ns: 10, status: 'error' },
  { ...spans[0]!, span_id: 'd', parent_span_id: 'a', span_name: 'child-d', start_ns: 70, duration_ns: 20, status: 'unset' },
]
const names = () => screen.getAllByRole('treeitem').map((r) => r.getAttribute('data-span-id'))

describe('Waterfall (full)', () => {
  it('collapses and expands with the keyboard and opens with Enter', async () => {
    const user = userEvent.setup()
    const onOpen = vi.fn()
    renderInRouter(<Waterfall spans={small} onOpen={onOpen} />)
    const tree = await screen.findByRole('tree')
    expect(names()).toEqual(['a', 'b', 'c', 'd'])
    act(() => tree.focus())
    expect(screen.getByRole('treeitem', { selected: true })).toHaveAttribute('data-span-id', 'a')
    await user.keyboard('{ArrowDown}{ArrowLeft}')
    expect(names()).toEqual(['a', 'b', 'd'])
    expect(screen.getByRole('treeitem', { selected: true })).toHaveAttribute('aria-expanded', 'false')
    await user.keyboard('{ArrowRight}')
    expect(names()).toEqual(['a', 'b', 'c', 'd'])
    await user.keyboard('{ArrowRight}{Enter}')
    expect(onOpen).toHaveBeenCalledWith('c')
    await user.keyboard('{ArrowLeft}')
    expect(screen.getByRole('treeitem', { selected: true })).toHaveAttribute('data-span-id', 'b')
  })

  it('filters to errors with their ancestors and searches', async () => {
    const user = userEvent.setup()
    renderInRouter(<Waterfall spans={small} />)
    await user.click(await screen.findByRole('radio', { name: 'Errors' }))
    expect(names()).toEqual(['a', 'b', 'c'])
    expect(screen.getByText('1 of 4 spans')).toBeInTheDocument()
    await user.click(screen.getByRole('radio', { name: 'All spans' }))
    await user.type(screen.getByRole('searchbox', { name: 'Search spans' }), 'child-d')
    expect(await screen.findByText('1 of 4 spans')).toBeInTheDocument()
    expect(names()).toEqual(['a', 'd'])
    await user.clear(screen.getByRole('searchbox', { name: 'Search spans' }))
    await user.type(screen.getByRole('searchbox', { name: 'Search spans' }), 'zzz-nothing')
    expect(await screen.findByText('No spans match')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Clear filters' }))
    expect(names()).toEqual(['a', 'b', 'c', 'd'])
  })

  it('collapse all leaves the roots, expand all restores', async () => {
    const user = userEvent.setup()
    renderInRouter(<Waterfall spans={small} />)
    await user.click(await screen.findByRole('button', { name: 'Collapse all' }))
    expect(names()).toEqual(['a'])
    await user.click(screen.getByRole('button', { name: 'Expand all' }))
    expect(names()).toEqual(['a', 'b', 'c', 'd'])
  })

  it('marks critical, error and root-cause rows', async () => {
    renderInRouter(<Waterfall spans={spans} critical={critical} rootCauseId={RC} />)
    const rc = (await screen.findAllByRole('treeitem')).find((r) => r.getAttribute('data-span-id') === RC)!
    expect(rc).toHaveAttribute('data-root-cause', 'true')
    expect(within(rc).getByLabelText('error')).toBeInTheDocument()
  })

  it('shows an empty state without spans', async () => {
    renderInRouter(<Waterfall spans={[]} />)
    expect(await screen.findByText('No spans in this trace')).toBeInTheDocument()
  })
})

describe('Waterfall (compact)', () => {
  it('shows at most 12 rows, keeps the root cause and links to the trace', async () => {
    const onOpen = vi.fn()
    const user = userEvent.setup()
    renderInRouter(
      <Waterfall compact spans={spans} critical={critical} rootCauseId={RC} traceId={trace.trace_id} onOpen={onOpen} />,
    )
    const group = await screen.findByRole('group', { name: 'Trace waterfall (summary)' })
    const rows = within(group).getAllByRole('button')
    expect(rows).toHaveLength(12)
    expect(rows.some((r) => r.getAttribute('data-span-id') === RC)).toBe(true)
    expect(screen.queryByRole('tree')).not.toBeInTheDocument()
    expect(screen.queryByRole('searchbox')).not.toBeInTheDocument()
    const link = screen.getByRole('link', { name: `Show all ${spans.length} spans` })
    expect(link).toHaveAttribute('href', `/traces/${trace.trace_id}?span=${RC}`)
    await user.click(rows.find((r) => r.getAttribute('data-span-id') === RC)!)
    expect(onOpen).toHaveBeenCalledWith(RC)
  })

  it('honours maxRows and omits the link when everything fits', async () => {
    renderInRouter(<Waterfall compact spans={small} maxRows={12} traceId="x" />)
    await screen.findByRole('group')
    expect(screen.queryByRole('link')).not.toBeInTheDocument()
  })
})

describe('focusWindow', () => {
  it('zooms to the given spans with padding, or not at all when they cover most of the trace', async () => {
    const { buildLayout } = await import('./layout')
    const { focusWindow } = await import('./Waterfall')
    const l = buildLayout(small)
    const [from, to] = focusWindow(l, ['c'])
    expect(from).toBeCloseTo(0.2 - 0.004)
    expect(to).toBeCloseTo(0.3 + 0.004)
    expect(focusWindow(l, ['a'])).toEqual([0, 1])
    expect(focusWindow(l, ['nope'])).toEqual([0, 1])
    expect(focusWindow(l, undefined)).toEqual([0, 1])
    // A short root in a very long window zooms to its real extent, not the MIN_BAR-wide bar.
    const long = buildLayout([
      { ...small[0]!, span_id: 'r', duration_ns: 500e6, start_ns: 0 },
      { ...small[1]!, span_id: 's', parent_span_id: '', start_ns: 0, duration_ns: 2 * 86_400e9 },
    ])
    const [lf, lt] = focusWindow(long, ['r'])
    expect(lf).toBe(0)
    expect(lt * long.totalNs).toBeCloseTo(520e6, -6)
  })
})
