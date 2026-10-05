import { describe, expect, it } from 'vitest'
import serviceMap from '../../api/__fixtures__/service-map.json'
import type { EdgeView, NodeView, ServiceMapView } from '../../api/types'
import { MINI, callDepths, isFailingEdge, layoutMiniMap } from './mapLayout'
import type { MiniNode } from './mapLayout'

const edge = (parent: string, child: string, errors = 0, calls = 100): EdgeView => ({
  parent,
  child,
  calls,
  errors,
  error_rate: errors / calls,
  avg_duration_ns: 1,
})
const node = (service: string, health: NodeView['health'] = 'ok'): NodeView => ({
  service,
  calls: 1,
  rate: 1,
  error_ratio: 0,
  p99_ns: 1,
  baseline_p99_ns: 1,
  health,
})

/** Looks up a laid-out node by service; a missing one fails the test. */
const finder = (nodes: MiniNode[]) => (service: string) =>
  nodes.find((n) => n.service === service) ?? expect.fail(`no node ${service}`)

describe('callDepths', () => {
  it('counts the fewest hops from an entry service', () => {
    const d = callDepths(['lg', 'fe', 'co', 'pay', 'cat'], [edge('lg', 'fe'), edge('fe', 'co'), edge('co', 'pay'), edge('fe', 'cat'), edge('lg', 'cat')])
    expect(Object.fromEntries(d)).toEqual({ lg: 0, fe: 1, co: 2, pay: 3, cat: 1 })
  })
  it('enters a cycle without an entry and ignores self-calls', () => {
    const d = callDepths(['b', 'a', 'solo'], [edge('a', 'b'), edge('b', 'a'), edge('solo', 'solo')])
    expect(Object.fromEntries(d)).toEqual({ a: 0, b: 1, solo: 0 })
  })
})

describe('layoutMiniMap', () => {
  it('places services in columns by depth and keeps every service', () => {
    const map: ServiceMapView = {
      nodes: [node('lg'), node('fe'), node('co'), node('pay', 'error')],
      edges: [edge('lg', 'fe'), edge('fe', 'co'), edge('co', 'pay', 40), edge('fe', 'ship')],
    }
    const l = layoutMiniMap(map)
    const at = finder(l.nodes)
    expect(l.nodes.map((n) => n.service).sort()).toEqual(['co', 'fe', 'lg', 'pay', 'ship'])
    expect(at('lg').x).toBe(MINI.padX)
    expect(at('co').x).toBe(MINI.padX + 2 * MINI.colGap)
    expect(at('co').x).toBe(at('ship').x)
    expect(at('co').y).not.toBe(at('ship').y)
    expect(at('pay').health).toBe('error')
    expect(at('ship').health).toBe('ok')
    expect(l.edges.at(-1)).toMatchObject({ parent: 'co', child: 'pay', failing: true })
    expect(l.width).toBe(MINI.padX * 2 + 3 * MINI.colGap)
  })
  it('orders a column by its callers so edges cross less', () => {
    const map: ServiceMapView = {
      nodes: [],
      edges: [edge('root', 'a'), edge('root', 'b'), edge('a', 'z'), edge('b', 'y')],
    }
    const at = finder(layoutMiniMap(map).nodes)
    expect(at('a').y).toBeLessThan(at('b').y)
    // z is called by a (above b), so it sits above y despite the name order.
    expect(at('z').y).toBeLessThan(at('y').y)
  })
  it('handles an empty map and the captured demo map', () => {
    expect(layoutMiniMap({ nodes: [], edges: [] })).toEqual({ nodes: [], edges: [], width: 0, height: 0 })
    const l = layoutMiniMap(serviceMap as ServiceMapView)
    expect(l.nodes.length).toBeGreaterThanOrEqual(serviceMap.nodes.length)
    const spots = new Set(l.nodes.map((n) => `${n.x},${n.y}`))
    expect(spots.size).toBe(l.nodes.length)
    for (const e of l.edges) expect(e.d).toMatch(/^M[\d.]+ [\d.]+[CQ]/)
  })
  it('marks an edge failing from 1 % errors', () => {
    expect(isFailingEdge(edge('a', 'b', 1, 100))).toBe(true)
    expect(isFailingEdge(edge('a', 'b', 1, 1000))).toBe(false)
    expect(isFailingEdge(edge('a', 'b', 0))).toBe(false)
  })
})
