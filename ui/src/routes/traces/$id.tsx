/** `/traces/:id`: header, story banner, the waterfall and the span drawer. */
import { useQuery } from '@tanstack/react-query'
import { Link, useNavigate, useParams, useSearch } from '@tanstack/react-router'
import { ArrowRight } from 'lucide-react'
import { useCallback, useMemo } from 'react'
import { isApiError } from '../../api/client'
import { api } from '../../api/queries'
import type { StoryView, TraceView } from '../../api/types'
import { sinceSearch } from '../../app/search'
import type { TraceSearch } from '../../app/search'
import { useSince } from '../../components/shell/TimeRange'
import { computeCriticalPath } from '../../components/trace/layout'
import { TraceDetail } from '../../components/trace/TraceDetail'
import { JaegerLink, NotFoundCard } from '../../components/trace/TraceLinks'
import { Badge } from '../../components/ui/Badge'
import { Button } from '../../components/ui/Button'
import { Card } from '../../components/ui/Card'
import { ErrorState } from '../../components/ui/ErrorState'
import { Skeleton } from '../../components/ui/Skeleton'
import { dateTime, duration, shortId } from '../../lib/format'

/** Header numbers derived from the spans. */
export function traceStats(trace: TraceView) {
  let start = Infinity
  let end = -Infinity
  const services = new Set<string>()
  let errors = 0
  const ids = new Set(trace.spans.map((s) => s.span_id))
  let root: TraceView['spans'][number] | undefined
  for (const s of trace.spans) {
    start = Math.min(start, s.start_ns)
    end = Math.max(end, s.start_ns + s.duration_ns)
    services.add(s.service_name)
    if (s.status === 'error') errors++
    // The entry span: a root (parent unknown or itself), earliest first, longest on ties.
    const isRoot = !ids.has(s.parent_span_id) || s.parent_span_id === s.span_id
    if (isRoot && (!root || s.start_ns < root.start_ns || (s.start_ns === root.start_ns && s.duration_ns > root.duration_ns)))
      root = s
  }
  return {
    root: root ?? trace.spans[0],
    startNs: Number.isFinite(start) ? start : 0,
    durationNs: Number.isFinite(start) ? Math.max(0, end - start) : 0,
    spans: trace.spans.length,
    services: services.size,
    errors,
    logs: trace.logs.length,
  }
}

function Stat({ label, value, tone }: { label: string; value: string; tone?: 'err' }) {
  return (
    <div className="flex flex-col gap-0.5">
      <dt className="text-[11px] uppercase tracking-[0.06em] text-muted">{label}</dt>
      <dd className={tone === 'err' ? 'tabular m-0 font-mono text-[15px] text-err' : 'tabular m-0 font-mono text-[15px] text-ink'}>
        {value}
      </dd>
    </div>
  )
}

function StoryBanner({ storyId, story }: { storyId: string; story: StoryView | undefined }) {
  const since = useSince()
  return (
    <Card
      variant="inner"
      className="tg-in flex flex-wrap items-center gap-3 px-4 py-3"
      role="region"
      aria-label="Story for this trace"
    >
      {story ? (
        <Badge kind={story.kind} shape="pill" glow>
          {story.kind === 'error' ? 'Error story' : 'Slow story'}
        </Badge>
      ) : (
        <Badge kind="neutral">story</Badge>
      )}
      <span className="min-w-0 flex-1 truncate text-ink">
        {story ? story.summary : 'Tayga built a story for this trace.'}
      </span>
      <Button asChild size="sm" variant="primary">
        <Link to="/stories/$storyId" params={{ storyId }} search={sinceSearch(since)}>
          Open story <ArrowRight aria-hidden size={14} />
        </Link>
      </Button>
    </Card>
  )
}

function TraceSkeleton() {
  return (
    <div className="flex flex-col gap-4" aria-busy="true" aria-label="Loading trace">
      <Card className="flex flex-col gap-3 p-5">
        <Skeleton className="h-5 w-1/3" />
        <Skeleton className="h-4 w-2/3" />
      </Card>
      <Card className="flex flex-col gap-2 p-5">
        <Skeleton className="h-9 w-1/2" />
        <Skeleton className="h-11" />
        {Array.from({ length: 10 }, (_, i) => (
          <Skeleton key={i} className="h-6" style={{ marginLeft: `${(i % 4) * 14}px` }} />
        ))}
      </Card>
    </div>
  )
}

/** URL updater for the waterfall state (span, q, only). */
function useTraceSearchUpdater() {
  const navigate = useNavigate({ from: '/traces/$traceId' })
  return useCallback(
    (patch: Partial<TraceSearch>, opts?: { push?: boolean }) => {
      void navigate({ search: (prev) => ({ ...prev, ...patch }), replace: !opts?.push, resetScroll: false })
    },
    [navigate],
  )
}

export function TracePage() {
  const { traceId } = useParams({ from: '/traces/$traceId' })
  const search = useSearch({ from: '/traces/$traceId' })
  const onSearch = useTraceSearchUpdater()
  const trace = useQuery(api.trace(traceId))
  const storyId = trace.data?.story_id ?? null
  const story = useQuery({ ...api.story(storyId ?? ''), enabled: storyId !== null })
  const hasLogs = (trace.data?.logs.length ?? 0) > 0
  const templates = useQuery({ ...api.traceLogTemplates(traceId), enabled: hasLogs })
  const stats = useMemo(() => (trace.data ? traceStats(trace.data) : null), [trace.data])
  // A story carries the analysed critical path and root cause; otherwise derive the path.
  const critical = useMemo(() => {
    if (story.data) return story.data.critical_path.segments.map((s) => s.span_id)
    return trace.data ? [...computeCriticalPath(trace.data.spans)] : []
  }, [story.data, trace.data])

  if (trace.isPending) return <TraceSkeleton />
  if (trace.isError) {
    if (isApiError(trace.error) && trace.error.status === 404)
      return (
        <NotFoundCard
          title="Trace not found"
          description={`No spans or logs are stored for trace ${shortId(traceId)}. It may have expired, or not been ingested yet.`}
        />
      )
    return (
      <Card>
        <ErrorState error={trace.error} onRetry={() => void trace.refetch()} />
      </Card>
    )
  }
  // Wait for the story (if any) so the waterfall opens with its root cause and critical path.
  const waitingForStory = storyId !== null && story.isPending
  const s = stats!
  const root = s.root

  return (
    <div className="flex flex-col gap-4">
      <Card className="tg-in flex flex-col gap-4 px-5 py-4">
        <div className="flex flex-wrap items-start gap-3">
          <div className="flex min-w-[min(100%,320px)] flex-1 flex-col gap-1">
            <span className="font-mono text-[11px] text-muted">trace {traceId}</span>
            <h2 className="m-0 truncate text-[17px] font-semibold">
              {root ? (
                <>
                  <span className="text-muted">{root.service_name}</span> {root.span_name}
                </>
              ) : (
                'Trace without spans'
              )}
            </h2>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <JaegerLink traceId={traceId} />
          </div>
        </div>
        <dl className="m-0 grid grid-cols-[repeat(auto-fit,minmax(110px,1fr))] gap-4">
          <Stat label="Duration" value={duration(s.durationNs)} />
          <Stat label="Spans" value={String(s.spans)} />
          <Stat label="Services" value={String(s.services)} />
          <Stat label="Errors" value={String(s.errors)} tone={s.errors > 0 ? 'err' : undefined} />
          <Stat label="Logs" value={String(s.logs)} />
          <Stat label="Started" value={s.spans > 0 ? dateTime(s.startNs) : '—'} />
        </dl>
      </Card>

      {storyId ? <StoryBanner storyId={storyId} story={story.data} /> : null}

      <Card className="tg-in px-5 py-4">
        {waitingForStory ? (
          <div className="flex flex-col gap-2" aria-busy="true">
            {Array.from({ length: 8 }, (_, i) => (
              <Skeleton key={i} className="h-6" />
            ))}
          </div>
        ) : (
          <TraceDetail
            trace={trace.data}
            critical={critical}
            rootCauseId={story.data?.root_cause.span_id ?? null}
            templates={templates.data}
            search={search}
            onSearch={onSearch}
            height="max(360px, calc(100dvh - 430px))"
          />
        )}
      </Card>
    </div>
  )
}
