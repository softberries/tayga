import { describe, expect, it } from 'vitest'
import serviceMap from '../../api/__fixtures__/service-map.json'
import type { EdgeView, NodeView, ServiceMapView } from '../../api/types'
import { hideInfra, mapGraph, topologyKey } from './model'

const node = (service: string): NodeView => ({ service, calls: 10, rate: 1, error_ratio: 0, p99_ns: 1, baseline_p99_ns: 1, health: 'ok' })
const edge = (parent: string, child: string, calls = 60, errors = 0): EdgeView => ({
  parent,
  child,
  calls,
  errors,
  error_rate: calls ? errors / calls : 0,
  avg_duration_ns: 1,
})

const map: ServiceMapView = {
  nodes: ['web', 'cart', 'flagd', 'otel'].map(node),
  edges: [edge('web', 'cart'), edge('web', 'flagd', 120, 6), edge('cart', 'flagd', 30), edge('flagd', 'otel'), edge('cart', 'otel', 6)],
}

describe('hideInfra', () => {
  it('removes the infra nodes and every edge touching them', () => {
    const r = hideInfra(map, ['flagd'], undefined, 60)
    expect(r.map.nodes.map((n) => n.service)).toEqual(['web', 'cart', 'otel'])
    expect(r.map.edges.map((e) => `${e.parent}>${e.child}`)).toEqual(['web>cart', 'cart>otel'])
  })

  it('builds one badge per caller with calls per minute and error rate, busiest first', () => {
    const { badges } = hideInfra(map, ['flagd', 'otel'], undefined, 120)
    expect([...badges.keys()].sort()).toEqual(['cart', 'web'])
    // 120 calls over a 2 minute window = 60/min; 5 % errors.
    expect(badges.get('web')?.services).toEqual([{ name: 'flagd', perMin: 60, errorRate: 0.05 }])
    expect(badges.get('cart')?.services.map((s) => [s.name, s.perMin])).toEqual([
      ['flagd', 15],
      ['otel', 3],
    ])
  })

  it('is failing only when a hidden edge is failing', () => {
    const { badges } = hideInfra(map, ['flagd'], undefined, 60)
    expect(badges.get('web')?.failing).toBe(true)
    expect(badges.get('cart')?.failing).toBe(false)
    // Under 1 % errors is not failing.
    const calm: ServiceMapView = { ...map, edges: [edge('web', 'flagd', 1000, 5)] }
    expect(hideInfra(calm, ['flagd'], undefined, 60).badges.get('web')?.failing).toBe(false)
  })

  it('keeps the kept service and its edges, even when it is infra', () => {
    const r = hideInfra(map, ['flagd'], 'flagd', 60)
    expect(r.map).toEqual(map)
    expect(r.badges.size).toBe(0)
  })

  it('infra to infra edges produce no badge', () => {
    const r = hideInfra(map, ['flagd', 'otel'], undefined, 60)
    expect(r.badges.has('flagd')).toBe(false)
    expect(r.map.edges.map((e) => `${e.parent}>${e.child}`)).toEqual(['web>cart'])
  })

  it('does nothing without infra services', () => {
    const r = hideInfra(map, [], undefined, 60)
    expect(r.map).toBe(map)
    expect(r.badges.size).toBe(0)
  })

  it('changes the layout key: the key is the drawn topology', () => {
    const real = serviceMap as ServiceMapView
    const hidden = topologyKey(mapGraph(hideInfra(real, ['flagd'], undefined, 3600).map))
    expect(hidden).not.toContain('flagd')
    expect(topologyKey(mapGraph(real))).toContain('flagd')
  })
})
