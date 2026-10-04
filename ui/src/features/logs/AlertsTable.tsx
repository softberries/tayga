/**
 * Log alerts as a table: kind, service, template (mono, truncated with the full text in a
 * tooltip), count against baseline, started, last seen, an active pulse and example traces.
 * The same table, without the service and template columns, lists one template's alerts.
 */
import { Link } from '@tanstack/react-router'
import type { LogAlertView } from '../../api/types'
import { sinceSearch } from '../../app/search'
import type { Since } from '../../app/search'
import { Badge } from '../../components/ui/Badge'
import { Tooltip } from '../../components/ui/Tooltip'
import { ago, dateTime, shortId } from '../../lib/format'
import { serviceColor } from '../../lib/serviceColor'
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

export interface AlertsTableProps {
  alerts: readonly LogAlertView[]
  since: Since
  nowMs: number
  /** Service and template columns; off when every row is the same template. */
  showTemplate?: boolean
  label?: string
}

export function AlertsTable({ alerts, since, nowMs, showTemplate = true, label = 'Log alerts' }: AlertsTableProps) {
  return (
    <div className="overflow-x-auto">
      <table aria-label={label} className="w-full border-collapse text-[13px]">
        <thead>
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
                    <Tooltip content={<span className="block max-w-[min(80vw,640px)] whitespace-pre-wrap break-words font-mono">{a.template}</span>}>
                      <Link
                        to="/logs/templates/$templateId"
                        params={{ templateId: a.template_id }}
                        search={sinceSearch(since)}
                        className="block w-[clamp(160px,28vw,420px)] truncate font-mono text-xs text-ink hover:underline"
                      >
                        {a.template}
                      </Link>
                    </Tooltip>
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
                {a.active ? (
                  <span className="inline-flex items-center gap-1.5 whitespace-nowrap text-xs text-err">
                    <span aria-hidden className="tg-pulse size-1.5 rounded-full bg-err" />
                    active
                  </span>
                ) : (
                  <span className="text-xs text-faint">ended</span>
                )}
              </td>
              <td className={td}>
                <ExampleTraces alert={a} since={since} />
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
