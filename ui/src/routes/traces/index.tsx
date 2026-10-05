/**
 * `/traces` explorer: filters (URL), a duration-over-time scatter whose rectangle brush
 * filters the results table, and a trace-id jump. Results refresh in live mode.
 */
import { keepPreviousData, useQuery } from '@tanstack/react-query'
import { useNavigate, useSearch } from '@tanstack/react-router'
import { useCallback, useMemo, useState } from 'react'
import { api } from '../../api/queries'
import type { TraceHit } from '../../api/types'
import { useLiveInterval } from '../../app/live'
import { formatRect, parseRect, sinceSearch } from '../../app/search'
import type { TracesSearch } from '../../app/search'
import { Scatter } from '../../components/charts/Scatter'
import type { ScatterPoint, ScatterTone, XYRect } from '../../components/charts/Scatter'
import { useSince } from '../../components/shell/TimeRange'
import { Button } from '../../components/ui/Button'
import { Card, PanelTitle } from '../../components/ui/Card'
import { EmptyState } from '../../components/ui/EmptyState'
import { Skeleton } from '../../components/ui/Skeleton'
import { RefreshNote, loadFailed } from '../../components/ui/StaleNote'
import { ToggleGroup } from '../../components/ui/ToggleGroup'
import { ErrorBanner } from '../../features/stories/ErrorBanner'
import { SINCE_SECS } from '../../features/stories/model'
import { TraceFilters } from '../../features/traces/TraceFilters'
import { TraceTable } from '../../features/traces/TraceTable'
import { TRACE_LIMIT, axisMs, plotMs, plotTime, rowsInRect, toneOf, traceQuery } from '../../features/traces/model'
import type { Sort } from '../../features/traces/model'
import { cx } from '../../lib/cx'
import { dateTime, duration } from '../../lib/format'
import { NARROW_QUERY, useMediaQuery } from '../../lib/useMediaQuery'
import { Reveal } from '../../components/ui/Reveal'

/** Error: the trace failed or has an error story. Slow: it has a slow story. */
const NAMES: Record<ScatterTone, string> = { accent: 'Other traces', slow: 'Slow stories', err: 'Errors and error stories' }
const DOT: Record<ScatterTone, string> = { accent: 'bg-accent', slow: 'bg-slow', err: 'bg-err' }
const SCALE = [
  { value: 'linear', label: 'Linear' },
  { value: 'log', label: 'Log' },
] as const

function Legend({ counts }: { counts: Record<ScatterTone, number> }) {
  return (
    <ul aria-label="Legend" className="m-0 flex list-none flex-wrap items-center gap-x-3 gap-y-1 p-0 text-xs text-muted">
      {(['accent', 'slow', 'err'] as const).map((t) => (
        <li key={t} className="flex items-center gap-1.5">
          <span aria-hidden className={cx('size-2 rounded-full', DOT[t])} />
          {NAMES[t]} <span className="tabular font-mono text-ink">{counts[t]}</span>
        </li>
      ))}
    </ul>
  )
}

function ResultsSkeleton() {
  return (
    <div aria-busy="true" aria-label="Loading traces" className="flex flex-col">
      {Array.from({ length: 8 }, (_, i) => (
        <div key={i} className="flex items-center gap-4 border-b border-line-soft px-4 py-3 last:border-b-0">
          <Skeleton className="h-3.5 w-20" />
          <Skeleton className="h-3.5 flex-1" />
          <Skeleton className="hidden h-3.5 w-40 sm:block" />
        </div>
      ))}
    </div>
  )
}

export function TracesExplorer() {
  const since = useSince()
  const search = useSearch({ from: '/traces/' })
  const navigate = useNavigate({ from: '/traces/' })
  const refetchInterval = useLiveInterval()
  const narrow = useMediaQuery(NARROW_QUERY)
  const [sort, setSort] = useState<Sort>({ key: 'start', desc: true })

  const services = useQuery(api.services())
  const traces = useQuery({
    ...api.traceSearch(traceQuery(search, since)),
    refetchInterval,
    // Keep the old points while a new filter loads, instead of flashing skeletons.
    placeholderData: keepPreviousData,
  })

  const onSearch = useCallback(
    (patch: Partial<TracesSearch>) =>
      void navigate({ search: (prev) => ({ ...prev, ...patch }), replace: true, resetScroll: false }),
    [navigate],
  )
  // A filter change replaces the plotted traces, so a brushed region no longer means anything.
  const onFilter = useCallback((patch: Partial<TracesSearch>) => onSearch({ ...patch, sel: undefined }), [onSearch])

  const rows: readonly TraceHit[] = useMemo(() => traces.data ?? [], [traces.data])
  const rect = useMemo(() => parseRect(search.sel), [search.sel])
  const selection = useMemo<XYRect | null>(() => (rect ? { x: [rect.t0, rect.t1], y: [rect.d0, rect.d1] } : null), [rect])
  const selected = useMemo(() => rowsInRect(rows, rect), [rows, rect])

  const byId = useMemo(() => new Map(rows.map((t) => [t.trace_id, t])), [rows])
  const points = useMemo<ScatterPoint[]>(
    () => rows.map((t) => ({ x: plotTime(t), y: plotMs(t), tone: toneOf(t), id: t.trace_id })),
    [rows],
  )
  const counts = useMemo(() => {
    const c: Record<ScatterTone, number> = { accent: 0, slow: 0, err: 0 }
    for (const p of points) c[p.tone]++
    return c
  }, [points])
  const endpoints = useMemo(() => {
    const m = new Map<string, number>()
    for (const t of rows) m.set(t.endpoint_name, (m.get(t.endpoint_name) ?? 0) + 1)
    return m
  }, [rows])
  // The endpoint list of each service as last seen without an endpoint filter, so with one
  // endpoint picked the picker still offers the others.
  const serviceKey = `${search.service ?? ''}|${search.touched ? 'any' : 'root'}`
  const [known, setKnown] = useState<ReadonlyMap<string, { data: unknown; endpoints: ReadonlyMap<string, number> }>>(new Map())
  if (!search.endpoint && traces.data && !traces.isPlaceholderData && known.get(serviceKey)?.data !== traces.data) {
    setKnown(new Map(known).set(serviceKey, { data: traces.data, endpoints }))
  }
  const endpointOptions = (search.endpoint ? known.get(serviceKey)?.endpoints : undefined) ?? endpoints

  const capped = rows.length >= TRACE_LIMIT
  const now = traces.dataUpdatedAt
  // The whole window, unless the limit cut it short: then the newest traces fill the width.
  const xRange = useMemo<[number, number] | undefined>(
    () => (capped || !now ? undefined : [now - SINCE_SECS[since] * 1000, now]),
    [capped, now, since],
  )

  const onSelect = useCallback(
    // Durations are never negative: a box dragged below the axis starts at 0.
    (r: XYRect | null) =>
      onSearch({ sel: r ? formatRect({ t0: r.x[0], t1: r.x[1], d0: Math.max(0, r.y[0]), d1: Math.max(0, r.y[1]) }) : undefined }),
    [onSearch],
  )
  const onPointClick = useCallback(
    (id: string) => void navigate({ to: '/traces/$traceId', params: { traceId: id }, search: sinceSearch(since) }),
    [navigate, since],
  )
  const tooltip = useCallback(
    (id: string) => {
      const t = byId.get(id)
      if (!t) return { title: id, lines: [] }
      return {
        title: `${t.endpoint_service} · ${t.endpoint_name}`,
        lines: [
          `${duration(t.duration_ns)} · ${t.span_count} spans${t.is_error ? ' · error' : ''}${t.story_kind ? ` · ${t.story_kind} story` : ''}`,
          dateTime(t.ts_ns),
          'Click to open the trace',
        ],
      }
    },
    [byId],
  )

  const logY = Boolean(search.log)
  const summary = useMemo(() => {
    if (rows.length === 0) return `No traces in the last ${since}.`
    let lo = Infinity
    let hi = 0
    for (const t of rows) {
      lo = Math.min(lo, t.duration_ns)
      hi = Math.max(hi, t.duration_ns)
    }
    return (
      `Duration over time of ${rows.length} traces in the last ${since}: ${counts.err} with errors or error stories, ${counts.slow} with slow stories; ` +
      `durations from ${duration(lo)} to ${duration(hi)}.` +
      (rect ? ` ${selected.length} selected between ${duration(rect.d0 * 1e6)} and ${duration(rect.d1 * 1e6)}.` : '')
    )
  }, [rows, since, counts, rect, selected])

  const filtered = Boolean(search.service || search.endpoint || search.min_ms !== undefined || search.max_ms !== undefined || search.errors)
  const empty = traces.isSuccess && rows.length === 0
  // Only the endpoint service is matched: the service may still appear deeper in traces.
  const rootOnly = Boolean(search.service && !search.touched)

  return (
    <div className="flex flex-col gap-3.5">
      <Card className="overflow-visible">
        <TraceFilters
          search={search}
          since={since}
          services={services.data ?? []}
          endpoints={endpointOptions}
          onSearch={onFilter}
        />
      </Card>

      {loadFailed(traces) ? <ErrorBanner what="traces" error={traces.error} onRetry={() => void traces.refetch()} /> : null}
      <RefreshNote queries={[traces]} />

      <Card className="flex min-w-0 flex-col gap-2 px-4 py-3.5" aria-busy={traces.isFetching || undefined}>
        <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
          <PanelTitle>Duration over time</PanelTitle>
          {rows.length > 0 ? <Legend counts={counts} /> : null}
          <div className="ml-auto flex items-center gap-2">
            {rect ? (
              <Button size="sm" onClick={() => onSearch({ sel: undefined })}>
                Clear selection
              </Button>
            ) : null}
            <ToggleGroup
              label="Duration scale"
              options={SCALE}
              value={logY ? 'log' : 'linear'}
              onValueChange={(v) => onSearch({ log: v === 'log' ? true : undefined })}
            />
          </div>
        </div>
        {traces.isPending ? (
          <Skeleton style={{ height: narrow ? 200 : 260 }} />
        ) : empty ? (
          <EmptyState
            title="No traces match"
            description={
              rootOnly
                ? `No trace in the last ${since} starts at ${search.service} with these filters. It may still take part in others.`
                : filtered
                  ? `Nothing in the last ${since} matches these filters.`
                  : `No traces in the last ${since}. A longer time range may show older ones.`
            }
            action={
              rootOnly ? (
                <Button size="sm" onClick={() => onFilter({ touched: true, endpoint: undefined })}>
                  Match {search.service} anywhere in the trace
                </Button>
              ) : filtered ? (
                <Button
                  size="sm"
                  onClick={() => onFilter({ service: undefined, touched: undefined, endpoint: undefined, min_ms: undefined, max_ms: undefined, errors: undefined })}
                >
                  Clear filters
                </Button>
              ) : null
            }
          />
        ) : rows.length > 0 ? (
          <>
            <Scatter
              points={points}
              names={NAMES}
              logY={logY}
              xRange={xRange}
              formatY={axisMs}
              tooltip={tooltip}
              selection={selection}
              onSelect={onSelect}
              onPointClick={onPointClick}
              height={narrow ? 200 : 260}
              summary={summary}
            />
            <p className="m-0 text-xs text-muted">
              {narrow ? 'Tap a point to open its trace.' : 'Drag across the chart to select traces; click a point to open its trace.'}
              {capped ? ` Showing the newest ${TRACE_LIMIT}; narrow the filters to reach older traces.` : ''}
            </p>
          </>
        ) : null}
      </Card>

      {rows.length > 0 ? (
        <Card className="overflow-hidden">
          <div className="flex flex-wrap items-center gap-2 border-b border-line px-4 py-2.5 text-xs text-muted">
            <PanelTitle>Traces</PanelTitle>
            <span aria-live="polite" className="ml-auto">
              {rect ? `${selected.length} of ${rows.length} selected` : `${rows.length} ${rows.length === 1 ? 'trace' : 'traces'}`}
              {capped ? ' (limit reached)' : ''}
            </span>
          </div>
          {selected.length === 0 ? (
            <EmptyState
              title="No traces in the selection"
              description="The selected area holds no points. Select another area or clear the selection."
              action={
                <Button size="sm" onClick={() => onSearch({ sel: undefined })}>
                  Clear selection
                </Button>
              }
            />
          ) : (
            <Reveal>
              <TraceTable rows={selected} extent={rows} logY={logY} since={since} sort={sort} onSort={setSort} />
            </Reveal>
          )}
        </Card>
      ) : traces.isPending ? (
        <Card className="overflow-hidden">
          <ResultsSkeleton />
        </Card>
      ) : null}
    </div>
  )
}
