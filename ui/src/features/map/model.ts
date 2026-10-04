/**
 * Pure helpers for the /map page: the graph handed to ELK, edge width and tone, search
 * matching and each service's callers and callees.
 */
import type { EdgeView, Health, NodeView, ServiceMapView } from '../../api/types'
import type { Since } from '../../app/search'
import { compact, percent } from '../../lib/format'
import { SINCE_SECS } from '../stories/model'
import { isFailingEdge } from '../stories/mapLayout'
import type { MapGraph } from './layout'

/** Every service on the map: nodes plus services seen only as callers or callees, by name. */
export function servicesOf(map: ServiceMapView): string[] {
  const s = new Set(map.nodes.map((n) => n.service))
  for (const e of map.edges) {
    s.add(e.parent)
    s.add(e.child)
  }
  return [...s].sort()
}

export function mapGraph(map: ServiceMapView): MapGraph {
  const seen = new Set<string>()
  const links: [string, string][] = []
  for (const e of map.edges) {
    const id = `${e.parent}->${e.child}`
    if (e.parent === e.child || seen.has(id)) continue
    seen.add(id)
    links.push([e.parent, e.child])
  }
  links.sort((a, b) => a[0].localeCompare(b[0]) || a[1].localeCompare(b[1]))
  return { services: servicesOf(map), links }
}

/** Identifies the topology: a live refresh with the same services and calls keeps the layout. */
export function topologyKey(g: MapGraph): string {
  return `${g.services.join(',')}|${g.links.map(([a, b]) => `${a}>${b}`).join(',')}`
}

export function callsPerMin(calls: number, since: Since): number {
  return calls / (SINCE_SECS[since] / 60)
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
export function mapSummary(map: ServiceMapView): string {
  const services = servicesOf(map).length
  const degraded = map.nodes.filter((n) => n.health !== 'ok').length
  const failing = map.edges.filter((e) => e.parent !== e.child && isFailingEdge(e)).length
  const parts = [`${services} ${services === 1 ? 'service' : 'services'}`]
  parts.push(degraded ? `${degraded} degraded` : 'all healthy')
  if (failing) parts.push(`${failing} failing ${failing === 1 ? 'call' : 'calls'}`)
  return parts.join(' · ')
}
