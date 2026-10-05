/**
 * ELK `layered` layout for the service map: callers left of callees, node boxes the size of
 * the service cards. ELK runs in a Web Worker (`workers/elk.worker.ts`); in tests, which
 * have no Worker, it runs on the main thread instead. ELK breaks call cycles itself.
 */
import ElkApi from 'elkjs/lib/elk-api.js'
import type { ELK, ElkNode } from 'elkjs/lib/elk-api.js'

/** Service card size in px; ELK reserves exactly this box per node. */
export const NODE_W = 190
export const NODE_H = 76

export const ELK_OPTIONS: Record<string, string> = {
  'elk.algorithm': 'layered',
  'elk.direction': 'RIGHT',
  'elk.layered.spacing.nodeNodeBetweenLayers': '64',
  // Edges are drawn along ELK's routes, so they run between the cards, never across them.
  'elk.edgeRouting': 'ORTHOGONAL',
  'elk.spacing.nodeNode': '28',
  'elk.spacing.componentComponent': '56',
  'elk.layered.nodePlacement.strategy': 'BRANDES_KOEPF',
  'elk.layered.crossingMinimization.strategy': 'LAYER_SWEEP',
  // Fixed seed: the same graph always gets the same picture.
  'elk.randomSeed': '1',
}

export interface MapGraph {
  /** Every service, in a stable order (the order ELK sees them). */
  services: readonly string[]
  /** Distinct `[caller, callee]` pairs; self-calls are left out. */
  links: readonly (readonly [string, string])[]
}

export interface Pos {
  x: number
  y: number
}

export interface MapLayout {
  /** Top-left corner of each service's card. */
  positions: Record<string, Pos>
  /** Each link's route by edge id (`caller->callee`): start, bend points, end. */
  routes: Record<string, Pos[]>
  width: number
  height: number
  /** Wall-clock layout time in ms (worker round trip included). */
  ms: number
}

export function elkGraph(graph: MapGraph): ElkNode {
  return {
    id: 'root',
    layoutOptions: ELK_OPTIONS,
    children: graph.services.map((id) => ({ id, width: NODE_W, height: NODE_H })),
    edges: graph.links.map(([s, t]) => ({ id: edgeId(s, t), sources: [s], targets: [t] })),
  }
}

export const edgeId = (caller: string, callee: string) => `${caller}->${callee}`

export function positionsOf(result: ElkNode): Omit<MapLayout, 'ms'> {
  const positions: Record<string, Pos> = {}
  for (const c of result.children ?? []) positions[c.id] = { x: c.x ?? 0, y: c.y ?? 0 }
  const routes: Record<string, Pos[]> = {}
  for (const e of result.edges ?? []) {
    const points: Pos[] = []
    for (const s of e.sections ?? []) points.push(s.startPoint, ...(s.bendPoints ?? []), s.endPoint)
    if (points.length >= 2) routes[e.id] = points.map(({ x, y }) => ({ x, y }))
  }
  return { positions, routes, width: result.width ?? 0, height: result.height ?? 0 }
}

interface Engine {
  elk: ELK
  /** Rejects when the worker dies, so a pending layout fails instead of hanging. */
  failed: Promise<never>
}

let engine: Promise<Engine> | null = null

async function createEngine(): Promise<Engine> {
  if (typeof Worker === 'undefined') {
    // Tests (jsdom, Node) have no Worker: ELK runs on the main thread there. Builds drop this
    // branch and its 1.4 MB chunk, as every supported browser has Web Workers.
    if (import.meta.env.MODE !== 'test') throw new Error('This browser cannot run the layout worker.')
    const { default: Bundled } = await import('elkjs/lib/elk.bundled.js')
    return { elk: new Bundled(), failed: new Promise<never>(() => {}) }
  }
  const { default: ElkWorker } = await import('../../workers/elk.worker?worker')
  let fail: (e: Error) => void = () => {}
  const failed = new Promise<never>((_, reject) => {
    fail = reject
  })
  failed.catch(() => {})
  const elk = new ElkApi({
    workerFactory: () => {
      const w = new ElkWorker({ name: 'elk-layout' })
      w.addEventListener('error', (e) => fail(new Error(e.message || 'The layout worker failed to start.')))
      return w
    },
  })
  return { elk, failed }
}

/** Lays out `graph`; one ELK instance (and worker) is shared by every call. */
export async function layoutGraph(graph: MapGraph): Promise<MapLayout> {
  const t0 = performance.now()
  const current = (engine ??= createEngine())
  try {
    const { elk, failed } = await current
    const result = await Promise.race([elk.layout(elkGraph(graph)), failed])
    return { ...positionsOf(result), ms: performance.now() - t0 }
  } catch (e) {
    // Start over with a fresh worker on the next attempt.
    if (engine === current) engine = null
    void current.then(({ elk }) => elk.terminateWorker()).catch(() => {})
    throw e
  }
}
