/**
 * Pure helpers for the /map page: the graph handed to ELK, edge width and tone, search
 * matching and each service's callers and callees.
 */
import type { EdgeView, Health, NodeView, ServiceMapView } from '../../api/types'
import type { Range } from '../../app/range'
import { compact, percent } from '../../lib/format'
import { isFailingEdge, servicesOf } from '../stories/mapLayout'
import type { MapGraph, Pos } from './layout'

export { servicesOf } from '../stories/mapLayout'

export function mapGraph(map: ServiceMapView, extra: readonly string[] = []): MapGraph {
  const seen = new Set<string>()
  const links: [string, string][] = []
  for (const e of map.edges) {
    const id = `${e.parent}->${e.child}`
    if (e.parent === e.child || seen.has(id)) continue
    seen.add(id)
    links.push([e.parent, e.child])
  }
  links.sort((a, b) => a[0].localeCompare(b[0]) || a[1].localeCompare(b[1]))
  return { services: servicesOf(map, extra), links }
}

/** What a caller's card says about the infrastructure services hidden from the map. */
export interface InfraBadge {
  /** Hidden callees, busiest first. */
  services: { name: string; perMin: number; errorRate: number }[]
  /** Any call into a hidden service is failing: the badge turns red. */
  failing: boolean
}

/**
 * Drops the infrastructure services and every call touching them, and summarises the calls a
 * visible service made into them as a badge on that caller. `keep` stays drawn even when it is
 * infrastructure (the service whose drawer is open). Calls between infrastructure services,
 * and from them, produce no badge. `extra` lists badged callers that would otherwise vanish
 * (every call they made went to hidden infrastructure, so no edge or node names them): pass it
 * on to `mapGraph`, `servicesOf` and `mapSummary` so they stay on the map.
 */
export function hideInfra(
  map: ServiceMapView,
  infra: readonly string[],
  opts: { keep?: string; range: Range },
): { map: ServiceMapView; badges: Map<string, InfraBadge>; extra: string[] } {
  const hidden = new Set(infra.filter((s) => s !== opts.keep))
  const badges = new Map<string, InfraBadge>()
  if (hidden.size === 0) return { map, badges, extra: [] }
  const edges: EdgeView[] = []
  for (const e of map.edges) {
    if (hidden.has(e.parent)) continue
    if (!hidden.has(e.child)) {
      edges.push(e)
      continue
    }
    if (e.parent === e.child) continue
    const badge = badges.get(e.parent) ?? { services: [], failing: false }
    badge.services.push({ name: e.child, perMin: callsPerMin(e.calls, opts.range), errorRate: e.error_rate })
    badge.failing ||= edgeTone(e) === 'err'
    badges.set(e.parent, badge)
  }
  for (const b of badges.values()) b.services.sort((a, c) => c.perMin - a.perMin || a.name.localeCompare(c.name))
  const kept: ServiceMapView = { edges, nodes: map.nodes.filter((n) => !hidden.has(n.service)) }
  const present = new Set(servicesOf(kept))
  return { map: kept, badges, extra: [...badges.keys()].filter((s) => !present.has(s)).sort() }
}

/** Identifies the topology: a live refresh with the same services and calls keeps the layout. */
export function topologyKey(g: MapGraph): string {
  return `${g.services.join(',')}|${g.links.map(([a, b]) => `${a}>${b}`).join(',')}`
}

export function callsPerMin(calls: number, range: Range): number {
  return calls / (range.secs / 60)
}

export const EDGE_MIN_W = 1.25
export const EDGE_MAX_W = 6

/** Stroke width from calls/min on a log scale: 1 call/min ≈ 1.6 px, 1k/min ≈ 4.6 px, clamped. */
export function edgeWidth(perMin: number): number {
  const w = EDGE_MIN_W + Math.log10(1 + Math.max(0, perMin)) * 1.1
  return Math.min(EDGE_MAX_W, Math.max(EDGE_MIN_W, w))
}

/**
 * `err`: failing (red, dashed and flowing), the same threshold as the home preview;
 * `warn`: some errors below that threshold; `ok`: none.
 */
export type EdgeTone = 'ok' | 'warn' | 'err'

export function edgeTone(e: Pick<EdgeView, 'errors' | 'error_rate'>): EdgeTone {
  if (isFailingEdge(e)) return 'err'
  return e.errors > 0 ? 'warn' : 'ok'
}

/** Few errors read as a muted red; failing calls as the full error color. */
export const TONE_STROKE: Record<EdgeTone, string> = {
  ok: 'var(--tg-edge)',
  warn: 'color-mix(in srgb, var(--tg-err) 35%, var(--tg-edge))',
  err: 'var(--tg-err)',
}

/** Calls per second: "0.05/s" below 1, "16.8/s", "1.2k/s". */
export function rateText(perSec: number): string {
  if (!Number.isFinite(perSec) || perSec <= 0) return '0/s'
  if (perSec < 0.01) return '<0.01/s'
  if (perSec < 1) return `${perSec.toFixed(2)}/s`
  return `${compact(perSec)}/s`
}

/** Error ratio: "0 %", "<0.1 %", "0.4 %", "38 %". */
export function errText(ratio: number): string {
  if (!(ratio > 0)) return '0 %'
  if (ratio < 0.001) return '<0.1 %'
  return percent(ratio)
}

export const HEALTH_COLOR: Record<Health, string> = {
  ok: 'var(--tg-node-line)',
  slow: 'var(--tg-slow)',
  error: 'var(--tg-err)',
}

/** Services whose name contains the query (case-insensitive); empty for a blank query. */
export function matchServices(services: readonly string[], q: string | undefined): string[] {
  const t = q?.trim().toLowerCase()
  if (!t) return []
  return services.filter((s) => s.toLowerCase().includes(t))
}

export interface Neighbours {
  /** Calls into the service, busiest first. */
  callers: EdgeView[]
  /** Calls out of the service, busiest first. */
  callees: EdgeView[]
}

export function neighbours(map: ServiceMapView, service: string): Neighbours {
  const byCalls = (a: EdgeView, b: EdgeView) => b.calls - a.calls || a.parent.localeCompare(b.parent)
  return {
    callers: map.edges.filter((e) => e.child === service && e.parent !== service).sort(byCalls),
    callees: map.edges.filter((e) => e.parent === service && e.child !== service).sort(byCalls),
  }
}

export function nodeOf(map: ServiceMapView, service: string): NodeView | undefined {
  return map.nodes.find((n) => n.service === service)
}

/** One-line page summary: "17 services · 2 degraded · 1 failing call". */
export function mapSummary(map: ServiceMapView, extra: readonly string[] = []): string {
  const services = servicesOf(map, extra).length
  const degraded = map.nodes.filter((n) => n.health !== 'ok').length
  const failing = map.edges.filter((e) => e.parent !== e.child && isFailingEdge(e)).length
  const parts = [`${services} ${services === 1 ? 'service' : 'services'}`]
  parts.push(degraded ? `${degraded} degraded` : 'all healthy')
  if (failing) parts.push(`${failing} failing ${failing === 1 ? 'call' : 'calls'}`)
  return parts.join(' · ')
}

/**
 * SVG path along an ELK route (orthogonal segments), each corner rounded with radius up to
 * `r` (less where a segment is shorter).
 */
export function routePath(points: readonly Pos[], r = 10): string {
  const [first, ...rest] = points
  if (!first) return ''
  let d = `M${first.x} ${first.y}`
  for (let i = 0; i < rest.length; i++) {
    const p = rest[i] as Pos
    const next = rest[i + 1]
    const prev = (i === 0 ? first : rest[i - 1]) as Pos
    if (!next) {
      d += `L${p.x} ${p.y}`
      break
    }
    const inLen = Math.hypot(p.x - prev.x, p.y - prev.y)
    const outLen = Math.hypot(next.x - p.x, next.y - p.y)
    const k = Math.min(r, inLen / 2, outLen / 2)
    if (k < 0.5) {
      d += `L${p.x} ${p.y}`
      continue
    }
    const a = { x: p.x - ((p.x - prev.x) / inLen) * k, y: p.y - ((p.y - prev.y) / inLen) * k }
    const b = { x: p.x + ((next.x - p.x) / outLen) * k, y: p.y + ((next.y - p.y) / outLen) * k }
    d += `L${a.x} ${a.y}Q${p.x} ${p.y} ${b.x} ${b.y}`
  }
  return d
}

/** The point halfway along a route, by length. */
export function routeMidpoint(points: readonly Pos[]): Pos {
  let total = 0
  for (let i = 1; i < points.length; i++) total += Math.hypot(points[i]!.x - points[i - 1]!.x, points[i]!.y - points[i - 1]!.y)
  let left = total / 2
  for (let i = 1; i < points.length; i++) {
    const a = points[i - 1]!
    const b = points[i]!
    const len = Math.hypot(b.x - a.x, b.y - a.y)
    if (left <= len && len > 0) return { x: a.x + ((b.x - a.x) * left) / len, y: a.y + ((b.y - a.y) * left) / len }
    left -= len
  }
  return points[0] ?? { x: 0, y: 0 }
}
