/**
 * Layout for the home page's static service-map preview: services in columns by call depth
 * (hops from an entry service), ordered within a column by their callers' positions.
 * The interactive map (React Flow + ELK) is the /map page's job.
 */
import type { EdgeView, Health, ServiceMapView } from '../../api/types'

export interface MiniNode {
  service: string
  health: Health
  depth: number
  x: number
  y: number
}

export interface MiniEdge {
  parent: string
  child: string
  /** Shown red, dashed and flowing. */
  failing: boolean
  /** SVG path between the two node circles. */
  d: string
}

export interface MiniMapLayout {
  nodes: MiniNode[]
  edges: MiniEdge[]
  width: number
  height: number
}

export const MINI = { r: 9, colGap: 132, rowGap: 40, padX: 66, padY: 22, labelGap: 22 } as const

/** An edge fails when at least 1 % of its calls returned an error. */
export const FAILING_EDGE_RATE = 0.01

/** Every service: nodes plus services seen only as callers or callees, by name. */
export function servicesOf(map: ServiceMapView): string[] {
  const s = new Set(map.nodes.map((n) => n.service))
  for (const e of map.edges) {
    s.add(e.parent)
    s.add(e.child)
  }
  return [...s].sort()
}

export function isFailingEdge(e: Pick<EdgeView, 'errors' | 'error_rate'>): boolean {
  return e.errors > 0 && e.error_rate >= FAILING_EDGE_RATE
}

/**
 * Depth of every service: entry services (no callers other than themselves) are 0, others
 * the fewest hops from one. A cycle with no entry is entered at its alphabetically first
 * service, so every service gets a depth.
 */
export function callDepths(services: readonly string[], edges: readonly Pick<EdgeView, 'parent' | 'child'>[]): Map<string, number> {
  const out = new Map<string, string[]>()
  const indeg = new Map<string, number>(services.map((s) => [s, 0]))
  for (const e of edges) {
    if (e.parent === e.child) continue
    out.set(e.parent, [...(out.get(e.parent) ?? []), e.child])
    indeg.set(e.child, (indeg.get(e.child) ?? 0) + 1)
  }
  const depth = new Map<string, number>()
  const bfs = (starts: string[]) => {
    const queue = [...starts]
    for (const s of starts) depth.set(s, 0)
    for (let i = 0; i < queue.length; i++) {
      const s = queue[i] as string
      for (const c of out.get(s) ?? []) {
        if (depth.has(c)) continue
        depth.set(c, (depth.get(s) ?? 0) + 1)
        queue.push(c)
      }
    }
  }
  const sorted = [...services].sort()
  bfs(sorted.filter((s) => (indeg.get(s) ?? 0) === 0))
  for (const s of sorted) if (!depth.has(s)) bfs([s])
  return depth
}

function edgePath(a: MiniNode, b: MiniNode): string {
  const r = MINI.r
  if (b.x > a.x) {
    const x1 = a.x + r
    const x2 = b.x - r
    const mx = (x1 + x2) / 2
    return `M${x1} ${a.y}C${mx} ${a.y} ${mx} ${b.y} ${x2} ${b.y}`
  }
  // Same column or a call back to a shallower service: bow out to the right.
  const cx = Math.max(a.x, b.x) + MINI.colGap * 0.45
  return `M${a.x + r} ${a.y}Q${cx} ${(a.y + b.y) / 2} ${b.x + r} ${b.y}`
}

export function layoutMiniMap(map: ServiceMapView): MiniMapLayout {
  const health = new Map<string, Health>(map.nodes.map((n) => [n.service, n.health]))
  for (const e of map.edges) {
    if (!health.has(e.parent)) health.set(e.parent, 'ok')
    if (!health.has(e.child)) health.set(e.child, 'ok')
  }
  const services = [...health.keys()]
  if (services.length === 0) return { nodes: [], edges: [], width: 0, height: 0 }
  const depths = callDepths(services, map.edges)
  const columns: string[][] = []
  for (const s of services) {
    const d = depths.get(s) ?? 0
    ;(columns[d] ??= []).push(s)
  }
  const callers = new Map<string, string[]>()
  for (const e of map.edges) if (e.parent !== e.child) callers.set(e.child, [...(callers.get(e.child) ?? []), e.parent])

  // Order each column by the mean row of its callers in earlier columns (barycenter), then name.
  const row = new Map<string, number>()
  const tallest = Math.max(...columns.map((c) => c?.length ?? 0))
  columns.forEach((col = [], d) => {
    const key = (s: string) => {
      const rows = (callers.get(s) ?? []).filter((p) => (depths.get(p) ?? 0) < d).map((p) => row.get(p) ?? 0)
      return rows.length ? rows.reduce((a, b) => a + b, 0) / rows.length : Number.POSITIVE_INFINITY
    }
    col.sort((a, b) => key(a) - key(b) || a.localeCompare(b))
    // Centre shorter columns; `row` is in units of the tallest column's rows.
    const offset = (tallest - col.length) / 2
    col.forEach((s, i) => row.set(s, offset + i))
  })

  const nodes: MiniNode[] = services
    .map((s) => ({
      service: s,
      health: health.get(s) ?? 'ok',
      depth: depths.get(s) ?? 0,
      x: MINI.padX + (depths.get(s) ?? 0) * MINI.colGap,
      y: MINI.padY + (row.get(s) ?? 0) * MINI.rowGap,
    }))
    .sort((a, b) => a.depth - b.depth || a.y - b.y)
  const byName = new Map(nodes.map((n) => [n.service, n]))
  const edges: MiniEdge[] = map.edges
    .filter((e) => e.parent !== e.child)
    .map((e) => {
      const a = byName.get(e.parent) as MiniNode
      const b = byName.get(e.child) as MiniNode
      return { parent: e.parent, child: e.child, failing: isFailingEdge(e), d: edgePath(a, b) }
    })
    // Failing edges last, so they draw on top.
    .sort((a, b) => Number(a.failing) - Number(b.failing))
  return {
    nodes,
    edges,
    width: MINI.padX * 2 + (columns.length - 1) * MINI.colGap,
    height: MINI.padY + (tallest - 1) * MINI.rowGap + MINI.labelGap + MINI.padY,
  }
}
