/**
 * ELK layout (main-thread fallback, as jsdom has no Worker): every service gets a position,
 * callers sit left of callees, the result is deterministic and cycles are handled.
 */
import { describe, expect, it } from 'vitest'
import serviceMap from '../../api/__fixtures__/service-map.json'
import type { ServiceMapView } from '../../api/types'
import { NODE_H, NODE_W, elkGraph, layoutGraph } from './layout'
import type { MapGraph } from './layout'
import { edgeTone, edgeWidth, mapGraph, mapSummary, matchServices, neighbours, servicesOf, topologyKey } from './model'

const fixture = serviceMap as ServiceMapView

const overlaps = (a: { x: number; y: number }, b: { x: number; y: number }) =>
  a.x < b.x + NODE_W && b.x < a.x + NODE_W && a.y < b.y + NODE_H && b.y < a.y + NODE_H

describe('map graph', () => {
  it('includes services seen only on edges and drops self-calls and duplicates', () => {
    const g = mapGraph({
      nodes: [],
      edges: [
        { parent: 'a', child: 'b', calls: 1, errors: 0, error_rate: 0, avg_duration_ns: 1 },
        { parent: 'a', child: 'b', calls: 2, errors: 0, error_rate: 0, avg_duration_ns: 1 },
        { parent: 'b', child: 'b', calls: 3, errors: 0, error_rate: 0, avg_duration_ns: 1 },
      ],
    })
    expect(g).toEqual({ services: ['a', 'b'], links: [['a', 'b']] })
    expect(elkGraph(g).children).toEqual([
      { id: 'a', width: NODE_W, height: NODE_H },
      { id: 'b', width: NODE_W, height: NODE_H },
    ])
  })

  it('keys the topology, not the numbers', () => {
    const changed = { ...fixture, edges: fixture.edges.map((e) => ({ ...e, calls: e.calls + 1 })) }
    expect(topologyKey(mapGraph(changed))).toBe(topologyKey(mapGraph(fixture)))
    const fewer = { ...fixture, edges: fixture.edges.slice(1) }
    expect(topologyKey(mapGraph(fewer))).not.toBe(topologyKey(mapGraph(fixture)))
  })
})

describe('layoutGraph', () => {
  it('places every fixture service, callers left of callees, without overlaps', async () => {
    const g = mapGraph(fixture)
    const { positions } = await layoutGraph(g)
    expect(Object.keys(positions).sort()).toEqual(servicesOf(fixture))
    for (const p of Object.values(positions)) {
      expect(Number.isFinite(p.x) && Number.isFinite(p.y)).toBe(true)
    }
    const at = (s: string) => positions[s] as { x: number; y: number }
    expect(at('frontend-proxy').x).toBeLessThan(at('frontend').x)
    expect(at('frontend').x).toBeLessThan(at('checkout').x)
    expect(at('checkout').x).toBeLessThan(at('payment').x)
    const list = Object.values(positions)
    for (let i = 0; i < list.length; i++)
      for (let j = i + 1; j < list.length; j++) expect(overlaps(list[i]!, list[j]!)).toBe(false)
  })

  it('is deterministic for a fixed graph', async () => {
    const g = mapGraph(fixture)
    const a = await layoutGraph(g)
    const b = await layoutGraph(g)
    expect(b.positions).toEqual(a.positions)
    expect([b.width, b.height]).toEqual([a.width, a.height])
  })

  it('lays out a cyclic graph', async () => {
    const g: MapGraph = {
      services: ['a', 'b', 'c', 'd'],
      links: [
        ['a', 'b'],
        ['b', 'c'],
        ['c', 'a'],
        ['c', 'd'],
      ],
    }
    const { positions } = await layoutGraph(g)
    expect(Object.keys(positions).sort()).toEqual(['a', 'b', 'c', 'd'])
    const xs = new Set(Object.values(positions).map((p) => p.x))
    // The cycle is broken into layers rather than stacked in one column.
    expect(xs.size).toBeGreaterThan(1)
  })

  it('handles an empty graph', async () => {
    const { positions } = await layoutGraph({ services: [], links: [] })
    expect(positions).toEqual({})
  })
})

describe('map model', () => {
  it('scales edge width by calls/min on a clamped log scale', () => {
    expect(edgeWidth(0)).toBe(1.25)
    expect(edgeWidth(10)).toBeGreaterThan(edgeWidth(1))
    expect(edgeWidth(1e9)).toBe(6)
  })

  it('tones edges by error rate with the failing-edge threshold', () => {
    expect(edgeTone({ errors: 0, error_rate: 0 })).toBe('ok')
    expect(edgeTone({ errors: 1, error_rate: 0.005 })).toBe('warn')
    expect(edgeTone({ errors: 5, error_rate: 1 })).toBe('err')
  })

  it('matches services, lists neighbours and summarises', () => {
    expect(matchServices(servicesOf(fixture), ' FRONT')).toEqual(['frontend', 'frontend-proxy', 'frontend-web'])
    expect(matchServices(servicesOf(fixture), '  ')).toEqual([])
    const n = neighbours(fixture, 'checkout')
    expect(n.callers.map((e) => e.parent)).toEqual(['frontend'])
    expect(n.callees[0]?.child).toBe('currency')
    expect(mapSummary(fixture)).toBe('19 services · all healthy · 3 failing calls')
  })
})
