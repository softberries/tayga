/**
 * Trace waterfall: indented span rows (service muted), bars on a track, critical-path bars
 * in accent, error bars in err, and the root-cause row tinted with a glowing bar.
 *
 * Full mode: virtualized rows, time axis, minimap with brush zoom, search, errors-only and
 * critical-path filters, expand/collapse and keyboard navigation (a WAI-ARIA tree).
 * Compact mode (the home inspector): at most `maxRows` rows chosen by importance, no
 * toolbar or minimap, and a "show all" link to the trace page.
 */
import { Link } from '@tanstack/react-router'
import { useVirtualizer } from '@tanstack/react-virtual'
import { ChevronRight, ChevronsDownUp, ChevronsUpDown, CircleAlert, Search, X, ZoomOut } from 'lucide-react'
import { memo, useCallback, useDeferredValue, useEffect, useMemo, useRef, useState } from 'react'
import type { CSSProperties, KeyboardEvent } from 'react'
import type { TraceSpan } from '../../api/types'
import { cx } from '../../lib/cx'
import { duration, msValue } from '../../lib/format'
import { serviceColor } from '../../lib/serviceColor'
import { NARROW_QUERY, useMediaQuery } from '../../lib/useMediaQuery'
import { Button } from '../ui/Button'
import { EmptyState } from '../ui/EmptyState'
import { ToggleGroup } from '../ui/ToggleGroup'
import { buildLayout, compactRows, niceTicks, projectBar, visibleRows } from './layout'
import type { Layout, LayoutRow, RowFilter } from './layout'
import { FULL_VIEW, Minimap } from './Minimap'
import type { ZoomWindow } from './Minimap'

export type { RowFilter } from './layout'

export interface WaterfallProps {
  spans: readonly TraceSpan[]
  /** Span ids on the critical path (accent bars). */
  critical?: Iterable<string>
  /** Root-cause span: tinted row and glowing bar. */
  rootCauseId?: string | null
  /** Highlighted row (keyboard cursor). Controlled when given with `onSelectedChange`. */
  selectedId?: string | null
  onSelectedChange?: (spanId: string) => void
  /** A row was activated (click, Enter or Space): open the span drawer. */
  onOpen?: (spanId: string) => void
  /** Row filter; controlled when given with `onFilterChange` (keep it in the URL). */
  filter?: RowFilter
  onFilterChange?: (filter: RowFilter) => void
  /** Search text over service, span name and attributes; controlled like `filter`. */
  query?: string
  onQueryChange?: (query: string) => void
  /** Inspector mode: at most `maxRows` rows, no toolbar or minimap, a "show all" link. */
  compact?: boolean
  /** Compact only (default 12). */
  maxRows?: number
  /** Compact only: the trace page the "show all" link opens. */
  traceId?: string
  /**
   * Start zoomed to the time window these spans cover (e.g. the story's critical path), when
   * that window is under 70 % of the trace. Full mode: Reset zoom shows everything. Compact
   * mode: the fixed scale, with axis labels measured from the window's start.
   */
  initialZoomTo?: readonly string[]
  /** Shown beside "Reset zoom" while that initial zoom is active (default "Zoomed in"). */
  zoomHint?: string
  /** Full only: height of the scrolling row area (default `min(70vh, 720px)`). */
  height?: CSSProperties['height']
  /** Accessible name of the tree. */
  label?: string
  className?: string
}

const ROW_H = 28
/** Narrowest initial zoom window (fraction of the trace). */
const MIN_VIEW = 1e-6
const MAX_INDENT = 10
const NARROW_MAX_INDENT = 3
const COMPACT_MAX_INDENT = 6
const COMPACT_ROW_H = 25
const COLS = 'minmax(240px, 32%) minmax(0, 1fr) 64px'
const NARROW_COLS = 'minmax(140px, 42%) minmax(0, 1fr) 56px'
const COMPACT_COLS = 'minmax(110px, 40%) minmax(0, 1fr) 52px'
const FILTERS = [
  { value: 'all', label: 'All spans' },
  { value: 'errors', label: 'Errors' },
  { value: 'critical', label: 'Critical path' },
] as const satisfies ReadonlyArray<{ value: RowFilter; label: string }>

const pct = (f: number) => `${(f * 100).toFixed(3)}%`
const rowDomId = (row: LayoutRow) => `wf-row-${row.index}`

/** Uncontrolled unless both the value and its setter are passed. */
function useControllable<T>(value: T | undefined, onChange: ((v: T) => void) | undefined, initial: T) {
  const [own, setOwn] = useState(initial)
  const controlled = value !== undefined && onChange !== undefined
  const set = useCallback(
    (v: T) => {
      if (!controlled) setOwn(v)
      onChange?.(v)
    },
    [controlled, onChange],
  )
  return [controlled ? value : own, set] as const
}

export function Waterfall(props: WaterfallProps) {
  const { spans, critical, rootCauseId } = props
  const criticalKey = critical ? [...critical].join('\n') : ''
  const layout = useMemo(
    () => buildLayout(spans, { critical: criticalKey ? criticalKey.split('\n') : [], rootCauseId }),
    [spans, criticalKey, rootCauseId],
  )
  if (spans.length === 0) {
    return (
      <EmptyState
        className={props.className}
        title="No spans in this trace"
        description="The trace has logs but no spans were stored for it."
      />
    )
  }
  return props.compact ? <CompactWaterfall layout={layout} {...props} /> : <FullWaterfall layout={layout} {...props} />
}

interface TicksProps {
  totalNs: number
  view: ZoomWindow
  count: number
}

/** Tick positions (fractions of the visible track) with labels relative to the trace start. */
function useTicks({ totalNs, view, count }: TicksProps) {
  return useMemo(() => {
    const from = view[0] * totalNs
    const to = view[1] * totalNs
    return niceTicks(from, to, count).map((ns) => ({ ns, at: (ns - from) / Math.max(1, to - from) }))
  }, [totalNs, view, count])
}

function AxisLabels({ ticks }: { ticks: Array<{ ns: number; at: number }> }) {
  return (
    <span aria-hidden className="relative h-4">
      {ticks.map((t) => (
        <span
          key={t.ns}
          className="absolute top-0 whitespace-nowrap"
          style={{
            left: pct(t.at),
            transform: t.at < 0.04 ? 'none' : t.at > 0.96 ? 'translateX(-100%)' : 'translateX(-50%)',
          }}
        >
          {duration(t.ns)}
        </span>
      ))}
    </span>
  )
}

interface RowProps {
  row: LayoutRow & { match?: boolean }
  layout: Layout
  view: ZoomWindow
  selected: boolean
  expanded: boolean
  /** Filtering or searching: collapse is off, so no chevrons. */
  flat: boolean
  revealed: boolean
  compact: boolean
  /** Compact rows that open something are buttons (full rows are tree items). */
  interactive?: boolean
  /** Below `sm`: less indentation and a narrower name column. */
  narrow?: boolean
  /** Position among visible siblings (tree items only). */
  setSize?: number
  posInSet?: number
  onClick: (row: LayoutRow) => void
  onToggle: (row: LayoutRow) => void
}

const SpanRow = memo(function SpanRow({
  row,
  layout,
  view,
  selected,
  expanded,
  flat,
  revealed,
  compact,
  interactive,
  narrow,
  setSize,
  posInSet,
  onClick,
  onToggle,
}: RowProps) {
  const s = row.span
  // Project the true extent, then widen to MIN_BAR: a pre-widened bar would cover the whole
  // view when zoomed far into a long trace.
  const bar = projectBar(row.startFrac, row.durFrac, view)
  const context = row.match === false
  // Indentation stops at MAX_INDENT levels so deep chains keep their names readable; the
  // remaining depth is shown as a number.
  const maxIndent = compact ? COMPACT_MAX_INDENT : narrow ? NARROW_MAX_INDENT : MAX_INDENT
  const indent = Math.min(row.depth, maxIndent) * (compact ? 10 : 12) + (compact ? 6 : 4)
  const extraDepth = row.depth - maxIndent
  const exceptions = useMemo(
    () =>
      s.events
        .filter((e) => e.name === 'exception')
        .map((e) => (e.ts_ns - layout.startNs) / layout.totalNs)
        .map((f) => (f - view[0]) / Math.max(1e-9, view[1] - view[0]))
        .filter((f) => f >= 0 && f <= 1),
    [s.events, layout.startNs, layout.totalNs, view],
  )
  const tone = row.error ? 'bg-err' : row.critical ? 'bg-accent' : 'bg-node-line'
  return (
    <div
      id={compact ? undefined : rowDomId(row)}
      role={compact ? (interactive ? 'button' : undefined) : 'treeitem'}
      tabIndex={compact && interactive ? 0 : undefined}
      onKeyDown={
        compact && interactive
          ? (e) => {
              if (e.key !== 'Enter' && e.key !== ' ') return
              e.preventDefault()
              onClick(row)
            }
          : undefined
      }
      aria-level={compact ? undefined : row.depth + 1}
      aria-setsize={compact ? undefined : setSize}
      aria-posinset={compact ? undefined : posInSet}
      aria-selected={compact ? undefined : selected}
      aria-expanded={compact || flat || row.childCount === 0 ? undefined : expanded}
      data-span-id={s.span_id}
      data-root-cause={row.rootCause || undefined}
      onClick={() => onClick(row)}
      className={cx(
        'tg-row grid h-full items-center gap-2 rounded-[6px]',
        (!compact || interactive) && 'cursor-pointer',
        // The root-cause tint stays when selected; selection then shows as the accent edge.
        row.rootCause && 'bg-err-soft',
        selected && !row.rootCause && 'bg-row-selected',
        selected && 'shadow-[inset_2px_0_0_var(--tg-accent)]',
        !selected && !row.rootCause && 'hover:bg-inner',
        revealed && 'tg-in [animation-duration:220ms]',
      )}
      style={{ gridTemplateColumns: compact ? COMPACT_COLS : narrow ? NARROW_COLS : COLS }}
    >
      <span className="flex min-w-0 items-center gap-1.5" style={{ paddingLeft: indent }}>
        {!compact && !flat && row.childCount > 0 ? (
          <span
            aria-hidden
            onClick={(e) => {
              e.stopPropagation()
              onToggle(row)
            }}
            className="-m-1 inline-flex shrink-0 p-1 text-muted hover:text-ink"
          >
            <ChevronRight
              size={12}
              className={cx('transition-transform duration-200 motion-reduce:transition-none', expanded && 'rotate-90')}
            />
          </span>
        ) : compact ? null : (
          <span aria-hidden className="w-3 shrink-0" />
        )}
        {extraDepth > 0 ? (
          <span className="shrink-0 font-mono text-[10px] text-faint" title={`depth ${row.depth}`}>
            +{extraDepth}
          </span>
        ) : null}
        <span aria-hidden className="size-2 shrink-0 rounded-full" style={{ backgroundColor: serviceColor(s.service_name) }} />
        <span className={cx('truncate text-[12px]', context ? 'text-muted' : 'text-ink')}>
          <span className="text-muted">{s.service_name}</span> {s.span_name}
          {row.rootCause ? <span className="sr-only"> (root cause)</span> : null}
        </span>
        {row.error ? <CircleAlert aria-label="error" size={12} className="shrink-0 text-err" /> : null}
      </span>
      <span className="relative h-[10px] rounded-[4px] bg-track">
        {bar.clipped ? null : (
          <span
            title={`${s.service_name} ${s.span_name}\n${duration(s.duration_ns)}, starts at +${duration(row.offsetNs)}`}
            className={cx(
              'absolute top-0 h-full rounded-[4px] transition-[left,width,box-shadow] duration-200 motion-reduce:transition-none',
              tone,
              row.rootCause && 'shadow-glow-err',
              context && 'opacity-40',
            )}
            style={{ left: pct(bar.left), width: pct(bar.width) }}
          />
        )}
        {exceptions.map((f, i) => (
          <span
            key={i}
            aria-hidden
            className="absolute top-1/2 size-[7px] -translate-x-1/2 -translate-y-1/2 rotate-45 border border-panel bg-err"
            style={{ left: pct(f) }}
          />
        ))}
      </span>
      <span className="tabular pr-1.5 text-right font-mono text-[11px] text-muted">{msValue(s.duration_ns)}</span>
    </div>
  )
})

/** The window covering `ids` with 4 % padding, or the full view when that is not a real zoom. */
export function focusWindow(layout: Layout, ids: readonly string[] | undefined): ZoomWindow {
  let from = Infinity
  let to = -Infinity
  for (const id of ids ?? []) {
    const i = layout.byId.get(id)
    if (i === undefined) continue
    // True extents, not the drawn bars: MIN_BAR would widen a short span in a long trace.
    const r = layout.rows[i]!
    const a = r.offsetNs / layout.totalNs
    const b = (r.offsetNs + Math.max(0, r.span.duration_ns)) / layout.totalNs
    from = Math.min(from, a)
    to = Math.max(to, b)
  }
  if (!(to > from) || to - from > 0.7) return FULL_VIEW
  const pad = (to - from) * 0.04
  return [Math.max(0, from - pad), Math.min(1, Math.max(to + pad, from + MIN_VIEW))]
}

interface ModeProps extends WaterfallProps {
  layout: Layout
}

function FullWaterfall({
  layout,
  rootCauseId,
  selectedId,
  onSelectedChange,
  onOpen,
  filter: filterProp,
  onFilterChange,
  query: queryProp,
  onQueryChange,
  height = 'min(70vh, 720px)',
  initialZoomTo,
  zoomHint = 'Zoomed in',
  label = 'Trace waterfall',
  className,
}: ModeProps) {
  const [filter, setFilter] = useControllable<RowFilter>(filterProp, onFilterChange, 'all')
  const [query, setQuery] = useControllable(queryProp, onQueryChange, '')
  const [selected, setSelected] = useControllable<string | null>(
    selectedId,
    onSelectedChange as ((v: string | null) => void) | undefined,
    rootCauseId ?? null,
  )
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(() => new Set())
  const [revealed, setRevealed] = useState<{ from: number; to: number } | null>(null)
  const [initialView] = useState<ZoomWindow>(() => focusWindow(layout, initialZoomTo))
  const [view, setView] = useState<ZoomWindow>(initialView)
  const narrow = useMediaQuery(NARROW_QUERY)
  const cols = narrow ? NARROW_COLS : COLS
  const deferredQuery = useDeferredValue(query)
  const flat = filter !== 'all' || deferredQuery.trim() !== ''

  const rows = useMemo(
    () => visibleRows(layout, { collapsed, filter, query: deferredQuery }),
    [layout, collapsed, filter, deferredQuery],
  )
  // aria-posinset/aria-setsize: virtualized items must state their place among visible siblings.
  const siblings = useMemo(() => {
    const count = new Map<number, number>()
    const pos = new Map<number, [number, number]>()
    for (const r of rows) {
      const n = (count.get(r.parent) ?? 0) + 1
      count.set(r.parent, n)
      pos.set(r.index, [n, 0])
    }
    for (const r of rows) pos.get(r.index)![1] = count.get(r.parent)!
    return pos
  }, [rows])
  const matches = useMemo(() => (flat ? rows.filter((r) => r.match).length : layout.rows.length), [rows, flat, layout])

  const scrollRef = useRef<HTMLDivElement>(null)
  // No React Compiler in this build; the virtualizer's unstable functions are fine here.
  // eslint-disable-next-line react-hooks/incompatible-library
  const virt = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_H,
    overscan: 12,
    getItemKey: (i) => rows[i]!.index,
  })

  const selIdx = selected === null ? -1 : rows.findIndex((r) => r.span.span_id === selected)
  // Bring the selected row into view: centered the first time (the story's root cause),
  // minimally afterwards (keyboard moves).
  const scrolledOnce = useRef(false)
  useEffect(() => {
    if (selIdx < 0) return
    virt.scrollToIndex(selIdx, { align: scrolledOnce.current ? 'auto' : 'center' })
    scrolledOnce.current = true
  }, [selIdx, virt])

  useEffect(() => {
    if (!revealed) return
    const t = window.setTimeout(() => setRevealed(null), 400)
    return () => window.clearTimeout(t)
  }, [revealed])

  const toggle = useCallback(
    (row: LayoutRow) => {
      const next = new Set(collapsed)
      if (next.delete(row.span.span_id)) setRevealed({ from: row.index + 1, to: row.subtreeEnd })
      else next.add(row.span.span_id)
      setCollapsed(next)
    },
    [collapsed],
  )

  const select = useCallback(
    (row: LayoutRow) => {
      setSelected(row.span.span_id)
      onOpen?.(row.span.span_id)
    },
    [setSelected, onOpen],
  )

  const zoomed = view[0] > 0 || view[1] < 1
  const zoomTo = (row: LayoutRow) => {
    const pad = Math.max(row.width * 0.08, 0.002)
    setView([Math.max(0, row.left - pad), Math.min(1, row.left + row.width + pad)])
  }

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (rows.length === 0) return
    const cur = selIdx
    const row = cur >= 0 ? rows[cur] : undefined
    const go = (i: number) => {
      const r = rows[Math.min(rows.length - 1, Math.max(0, i))]
      if (r) setSelected(r.span.span_id)
    }
    const page = Math.max(1, Math.floor((scrollRef.current?.clientHeight ?? ROW_H * 10) / ROW_H) - 1)
    switch (e.key) {
      case 'ArrowDown':
        go(cur + 1)
        break
      case 'ArrowUp':
        go(cur < 0 ? 0 : cur - 1)
        break
      case 'Home':
        go(0)
        break
      case 'End':
        go(rows.length - 1)
        break
      case 'PageDown':
        go(cur + page)
        break
      case 'PageUp':
        go(cur - page)
        break
      case 'ArrowRight':
        if (!row) go(0)
        else if (!flat && row.childCount > 0 && collapsed.has(row.span.span_id)) toggle(row)
        else if (row.childCount > 0) go(cur + 1)
        break
      case 'ArrowLeft':
        if (!row) break
        if (!flat && row.childCount > 0 && !collapsed.has(row.span.span_id)) toggle(row)
        else if (row.parent >= 0) go(rows.findIndex((r) => r.index === row.parent))
        break
      case 'Enter':
      case ' ':
        if (row) onOpen?.(row.span.span_id)
        break
      case 'z':
        if (row) zoomTo(row)
        break
      case 'Escape':
        if (zoomed) setView(FULL_VIEW)
        else return
        break
      default:
        return
    }
    e.preventDefault()
  }

  const ticks = useTicks({ totalNs: layout.totalNs, view, count: narrow ? 2 : 6 })
  const parents = useMemo(() => layout.rows.filter((r) => r.childCount > 0).map((r) => r.span.span_id), [layout])

  return (
    <div className={cx('flex min-w-0 flex-col gap-3', className)}>
      <div className="flex flex-wrap items-center gap-2">
        <label className="relative flex min-w-[220px] flex-1 items-center sm:max-w-[360px]">
          <Search aria-hidden size={14} className="pointer-events-none absolute left-2.5 text-muted" />
          <input
            type="search"
            aria-label="Search spans"
            placeholder="Search service, span or attribute"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            className="h-[34px] w-full rounded-field border border-field-line bg-field pl-8 pr-2 text-[13px] text-ink shadow-inset placeholder:text-faint"
          />
        </label>
        <ToggleGroup label="Show spans" options={FILTERS} value={filter} onValueChange={setFilter} />
        <span className="tabular text-xs text-muted" aria-live="polite">
          {flat ? `${matches} of ${layout.rows.length} spans` : `${layout.rows.length} spans`}
        </span>
        <div className="ml-auto flex flex-wrap items-center justify-end gap-1">
          {zoomed && view === initialView ? (
            <span className="whitespace-nowrap text-xs text-muted" role="status">
              {zoomHint} ·
            </span>
          ) : null}
          {zoomed ? (
            <Button size="sm" variant="secondary" onClick={() => setView(FULL_VIEW)}>
              <ZoomOut aria-hidden size={14} /> Reset zoom
            </Button>
          ) : null}
          <Button size="sm" variant="ghost" disabled={flat || collapsed.size === 0} onClick={() => setCollapsed(new Set())}>
            <ChevronsUpDown aria-hidden size={14} /> Expand all
          </Button>
          <Button size="sm" variant="ghost" disabled={flat || parents.length === 0} onClick={() => setCollapsed(new Set(parents))}>
            <ChevronsDownUp aria-hidden size={14} /> Collapse all
          </Button>
        </div>
      </div>

      <Minimap rows={layout.rows} view={view} onViewChange={setView} />

      {/* Narrow screens scroll the rows sideways rather than squeezing the name column. */}
      <div className="overflow-x-auto">
      <div className={narrow ? 'min-w-[400px]' : 'min-w-[720px]'}>
        <div
          className="grid gap-2 overflow-hidden border-b border-line pb-1.5 font-mono text-[10.5px] text-faint [scrollbar-gutter:stable]"
          style={{ gridTemplateColumns: cols }}
        >
          <span className="pl-1 font-sans text-[11px] uppercase tracking-[0.06em] text-muted">Service · span</span>
          <AxisLabels ticks={ticks} />
          <span className="pr-1.5 text-right">ms</span>
        </div>

        {rows.length === 0 ? (
          <EmptyState
            icon={<Search size={18} />}
            title="No spans match"
            description={filter === 'errors' ? 'No span in this trace has an error status.' : 'Try another search or filter.'}
            action={
              <Button
                size="sm"
                onClick={() => {
                  setQuery('')
                  setFilter('all')
                }}
              >
                <X aria-hidden size={14} /> Clear filters
              </Button>
            }
          />
        ) : (
          <div
            ref={scrollRef}
            role="tree"
            aria-label={label}
            aria-activedescendant={selIdx >= 0 ? rowDomId(rows[selIdx]!) : undefined}
            tabIndex={0}
            onKeyDown={onKeyDown}
            onFocus={() => {
              if (selIdx < 0 && rows[0]) setSelected(rows[0].span.span_id)
            }}
            className="relative overflow-auto overscroll-contain rounded-control pt-1 [scrollbar-gutter:stable] focus-visible:outline-offset-0"
            style={{ height }}
          >
            <div className="relative" style={{ height: virt.getTotalSize() }}>
              {/* Tick gridlines behind the bars. */}
              <div aria-hidden className="pointer-events-none absolute inset-0 grid gap-2" style={{ gridTemplateColumns: cols }}>
                <span />
                <span className="relative">
                  {ticks.map((t) => (
                    <span key={t.ns} className="absolute inset-y-0 w-px bg-line-soft" style={{ left: pct(t.at) }} />
                  ))}
                </span>
              </div>
              {virt.getVirtualItems().map((vi) => {
                const row = rows[vi.index]!
                return (
                  <div
                    key={vi.key}
                    className="absolute left-0 top-0 w-full py-px"
                    style={{ height: ROW_H, transform: `translateY(${vi.start}px)` }}
                  >
                    <SpanRow
                      row={row}
                      layout={layout}
                      view={view}
                      selected={row.span.span_id === selected}
                      expanded={!collapsed.has(row.span.span_id)}
                      flat={flat}
                      revealed={revealed !== null && row.index >= revealed.from && row.index < revealed.to}
                      compact={false}
                      narrow={narrow}
                      setSize={siblings.get(row.index)?.[1]}
                      posInSet={siblings.get(row.index)?.[0]}
                      onClick={select}
                      onToggle={toggle}
                    />
                  </div>
                )
              })}
            </div>
          </div>
        )}
      </div>
      </div>
      <p className="m-0 text-[11px] text-faint">
        Arrow keys move, Left/Right collapse and expand, Enter opens the span, Z zooms to it, Escape resets the zoom.
      </p>
    </div>
  )
}

function CompactWaterfall({
  layout,
  rootCauseId,
  selectedId,
  onOpen,
  maxRows = 12,
  traceId,
  initialZoomTo,
  label,
  className,
}: ModeProps) {
  const rows = useMemo(() => compactRows(layout, maxRows), [layout, maxRows])
  const hidden = layout.rows.length - rows.length
  const zoomKey = initialZoomTo?.join('\n') ?? ''
  // Keyed by content, so a new array with the same ids keeps the view.
  const view = useMemo(() => focusWindow(layout, zoomKey ? zoomKey.split('\n') : undefined), [layout, zoomKey])
  const ticks = useMemo(
    () => [0, 0.5, 1].map((at) => ({ ns: Math.round((view[1] - view[0]) * layout.totalNs * at), at })),
    [layout.totalNs, view],
  )
  const noop = useCallback(() => {}, [])
  const open = useCallback((row: LayoutRow) => onOpen?.(row.span.span_id), [onOpen])
  return (
    <div className={cx('flex min-w-0 flex-col gap-0.5', className)} aria-label={label ?? 'Trace waterfall (summary)'} role="group">
      <div className="grid gap-2 pb-1 font-mono text-[10.5px] text-faint" style={{ gridTemplateColumns: COMPACT_COLS }}>
        <span />
        <AxisLabels ticks={ticks} />
        <span />
      </div>
      {rows.map((row) => (
        <div key={row.index} style={{ height: COMPACT_ROW_H }}>
          <SpanRow
            row={row}
            layout={layout}
            view={view}
            selected={row.span.span_id === (selectedId ?? undefined)}
            expanded
            flat
            revealed={false}
            compact
            interactive={onOpen !== undefined}
            onClick={onOpen ? open : noop}
            onToggle={noop}
          />
        </div>
      ))}
      {hidden > 0 ? (
        <div className="pt-1.5 text-xs">
          {traceId ? (
            <Link
              to="/traces/$traceId"
              params={{ traceId }}
              search={rootCauseId ? { span: rootCauseId } : {}}
              className="text-accent hover:underline"
            >
              Show all {layout.rows.length} spans
            </Link>
          ) : (
            <span className="text-muted">+{hidden} more spans</span>
          )}
        </div>
      ) : null}
    </div>
  )
}
