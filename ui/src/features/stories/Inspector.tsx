/**
 * The selected group's sample story beside the table: request path, compact waterfall,
 * "compared with normal" and links. Resizable from its left edge on wide screens (width kept
 * in localStorage); it slides in once and cross-fades between groups.
 */
import { useQuery } from '@tanstack/react-query'
import type { UseQueryResult } from '@tanstack/react-query'
import { Link, useNavigate } from '@tanstack/react-router'
import { ArrowRight } from 'lucide-react'
import { m } from 'motion/react'
import { useCallback, useMemo } from 'react'
import type { ReactNode } from 'react'
import { isApiError } from '../../api/client'
import { api } from '../../api/queries'
import type { StoryGroup, StoryView, TraceView } from '../../api/types'
import { PathChips } from '../../components/trace/PathChips'
import { Waterfall } from '../../components/trace/Waterfall'
import { Button } from '../../components/ui/Button'
import { EmptyState } from '../../components/ui/EmptyState'
import { ErrorState } from '../../components/ui/ErrorState'
import { Skeleton } from '../../components/ui/Skeleton'
import { useResizableWidth } from '../../components/ui/useResizableWidth'
import { cx } from '../../lib/cx'
import { duration, shortId } from '../../lib/format'
import { useMediaQuery } from '../../lib/useMediaQuery'
import { splitSummary } from './model'

export const INSPECTOR_MIN = 360
export const INSPECTOR_KEY = 'tayga-inspector-width'
/** Side by side with the table from here; stacked below it otherwise. */
export const WIDE_QUERY = '(min-width: 1180px)'

const inspectorMax = () => Math.max(INSPECTOR_MIN, Math.min(760, Math.round(window.innerWidth * 0.5)))

const OPS_SHOWN = 5
const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`

function Compared({ story }: { story: StoryView }) {
  const d = story.baseline_diff
  const empty = !d || (d.new_ops.length === 0 && d.missing_ops.length === 0 && d.slower_ops.length === 0)
  // The story page lists them all; the inspector keeps the first few.
  const ops = (list: readonly string[], tone: string) => (
    <>
      {list.slice(0, OPS_SHOWN).map((op, i) => (
        <span key={op}>
          {i > 0 ? ', ' : null}
          <span className={cx('font-mono', tone)}>{op}</span>
        </span>
      ))}
      {list.length > OPS_SHOWN ? <span className="text-muted"> and {list.length - OPS_SHOWN} more</span> : null}
    </>
  )
  return (
    <section aria-label="Compared with normal" className="flex flex-col gap-2 rounded-field border border-panel-line bg-inner p-3">
      <span className="text-[11px] uppercase tracking-[0.06em] text-muted">Compared with normal</span>
      {empty ? (
        <span className="text-muted">{d ? 'Every operation ran as usual for this endpoint.' : 'No baseline for this endpoint yet.'}</span>
      ) : (
        <ul className="m-0 flex list-none flex-col gap-1.5 p-0">
          {d.missing_ops.length > 0 ? <li>Usually present, missing here: {ops(d.missing_ops, 'text-slow')}</li> : null}
          {d.slower_ops.slice(0, 3).map((o) => (
            <li key={o.op}>
              Slower than usual: <span className="font-mono text-slow">{o.op}</span>{' '}
              <span className="text-muted">
                {duration(o.duration_ns)} vs p95 {duration(o.baseline_p95_ns)}
              </span>
            </li>
          ))}
          {d.new_ops.length > 0 ? <li>New here: {ops(d.new_ops, 'text-accent')}</li> : null}
        </ul>
      )}
    </section>
  )
}

function WaterfallPreview({ story, trace }: { story: StoryView; trace: UseQueryResult<TraceView> }) {
  const navigate = useNavigate()
  const critical = useMemo(() => story.critical_path.segments.map((s) => s.span_id), [story])
  const open = useCallback(
    (span: string) => void navigate({ to: '/stories/$storyId', params: { storyId: story.story_id }, search: { span } }),
    [navigate, story.story_id],
  )
  if (trace.isPending)
    return (
      <div className="flex flex-col gap-1.5" aria-busy="true" aria-label="Loading waterfall">
        {Array.from({ length: 6 }, (_, i) => (
          <Skeleton key={i} className="h-[19px]" />
        ))}
      </div>
    )
  if (trace.isError)
    return isApiError(trace.error) && trace.error.status === 404 ? (
      <p className="m-0 text-muted">The trace is no longer stored; the story summary still applies.</p>
    ) : (
      <ErrorState error={trace.error} onRetry={() => void trace.refetch()} className="py-4" />
    )
  return (
    <Waterfall
      compact
      spans={trace.data.spans}
      critical={critical}
      rootCauseId={story.root_cause.span_id}
      traceId={story.trace_id}
      maxRows={10}
      onOpen={open}
    />
  )
}

function StoryBody({ group }: { group: StoryGroup }) {
  const story = useQuery(api.story(group.sample_story_id))
  const s = story.data
  const trace = useQuery({ ...api.trace(s?.trace_id ?? ''), enabled: s !== undefined })
  if (story.isPending)
    return (
      <div className="flex flex-col gap-3" aria-busy="true" aria-label="Loading story">
        <Skeleton className="h-3 w-48" />
        <Skeleton className="h-5 w-4/5" />
        <Skeleton className="h-6 w-3/5" />
        <Skeleton className="h-40" />
      </div>
    )
  if (story.isError)
    return isApiError(story.error) && story.error.status === 404 ? (
      <EmptyState title="This story has expired" description="Its group is still counted; newer stories will replace the sample." />
    ) : (
      <ErrorState error={story.error} onRetry={() => void story.refetch()} />
    )
  const st = story.data
  const slow = st.kind === 'slow'
  const { title, detail } = splitSummary(st.summary)
  const services = trace.data ? new Set(trace.data.spans.map((sp) => sp.service_name)).size : null
  return (
    <>
      <div className="flex flex-col gap-1.5">
        <span className={cx('font-mono text-[11px]', slow ? 'text-slow' : 'text-err')}>
          {slow ? 'SLOW' : 'ERROR'} · trace {shortId(st.trace_id)} · {duration(st.duration_ns)}
        </span>
        <h2 className="m-0 break-words text-[17px] font-semibold leading-snug [text-wrap:balance]">{title}</h2>
        {detail ? (
          <p className="m-0 line-clamp-3 break-words text-muted" title={detail}>
            {detail}
          </p>
        ) : null}
      </div>
      <PathChips services={st.path_services} kind={st.kind} />
      <WaterfallPreview story={st} trace={trace} />
      <Compared story={st} />
      <div className="flex flex-wrap gap-2">
        <Button asChild variant="primary">
          <Link to="/stories/$storyId" params={{ storyId: st.story_id }}>
            Open story <ArrowRight aria-hidden size={14} />
          </Link>
        </Button>
        <Button asChild>
          <Link to="/stories/$storyId" params={{ storyId: st.story_id }} hash="logs">
            {trace.data ? `${plural(trace.data.logs.length, 'log')} · ${plural(services ?? 0, 'service')}` : 'Logs'}
          </Link>
        </Button>
      </div>
    </>
  )
}

function Frame({ wide, children }: { wide: boolean; children: ReactNode }) {
  const { width, handleProps } = useResizableWidth({
    defaultWidth: 460,
    min: INSPECTOR_MIN,
    max: inspectorMax,
    storageKey: INSPECTOR_KEY,
  })
  return (
    <m.aside
      aria-label="Selected story"
      initial={{ opacity: 0, x: 24 }}
      animate={{ opacity: 1, x: 0 }}
      transition={{ type: 'tween', duration: 0.3, ease: [0.2, 0.8, 0.2, 1] }}
      style={wide ? { width } : undefined}
      className={cx(
        'relative flex min-w-0 flex-col gap-4 rounded-panel border border-panel-line bg-panel px-5 py-[18px] shadow-panel-lg',
        wide ? 'sticky top-[84px] max-h-[calc(100dvh-100px)] shrink-0 self-start overflow-y-auto' : 'w-full',
      )}
    >
      {wide ? (
        <div
          {...handleProps}
          aria-label="Resize inspector"
          className="absolute inset-y-4 left-0 w-2 -translate-x-1/2 cursor-col-resize touch-none rounded-full after:absolute after:inset-y-0 after:left-[3px] after:w-0.5 after:rounded-full after:bg-transparent hover:after:bg-accent focus-visible:after:bg-accent"
        />
      ) : null}
      {children}
    </m.aside>
  )
}

export function Inspector({ group, pending }: { group: StoryGroup | undefined; pending?: boolean }) {
  const wide = useMediaQuery(WIDE_QUERY)
  return (
    <Frame wide={wide}>
      {pending ? (
        <div className="flex flex-col gap-3" aria-busy="true" aria-label="Loading story">
          <Skeleton className="h-3 w-48" />
          <Skeleton className="h-5 w-4/5" />
          <Skeleton className="h-40" />
        </div>
      ) : group ? (
        <m.div
          key={group.fingerprint}
          className="flex flex-col gap-4"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          transition={{ duration: 0.2 }}
        >
          <StoryBody group={group} />
        </m.div>
      ) : (
        <EmptyState title="Select a story group" description="Its latest story shows here." />
      )}
    </Frame>
  )
}
