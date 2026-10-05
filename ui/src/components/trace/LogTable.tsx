/**
 * The plan-4 story log table: time (ms), service, span, severity, body, and the template
 * with its new/spike badge. Service chips and a minimum-severity filter; rows virtualized.
 */
import { Link } from '@tanstack/react-router'
import { useVirtualizer } from '@tanstack/react-virtual'
import { useMemo, useRef } from 'react'
import type { ReactNode } from 'react'
import type { TraceLog, TraceLogTemplate } from '../../api/types'
import { SEVERITY_MIN } from '../../app/search'
import type { SeverityFilter } from '../../app/search'
import { cx } from '../../lib/cx'
import { clockMs } from '../../lib/format'
import { serviceColor } from '../../lib/serviceColor'
import { severityKind, severityLabel } from '../../lib/severity'
import { NARROW_QUERY, useMediaQuery } from '../../lib/useMediaQuery'
import { Badge } from '../ui/Badge'
import { EmptyState } from '../ui/EmptyState'
import { ToggleGroup } from '../ui/ToggleGroup'

export interface LogTableProps {
  logs: readonly TraceLog[]
  /** span id → span name, for the span column. */
  spanNames: ReadonlyMap<string, string>
  templates?: ReadonlyMap<string, TraceLogTemplate>
  service?: string
  onServiceChange: (service: string | undefined) => void
  sev?: SeverityFilter
  onSevChange: (sev: SeverityFilter | undefined) => void
  /** Clicking a span name opens it (span drawer). */
  onSpanClick?: (spanId: string) => void
  /** Max height of the scrolling rows. */
  height?: number
}

const COLS = '96px 128px minmax(120px, 170px) 72px minmax(240px, 1fr) minmax(160px, 26%)'
const AREAS = '"time svc span sev body tmpl"'
/** Below `sm` each log stacks: time, service and severity; then the body; then span and template. */
const NARROW_COLS = 'auto minmax(0, 1fr) auto'
const NARROW_AREAS = '"time svc sev" "body body body" "span tmpl tmpl"'
const SEV_OPTIONS = [
  { value: 'all', label: 'All' },
  { value: 'info', label: 'Info+' },
  { value: 'warn', label: 'Warn+' },
  { value: 'error', label: 'Error+' },
] as const

function Chip({ active, onClick, children }: { active: boolean; onClick: () => void; children: ReactNode }) {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onClick}
      className={cx(
        'inline-flex h-7 cursor-pointer items-center gap-1.5 rounded-full border px-2.5 text-xs transition-colors duration-150',
        active
          ? 'border-accent bg-row-selected text-ink'
          : 'border-field-line bg-field text-muted hover:border-accent hover:text-ink',
      )}
    >
      {children}
    </button>
  )
}

export function LogTable({
  logs,
  spanNames,
  templates,
  service,
  onServiceChange,
  sev,
  onSevChange,
  onSpanClick,
  height = 440,
}: LogTableProps) {
  const sorted = useMemo(() => [...logs].sort((a, b) => a.ts_ns - b.ts_ns), [logs])
  const perService = useMemo(() => {
    const m = new Map<string, number>()
    for (const l of sorted) m.set(l.service_name, (m.get(l.service_name) ?? 0) + 1)
    return [...m].sort(([a], [b]) => a.localeCompare(b))
  }, [sorted])
  const rows = useMemo(() => {
    const min = sev ? SEVERITY_MIN[sev] : 0
    return sorted.filter((l) => (!service || l.service_name === service) && l.severity_number >= min)
  }, [sorted, service, sev])

  const narrow = useMediaQuery(NARROW_QUERY)
  const scrollRef = useRef<HTMLDivElement>(null)
  // No React Compiler in this build; the virtualizer's unstable functions are fine here.
  // eslint-disable-next-line react-hooks/incompatible-library
  const virt = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 34,
    overscan: 10,
    getItemKey: (i) => rows[i]!.log_id,
  })

  if (logs.length === 0) return <EmptyState title="No logs for this trace" description="None of its spans emitted a log." />

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2">
        <div role="group" aria-label="Filter logs by service" className="flex flex-wrap items-center gap-1.5">
          <Chip active={!service} onClick={() => onServiceChange(undefined)}>
            all <span className="tabular font-mono text-[11px] text-muted">{sorted.length}</span>
          </Chip>
          {perService.map(([svc, n]) => (
            <Chip key={svc} active={service === svc} onClick={() => onServiceChange(service === svc ? undefined : svc)}>
              <span aria-hidden className="size-2 rounded-full" style={{ backgroundColor: serviceColor(svc) }} />
              {svc} <span className="tabular font-mono text-[11px] text-muted">{n}</span>
            </Chip>
          ))}
        </div>
        <ToggleGroup
          className="ml-auto"
          label="Minimum severity"
          options={SEV_OPTIONS}
          value={sev ?? 'all'}
          onValueChange={(v) => onSevChange(v === 'all' ? undefined : v)}
        />
      </div>

      {rows.length === 0 ? (
        <EmptyState title="No logs match" description="Try another service or a lower severity." />
      ) : (
        <div className={narrow ? undefined : 'overflow-x-auto'}>
          <div role="table" aria-label="Logs" aria-rowcount={rows.length + 1} className={narrow ? undefined : 'min-w-[920px]'}>
            <div role="rowgroup">
              <div
                role="row"
                aria-rowindex={1}
                className={cx(
                  'grid gap-3 border-b border-line px-2 pb-1.5 text-[11px] uppercase tracking-[0.06em] text-muted [scrollbar-gutter:stable]',
                  narrow && 'sr-only',
                )}
                style={{ gridTemplateColumns: COLS, gridTemplateAreas: AREAS }}
              >
                {['Time', 'Service', 'Span', 'Severity', 'Body', 'Template'].map((h) => (
                  <span role="columnheader" key={h}>
                    {h}
                  </span>
                ))}
              </div>
            </div>
            <div
              ref={scrollRef}
              role="rowgroup"
              className="relative overflow-y-auto overscroll-contain [scrollbar-gutter:stable]"
              style={{ maxHeight: height }}
            >
              <div className="relative" style={{ height: virt.getTotalSize() }}>
                {virt.getVirtualItems().map((vi) => {
                  const l = rows[vi.index]!
                  const t = templates?.get(l.log_id)
                  const span = spanNames.get(l.span_id)
                  return (
                    <div
                      key={vi.key}
                      ref={virt.measureElement}
                      data-index={vi.index}
                      role="row"
                      aria-rowindex={vi.index + 2}
                      className={cx(
                        'tg-row absolute left-0 top-0 grid w-full items-start border-b border-line-soft px-2 py-1.5 text-[12px] hover:bg-inner',
                        narrow ? 'gap-x-2 gap-y-1' : 'gap-3',
                      )}
                      style={{
                        gridTemplateColumns: narrow ? NARROW_COLS : COLS,
                        gridTemplateAreas: narrow ? NARROW_AREAS : AREAS,
                        transform: `translateY(${vi.start}px)`,
                      }}
                    >
                      <span role="cell" style={{ gridArea: 'time' }} className="tabular font-mono text-[11.5px] text-muted">
                        {clockMs(l.ts_ns)}
                      </span>
                      <span role="cell" style={{ gridArea: 'svc' }} className="flex min-w-0 items-center gap-1.5">
                        <span aria-hidden className="size-2 shrink-0 rounded-full" style={{ backgroundColor: serviceColor(l.service_name) }} />
                        <span className="truncate">{l.service_name}</span>
                      </span>
                      <span role="cell" style={{ gridArea: 'span' }} className="min-w-0 truncate">
                        {span && onSpanClick ? (
                          <button
                            type="button"
                            onClick={() => onSpanClick(l.span_id)}
                            className="max-w-full cursor-pointer truncate text-left text-accent hover:underline"
                            title={`Open span ${span}`}
                          >
                            {span}
                          </button>
                        ) : (
                          <span className="text-muted">{span ?? '—'}</span>
                        )}
                      </span>
                      <span role="cell" style={{ gridArea: 'sev' }}>
                        <Badge kind={severityKind(l.severity_number)}>{severityLabel(l.severity_text, l.severity_number)}</Badge>
                      </span>
                      <span role="cell" style={{ gridArea: 'body' }} className="whitespace-pre-wrap break-words font-mono text-[11.5px] text-ink [overflow-wrap:anywhere]">
                        {l.body}
                      </span>
                      <span role="cell" style={{ gridArea: 'tmpl' }} className="flex min-w-0 items-center gap-1.5">
                        {t ? (
                          <>
                            <Link
                              to="/logs/templates/$templateId"
                              params={{ templateId: t.template_id }}
                              title={t.template}
                              className="min-w-0 truncate font-mono text-[11.5px] text-accent hover:underline"
                            >
                              {t.template}
                            </Link>
                            {t.alert ? <Badge kind={t.alert}>{t.alert}</Badge> : null}
                          </>
                        ) : (
                          <span className="text-faint">—</span>
                        )}
                      </span>
                    </div>
                  )
                })}
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
