/**
 * Span drawer: attributes, resource, events (exceptions formatted), the span's logs and its
 * timing. Every value from the trace is rendered as React text, never as HTML.
 */
import { Link } from '@tanstack/react-router'
import { Check, Copy, Search } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import type { TraceLog, TraceLogTemplate, TraceSpan } from '../../api/types'
import { cx } from '../../lib/cx'
import { clockMs, duration, percent } from '../../lib/format'
import { serviceColor } from '../../lib/serviceColor'
import { severityKind, severityLabel } from '../../lib/severity'
import { Badge } from '../ui/Badge'
import { EmptyState } from '../ui/EmptyState'
import { Sheet } from '../ui/Sheet'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '../ui/Tabs'

export interface SpanDrawerProps {
  /** The open span; null closes the drawer. */
  span: TraceSpan | null
  onClose: () => void
  /** Trace window (earliest span start to latest span end), for offsets and shares. */
  traceStartNs: number
  traceTotalNs: number
  /** All logs of the trace; the drawer shows the ones of this span. */
  logs: readonly TraceLog[]
  /** Template per log id (`/traces/{id}/log-templates`), when loaded. */
  templates?: ReadonlyMap<string, TraceLogTemplate>
  rootCause?: boolean
  critical?: boolean
  /** Return focus on close (see Sheet). */
  onCloseAutoFocus?: (e: Event) => void
}

function CopyButton({ value, label }: { value: string; label: string }) {
  const [done, setDone] = useState(false)
  useEffect(() => {
    if (!done) return
    const t = window.setTimeout(() => setDone(false), 1500)
    return () => window.clearTimeout(t)
  }, [done])
  return (
    <button
      type="button"
      aria-label={done ? `Copied ${label}` : `Copy ${label}`}
      title={done ? 'Copied' : 'Copy value'}
      onClick={() => {
        navigator.clipboard
          ?.writeText(value)
          .then(() => setDone(true))
          .catch(() => {
            // Clipboard blocked (insecure origin, permissions): nothing to report.
          })
      }}
      className="inline-flex size-6 shrink-0 cursor-pointer items-center justify-center rounded-control text-muted opacity-60 transition-opacity hover:bg-rail-active hover:text-ink hover:opacity-100 focus-visible:opacity-100 group-hover:opacity-100"
    >
      {done ? <Check aria-hidden size={13} className="text-ok" /> : <Copy aria-hidden size={13} />}
    </button>
  )
}

/** Key/value table with optional search and per-row copy buttons. */
export function KeyValueTable({
  pairs,
  searchable,
  label,
  empty,
}: {
  pairs: ReadonlyArray<readonly [string, string]>
  searchable?: boolean
  label: string
  empty: string
}) {
  const [q, setQ] = useState('')
  const shown = useMemo(() => {
    const t = q.trim().toLowerCase()
    return t === '' ? pairs : pairs.filter(([k, v]) => k.toLowerCase().includes(t) || v.toLowerCase().includes(t))
  }, [pairs, q])
  if (pairs.length === 0) return <EmptyState title={empty} className="py-6" />
  return (
    <div className="flex flex-col gap-2">
      {searchable ? (
        <label className="relative flex items-center">
          <Search aria-hidden size={14} className="pointer-events-none absolute left-2.5 text-muted" />
          <input
            type="search"
            aria-label={`Search ${label}`}
            placeholder={`Search ${pairs.length} ${label}`}
            value={q}
            onChange={(e) => setQ(e.target.value)}
            className="h-[32px] w-full rounded-field border border-field-line bg-field pl-8 pr-2 text-[13px] text-ink shadow-inset placeholder:text-faint"
          />
        </label>
      ) : null}
      {shown.length === 0 ? (
        <p className="m-0 py-3 text-center text-muted">No {label} match “{q}”.</p>
      ) : (
        <table className="w-full table-fixed border-collapse font-mono text-[11.5px]">
          <caption className="sr-only">{label}</caption>
          <tbody>
            {shown.map(([k, v], i) => (
              <tr key={`${i}-${k}`} className="group border-b border-line-soft align-top last:border-b-0">
                <th scope="row" className="w-[42%] break-words py-1.5 pr-3 text-left font-normal text-muted">
                  {k}
                </th>
                <td className="whitespace-pre-wrap break-words py-1.5 text-ink [overflow-wrap:anywhere]">
                  {v === '' ? <span className="text-faint">(empty)</span> : v}
                </td>
                <td className="w-7 py-1">
                  <CopyButton value={v} label={k} />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  )
}

const EXCEPTION_KEYS = new Set(['exception.type', 'exception.message', 'exception.stacktrace'])

function EventItem({ event, spanStartNs }: { event: TraceSpan['events'][number]; spanStartNs: number }) {
  const attr = (k: string) => event.attrs.find(([key]) => key === k)?.[1]
  const isException = event.name === 'exception'
  const rest = isException ? event.attrs.filter(([k]) => !EXCEPTION_KEYS.has(k)) : event.attrs
  const type = attr('exception.type')
  const message = attr('exception.message')
  const stack = attr('exception.stacktrace')
  return (
    <li className={cx('flex flex-col gap-2 rounded-field border p-3', isException ? 'border-err/40 bg-err-soft/40' : 'border-panel-line bg-inner')}>
      <div className="flex items-baseline gap-2">
        <span className={cx('font-medium', isException && 'text-err')}>{event.name}</span>
        <span className="ml-auto font-mono text-[11px] text-muted">+{duration(Math.max(0, event.ts_ns - spanStartNs))}</span>
      </div>
      {isException ? (
        <>
          {type ? <div className="font-mono text-[12px] font-medium text-err">{type}</div> : null}
          {message ? <p className="m-0 whitespace-pre-wrap break-words text-ink [overflow-wrap:anywhere]">{message}</p> : null}
          {stack ? (
            <pre
              aria-label="Stack trace"
              className="m-0 max-h-[360px] overflow-auto whitespace-pre-wrap break-words rounded-control border border-panel-line bg-panel p-3 font-mono text-[11px] leading-[1.55] text-ink-2 [overflow-wrap:anywhere]"
            >
              {stack}
            </pre>
          ) : null}
        </>
      ) : null}
      {rest.length > 0 ? <KeyValueTable pairs={rest} label="event attributes" empty="" /> : null}
    </li>
  )
}

function Stat({ label, value, hint }: { label: string; value: string; hint?: string }) {
  return (
    <div className="flex flex-col gap-0.5 rounded-field border border-panel-line bg-inner px-3 py-2.5">
      <dt className="text-[11px] uppercase tracking-[0.06em] text-muted">{label}</dt>
      <dd className="tabular m-0 font-mono text-[15px] text-ink">{value}</dd>
      {hint ? <dd className="m-0 text-[11px] text-muted">{hint}</dd> : null}
    </div>
  )
}

export function SpanDrawer({
  span,
  onClose,
  traceStartNs,
  traceTotalNs,
  logs,
  templates,
  rootCause,
  critical,
  onCloseAutoFocus,
}: SpanDrawerProps) {
  return (
    <Sheet
      open={span !== null}
      onOpenChange={(o) => {
        if (!o) onClose()
      }}
      title={span ? span.span_name || '(unnamed span)' : ''}
      subtitle={span ? <DrawerSubtitle span={span} rootCause={rootCause} critical={critical} /> : null}
      storageKey="tayga-span-drawer-width"
      onCloseAutoFocus={onCloseAutoFocus}
      defaultWidth={560}
    >
      {span ? (
        <DrawerBody span={span} traceStartNs={traceStartNs} traceTotalNs={traceTotalNs} logs={logs} templates={templates} />
      ) : null}
    </Sheet>
  )
}

function DrawerSubtitle({ span, rootCause, critical }: { span: TraceSpan; rootCause?: boolean; critical?: boolean }) {
  return (
    <span className="flex flex-wrap items-center gap-x-2 gap-y-1">
      <Link
        to="/map"
        search={{ service: span.service_name }}
        className="inline-flex items-center gap-1.5 text-ink hover:text-accent hover:underline"
        title="Open the service on the map"
      >
        <span aria-hidden className="size-2 rounded-full" style={{ backgroundColor: serviceColor(span.service_name) }} />
        {span.service_name}
      </Link>
      <span className="text-faint">·</span>
      <span>{span.kind}</span>
      <span className="text-faint">·</span>
      <span className="font-mono">{duration(span.duration_ns)}</span>
      {span.status === 'error' ? <Badge kind="error">error</Badge> : null}
      {rootCause ? (
        <Badge kind="error" glow>
          root cause
        </Badge>
      ) : null}
      {critical ? <Badge kind="neutral">critical path</Badge> : null}
    </span>
  )
}

function DrawerBody({
  span,
  traceStartNs,
  traceTotalNs,
  logs,
  templates,
}: Pick<SpanDrawerProps, 'traceStartNs' | 'traceTotalNs' | 'logs' | 'templates'> & { span: TraceSpan }) {
  const spanLogs = useMemo(
    () => logs.filter((l) => l.span_id === span.span_id).sort((a, b) => a.ts_ns - b.ts_ns),
    [logs, span.span_id],
  )
  const offset = Math.max(0, span.start_ns - traceStartNs)
  const total = Math.max(1, traceTotalNs)
  const selfShare = span.duration_ns > 0 ? span.self_ns / span.duration_ns : 0
  return (
    <div className="flex flex-col gap-3">
      <p className="m-0 font-mono text-[11px] text-muted">
        span {span.span_id}
        {span.parent_span_id ? ` · parent ${span.parent_span_id}` : ' · root'}
      </p>
      {span.status_message ? (
        <p className="m-0 whitespace-pre-wrap break-words rounded-field bg-err-soft px-3 py-2 text-err [overflow-wrap:anywhere]">
          {span.status_message}
        </p>
      ) : null}
      <Tabs defaultValue="attributes">
        <TabsList aria-label="Span details" className="overflow-x-auto">
          <TabsTrigger value="attributes">Attributes {span.attrs.length}</TabsTrigger>
          <TabsTrigger value="resource">Resource {span.resource.length}</TabsTrigger>
          <TabsTrigger value="events">Events {span.events.length}</TabsTrigger>
          <TabsTrigger value="logs">Logs {spanLogs.length}</TabsTrigger>
          <TabsTrigger value="timing">Timing</TabsTrigger>
        </TabsList>
        <TabsContent value="attributes">
          <KeyValueTable pairs={span.attrs} searchable label="attributes" empty="No attributes on this span" />
        </TabsContent>
        <TabsContent value="resource">
          <KeyValueTable pairs={span.resource} searchable label="resource attributes" empty="No resource attributes" />
        </TabsContent>
        <TabsContent value="events">
          {span.events.length === 0 ? (
            <EmptyState title="No events on this span" className="py-6" />
          ) : (
            <ol className="m-0 flex list-none flex-col gap-2 p-0">
              {span.events.map((e, i) => (
                <EventItem key={i} event={e} spanStartNs={span.start_ns} />
              ))}
            </ol>
          )}
        </TabsContent>
        <TabsContent value="logs">
          {spanLogs.length === 0 ? (
            <EmptyState title="No logs for this span" className="py-6" />
          ) : (
            <ol className="m-0 flex list-none flex-col p-0">
              {spanLogs.map((l) => {
                const t = templates?.get(l.log_id)
                return (
                  <li key={l.log_id} className="flex flex-col gap-1 border-b border-line-soft py-2 last:border-b-0">
                    <div className="flex items-center gap-2 text-[11px]">
                      <span className="tabular font-mono text-muted">{clockMs(l.ts_ns)}</span>
                      <Badge kind={severityKind(l.severity_number)}>{severityLabel(l.severity_text, l.severity_number)}</Badge>
                    </div>
                    <p className="m-0 whitespace-pre-wrap break-words font-mono text-[12px] text-ink [overflow-wrap:anywhere]">{l.body}</p>
                    {t ? (
                      <div className="flex min-w-0 items-center gap-1.5 text-[11px]">
                        <span className="text-muted">template</span>
                        <Link
                          to="/logs/templates/$templateId"
                          params={{ templateId: t.template_id }}
                          className="min-w-0 truncate font-mono text-accent hover:underline"
                          title={t.template}
                        >
                          {t.template}
                        </Link>
                        {t.alert ? <Badge kind={t.alert}>{t.alert}</Badge> : null}
                      </div>
                    ) : null}
                  </li>
                )
              })}
            </ol>
          )}
        </TabsContent>
        <TabsContent value="timing">
          <dl className="m-0 grid grid-cols-2 gap-2">
            <Stat label="Start offset" value={`+${duration(offset)}`} hint="from the trace start" />
            <Stat label="Duration" value={duration(span.duration_ns)} />
            <Stat label="Self time" value={duration(span.self_ns)} hint={`${percent(selfShare)} of the span`} />
            <Stat
              label="Share of trace window"
              value={percent(span.duration_ns / total)}
              hint={`of the ${duration(total)} trace window`}
            />
          </dl>
          <div className="mt-4 flex flex-col gap-1.5">
            <span className="text-[11px] uppercase tracking-[0.06em] text-muted">Position in the trace</span>
            <div className="relative h-3 rounded-[4px] bg-track" aria-hidden>
              <span
                className="absolute inset-y-0 rounded-[4px] bg-accent"
                style={{
                  left: `${Math.min(100, (offset / total) * 100)}%`,
                  width: `${Math.max(0.4, Math.min(100 - (offset / total) * 100, (span.duration_ns / total) * 100))}%`,
                }}
              />
            </div>
          </div>
        </TabsContent>
      </Tabs>
    </div>
  )
}
