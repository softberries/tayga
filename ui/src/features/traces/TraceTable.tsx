/**
 * Trace search results: start, endpoint (links to the trace), duration as a bar plus the
 * number, span count, error flag and story link. Sorted locally, rows virtualized.
 */
import { Link } from '@tanstack/react-router'
import { useVirtualizer } from '@tanstack/react-virtual'
import { useMemo, useRef } from 'react'
import type { KeyboardEvent } from 'react'
import type { TraceHit } from '../../api/types'
import { sinceSearch } from '../../app/search'
import type { Since } from '../../app/search'
import { Badge } from '../../components/ui/Badge'
import { SortHeader } from '../../components/ui/SortHeader'
import { cx } from '../../lib/cx'
import { clockMs, dateTime, duration } from '../../lib/format'
import { serviceColor } from '../../lib/serviceColor'
import { NARROW_QUERY, useMediaQuery } from '../../lib/useMediaQuery'
import { barFraction, plotMs, sortRows, toneOf } from './model'
import type { Sort, SortKey } from './model'
import { TruncationTooltip } from '../../components/ui/Tooltip'

const COLS = '112px minmax(0, 1fr) minmax(150px, 240px) 52px 56px 76px'
/** Header row height: the sticky header sits inside the scroller, above the rows. */
const HEADER_H = 34
const AREAS = '"start ep dur spans err story"'
/** Phones: the endpoint on its own line, then start, duration and the story link. */
const NARROW_COLS = 'auto minmax(0, 1fr) auto'
const NARROW_AREAS = '"ep ep err" "start dur story"'

const BAR: Record<ReturnType<typeof toneOf>, string> = { err: 'bg-err', slow: 'bg-slow', accent: 'bg-accent' }

/** Start time: wall clock with ms for short ranges, the date too for 24h and 7d. */
function startLabel(ns: number, since: Since): string {
  return since === '15m' || since === '1h' ? clockMs(ns) : dateTime(ns).slice(5)
}

export interface TraceTableProps {
  rows: readonly TraceHit[]
  /** All plotted rows: the duration bars share their extent with the chart. */
  extent: readonly TraceHit[]
  logY: boolean
  since: Since
  sort: Sort
  onSort: (s: Sort) => void
  height?: number
}

export function TraceTable({ rows, extent, logY, since, sort, onSort, height = 520 }: TraceTableProps) {
  const sorted = useMemo(() => sortRows(rows, sort), [rows, sort])
  const [lo, hi] = useMemo(() => {
    let a = Infinity
    let b = 0
    for (const t of extent) {
      const ms = plotMs(t)
      a = Math.min(a, ms)
      b = Math.max(b, ms)
    }
    return [Number.isFinite(a) ? a : 0, b]
  }, [extent])

  const narrow = useMediaQuery(NARROW_QUERY)
  const rowHeight = narrow ? 58 : 38
  // The scroller holds the sticky header too; rows start below it.
  const scrollRef = useRef<HTMLDivElement>(null)
  // No React Compiler in this build; the virtualizer's unstable functions are fine here.
  // eslint-disable-next-line react-hooks/incompatible-library
  const virt = useVirtualizer({
    count: sorted.length,
    getScrollElement: () => scrollRef.current,
    // Fixed row heights (one line, or two on phones): no per-row measuring.
    estimateSize: () => rowHeight,
    overscan: 12,
    scrollMargin: HEADER_H,
    getItemKey: (i) => sorted[i]!.trace_id,
  })

  // Keyboard scrolling for the focusable scroller: rows outside the viewport are not mounted,
  // so Home/End/PageUp/PageDown move the virtualizer, which mounts the rows there.
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const el = scrollRef.current
    if (!el || sorted.length === 0) return
    const page = Math.max(rowHeight, el.clientHeight - HEADER_H - rowHeight)
    if (e.key === 'Home') virt.scrollToIndex(0, { align: 'start' })
    else if (e.key === 'End') virt.scrollToIndex(sorted.length - 1, { align: 'end' })
    else if (e.key === 'PageDown') virt.scrollToOffset(el.scrollTop + page)
    else if (e.key === 'PageUp') virt.scrollToOffset(Math.max(0, el.scrollTop - page))
    else return
    e.preventDefault()
  }

  const header = (key: SortKey, label: string, className?: string) => (
    <SortHeader
      dir={sort.key === key ? (sort.desc ? 'descending' : 'ascending') : 'none'}
      // Text sorts A→Z first; times, durations and counts largest first.
      onSort={() => onSort({ key, desc: sort.key === key ? !sort.desc : key !== 'endpoint' })}
      className={className}
    >
      {label}
    </SortHeader>
  )

  return (
    <div
      ref={scrollRef}
      role="region"
      aria-label="Trace results"
      tabIndex={0}
      onKeyDown={onKeyDown}
      className="relative overflow-y-auto overscroll-contain [scrollbar-gutter:stable] focus-visible:-outline-offset-2"
      style={{ maxHeight: height }}
    >
    <div role="table" aria-label="Traces" aria-rowcount={sorted.length + 1}>
      <div role="rowgroup" className="sticky top-0 z-10 bg-panel">
        <div
          role="row"
          aria-rowindex={1}
          className={cx('grid items-center gap-3 border-b border-line px-4', narrow && 'grid-cols-3')}
          style={{ height: HEADER_H, ...(narrow ? {} : { gridTemplateColumns: COLS, gridTemplateAreas: AREAS }) }}
        >
          {header('start', 'Start')}
          {header('endpoint', 'Endpoint')}
          {header('duration', 'Duration', narrow ? 'text-right' : undefined)}
          {narrow ? null : (
            <>
              {header('spans', 'Spans', 'text-right')}
              <span role="columnheader" className="text-[11px] uppercase tracking-[0.06em] text-muted">
                Error
              </span>
              <span role="columnheader" className="text-[11px] uppercase tracking-[0.06em] text-muted">
                Story
              </span>
            </>
          )}
        </div>
      </div>
      <div role="rowgroup" className="relative" style={{ height: virt.getTotalSize() }}>
          {virt.getVirtualItems().map((vi) => {
            const t = sorted[vi.index]!
            const tone = toneOf(t)
            const ms = plotMs(t)
            return (
              <div
                key={vi.key}
                data-trace-id={t.trace_id}
                role="row"
                aria-rowindex={vi.index + 2}
                className={cx(
                  'tg-row absolute left-0 top-0 grid w-full items-center border-b border-line-soft px-4 hover:bg-inner',
                  narrow ? 'content-center gap-x-3 gap-y-1' : 'gap-3',
                )}
                style={{
                  gridTemplateColumns: narrow ? NARROW_COLS : COLS,
                  gridTemplateAreas: narrow ? NARROW_AREAS : AREAS,
                  height: rowHeight,
                  transform: `translateY(${vi.start - HEADER_H}px)`,
                }}
              >
                <span role="cell" style={{ gridArea: 'start' }} className="tabular font-mono text-[11.5px] text-muted" title={dateTime(t.ts_ns)}>
                  {startLabel(t.ts_ns, since)}
                </span>
                <span role="cell" style={{ gridArea: 'ep' }} className="flex min-w-0 items-center gap-2">
                  <span aria-hidden className="size-2 shrink-0 rounded-full" style={{ backgroundColor: serviceColor(t.endpoint_service) }} />
                  <Link
                    to="/traces/$traceId"
                    params={{ traceId: t.trace_id }}
                    search={sinceSearch(since)}
                    className="flex min-w-0 items-baseline gap-1.5 rounded-badge hover:underline"
                  >
                    <span className="shrink-0 text-xs text-muted">{t.endpoint_service}</span>
                    <TruncationTooltip content={`${t.endpoint_service} · ${t.endpoint_name}`} openOnHostFocus>
                      <span className="min-w-0 truncate font-mono text-xs text-ink">{t.endpoint_name}</span>
                    </TruncationTooltip>
                  </Link>
                </span>
                <span role="cell" style={{ gridArea: 'dur' }} className="flex min-w-0 items-center gap-2">
                  <span aria-hidden className="h-1.5 min-w-0 flex-1 overflow-hidden rounded-full bg-track">
                    <span
                      className={cx('block h-full rounded-full', BAR[tone])}
                      style={{ width: `${barFraction(ms, lo, hi, logY) * 100}%` }}
                    />
                  </span>
                  <span className={cx('tabular w-[68px] shrink-0 text-right font-mono text-xs', tone === 'accent' ? 'text-ink' : tone === 'err' ? 'text-err' : 'text-slow')}>
                    {duration(t.duration_ns)}
                  </span>
                </span>
                <span role="cell" style={{ gridArea: 'spans' }} className={cx('tabular text-right font-mono text-xs text-muted', narrow && 'hidden')}>
                  {t.span_count}
                </span>
                <span role="cell" style={{ gridArea: 'err' }} className={narrow ? 'justify-self-end' : undefined}>
                  {t.is_error ? <Badge kind="error">error</Badge> : <span className="sr-only">no error</span>}
                </span>
                <span role="cell" style={{ gridArea: 'story' }} className={narrow ? 'justify-self-end' : undefined}>
                  {t.story_id ? (
                    <Link
                      to="/stories/$storyId"
                      params={{ storyId: t.story_id }}
                      search={sinceSearch(since)}
                      aria-label={`${t.story_kind ?? 'Story'} story of trace ${t.trace_id}`}
                      className={cx('text-xs hover:underline', t.story_kind === 'error' ? 'text-err' : t.story_kind === 'slow' ? 'text-slow' : 'text-accent')}
                    >
                      {t.story_kind ? `${t.story_kind} story` : 'story'}
                    </Link>
                  ) : (
                    <span className="sr-only">no story</span>
                  )}
                </span>
              </div>
            )
          })}
        </div>
      </div>
    </div>
  )
}
