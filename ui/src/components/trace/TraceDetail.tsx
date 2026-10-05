import { useCallback, useMemo, useRef, useState } from 'react'
import type { CSSProperties } from 'react'
import type { SpanFilter, TraceSearch } from '../../app/search'
import type { TraceLogTemplate, TraceView } from '../../api/types'
import { SpanDrawer } from './SpanDrawer'
import { Waterfall } from './Waterfall'
import type { RowFilter } from './Waterfall'

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
  /** Text shown while that initial zoom is active, e.g. "Showing the root request". */
  zoomHint?: string
}

/** The full waterfall plus its span drawer, wired to URL state. Shared by trace and story pages. */
export function TraceDetail({
  trace,
  critical,
  rootCauseId,
  templates,
  search,
  onSearch,
  height,
  initialZoomTo,
  zoomHint,
}: TraceDetailProps) {
  // The highlighted row starts at the open span, else the root cause (story page), and
  // follows the URL's span on back/forward (state adjusted during render, not in an effect).
  const [selected, setSelected] = useState<string | null>(search.span ?? rootCauseId ?? null)
  const [urlSpan, setUrlSpan] = useState(search.span)
  if (search.span !== urlSpan) {
    setUrlSpan(search.span)
    if (search.span) setSelected(search.span)
  }
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

  const onOpen = useCallback((id: string) => onSearch({ span: id }, { push: true }), [onSearch])
  const onFilterChange = useCallback(
    (f: RowFilter) => onSearch({ only: f === 'all' ? undefined : (f as SpanFilter) }),
    [onSearch],
  )
  const onQueryChange = useCallback((q: string) => onSearch({ q: q === '' ? undefined : q }), [onSearch])
  const onClose = useCallback(() => onSearch({ span: undefined }), [onSearch])

  // The drawer has no Radix trigger, so return focus to the waterfall tree ourselves.
  const wrap = useRef<HTMLDivElement>(null)
  const onCloseAutoFocus = useCallback((e: Event) => {
    const tree = wrap.current?.querySelector<HTMLElement>('[role="tree"]')
    if (!tree) return
    e.preventDefault()
    tree.focus()
  }, [])

  return (
    <div ref={wrap}>
      <Waterfall
        spans={trace.spans}
        critical={critical}
        rootCauseId={rootCauseId}
        selectedId={selected}
        onSelectedChange={setSelected}
        onOpen={onOpen}
        filter={search.only ?? 'all'}
        onFilterChange={onFilterChange}
        query={search.q ?? ''}
        onQueryChange={onQueryChange}
        height={height}
        initialZoomTo={initialZoomTo}
        zoomHint={zoomHint}
      />
      <SpanDrawer
        span={open}
        onClose={onClose}
        traceStartNs={bounds.start}
        traceTotalNs={bounds.total}
        logs={trace.logs}
        templates={byLog}
        rootCause={open !== null && open.span_id === rootCauseId}
        critical={open !== null && criticalSet.has(open.span_id)}
        onCloseAutoFocus={onCloseAutoFocus}
      />
    </div>
  )
}
