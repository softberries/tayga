import { useMemo, useState } from 'react'
import type { CSSProperties } from 'react'
import type { SpanFilter, TraceSearch } from '../../app/search'
import type { TraceLogTemplate, TraceView } from '../../api/types'
import { SpanDrawer } from './SpanDrawer'
import { Waterfall } from './Waterfall'

export interface TraceDetailProps {
  trace: TraceView
  critical?: readonly string[]
  rootCauseId?: string | null
  templates?: readonly TraceLogTemplate[]
  /** Waterfall state from the URL: open span, search text, row filter. */
  search: TraceSearch
  /** Merge a change into the URL; opening a span adds a history entry, the rest replace. */
  onSearch: (patch: Partial<TraceSearch>, opts?: { push?: boolean }) => void
  height?: CSSProperties['height']
  /** Open zoomed to these spans' time window (see Waterfall `initialZoomTo`). */
  initialZoomTo?: readonly string[]
}

/** The full waterfall plus its span drawer, wired to URL state. Shared by trace and story pages. */
export function TraceDetail({ trace, critical, rootCauseId, templates, search, onSearch, height, initialZoomTo }: TraceDetailProps) {
  // The highlighted row starts at the open span, else the root cause (story page).
  const [selected, setSelected] = useState<string | null>(search.span ?? rootCauseId ?? null)
  const bounds = useMemo(() => {
    let start = Infinity
    let end = -Infinity
    for (const s of trace.spans) {
      start = Math.min(start, s.start_ns)
      end = Math.max(end, s.start_ns + s.duration_ns)
    }
    return Number.isFinite(start) ? { start, total: Math.max(1, end - start) } : { start: 0, total: 1 }
  }, [trace.spans])
  const byLog = useMemo(() => new Map((templates ?? []).map((t) => [t.log_id, t])), [templates])
  const criticalSet = useMemo(() => new Set(critical ?? []), [critical])
  const open = search.span ? (trace.spans.find((s) => s.span_id === search.span) ?? null) : null

  return (
    <>
      <Waterfall
        spans={trace.spans}
        critical={critical}
        rootCauseId={rootCauseId}
        selectedId={selected}
        onSelectedChange={setSelected}
        onOpen={(id) => onSearch({ span: id }, { push: true })}
        filter={search.only ?? 'all'}
        onFilterChange={(f) => onSearch({ only: f === 'all' ? undefined : (f as SpanFilter) })}
        query={search.q ?? ''}
        onQueryChange={(q) => onSearch({ q: q === '' ? undefined : q })}
        height={height}
        initialZoomTo={initialZoomTo}
      />
      <SpanDrawer
        span={open}
        onClose={() => onSearch({ span: undefined })}
        traceStartNs={bounds.start}
        traceTotalNs={bounds.total}
        logs={trace.logs}
        templates={byLog}
        rootCause={open !== null && open.span_id === rootCauseId}
        critical={open !== null && criticalSet.has(open.span_id)}
      />
    </>
  )
}
