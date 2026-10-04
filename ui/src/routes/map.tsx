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
import type { EdgeTypes, NodeTypes } from '@xyflow/react'
import { ExternalLink, Network, Search } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { api } from '../api/queries'
import type { Health, ServiceMapView } from '../api/types'
import { useLiveInterval } from '../app/live'
import type { MapSearch, Since } from '../app/search'
import { useSince } from '../components/shell/TimeRange'
import { Button } from '../components/ui/Button'
import { Card, PanelTitle } from '../components/ui/Card'
import { EmptyState } from '../components/ui/EmptyState'
import { ErrorState } from '../components/ui/ErrorState'
import { Skeleton } from '../components/ui/Skeleton'
import { ServiceDrawer } from '../features/map/ServiceDrawer'
import { ServiceEdge } from '../features/map/ServiceEdge'
import type { ServiceEdgeType } from '../features/map/ServiceEdge'
import { ServiceNode } from '../features/map/ServiceNode'
import type { ServiceNodeType } from '../features/map/ServiceNode'
import { NODE_H, NODE_W, layoutGraph } from '../features/map/layout'
import type { MapLayout } from '../features/map/layout'
import { HEALTH_COLOR, callsPerMin, edgeTone, edgeWidth, mapGraph, mapSummary, matchServices, topologyKey } from '../features/map/model'
import { describeMap } from '../features/stories/MiniMap'
import { ErrorBanner } from '../features/stories/ErrorBanner'
import { cx } from '../lib/cx'
import { NARROW_QUERY, REDUCED_MOTION_QUERY, useMediaQuery } from '../lib/useMediaQuery'
import { useAppliedTheme } from '../theme/useAppliedTheme'

const nodeTypes: NodeTypes = { service: ServiceNode }
const edgeTypes: EdgeTypes = { service: ServiceEdge }
/** Fit the whole map, but no further out than readable cards (phones pan instead). */
const fitOptions = (narrow: boolean) => ({ padding: 0.12, maxZoom: 1, minZoom: narrow ? 0.6 : 0.15 })

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
        failing calls
      </li>
      <li>line width = calls/min</li>
    </ul>
  )
}

interface CanvasProps {
  map: ServiceMapView
  layout: MapLayout
  since: Since
  matches: ReadonlySet<string>
  active: string | undefined
  onOpen: (service: string) => void
}

function Canvas({ map, layout, since, matches, active, onOpen }: CanvasProps) {
  const { fitView, getViewport, setViewport } = useReactFlow()
  const narrow = useMediaQuery(NARROW_QUERY)
  const reduce = useMediaQuery(REDUCED_MOTION_QUERY)
  const applied = useAppliedTheme()
  const [pinned, setPinned] = useState<string | null>(null)
  const searching = matches.size > 0
  const fit = useMemo(() => fitOptions(narrow), [narrow])
  const duration = reduce ? 0 : 300

  const nodes = useMemo<ServiceNodeType[]>(() => {
    const views = new Map(map.nodes.map((n) => [n.service, n]))
    return Object.entries(layout.positions)
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
          active: service === active,
          onOpen,
        },
      }))
      // Tab order follows the picture: left to right, then top to bottom.
      .sort((a, b) => a.position.x - b.position.x || a.position.y - b.position.y)
  }, [map.nodes, layout, matches, searching, active, onOpen])

  const edges = useMemo<ServiceEdgeType[]>(
    () =>
      map.edges
        .filter((e) => e.parent !== e.child && layout.positions[e.parent] && layout.positions[e.child])
        .map((e) => {
          const id = `${e.parent}->${e.child}`
          const perMin = callsPerMin(e.calls, since)
          const tone = edgeTone(e)
          return {
            id,
            type: 'service' as const,
            source: e.parent,
            target: e.child,
            // Failing edges draw on top.
            zIndex: tone === 'err' ? 1 : 0,
            data: {
              edge: e,
              perMin,
              width: edgeWidth(perMin),
              tone,
              pinned: pinned === id,
              dimmed: searching && !matches.has(e.parent) && !matches.has(e.child),
            },
          }
        }),
    [map.edges, layout, since, pinned, searching, matches],
  )

  /** Pans a card hidden behind the drawer into the visible part of the canvas. */
  const reveal = useCallback(
    (service: string) => {
      const card = document.querySelector<HTMLElement>(`.tg-map [data-service="${CSS.escape(service)}"]`)
      const panel = document.querySelector<HTMLElement>('[role="dialog"]')
      if (!card || !panel) return
      const panelLeft = window.innerWidth - panel.offsetWidth
      // On phones the drawer covers the page; nothing to reveal.
      if (panelLeft < window.innerWidth * 0.4) return
      const overlap = card.getBoundingClientRect().right - (panelLeft - 32)
      if (overlap <= 0) return
      const vp = getViewport()
      void setViewport({ ...vp, x: vp.x - overlap }, { duration })
    },
    [getViewport, setViewport, duration],
  )
  const activeRef = useRef(active)
  useEffect(() => {
    activeRef.current = active
  }, [active])

  // Fit on load and when the topology changes, then keep the open service in view.
  const fitted = useRef(false)
  useEffect(() => {
    let live = true
    fitted.current = false
    void fitView({ ...fit, duration: 0 }).then(() => {
      if (!live) return
      fitted.current = true
      if (activeRef.current) reveal(activeRef.current)
    })
    return () => {
      live = false
    }
  }, [layout, fit, fitView, reveal])

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
      <Controls position="bottom-right" showInteractive={false} fitViewOptions={{ ...fit, duration }} className="tg-map-controls" />
      {narrow ? null : (
        <MiniMap
          position="top-right"
          pannable
          zoomable
          ariaLabel="Service map overview"
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
  const since = useSince()
  const search = useSearch({ from: '/map' })
  const navigate = useNavigate({ from: '/map' })
  const refetchInterval = useLiveInterval()
  const reduce = useMediaQuery(REDUCED_MOTION_QUERY)
  const { fitView } = useReactFlow()

  const map = useQuery({ ...api.serviceMap(since), refetchInterval, placeholderData: keepPreviousData })
  const config = useQuery(api.config())
  const grafana = grafanaMapUrl(config.data?.grafana_url)

  const graph = useMemo(() => (map.data ? mapGraph(map.data) : null), [map.data])
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
  const empty = map.isSuccess && graph !== null && graph.services.length === 0
  const q = search.q ?? ''

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3.5">
      <Card className="flex flex-wrap items-center gap-x-4 gap-y-2.5 px-4 py-3">
        <PanelTitle>Service map</PanelTitle>
        <span aria-live="polite" className="text-xs text-muted">
          {data && !empty ? mapSummary(data) : map.isPending ? 'Loading…' : ''}
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
          {grafana ? (
            <Button asChild size="sm" variant="secondary">
              <a href={grafana} target="_blank" rel="noreferrer noopener">
                <ExternalLink aria-hidden size={14} /> Open in Grafana
              </a>
            </Button>
          ) : null}
        </div>
      </Card>

      {map.isError && data ? <ErrorBanner what="the service map" error={map.error} onRetry={() => void map.refetch()} /> : null}

      <Card
        className="relative flex min-h-[460px] flex-1 overflow-hidden max-sm:min-h-[62dvh]"
        aria-busy={map.isFetching || layout.isFetching || undefined}
      >
        {map.isPending ? (
          <CanvasSkeleton label="Loading the service map" />
        ) : map.isError && !data ? (
          <ErrorState error={map.error} onRetry={() => void map.refetch()} className="m-auto" />
        ) : empty ? (
          <EmptyState
            className="m-auto"
            icon={<Network size={18} />}
            title="No service calls in this window"
            description={`Tayga has seen no spans in the last ${since}. A longer time range may show older calls.`}
          />
        ) : layout.isError ? (
          <ErrorState error={layout.error} title="Could not lay out the map" onRetry={() => void layout.refetch()} className="m-auto" />
        ) : !layout.data || !data ? (
          <CanvasSkeleton label={`Laying out ${graph?.services.length ?? 0} services`} />
        ) : (
          <section aria-label="Service map canvas" className="absolute inset-0">
            <p className="sr-only">{describeMap(data)}</p>
            <Canvas map={data} layout={layout.data} since={since} matches={matches} active={search.service} onOpen={onOpen} />
            <Legend />
          </section>
        )}
      </Card>

      <ServiceDrawer
        service={search.service}
        map={data}
        since={since}
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
