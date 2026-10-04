/**
 * Log templates (≤ 200 from /log-templates) in a virtualized table: service, template, hits
 * in the window, a sparkline, first seen and an alerting badge. Sorted locally. The sparkline
 * comes from the template's detail query, fetched only for rows that stay on screen.
 */
import { useQuery } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { useVirtualizer } from '@tanstack/react-virtual'
import { useEffect, useMemo, useRef, useState } from 'react'
import type { KeyboardEvent } from 'react'
import { api } from '../../api/queries'
import type { LogTemplateView } from '../../api/types'
import { sinceSearch } from '../../app/search'
import type { Since } from '../../app/search'
import { Spark } from '../../components/charts/Spark'
import { Badge } from '../../components/ui/Badge'
import { Skeleton } from '../../components/ui/Skeleton'
import { SortHeader } from '../../components/ui/SortHeader'
import { cx } from '../../lib/cx'
import { ago, compact, dateTime } from '../../lib/format'
import { serviceColor } from '../../lib/serviceColor'
import { NARROW_QUERY, useMediaQuery } from '../../lib/useMediaQuery'
import { SINCE_SECS, denseSeries, peak } from '../stories/model'
import { sortTemplates } from './model'
import type { TemplateSort, TemplateSortKey } from './model'

const COLS = '140px minmax(0, 1fr) 72px 120px 84px 80px'
const AREAS = '"svc tmpl count spark first alert"'
const NARROW_COLS = 'minmax(0, 1fr) auto auto'
const NARROW_AREAS = '"tmpl tmpl tmpl" "svc count alert"'
const HEADER_H = 34
/** How long a row must stay mounted before it fetches its sparkline (fast scrolling skips it). */
const SPARK_DELAY_MS = 150

function RowSpark({ id, since, nowMs, hits }: { id: string; since: Since; nowMs: number; hits: number }) {
  const [ready, setReady] = useState(false)
  useEffect(() => {
    const t = setTimeout(() => setReady(true), SPARK_DELAY_MS)
    return () => clearTimeout(t)
  }, [])
  const detail = useQuery({ ...api.logTemplate(id, since), enabled: ready, staleTime: 60_000 })
  if (detail.isError) return <span className="text-xs text-faint" title="The trend could not be loaded">—</span>
  if (!detail.data) return <Skeleton className="h-[22px] w-full" />
  const values = denseSeries(detail.data.buckets, detail.data.bucket_secs, SINCE_SECS[since], nowMs)
  return <Spark values={values} tone="accent" height={22} label={`${hits} hits over ${since}, peak ${peak(values)} per bucket`} />
}

export interface TemplatesTableProps {
  rows: readonly LogTemplateView[]
  since: Since
  /** When the rows were fetched: the end of the sparkline window and "first seen" reference. */
  nowMs: number
  sort: TemplateSort
  onSort: (s: TemplateSort) => void
  height?: number
}

export function TemplatesTable({ rows, since, nowMs, sort, onSort, height = 560 }: TemplatesTableProps) {
  const sorted = useMemo(() => sortTemplates(rows, sort), [rows, sort])
  const narrow = useMediaQuery(NARROW_QUERY)
  const rowHeight = narrow ? 64 : 42
  const scrollRef = useRef<HTMLDivElement>(null)
  // No React Compiler in this build; the virtualizer's unstable functions are fine here.
  // eslint-disable-next-line react-hooks/incompatible-library
  const virt = useVirtualizer({
    count: sorted.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => rowHeight,
    overscan: 8,
    scrollMargin: HEADER_H,
    getItemKey: (i) => sorted[i]!.template_id,
  })

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

  const header = (key: TemplateSortKey, label: string, className?: string) => (
    <SortHeader
      dir={sort.key === key ? (sort.desc ? 'descending' : 'ascending') : 'none'}
      onSort={() => onSort({ key, desc: sort.key === key ? !sort.desc : key !== 'template' })}
      className={className}
    >
      {label}
    </SortHeader>
  )
  const plain = 'text-[11px] uppercase tracking-[0.06em] text-muted'

  return (
    <div
      ref={scrollRef}
      role="region"
      aria-label="Log templates"
      tabIndex={0}
      onKeyDown={onKeyDown}
      className="relative overflow-y-auto overscroll-contain [scrollbar-gutter:stable] focus-visible:-outline-offset-2"
      style={{ maxHeight: height }}
    >
      <div role="table" aria-label="Templates" aria-rowcount={sorted.length + 1}>
        <div role="rowgroup" className="sticky top-0 z-10 bg-panel">
          <div
            role="row"
            aria-rowindex={1}
            className="grid items-center gap-3 border-b border-line px-4"
            style={{ height: HEADER_H, gridTemplateColumns: narrow ? '1fr auto' : COLS, ...(narrow ? {} : { gridTemplateAreas: AREAS }) }}
          >
            {narrow ? (
              <>
                {header('template', 'Template')}
                {header('count', 'Hits', 'text-right')}
              </>
            ) : (
              <>
                <span role="columnheader" className={plain}>
                  Service
                </span>
                {header('template', 'Template')}
                {header('count', 'Hits', 'text-right')}
                <span role="columnheader" className={plain}>
                  Trend
                </span>
                {header('first', 'First seen')}
                <span role="columnheader" className={plain}>
                  Status
                </span>
              </>
            )}
          </div>
        </div>
        <div role="rowgroup" className="relative" style={{ height: virt.getTotalSize() }}>
          {virt.getVirtualItems().map((vi) => {
            const t = sorted[vi.index]!
            return (
              <div
                key={vi.key}
                data-template-id={t.template_id}
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
                <span role="cell" style={{ gridArea: 'svc' }} className="flex min-w-0 items-center gap-1.5 text-xs">
                  <span aria-hidden className="size-2 shrink-0 rounded-full" style={{ backgroundColor: serviceColor(t.service) }} />
                  <span className="truncate">{t.service}</span>
                </span>
                <span role="cell" style={{ gridArea: 'tmpl' }} className="min-w-0">
                  <Link
                    to="/logs/templates/$templateId"
                    params={{ templateId: t.template_id }}
                    search={sinceSearch(since)}
                    title={t.template}
                    className="block truncate rounded-badge font-mono text-xs text-ink hover:underline"
                  >
                    {t.template}
                  </Link>
                </span>
                <span role="cell" style={{ gridArea: 'count' }} className="tabular text-right font-mono text-xs" title={String(t.count)}>
                  {compact(t.count)}
                </span>
                <span role="cell" style={{ gridArea: 'spark' }} className={narrow ? 'hidden' : undefined}>
                  {narrow ? null : <RowSpark id={t.template_id} since={since} nowMs={nowMs} hits={t.count} />}
                </span>
                <span role="cell" style={{ gridArea: 'first' }} className={cx('tabular whitespace-nowrap text-xs text-muted', narrow && 'hidden')} title={dateTime(t.first_seen_ns)}>
                  {ago(t.first_seen_ns, nowMs)}
                </span>
                <span role="cell" style={{ gridArea: 'alert' }} className={narrow ? 'justify-self-end' : undefined}>
                  {t.alerting ? <Badge kind="spike">alerting</Badge> : <span className="sr-only">not alerting</span>}
                </span>
              </div>
            )
          })}
        </div>
      </div>
    </div>
  )
}
