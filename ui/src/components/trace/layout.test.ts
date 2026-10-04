import { describe, expect, it } from 'vitest'
import type { TraceSpan } from '../../api/types'
import {
  MIN_BAR,
  buildLayout,
  compactRows,
  computeCriticalPath,
  niceTicks,
  projectBar,
  visibleRows,
} from './layout'

function span(id: string, parent: string, start: number, dur: number, extra: Partial<TraceSpan> = {}): TraceSpan {
  return {
    span_id: id,
    parent_span_id: parent,
    service_name: 'svc',
    span_name: `op-${id}`,
    kind: 'server',
    start_ns: start,
    duration_ns: dur,
    status: id === 'c' ? 'error' : 'unset',
    status_message: '',
    attrs: [],
    resource: [],
    events: [],
    self_ns: dur,
    ...extra,
  }
}

const ids = (rows: { span: TraceSpan; depth: number }[]) => rows.map((r) => [r.span.span_id, r.depth])

describe('buildLayout (mirrors crates/tayga-api/src/svg.rs waterfall)', () => {
  it('orders depth first with children by start time and marks spans', () => {
    const spans = [span('b', 'a', 10, 30), span('a', '', 0, 100), span('c', 'b', 15, 10), span('d', 'a', 50, 40)]
    const l = buildLayout(spans, { critical: ['a', 'd'], rootCauseId: 'c' })
    expect(ids(l.rows)).toEqual([
      ['a', 0],
      ['b', 1],
      ['c', 2],
      ['d', 1],
    ])
    expect(l.rows.map((r) => r.critical)).toEqual([true, false, false, true])
    expect(l.rows[2]!.rootCause && l.rows[2]!.error).toBe(true)
    expect(l.rows[0]!.left).toBe(0)
    expect(l.rows[0]!.width).toBe(1)
    expect(l.rows[1]!.left).toBeCloseTo(0.1)
    expect(l.rows[1]!.width).toBeCloseTo(0.3)
    expect(l.totalNs).toBe(100)
  })

  it('records parents, child counts and subtree ends for collapse', () => {
    const spans = [span('b', 'a', 10, 30), span('a', '', 0, 100), span('c', 'b', 15, 10), span('d', 'a', 50, 40)]
    const l = buildLayout(spans)
    expect(l.rows.map((r) => r.parent)).toEqual([-1, 0, 1, 0])
    expect(l.rows.map((r) => r.childCount)).toEqual([2, 1, 0, 0])
    // subtreeEnd: index one past the last descendant.
    expect(l.rows.map((r) => r.subtreeEnd)).toEqual([4, 3, 3, 4])
    expect(l.byId.get('c')).toBe(2)
  })

  it('breaks start-time ties by input order, like the Rust sort on (start, index)', () => {
    const spans = [span('r', '', 0, 10), span('y', 'r', 5, 1), span('x', 'r', 5, 1)]
    expect(ids(buildLayout(spans).rows)).toEqual([
      ['r', 0],
      ['y', 1],
      ['x', 1],
    ])
  })

  it('handles cycles, orphans and self-parents: every span exactly once', () => {
    const spans = [span('x', 'y', 0, 10), span('y', 'x', 5, 10), span('o', 'missing', 1, 1), span('s', 's', 2, 1)]
    const l = buildLayout(spans)
    expect(l.rows).toHaveLength(4)
    expect(l.rows.map((r) => r.span.span_id).sort()).toEqual(['o', 's', 'x', 'y'])
    // Orphan and self-parent are roots; the x<->y cycle is appended at depth 0.
    expect(ids(l.rows)).toEqual([
      ['o', 0],
      ['s', 0],
      ['x', 0],
      ['y', 1],
    ])
  })

  it('keeps duplicate span ids once each and never loops', () => {
    const spans = [span('a', '', 0, 10), span('a', '', 1, 5), span('b', 'a', 2, 2)]
    const l = buildLayout(spans)
    expect(l.rows).toHaveLength(3)
  })

  it('extreme timestamps stay finite and inside [0, 1]', () => {
    const spans = [
      span('min', '', -Number.MAX_SAFE_INTEGER, 100),
      span('max', '', Number.MAX_VALUE, 100),
      span('large_dur', '', 0, Number.MAX_VALUE),
      span('nan', '', Number.NaN, Number.NaN),
      span('neg', '', 5, -50),
    ]
    const l = buildLayout(spans)
    expect(l.rows).toHaveLength(5)
    for (const r of l.rows) {
      expect(Number.isFinite(r.left), r.span.span_id).toBe(true)
      expect(Number.isFinite(r.width), r.span.span_id).toBe(true)
      expect(r.left).toBeGreaterThanOrEqual(0)
      expect(r.width).toBeGreaterThanOrEqual(MIN_BAR)
      expect(r.left + r.width).toBeLessThanOrEqual(1 + 1e-12)
    }
    expect(Number.isFinite(l.totalNs)).toBe(true)
  })

  it('gives zero-length spans the minimum width, still inside the track', () => {
    const l = buildLayout([span('a', '', 0, 100), span('z', 'a', 100, 0)])
    const z = l.rows[1]!
    expect(z.width).toBe(MIN_BAR)
    expect(z.left + z.width).toBeLessThanOrEqual(1)
  })

  it('empty input gives an empty layout', () => {
    const l = buildLayout([])
    expect(l.rows).toEqual([])
    expect(l.totalNs).toBe(1)
  })

  it('lays out 10,000 spans in under 50 ms', () => {
    const spans: TraceSpan[] = []
    for (let i = 0; i < 10_000; i++) {
      const parent = i === 0 ? '' : `s${Math.floor((i - 1) / 4)}`
      spans.push(
        span(`s${i}`, parent, 1.79e18 + i * 1000, 5000 + (i % 7) * 100, {
          attrs: [
            ['http.route', `/r/${i}`],
            ['k', 'v'],
          ],
        }),
      )
    }
    // Warm-up run: the bound is about steady-state cost, not JIT start.
    buildLayout(spans, { critical: ['s0', 's1'], rootCauseId: 's9999' })
    const t0 = performance.now()
    const l = buildLayout(spans, { critical: ['s0', 's1'], rootCauseId: 's9999' })
    const ms = performance.now() - t0
    expect(l.rows).toHaveLength(10_000)
    expect(ms).toBeLessThan(50)
  })
})

describe('visibleRows', () => {
  const spans = [
    span('a', '', 0, 100, { service_name: 'frontend' }),
    span('b', 'a', 10, 30, { service_name: 'checkout', attrs: [['order.id', 'XYZ-42']] }),
    span('c', 'b', 15, 10, { service_name: 'payment' }),
    span('d', 'a', 50, 40, { service_name: 'shipping' }),
  ]
  const l = buildLayout(spans, { critical: ['a', 'd'] })

  it('hides descendants of collapsed rows', () => {
    expect(visibleRows(l, { collapsed: new Set(['b']) }).map((r) => r.span.span_id)).toEqual(['a', 'b', 'd'])
    expect(visibleRows(l, { collapsed: new Set(['a']) }).map((r) => r.span.span_id)).toEqual(['a'])
  })

  it('errors only keeps error spans and their ancestors, ancestors marked as context', () => {
    const v = visibleRows(l, { filter: 'errors' })
    expect(v.map((r) => [r.span.span_id, r.match])).toEqual([
      ['a', false],
      ['b', false],
      ['c', true],
    ])
  })

  it('critical only keeps critical spans', () => {
    expect(visibleRows(l, { filter: 'critical' }).map((r) => r.span.span_id)).toEqual(['a', 'd'])
  })

  it('searches service, name and attributes, case-insensitively, ignoring collapse', () => {
    expect(visibleRows(l, { query: 'xyz-42', collapsed: new Set(['a']) }).map((r) => [r.span.span_id, r.match])).toEqual([
      ['a', false],
      ['b', true],
    ])
    expect(visibleRows(l, { query: 'SHIPPING' }).map((r) => r.span.span_id)).toEqual(['a', 'd'])
    expect(visibleRows(l, { query: 'op-c' }).map((r) => r.span.span_id)).toEqual(['a', 'b', 'c'])
    expect(visibleRows(l, { query: 'nothing-like-this' })).toEqual([])
  })

  it('combines a filter with a search', () => {
    expect(visibleRows(l, { filter: 'critical', query: 'shipping' }).map((r) => r.span.span_id)).toEqual(['a', 'd'])
    expect(visibleRows(l, { filter: 'errors', query: 'shipping' })).toEqual([])
  })
})

describe('computeCriticalPath', () => {
  it('follows the last-finishing child, then earlier children that finish before it starts', () => {
    // a [0,100]: b [10,40], d [50,90], e [45, 60] overlaps d so is not critical.
    const spans = [
      span('a', '', 0, 100),
      span('b', 'a', 10, 30),
      span('c', 'b', 15, 10),
      span('d', 'a', 50, 40),
      span('e', 'a', 45, 15),
    ]
    expect([...computeCriticalPath(spans)].sort()).toEqual(['a', 'b', 'c', 'd'])
  })

  it('clamps children that end after their parent', () => {
    const spans = [span('a', '', 0, 100), span('late', 'a', 90, 50), span('early', 'a', 10, 20)]
    expect([...computeCriticalPath(spans)].sort()).toEqual(['a', 'early', 'late'])
  })

  it('uses the longest root and survives cycles', () => {
    const spans = [span('r1', '', 0, 10), span('r2', '', 0, 50), span('x', 'y', 1, 1), span('y', 'x', 2, 1)]
    expect([...computeCriticalPath(spans)]).toEqual(['r2'])
    expect(computeCriticalPath([]).size).toBe(0)
  })
})

describe('compactRows', () => {
  const spans = [span('root', '', 0, 1000)]
  for (let i = 0; i < 30; i++) spans.push(span(`n${i}`, 'root', i * 10, 5))
  spans.push(span('deep', 'n20', 205, 1, { status: 'error' }))
  const l = buildLayout(spans, { critical: ['root', 'n29'], rootCauseId: 'deep' })

  it('keeps at most max rows in tree order, root cause chain and critical spans first', () => {
    const rows = compactRows(l, 12)
    expect(rows).toHaveLength(12)
    const got = rows.map((r) => r.span.span_id)
    for (const must of ['root', 'n20', 'deep', 'n29']) expect(got).toContain(must)
    const order = rows.map((r) => l.byId.get(r.span.span_id)!)
    expect([...order].sort((a, b) => a - b)).toEqual(order)
  })

  it('returns every row when they fit', () => {
    const small = buildLayout([span('a', '', 0, 1), span('b', 'a', 0, 1)])
    expect(compactRows(small, 12)).toHaveLength(2)
  })
})

describe('projectBar (zoom)', () => {
  it('maps a bar into the zoom window and clips it', () => {
    const id = projectBar(0.5, 0.1, [0, 1])
    expect(id.clipped).toBe(false)
    expect(id.left).toBeCloseTo(0.5)
    expect(id.width).toBeCloseTo(0.1)
    const p = projectBar(0.5, 0.1, [0.5, 0.75])
    expect(p.left).toBeCloseTo(0)
    expect(p.width).toBeCloseTo(0.4)
    const out = projectBar(0.1, 0.1, [0.5, 1])
    expect(out.clipped).toBe(true)
    const partial = projectBar(0.4, 0.2, [0.5, 1])
    expect(partial.left).toBe(0)
    expect(partial.width).toBeCloseTo(0.2)
  })
})

describe('niceTicks', () => {
  it('picks round steps across the window', () => {
    expect(niceTicks(0, 100e6, 5)).toEqual([0, 20e6, 40e6, 60e6, 80e6, 100e6])
    const t = niceTicks(0, 65.1e6, 4)
    expect(t[0]).toBe(0)
    expect(t.every((v) => v <= 65.1e6)).toBe(true)
    expect(t.length).toBeGreaterThanOrEqual(3)
  })

  it('handles an offset window and degenerate input', () => {
    const t = niceTicks(12e6, 38e6, 4)
    expect(t[0]).toBeGreaterThanOrEqual(12e6)
    expect(t.at(-1)!).toBeLessThanOrEqual(38e6)
    expect(niceTicks(0, 0, 4)).toEqual([0])
  })
})
