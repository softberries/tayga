/**
 * `/stories/:id`: header, group trend, the waterfall opened at the root cause, "compared with
 * normal", the story's logs and related log alerts.
 */
import { useQuery } from '@tanstack/react-query'
import type { UseQueryResult } from '@tanstack/react-query'
import { Link, useNavigate, useParams, useSearch } from '@tanstack/react-router'
import { ArrowRight } from 'lucide-react'
import { useCallback, useMemo, useState } from 'react'
import { isApiError } from '../../api/client'
import { api } from '../../api/queries'
import type { GroupDetail, LogAlertView, StoryView, TraceLogTemplate, TraceView } from '../../api/types'
import { rangeBounds, rangeParams, rangePhrase, rangeSearch, trendRange } from '../../app/range'
import type { Range } from '../../app/range'
import type { StorySearch } from '../../app/search'
import { TimeSeries } from '../../components/charts/TimeSeries'
import { useRange } from '../../app/useRange'
import { LogTable } from '../../components/trace/LogTable'
import { PathChips } from '../../components/trace/PathChips'
import { TraceDetail } from '../../components/trace/TraceDetail'
import { JaegerLink, NotFoundCard } from '../../components/trace/TraceLinks'
import { Badge } from '../../components/ui/Badge'
import { Button } from '../../components/ui/Button'
import { Card, PanelTitle } from '../../components/ui/Card'
import { EmptyState } from '../../components/ui/EmptyState'
import { ErrorState } from '../../components/ui/ErrorState'
import { Skeleton } from '../../components/ui/Skeleton'
import { RefreshNote } from '../../components/ui/StaleNote'
import { compact, dateTime, duration, shortId } from '../../lib/format'

function useStorySearchUpdater() {
  const navigate = useNavigate({ from: '/stories/$storyId' })
  return useCallback(
    (patch: Partial<StorySearch>, opts?: { push?: boolean }) => {
      void navigate({ search: (prev) => ({ ...prev, ...patch }), replace: !opts?.push, resetScroll: false })
    },
    [navigate],
  )
}

function Fact({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex flex-col gap-0.5">
      <dt className="text-[11px] uppercase tracking-[0.06em] text-muted">{label}</dt>
      <dd className="tabular m-0 font-mono text-[14px] text-ink">{value}</dd>
    </div>
  )
}

function StoryHeader({ story, services }: { story: StoryView; services: number | null }) {
  const rc = story.root_cause
  return (
    <Card className="tg-in flex flex-col gap-4 px-5 py-4">
      <div className="flex flex-wrap items-start gap-3">
        <div className="flex min-w-[min(100%,320px)] flex-1 flex-col gap-2">
          <div className="flex flex-wrap items-center gap-2">
            <Badge kind={story.kind} shape="pill" glow>
              {story.kind === 'error' ? 'Error story' : 'Slow story'}
            </Badge>
            <span className="font-mono text-[11px] text-muted">
              {story.endpoint_service} {story.endpoint_name}
            </span>
          </div>
          <h2 className="m-0 break-words text-[18px] font-semibold leading-snug [text-wrap:balance]">{story.summary}</h2>
          <p className="m-0 text-muted">
            Root cause in <span className="text-ink">{rc.service}</span>{' '}
            <span className="font-mono text-[12px] text-ink">{rc.span_name}</span>
            {rc.exception_type ? (
              <>
                {' '}
                · <span className="font-mono text-[12px] text-err">{rc.exception_type}</span>
              </>
            ) : null}
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <JaegerLink traceId={story.trace_id} />
          <Button asChild size="sm" variant="secondary">
            <Link to="/traces/$traceId" params={{ traceId: story.trace_id }}>
              Open trace <ArrowRight aria-hidden size={14} />
            </Link>
          </Button>
        </div>
      </div>
      <PathChips services={story.path_services} kind={story.kind} />
      <dl className="m-0 grid grid-cols-[repeat(auto-fit,minmax(110px,1fr))] gap-4">
        <Fact label="Time" value={dateTime(story.ts_ns)} />
        <Fact label="Root span" value={duration(story.duration_ns)} />
        <Fact label="Spans" value={String(story.span_count)} />
        <Fact label="Services" value={services === null ? String(story.path_services.length) : String(services)} />
        <Fact label="Trace" value={shortId(story.trace_id)} />
      </dl>
      {story.flags.length > 0 ? (
        <ul aria-label="Flags" className="m-0 flex list-none flex-wrap gap-1.5 p-0">
          {story.flags.map((f) => (
            <li key={f}>
              <Badge kind="slow">{f}</Badge>
            </li>
          ))}
        </ul>
      ) : null}
    </Card>
  )
}

function GroupTrend({ story, group, range }: { story: StoryView; group: UseQueryResult<GroupDetail>; range: Range }) {
  const points = useMemo(
    () => (group.data?.group.buckets ?? []).map(([t, n]) => [t * 1000, n] as const),
    [group.data],
  )
  const [start, end] = rangeBounds(range, group.dataUpdatedAt)
  // Bars sit at their bucket's start: half a bucket on each side keeps the first and last whole.
  const step = (group.data?.group.bucket_secs ?? 60) * 1000
  const xRange = useMemo(() => [start - step / 2, end + step / 2] as const, [start, end, step])
  const series = useMemo(
    () => [{ name: 'Stories', points, tone: story.kind === 'slow' ? ('slow' as const) : ('err' as const), type: 'bar' as const }],
    [points, story.kind],
  )
  const peak = points.reduce((m, [, n]) => Math.max(m, n), 0)
  return (
    <Card className="tg-in flex min-w-0 flex-col gap-3 px-5 py-4">
      <div className="flex items-baseline gap-2">
        <PanelTitle>Group trend</PanelTitle>
        {group.data ? (
          <span className="ml-auto text-xs text-muted">
            {compact(group.data.group.stories)} stories · {range.label}
          </span>
        ) : null}
      </div>
      {group.isPending ? (
        <Skeleton className="h-[180px]" />
      ) : !group.data ? (
        isApiError(group.error) && group.error.status === 404 ? (
          <EmptyState title="No stories of this group in the range" description="Pick a longer time range to see its trend." />
        ) : (
          <ErrorState error={group.error} onRetry={() => void group.refetch()} />
        )
      ) : points.length === 0 ? (
        <EmptyState title="No stories of this group in the range" />
      ) : (
        <TimeSeries
          series={series}
          height={180}
          markAt={story.ts_ns / 1e6}
          markLabel="this story"
          xRange={xRange}
          summary={`Stories per ${group.data.group.bucket_secs} seconds in this group over ${rangePhrase(range)}: ${group.data.group.stories} in total, at most ${peak} in one bucket.`}
        />
      )}
    </Card>
  )
}

function ComparedWithNormal({ story }: { story: StoryView }) {
  const d = story.baseline_diff
  const top = story.critical_path.top.slice(0, 5)
  const maxSelf = top.reduce((m, c) => Math.max(m, c.self_time_ns), 1)
  const empty = !d || (d.new_ops.length === 0 && d.missing_ops.length === 0 && d.slower_ops.length === 0)
  return (
    <Card className="tg-in flex min-w-0 flex-col gap-4 px-5 py-4">
      <PanelTitle>Compared with normal</PanelTitle>
      {empty ? (
        <p className="m-0 text-muted">
          {d ? 'Every operation ran as usual for this endpoint.' : 'No baseline for this endpoint yet.'}
        </p>
      ) : (
        <ul className="m-0 flex list-none flex-col gap-2 p-0">
          {d.slower_ops.map((o) => (
            <li key={`slow-${o.op}`}>
              Slower than usual: <span className="font-mono text-slow">{o.op}</span>{' '}
              <span className="text-muted">
                {duration(o.duration_ns)} vs p95 {duration(o.baseline_p95_ns)}
              </span>
            </li>
          ))}
          {d.new_ops.map((op) => (
            <li key={`new-${op}`}>
              New operation: <span className="font-mono text-accent">{op}</span>
            </li>
          ))}
          {d.missing_ops.length > 0 ? (
            <li>
              Usually present, missing here:{' '}
              {d.missing_ops.map((op, i) => (
                <span key={op}>
                  {i > 0 ? ', ' : null}
                  <span className="font-mono text-slow">{op}</span>
                </span>
              ))}
            </li>
          ) : null}
        </ul>
      )}
      {story.also_failed.length > 0 ? (
        <div className="flex flex-col gap-1.5">
          <span className="text-[11px] uppercase tracking-[0.06em] text-muted">Also failed</span>
          <ul className="m-0 flex list-none flex-wrap gap-1.5 p-0">
            {story.also_failed.map((s) => (
              <li key={s.span_id}>
                <Badge kind="error">
                  {s.service} {s.name}
                </Badge>
              </li>
            ))}
          </ul>
        </div>
      ) : null}
      {top.length > 0 ? (
        <div className="flex flex-col gap-1.5">
          <span className="text-[11px] uppercase tracking-[0.06em] text-muted">Where the time went (critical path)</span>
          <ul className="m-0 flex list-none flex-col gap-1.5 p-0">
            {top.map((c) => (
              <li key={c.span_id} className="grid grid-cols-[minmax(0,1fr)_minmax(60px,30%)_64px] items-center gap-2 text-[12px]">
                <span className="truncate">
                  <span className="text-muted">{c.service}</span> {c.name}
                </span>
                <span aria-hidden className="h-1.5 rounded-full bg-track">
                  <span className="block h-full rounded-full bg-accent" style={{ width: `${(c.self_time_ns / maxSelf) * 100}%` }} />
                </span>
                <span className="tabular text-right font-mono text-[11px] text-muted">{duration(c.self_time_ns)}</span>
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </Card>
  )
}

function RelatedAlerts({
  alerts,
  traceId,
  templates,
}: {
  alerts: UseQueryResult<LogAlertView[]>
  traceId: string
  templates: readonly TraceLogTemplate[] | undefined
}) {
  const related = useMemo(() => {
    const ids = new Set((templates ?? []).map((t) => t.template_id))
    return (alerts.data ?? []).filter((a) => ids.has(a.template_id) || a.example_traces.some((e) => e.trace_id === traceId))
  }, [alerts.data, templates, traceId])
  return (
    <Card className="tg-in flex flex-col gap-3 px-5 py-4">
      <PanelTitle>Related log alerts</PanelTitle>
      {alerts.isPending ? (
        <Skeleton className="h-16" />
      ) : !alerts.data ? (
        <ErrorState error={alerts.error} onRetry={() => void alerts.refetch()} />
      ) : related.length === 0 ? (
        <p className="m-0 text-muted">No log alert in the last 7 days involves this story's logs.</p>
      ) : (
        <ul className="m-0 flex list-none flex-col p-0">
          {related.map((a) => (
            <li key={a.alert_id} className="flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-line-soft py-2 last:border-b-0">
              <Badge kind={a.kind}>{a.kind}</Badge>
              <span className="text-muted">{a.service}</span>
              <Link
                to="/logs/templates/$templateId"
                params={{ templateId: a.template_id }}
                className="min-w-0 flex-1 truncate font-mono text-[12px] text-accent hover:underline"
                title={a.template}
              >
                {a.template}
              </Link>
              <span className="tabular font-mono text-[11px] text-muted">{dateTime(a.started_at_ns)}</span>
              <span className="tabular font-mono text-[11px] text-muted">
                peak {a.peak_count} vs {a.baseline_per_window.toFixed(1)} / window
              </span>
              {a.active ? (
                <Badge kind="error" shape="pill" pulse>
                  active
                </Badge>
              ) : (
                <span className="text-[11px] text-muted">ended</span>
              )}
            </li>
          ))}
        </ul>
      )}
    </Card>
  )
}

function StorySkeleton() {
  return (
    <div className="flex flex-col gap-4" aria-busy="true" aria-label="Loading story">
      <Card className="flex flex-col gap-3 p-5">
        <Skeleton className="h-5 w-24" />
        <Skeleton className="h-6 w-2/3" />
        <Skeleton className="h-4 w-1/2" />
      </Card>
      <div className="grid gap-4 lg:grid-cols-2">
        <Skeleton className="h-[240px] rounded-panel" />
        <Skeleton className="h-[240px] rounded-panel" />
      </div>
      <Skeleton className="h-[360px] rounded-panel" />
    </div>
  )
}

function WaterfallCard({
  story,
  trace,
  templates,
  search,
  onSearch,
}: {
  story: StoryView
  trace: UseQueryResult<TraceView>
  templates: readonly TraceLogTemplate[] | undefined
  search: StorySearch
  onSearch: ReturnType<typeof useStorySearchUpdater>
}) {
  const critical = useMemo(() => story.critical_path.segments.map((s) => s.span_id), [story])
  return (
    <Card className="tg-in flex flex-col gap-3 px-5 py-4">
      <PanelTitle>Waterfall</PanelTitle>
      {trace.isPending ? (
        <div className="flex flex-col gap-2" aria-busy="true">
          {Array.from({ length: 8 }, (_, i) => (
            <Skeleton key={i} className="h-6" />
          ))}
        </div>
      ) : !trace.data ? (
        isApiError(trace.error) && trace.error.status === 404 ? (
          <EmptyState title="The trace is no longer stored" description="Its spans have expired; the story summary above still applies." />
        ) : (
          <ErrorState error={trace.error} onRetry={() => void trace.refetch()} />
        )
      ) : (
        <TraceDetail
          trace={trace.data}
          critical={critical}
          rootCauseId={story.root_cause.span_id}
          templates={templates}
          search={search}
          onSearch={onSearch}
          height="min(62vh, 640px)"
          initialZoomTo={critical}
          zoomHint="Showing the critical path"
        />
      )}
    </Card>
  )
}

export function StoryPage() {
  const { storyId } = useParams({ from: '/stories/$storyId' })
  const search = useSearch({ from: '/stories/$storyId' })
  const onSearch = useStorySearchUpdater()
  const range = useRange()
  const story = useQuery(api.story(storyId))
  const s = story.data
  const trace = useQuery({ ...api.trace(s?.trace_id ?? ''), enabled: s !== undefined })
  // The trend uses the header range, widened until it contains the story itself, so an
  // older story still shows its group (and the API does not answer 404 for an empty range).
  const [now] = useState(() => Date.now())
  const trend = s ? trendRange(range, s.ts_ns, now) : range
  const group = useQuery({ ...api.storyGroup(s?.fingerprint ?? '', rangeParams(trend)), enabled: s !== undefined })
  const hasLogs = (trace.data?.logs.length ?? 0) > 0
  const templates = useQuery({ ...api.traceLogTemplates(s?.trace_id ?? ''), enabled: hasLogs })
  const alerts = useQuery({ ...api.logAlerts({ since: '7d' }), enabled: s !== undefined })
  const spanNames = useMemo(() => new Map((trace.data?.spans ?? []).map((sp) => [sp.span_id, sp.span_name])), [trace.data])
  const templateMap = useMemo(() => new Map((templates.data ?? []).map((t) => [t.log_id, t])), [templates.data])
  const services = useMemo(
    () => (trace.data ? new Set(trace.data.spans.map((sp) => sp.service_name)).size : null),
    [trace.data],
  )

  if (story.isPending) return <StorySkeleton />
  if (!story.data) {
    if (isApiError(story.error) && story.error.status === 404)
      return (
        <NotFoundCard
          title="Story not found"
          description={`There is no story ${shortId(storyId)}. It may have expired, or the link is wrong.`}
        />
      )
    return (
      <Card>
        <ErrorState error={story.error} onRetry={() => void story.refetch()} />
      </Card>
    )
  }
  const st = story.data
  return (
    <div className="flex flex-col gap-4">
      <RefreshNote queries={[story, trace, group, templates, alerts]} />
      <StoryHeader story={st} services={services} />
      <div className="grid gap-4 lg:grid-cols-2">
        <GroupTrend story={st} group={group} range={trend} />
        <ComparedWithNormal story={st} />
      </div>
      <WaterfallCard story={st} trace={trace} templates={templates.data} search={search} onSearch={onSearch} />
      <Card className="tg-in flex flex-col gap-3 px-5 py-4" id="logs">
        <div className="flex items-baseline gap-2">
          <PanelTitle>Logs</PanelTitle>
          {trace.data ? <span className="text-xs text-muted">{trace.data.logs.length} in this trace</span> : null}
        </div>
        {trace.isPending ? (
          <Skeleton className="h-40" />
        ) : !trace.data ? (
          <p className="m-0 text-muted">Logs are unavailable while the trace cannot be loaded.</p>
        ) : (
          <LogTable
            logs={trace.data.logs}
            spanNames={spanNames}
            templates={templateMap}
            service={search.log_service}
            onServiceChange={(svc) => onSearch({ log_service: svc })}
            sev={search.sev}
            onSevChange={(sev) => onSearch({ sev })}
            onSpanClick={(id) => onSearch({ span: id }, { push: true })}
          />
        )}
      </Card>
      <RelatedAlerts alerts={alerts} traceId={st.trace_id} templates={templates.data} />
      <p className="m-0 text-xs text-muted">
        <Link to="/" search={rangeSearch(range)} className="text-accent hover:underline">
          Back to stories
        </Link>
      </p>
    </div>
  )
}
