/**
 * Log alerts as a table: kind, service, template (mono, truncated with the full text in a
 * tooltip), count against baseline, started, last seen, an active pulse and example traces.
 * The same table, without the service and template columns, lists one template's alerts.
 * Below `sm` each alert is a card: the template on its own line, then kind, service, count
 * and status, then the example traces. The table scrolls inside a viewport-relative region.
 */
import { Link } from '@tanstack/react-router'
import type { LogAlertView } from '../../api/types'
import { sinceSearch } from '../../app/search'
import type { Since } from '../../app/search'
import { Badge } from '../../components/ui/Badge'
import { TruncationTooltip } from '../../components/ui/Tooltip'
import { cx } from '../../lib/cx'
import { ago, dateTime, shortId } from '../../lib/format'
import { serviceColor } from '../../lib/serviceColor'
import { NARROW_QUERY, useMediaQuery } from '../../lib/useMediaQuery'
import { countVsBaseline, exampleLink } from './model'

const th = 'whitespace-nowrap px-3 py-2 text-left text-[11px] font-normal uppercase tracking-[0.06em] text-muted first:pl-4 last:pr-4'
const td = 'px-3 py-2.5 align-middle first:pl-4 last:pr-4'

function Moment({ ns, nowMs }: { ns: number; nowMs: number }) {
  return (
    <time title={dateTime(ns)} dateTime={new Date(ns / 1e6).toISOString()} className="tabular whitespace-nowrap text-xs text-muted">
      {ago(ns, nowMs)}
    </time>
  )
}

export function ExampleTraces({ alert, since }: { alert: LogAlertView; since: Since }) {
  if (alert.example_traces.length === 0) return <span className="text-xs text-faint">none</span>
  return (
    <ul aria-label="Example traces" className="m-0 flex min-w-[250px] list-none flex-wrap gap-x-2 gap-y-1 p-0">
      {alert.example_traces.map((e) => {
        const target = exampleLink(e)
        return (
          <li key={e.trace_id}>
            <Link
              {...target}
              search={sinceSearch(since)}
              aria-label={`${e.story_id ? 'Story' : 'Trace'} ${e.trace_id}`}
              title={e.story_id ? `Story ${e.story_id}` : `Trace ${e.trace_id}`}
              className={`rounded-badge border px-1.5 py-px font-mono text-[11px] hover:underline ${e.story_id ? 'border-accent/50 text-accent' : 'border-field-line text-muted hover:text-ink'}`}
            >
              {shortId(e.trace_id)}
            </Link>
          </li>
        )
      })}
    </ul>
  )
}

function Status({ active }: { active: boolean }) {
  return active ? (
    <span className="inline-flex items-center gap-1.5 whitespace-nowrap text-xs text-err">
      <span aria-hidden className="tg-pulse size-1.5 rounded-full bg-err" />
      active
    </span>
  ) : (
    <span className="text-xs text-faint">ended</span>
  )
}

function TemplateLink({ a, since, className }: { a: LogAlertView; since: Since; className: string }) {
  return (
    <TruncationTooltip content={<span className="font-mono">{a.template}</span>}>
      <Link
        to="/logs/templates/$templateId"
        params={{ templateId: a.template_id }}
        search={sinceSearch(since)}
        className={cx('block truncate font-mono text-xs text-ink hover:underline', className)}
      >
        {a.template}
      </Link>
    </TruncationTooltip>
  )
}

export interface AlertsTableProps {
  alerts: readonly LogAlertView[]
  since: Since
  nowMs: number
  /** Service and template columns; off when every row is the same template. */
  showTemplate?: boolean
  label?: string
  /** Max height of the scrolling region: a number of px or any CSS length. */
  height?: number | string
}

function AlertCards({ alerts, since, nowMs, showTemplate, label }: Required<Omit<AlertsTableProps, 'height'>>) {
  return (
    <ul aria-label={label} className="m-0 list-none p-0">
      {alerts.map((a) => (
        <li key={a.alert_id} className="flex flex-col gap-1.5 border-b border-line-soft px-4 py-3 last:border-b-0">
          {showTemplate ? <TemplateLink a={a} since={since} className="w-full" /> : null}
          <div className="flex flex-wrap items-center gap-x-2.5 gap-y-1 text-xs">
            <Badge kind={a.kind}>{a.kind}</Badge>
            {showTemplate ? (
              <span className="flex min-w-0 items-center gap-1.5">
                <span aria-hidden className="size-2 shrink-0 rounded-full" style={{ backgroundColor: serviceColor(a.service) }} />
                <span className="truncate">{a.service}</span>
              </span>
            ) : null}
            <span className="tabular font-mono">{countVsBaseline(a)}</span>
            <Status active={a.active} />
            <span className="ml-auto">
              <Moment ns={a.last_at_ns} nowMs={nowMs} />
            </span>
          </div>
          <ExampleTraces alert={a} since={since} />
        </li>
      ))}
    </ul>
  )
}

export function AlertsTable({ alerts, since, nowMs, showTemplate = true, label = 'Log alerts', height = 'min(70vh, 760px)' }: AlertsTableProps) {
  const narrow = useMediaQuery(NARROW_QUERY)
  return (
    <div
      role="region"
      aria-label={`${label} (scrollable)`}
      tabIndex={0}
      className="overflow-auto overscroll-contain focus-visible:-outline-offset-2"
      style={{ maxHeight: height }}
    >
      {narrow ? (
        <AlertCards alerts={alerts} since={since} nowMs={nowMs} showTemplate={showTemplate} label={label} />
      ) : (
        <table aria-label={label} className="w-full border-collapse text-[13px]">
          <thead className="sticky top-0 z-10 bg-panel">
            <tr className="border-b border-line">
              <th scope="col" className={th}>
                Kind
              </th>
              {showTemplate ? (
                <>
                  <th scope="col" className={th}>
                    Service
                  </th>
                  <th scope="col" className={th}>
                    Template
                  </th>
                </>
              ) : null}
              <th scope="col" className={th}>
                Count vs baseline
              </th>
              <th scope="col" className={th}>
                Started
              </th>
              <th scope="col" className={th}>
                Last seen
              </th>
              <th scope="col" className={th}>
                Status
              </th>
              <th scope="col" className={th}>
                Example traces
              </th>
            </tr>
          </thead>
          <tbody>
            {alerts.map((a) => (
              <tr key={a.alert_id} className="tg-row border-b border-line-soft last:border-b-0 hover:bg-inner">
                <td className={td}>
                  <Badge kind={a.kind}>{a.kind}</Badge>
                </td>
                {showTemplate ? (
                  <>
                    <td className={td}>
                      <span className="flex items-center gap-1.5 whitespace-nowrap">
                        <span aria-hidden className="size-2 shrink-0 rounded-full" style={{ backgroundColor: serviceColor(a.service) }} />
                        {a.service}
                      </span>
                    </td>
                    <td className={td}>
                      <TemplateLink a={a} since={since} className="w-[clamp(160px,28vw,420px)]" />
                    </td>
                  </>
                ) : null}
                <td className={`${td} tabular whitespace-nowrap font-mono text-xs`}>{countVsBaseline(a)}</td>
                <td className={td}>
                  <Moment ns={a.started_at_ns} nowMs={nowMs} />
                </td>
                <td className={td}>
                  <Moment ns={a.last_at_ns} nowMs={nowMs} />
                </td>
                <td className={td}>
                  <Status active={a.active} />
                </td>
                <td className={td}>
                  <ExampleTraces alert={a} since={since} />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  )
}
