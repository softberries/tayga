/**
 * `/map`: the interactive service map. React Flow canvas laid out by ELK (in a Web Worker),
 * custom health nodes, call edges sized by calls/min and colored by error rate, a search
 * that highlights and zooms to services, and the service drawer (`?service=`). This route is
 * its own chunk, so React Flow and ELK load only here.
 */
import '@xyflow/react/dist/style.css'
import '../features/map/map.css'
import { keepPreviousData, useQuery } from '@tanstack/react-query'
import { useNavigate, useSearch } from '@tanstack/react-router'
import { Background, BackgroundVariant, Controls, MiniMap, ReactFlow, ReactFlowProvider, useReactFlow } from '@xyflow/react'
import type { EdgeTypes, FitViewOptions, NodeTypes } from '@xyflow/react'
import { ExternalLink, Network, Search } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { api } from '../api/queries'
import { DEFAULT_INFRA_SERVICES } from '../api/schemas'
import type { Health, ServiceMapView } from '../api/types'
import type { MapSearch } from '../app/search'
import { rangeParams, rangePhrase, widerHint } from '../app/range'
import type { Range } from '../app/range'
import { useAutoRefresh, useRange } from '../app/useRange'
import { Button } from '../components/ui/Button'
import { Card, PanelTitle } from '../components/ui/Card'
import { EmptyState } from '../components/ui/EmptyState'
import { ErrorState } from '../components/ui/ErrorState'
import { Skeleton } from '../components/ui/Skeleton'
import { Switch } from '../components/ui/Switch'
import { RefreshNote, loadFailed } from '../components/ui/StaleNote'
import { ServiceDrawer } from '../features/map/ServiceDrawer'
import { ServiceEdge } from '../features/map/ServiceEdge'
import type { ServiceEdgeType } from '../features/map/ServiceEdge'
import { ServiceNode } from '../features/map/ServiceNode'
import type { ServiceNodeType } from '../features/map/ServiceNode'
import { NODE_H, NODE_W, edgeId, layoutGraph } from '../features/map/layout'
import type { MapLayout } from '../features/map/layout'
import { HEALTH_COLOR, TONE_STROKE, callsPerMin, edgeTone, edgeWidth, hideInfra, mapGraph, mapSummary, matchServices, servicesOf, topologyKey } from '../features/map/model'
import type { InfraBadge } from '../features/map/model'
import { describeMap } from '../features/stories/MiniMap'
import { cx } from '../lib/cx'
import { NARROW_QUERY, REDUCED_MOTION_QUERY, useMediaQuery } from '../lib/useMediaQuery'
import { useAppliedTheme } from '../theme/useAppliedTheme'

const nodeTypes: NodeTypes = { service: ServiceNode }
const edgeTypes: EdgeTypes = { service: ServiceEdge }
const MINIMAP = { width: 168, height: 104 } as const
/**
 * Fit the whole map, but no further out than readable cards (phones pan instead). On wide
 * screens the bottom band is kept clear for the legend, minimap and controls.
 */
const fitOptions = (narrow: boolean): FitViewOptions => ({
  padding: narrow ? 0.12 : { top: '24px', left: '24px', right: '24px', bottom: `${MINIMAP.height + 44}px` as const },
  maxZoom: 1,
  minZoom: narrow ? 0.6 : 0.15,
})

/** The service drawer's panel (the Sheet's aside), found by its class. */
export const DRAWER_CLASS = 'tg-service-drawer'

/** `{grafana_url}/d/tayga-service-map` when the API reports an http(s) Grafana URL. */
export function grafanaMapUrl(base: string | null | undefined): string | null {
  if (!base || !/^https?:\/\//i.test(base)) return null
  return `${base.replace(/\/+$/, '')}/d/tayga-service-map`
}

function Legend() {
  const ring = (h: Health) => (
    <span aria-hidden className="size-2.5 rounded-full border-2" style={{ borderColor: HEALTH_COLOR[h] }} />
  )
  return (
    <ul
      aria-label="Legend"
      className="absolute bottom-3 left-3 z-[5] m-0 flex max-w-[calc(100%-5rem)] list-none flex-wrap items-center gap-x-3.5 gap-y-1 rounded-field border border-panel-line bg-panel px-3 py-2 text-xs text-muted shadow-panel"
    >
      <li className="flex items-center gap-1.5">{ring('ok')}healthy</li>
      <li className="flex items-center gap-1.5">{ring('error')}errors</li>
      <li className="flex items-center gap-1.5">{ring('slow')}slow</li>
      <li className="flex items-center gap-1.5">
        <svg aria-hidden width="22" height="6" className="overflow-visible">
          <path d="M0 3H22" strokeWidth={2} strokeDasharray="5 3" style={{ stroke: 'var(--tg-err)' }} />
        </svg>
        failing calls (≥1 % errors)
      </li>
      <li className="flex items-center gap-1.5">
        <svg aria-hidden width="22" height="6" className="overflow-visible">
          <path d="M0 3H22" strokeWidth={2} style={{ stroke: TONE_STROKE.warn }} />
        </svg>
        some errors (&lt;1 %)
      </li>
      <li>line width = calls/min</li>
    </ul>
  )
}

interface CanvasProps {
  map: ServiceMapView
  layout: MapLayout
  range: Range
  /** Callers kept on the map by their infra badge alone (no edge or node of their own). */
  extra: readonly string[]
  /** Hidden infrastructure callees per caller. */
  infra: ReadonlyMap<string, InfraBadge>
  matches: ReadonlySet<string>
  active: string | undefined
  onOpen: (service: string) => void
}

function Canvas({ map, layout, range, extra, infra, matches, active, onOpen }: CanvasProps) {
  const { fitView, getViewport, setViewport } = useReactFlow()
  const narrow = useMediaQuery(NARROW_QUERY)
  const reduce = useMediaQuery(REDUCED_MOTION_QUERY)
  const applied = useAppliedTheme()
  const [pinned, setPinned] = useState<string | null>(null)
  const searching = matches.size > 0
  const fit = useMemo(() => fitOptions(narrow), [narrow])
  const duration = reduce ? 0 : 300

  /**
   * Centers a card in the visible part of the canvas (left of the drawer when it is open)
   * when any of it is outside that area. Used when the drawer opens and on keyboard focus.
   */
  const reveal = useCallback(
    (service: string) => {
      const card = document.querySelector<HTMLElement>(`.tg-map [data-service="${CSS.escape(service)}"]`)
      const pane = document.querySelector<HTMLElement>('.react-flow.tg-map')
      if (!card || !pane) return
      const view = pane.getBoundingClientRect()
      let right = view.right
      const drawer = document.querySelector<HTMLElement>(`.${DRAWER_CLASS}`)
      if (drawer) {
        const left = window.innerWidth - drawer.offsetWidth
        // On phones the drawer covers the canvas; centre on the whole canvas instead.
        if (left - view.left > view.width * 0.4) right = Math.min(right, left - 16)
      }
      const r = card.getBoundingClientRect()
      const inside = r.left >= view.left && r.right <= right && r.top >= view.top && r.bottom <= view.bottom
      if (inside) return
      const dx = (view.left + right) / 2 - (r.left + r.right) / 2
      const dy = (view.top + view.bottom) / 2 - (r.top + r.bottom) / 2
      const vp = getViewport()
      void setViewport({ ...vp, x: vp.x + dx, y: vp.y + dy }, { duration })
    },
    [getViewport, setViewport, duration],
  )

  const nodes = useMemo<ServiceNodeType[]>(() => {
    const views = new Map(map.nodes.map((n) => [n.service, n]))
    // A layout kept from before a toggle may still place services the map no longer has.
    const drawn = new Set(servicesOf(map, extra))
    return Object.entries(layout.positions)
      .filter(([service]) => drawn.has(service))
      .map(([service, position]) => ({
        id: service,
        type: 'service' as const,
        position,
        // Fixed card size: no measuring pass before the nodes show.
        width: NODE_W,
        height: NODE_H,
        // Nodes are neither draggable nor selectable, so React Flow would make them ignore
        // the pointer; the card is a button.
        style: { pointerEvents: 'all' as const },
        data: {
          service,
          view: views.get(service) ?? null,
          match: matches.has(service),
          dimmed: searching && !matches.has(service),
          infra: infra.get(service) ?? null,
          active: service === active,
          onOpen,
          onFocusCard: reveal,
        },
      }))
      // Tab order follows the picture: left to right, then top to bottom.
      .sort((a, b) => a.position.x - b.position.x || a.position.y - b.position.y)
  }, [map, layout, extra, infra, matches, searching, active, onOpen, reveal])

  const edges = useMemo<ServiceEdgeType[]>(
    () =>
      map.edges
        .filter((e) => e.parent !== e.child && layout.positions[e.parent] && layout.positions[e.child])
        .map((e) => {
          const id = edgeId(e.parent, e.child)
          const perMin = callsPerMin(e.calls, range)
          const tone = edgeTone(e)
          return {
            id,
            type: 'service' as const,
            source: e.parent,
            target: e.child,
            data: {
              edge: e,
              perMin,
              width: edgeWidth(perMin),
              tone,
              pinned: pinned === id,
              dimmed: searching && !matches.has(e.parent) && !matches.has(e.child),
              route: layout.routes[id],
            },
          }
        })
        // Failing edges last, so they draw over the others (all edges stay under the cards).
        .sort((a, b) => Number(a.data.tone === 'err') - Number(b.data.tone === 'err')),
    [map.edges, layout, range, pinned, searching, matches],
  )

  const activeRef = useRef(active)
  useEffect(() => {
    activeRef.current = active
  }, [active])

  // Phones start on the degraded services, when there are any; wide screens fit everything.
  const degraded = useMemo(
    () => map.nodes.filter((n) => n.health !== 'ok' && layout.positions[n.service]).map((n) => ({ id: n.service })),
    [map.nodes, layout],
  )
  const focusDegraded = narrow && degraded.length > 0

  // Fit on load and when the topology changes, then keep the open service in view.
  const fitted = useRef(false)
  useEffect(() => {
    let live = true
    fitted.current = false
    void fitView({ ...fit, ...(focusDegraded ? { nodes: degraded, minZoom: 0.6 } : {}), duration: 0 }).then(() => {
      if (!live) return
      fitted.current = true
      if (activeRef.current) reveal(activeRef.current)
    })
    return () => {
      live = false
    }
    // `degraded` changes with every refresh; refit only for a new layout or screen size.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [layout, fit, fitView, reveal, focusDegraded])

  useEffect(() => {
    if (!active || !fitted.current) return
    // After the drawer has its width.
    const id = requestAnimationFrame(() => reveal(active))
    return () => cancelAnimationFrame(id)
  }, [active, reveal])

  return (
    <ReactFlow
      nodes={nodes}
      edges={edges}
      nodeTypes={nodeTypes}
      edgeTypes={edgeTypes}
      colorMode={applied === 'dark' ? 'dark' : 'light'}
      fitView
      fitViewOptions={fit}
      minZoom={0.15}
      maxZoom={2}
      nodesDraggable={false}
      nodesConnectable={false}
      nodesFocusable={false}
      edgesFocusable={false}
      elementsSelectable={false}
      onEdgeClick={(_, e) => setPinned((p) => (p === e.id ? null : e.id))}
      onPaneClick={() => setPinned(null)}
      className="tg-map"
    >
      <Background variant={BackgroundVariant.Lines} gap={32} />
      <Controls
        position="bottom-right"
        showInteractive={false}
        fitViewOptions={{ ...fit, duration }}
        className={cx('tg-map-controls', !narrow && 'tg-map-controls-beside')}
      />
      {narrow ? null : (
        <MiniMap
          position="bottom-right"
          pannable
          zoomable
          ariaLabel="Service map overview"
          style={MINIMAP}
          nodeColor={(n) => HEALTH_COLOR[(n as ServiceNodeType).data.view?.health ?? 'ok']}
          nodeBorderRadius={6}
          className="tg-map-minimap"
        />
      )}
    </ReactFlow>
  )
}

function CanvasSkeleton({ label }: { label: string }) {
  return (
    <div aria-busy="true" aria-label={label} className="absolute inset-0 flex items-center justify-center p-6">
      <Skeleton className="absolute inset-4" />
      <span className="relative text-xs text-muted">{label}</span>
    </div>
  )
}

function MapView() {
  const range = useRange()
  const search = useSearch({ from: '/_shell/map' })
  const navigate = useNavigate({ from: '/map' })
  const refetchInterval = useAutoRefresh()
  const reduce = useMediaQuery(REDUCED_MOTION_QUERY)
  const { fitView } = useReactFlow()

  const map = useQuery({ ...api.serviceMap(rangeParams(range)), refetchInterval, placeholderData: keepPreviousData })
  const config = useQuery(api.config())
  const grafana = grafanaMapUrl(config.data?.grafana_url)

  const showInfra = search.infra === true
  const infraServices = config.data?.infra_services ?? DEFAULT_INFRA_SERVICES
  // The drawer's service stays drawn even when it is infrastructure.
  const { visible, badges, extra } = useMemo(() => {
    if (!map.data) return { visible: undefined, badges: new Map<string, InfraBadge>(), extra: [] as string[] }
    const r = hideInfra(map.data, showInfra ? [] : infraServices, { keep: search.service, range })
    return { visible: r.map, badges: r.badges, extra: r.extra }
  }, [map.data, showInfra, infraServices, search.service, range])
  const hiddenInfra = useMemo(() => {
    if (!map.data || showInfra) return 0
    const seen = new Set(servicesOf(map.data))
    return infraServices.filter((s) => s !== search.service && seen.has(s)).length
  }, [map.data, showInfra, infraServices, search.service])

  // The layout's key is the drawn topology, so toggling infrastructure lays the map out again.
  const graph = useMemo(() => (visible ? mapGraph(visible, extra) : null), [visible, extra])
  const key = graph ? topologyKey(graph) : ''
  const layout = useQuery({
    queryKey: ['map-layout', key],
    queryFn: () => layoutGraph(graph ?? { services: [], links: [] }),
    enabled: graph !== null && graph.services.length > 0,
    staleTime: Infinity,
    retry: false,
    placeholderData: keepPreviousData,
  })

  const matchList = useMemo(() => (graph ? matchServices(graph.services, search.q) : []), [graph, search.q])
  const matches = useMemo(() => new Set(matchList), [matchList])

  const setSearch = useCallback(
    (patch: Partial<MapSearch>, replace = false) =>
      void navigate({ search: (prev) => ({ ...prev, ...patch }), replace, resetScroll: false }),
    [navigate],
  )
  const onOpen = useCallback((service: string) => setSearch({ service }), [setSearch])
  const focusMatches = () => {
    if (matchList.length === 0) return
    void fitView({ nodes: matchList.map((id) => ({ id })), padding: 0.45, maxZoom: 1.25, duration: reduce ? 0 : 400 })
  }
  // Closing the drawer returns focus to the service's card.
  const lastOpen = useRef<string | undefined>(undefined)
  useEffect(() => {
    if (search.service) lastOpen.current = search.service
  }, [search.service])
  const onCloseAutoFocus = useCallback((e: Event) => {
    const s = lastOpen.current
    const el = s ? document.querySelector<HTMLElement>(`[data-service="${CSS.escape(s)}"]`) : null
    if (el) {
      e.preventDefault()
      el.focus()
    }
  }, [])

  const data = map.data
  const drawn = visible ?? data
  const empty = map.isSuccess && graph !== null && graph.services.length === 0
  const q = search.q ?? ''

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3.5">
      <Card className="flex flex-wrap items-center gap-x-4 gap-y-2.5 px-4 py-3">
        <PanelTitle>Service map</PanelTitle>
        <span aria-live="polite" className="text-xs text-muted">
          {drawn && !empty ? `${mapSummary(drawn, extra)}${hiddenInfra ? ` · ${hiddenInfra} infra hidden` : ''}` : map.isPending ? 'Loading…' : ''}
        </span>
        <div className="ml-auto flex min-w-0 flex-wrap items-center gap-2 max-sm:w-full">
          <label className="relative flex min-w-0 items-center max-sm:flex-1">
            <span className="sr-only">Find a service</span>
            <Search aria-hidden size={14} className="pointer-events-none absolute left-2.5 text-muted" />
            <input
              type="search"
              value={q}
              placeholder="Find a service"
              onChange={(e) => setSearch({ q: e.target.value || undefined }, true)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') {
                  e.preventDefault()
                  focusMatches()
                }
              }}
              aria-describedby="map-search-status"
              className="h-[30px] w-56 min-w-0 rounded-control border border-field-line bg-field pl-8 pr-2 text-[13px] text-ink outline-none placeholder:text-faint focus-visible:border-accent max-sm:w-full"
            />
          </label>
          <span id="map-search-status" aria-live="polite" className={cx('text-xs', q && matchList.length === 0 ? 'text-err' : 'text-muted')}>
            {q ? (matchList.length === 0 ? 'No match' : `${matchList.length} ${matchList.length === 1 ? 'match' : 'matches'} · Enter to zoom`) : ''}
          </span>
          {infraServices.length > 0 ? (
            <div className="flex items-center gap-2">
              <Switch
                id="map-show-infra"
                checked={showInfra}
                onCheckedChange={(on) => setSearch({ infra: on ? true : undefined })}
              />
              <label htmlFor="map-show-infra" className="cursor-pointer select-none text-[13px] text-ink">
                Show infrastructure
              </label>
            </div>
          ) : null}
          {grafana ? (
            <Button asChild size="sm" variant="secondary">
              <a href={grafana} target="_blank" rel="noreferrer noopener">
                <ExternalLink aria-hidden size={14} /> Open in Grafana
              </a>
            </Button>
          ) : null}
        </div>
      </Card>

      <RefreshNote queries={[map]} />

      <Card
        className="relative flex min-h-[460px] flex-1 overflow-hidden max-sm:min-h-[62dvh]"
        aria-busy={map.isFetching || layout.isFetching || undefined}
      >
        {map.isPending ? (
          <CanvasSkeleton label="Loading the service map" />
        ) : loadFailed(map) ? (
          <ErrorState error={map.error} onRetry={() => void map.refetch()} className="m-auto" />
        ) : empty ? (
          <EmptyState
            className="m-auto"
            icon={<Network size={18} />}
            title="No service calls in this window"
            description={`Tayga has seen no spans in ${rangePhrase(range)}.${widerHint(range, 'calls')}`}
          />
        ) : layout.isError ? (
          <ErrorState error={layout.error} title="Could not lay out the map" onRetry={() => void layout.refetch()} className="m-auto" />
        ) : !layout.data || !drawn ? (
          <CanvasSkeleton label={`Laying out ${graph?.services.length ?? 0} services`} />
        ) : (
          <section aria-label="Service map canvas" className="absolute inset-0">
            <p className="sr-only">{describeMap(drawn, extra)}</p>
            <Canvas map={drawn} layout={layout.data} range={range} extra={extra} infra={badges} matches={matches} active={search.service} onOpen={onOpen} />
            <Legend />
          </section>
        )}
      </Card>

      <ServiceDrawer
        service={search.service}
        map={data}
        range={range}
        onClose={() => setSearch({ service: undefined })}
        onCloseAutoFocus={onCloseAutoFocus}
      />
    </div>
  )
}

export function MapPage() {
  return (
    <ReactFlowProvider>
      <MapView />
    </ReactFlowProvider>
  )
}
