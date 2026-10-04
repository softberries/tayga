/**
 * Waterfall layout, pure and framework-free. Mirrors `waterfall()` in
 * crates/tayga-api/src/svg.rs: depth-first rows with children by start time; spans that no
 * root reaches (parent cycles) are appended at depth 0, so every span appears exactly once.
 * Bars are fractions of the trace window in [0, 1].
 */
import type { TraceSpan } from '../../api/types'

/** Smallest bar width, as a fraction of the track (Rust: 0.3 %). */
export const MIN_BAR = 0.003

export interface LayoutRow {
  span: TraceSpan
  /** Position in `Layout.rows` (depth-first order). */
  index: number
  depth: number
  /** Row index of the tree parent, -1 for roots. */
  parent: number
  childCount: number
  /** One past the row index of the last descendant: the subtree is `[index, subtreeEnd)`. */
  subtreeEnd: number
  /** Bar start and width as fractions of the trace window. */
  left: number
  width: number
  /** Start relative to the trace start, in ns. */
  offsetNs: number
  critical: boolean
  error: boolean
  rootCause: boolean
}

export interface Layout {
  rows: LayoutRow[]
  /** span id → row index (the last span wins for duplicate ids, like the Rust index). */
  byId: Map<string, number>
  startNs: number
  /** Trace window length in ns, at least 1. */
  totalNs: number
}

export interface LayoutOptions {
  critical?: Iterable<string>
  rootCauseId?: string | null
}

const finite = (v: number, fallback = 0) => (Number.isFinite(v) ? v : fallback)
const startOf = (s: TraceSpan) => finite(s.start_ns)
const durOf = (s: TraceSpan) => Math.max(0, finite(s.duration_ns))
const endOf = (s: TraceSpan) => Math.min(startOf(s) + durOf(s), Number.MAX_VALUE)

/** Parent links resolved to indices: `children[i]` sorted by (start, input index). */
function tree(spans: readonly TraceSpan[]) {
  const index = new Map<string, number>()
  spans.forEach((s, i) => index.set(s.span_id, i))
  const children: number[][] = spans.map(() => [])
  const roots: number[] = []
  spans.forEach((s, i) => {
    const p = index.get(s.parent_span_id)
    if (p !== undefined && p !== i) children[p]!.push(i)
    else roots.push(i)
  })
  const byStart = (a: number, b: number) => startOf(spans[a]!) - startOf(spans[b]!) || a - b
  for (const c of children) if (c.length > 1) c.sort(byStart)
  roots.sort(byStart)
  return { index, children, roots }
}

export function buildLayout(spans: readonly TraceSpan[], opts: LayoutOptions = {}): Layout {
  const critical = new Set(opts.critical ?? [])
  const rc = opts.rootCauseId ?? ''
  if (spans.length === 0) return { rows: [], byId: new Map(), startNs: 0, totalNs: 1 }

  const { children, roots } = tree(spans)
  let startNs = Infinity
  let endNs = -Infinity
  for (const s of spans) {
    startNs = Math.min(startNs, startOf(s))
    endNs = Math.max(endNs, endOf(s))
  }
  let totalNs = endNs - startNs
  if (!Number.isFinite(totalNs)) totalNs = Number.MAX_VALUE
  totalNs = Math.max(1, totalNs)

  const rows: LayoutRow[] = []
  const byId = new Map<string, number>()
  const visited = new Uint8Array(spans.length)
  // Explicit stack: deep traces must not overflow the call stack.
  const stack: Array<[span: number, depth: number, parentRow: number]> = []
  const walk = (root: number) => {
    stack.push([root, 0, -1])
    while (stack.length > 0) {
      const [i, depth, parent] = stack.pop()!
      if (visited[i]) continue
      visited[i] = 1
      const s = spans[i]!
      const offsetNs = Math.max(0, startOf(s) - startNs)
      const left = Math.min(1, finite(offsetNs / totalNs))
      const width = Math.min(1, Math.max(MIN_BAR, finite(durOf(s) / totalNs)))
      const row: LayoutRow = {
        span: s,
        index: rows.length,
        depth,
        parent,
        childCount: 0,
        subtreeEnd: rows.length + 1,
        // Keep the whole bar on the track: a bar that would overflow moves left.
        left: Math.min(left, 1 - width),
        width,
        offsetNs,
        critical: critical.has(s.span_id),
        error: s.status === 'error',
        rootCause: rc !== '' && s.span_id === rc,
      }
      if (parent >= 0) rows[parent]!.childCount++
      byId.set(s.span_id, row.index)
      rows.push(row)
      const kids = children[i]!
      for (let k = kids.length - 1; k >= 0; k--) stack.push([kids[k]!, depth + 1, row.index])
    }
  }
  for (const r of roots) walk(r)
  for (let i = 0; i < spans.length; i++) if (!visited[i]) walk(i)

  // Subtrees are contiguous in DFS order; propagate each row's end to its parent.
  for (let i = rows.length - 1; i >= 0; i--) {
    const r = rows[i]!
    if (r.parent >= 0) {
      const p = rows[r.parent]!
      p.subtreeEnd = Math.max(p.subtreeEnd, r.subtreeEnd)
    }
  }
  return { rows, byId, startNs, totalNs }
}

export type RowFilter = 'all' | 'errors' | 'critical'

export interface ViewOptions {
  /** Span ids whose descendants are hidden. Ignored while a filter or search is active. */
  collapsed?: ReadonlySet<string>
  filter?: RowFilter
  query?: string
}

export interface VisibleRow extends LayoutRow {
  /** False for ancestors shown only as context for a filter or search match. */
  match: boolean
}

function matchesQuery(s: TraceSpan, q: string): boolean {
  if (s.service_name.toLowerCase().includes(q) || s.span_name.toLowerCase().includes(q)) return true
  for (const [k, v] of s.attrs) if (k.toLowerCase().includes(q) || v.toLowerCase().includes(q)) return true
  return false
}

/**
 * Rows to render. Without a filter or search: the tree minus collapsed subtrees. With one:
 * matching rows plus their ancestors (as context, `match: false`), regardless of collapse.
 */
export function visibleRows(layout: Layout, opts: ViewOptions = {}): VisibleRow[] {
  const { rows } = layout
  const filter = opts.filter ?? 'all'
  const q = (opts.query ?? '').trim().toLowerCase()
  if (filter === 'all' && q === '') {
    const collapsed = opts.collapsed
    const out: VisibleRow[] = []
    for (let i = 0; i < rows.length; ) {
      const r = rows[i]!
      out.push({ ...r, match: true })
      i = collapsed?.has(r.span.span_id) ? r.subtreeEnd : i + 1
    }
    return out
  }
  const keep = new Uint8Array(rows.length) // 1 = context, 2 = match
  for (const r of rows) {
    const ok =
      (filter === 'all' || (filter === 'errors' ? r.error : r.critical)) && (q === '' || matchesQuery(r.span, q))
    if (!ok) continue
    keep[r.index] = 2
    for (let p = r.parent; p >= 0 && keep[p] === 0; p = rows[p]!.parent) keep[p] = 1
  }
  const out: VisibleRow[] = []
  for (const r of rows) if (keep[r.index]) out.push({ ...r, match: keep[r.index] === 2 })
  return out
}

/**
 * Critical path when no story supplies one: from the longest root, repeatedly take the
 * child that finishes last before the current cursor (children clamped to the parent's end),
 * descend into it, then continue with children that finish before it started.
 */
export function computeCriticalPath(spans: readonly TraceSpan[]): Set<string> {
  const out = new Set<string>()
  if (spans.length === 0) return out
  const { children, roots } = tree(spans)
  let root = roots[0]
  for (const r of roots) if (durOf(spans[r]!) > durOf(spans[root!]!)) root = r
  if (root === undefined) return out // every span sits in a cycle
  const byEndDesc = children.map((c) => (c.length > 1 ? [...c].sort((a, b) => endOf(spans[b]!) - endOf(spans[a]!)) : c))
  const seen = new Uint8Array(spans.length)
  const frames: Array<{ i: number; bound: number; k: number }> = [{ i: root, bound: endOf(spans[root]!), k: 0 }]
  seen[root] = 1
  out.add(spans[root]!.span_id)
  while (frames.length > 0) {
    const f = frames[frames.length - 1]!
    const kids = byEndDesc[f.i]!
    const parentEnd = endOf(spans[f.i]!)
    while (f.k < kids.length && (seen[kids[f.k]!] || Math.min(endOf(spans[kids[f.k]!]!), parentEnd) > f.bound)) f.k++
    if (f.k >= kids.length) {
      frames.pop()
      continue
    }
    const c = kids[f.k++]!
    seen[c] = 1
    out.add(spans[c]!.span_id)
    const childBound = Math.min(endOf(spans[c]!), parentEnd)
    f.bound = startOf(spans[c]!)
    frames.push({ i: c, bound: childBound, k: 0 })
  }
  return out
}

/**
 * At most `max` rows for the compact inspector waterfall, in tree order. Priority: the root
 * cause and its ancestors, then critical spans, then errors, then the longest spans.
 */
export function compactRows(layout: Layout, max: number): LayoutRow[] {
  const { rows } = layout
  if (rows.length <= max) return rows
  const tier = new Uint8Array(rows.length)
  for (const r of rows) tier[r.index] = r.critical ? 2 : r.error ? 1 : 0
  const rc = rows.find((r) => r.rootCause)
  for (let p = rc ? rc.index : -1; p >= 0; p = rows[p]!.parent) tier[p] = 3
  const picked = rows
    .map((r) => r.index)
    .sort((a, b) => tier[b]! - tier[a]! || rows[b]!.width - rows[a]!.width || a - b)
    .slice(0, max)
    .sort((a, b) => a - b)
  return picked.map((i) => rows[i]!)
}

/** A bar re-projected into the zoom window `[from, to]` (fractions of the trace). */
export function projectBar(left: number, width: number, view: readonly [number, number]) {
  const [from, to] = view
  const span = Math.max(1e-9, to - from)
  const a = (left - from) / span
  const b = (left + width - from) / span
  if (b < 0 || a > 1) return { left: 0, width: 0, clipped: true }
  const l = Math.max(0, a)
  const r = Math.min(1, b)
  return { left: l, width: Math.max(MIN_BAR, r - l), clipped: false }
}

/** Round tick values (ns) inside `[from, to]`, about `count` intervals apart. */
export function niceTicks(from: number, to: number, count: number): number[] {
  const range = to - from
  if (!(range > 0) || !Number.isFinite(range)) return [from]
  const raw = range / Math.max(1, count)
  const pow = 10 ** Math.floor(Math.log10(raw))
  const step = ([1, 2, 2.5, 5, 10].map((m) => m * pow).find((s) => s >= raw) ?? 10 * pow)
  const out: number[] = []
  for (let v = Math.ceil(from / step) * step; v <= to + step * 1e-9; v += step) out.push(Math.round(v * 1e3) / 1e3)
  return out
}
