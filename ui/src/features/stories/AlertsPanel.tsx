/** The 5 newest log alerts (the API returns them newest first), linked to their templates. */
import { Link } from '@tanstack/react-router'
import type { LogAlertView } from '../../api/types'
import { sinceSearch } from '../../app/search'
import type { Since } from '../../app/search'
import { Badge } from '../../components/ui/Badge'
import { ago } from '../../lib/format'

export const ALERTS_SHOWN = 5

function rate(a: LogAlertView, nowMs: number): string {
  if (a.kind === 'new') return `first seen ${ago(a.started_at_ns, nowMs)}`
  return `${a.peak_count} vs ${a.baseline_per_window.toFixed(1)} / window`
}

export function AlertsPanel({ alerts, since, nowMs }: { alerts: readonly LogAlertView[]; since: Since; nowMs: number }) {
  if (alerts.length === 0) return <p className="m-0 text-muted">No log alerts in this window.</p>
  const newest = [...alerts].sort((a, b) => b.last_at_ns - a.last_at_ns).slice(0, ALERTS_SHOWN)
  return (
    <>
      <ul className="m-0 flex list-none flex-col gap-2.5 p-0">
        {newest.map((a) => (
          <li key={a.alert_id}>
            <Link
              to="/logs/templates/$templateId"
              params={{ templateId: a.template_id }}
              search={sinceSearch(since)}
              className="tg-card flex flex-col gap-1.5 rounded-field border border-panel-line bg-inner px-3 py-2.5 hover:border-field-line"
            >
              <span className="flex min-w-0 items-center gap-2">
                <Badge kind={a.kind}>{a.kind}</Badge>
                <span className="truncate text-muted">{a.service}</span>
                {a.active ? <span className="sr-only">active</span> : null}
                {a.active ? <span aria-hidden className="tg-pulse size-1.5 shrink-0 rounded-full bg-err" /> : null}
                <span className="tabular ml-auto shrink-0 text-xs text-muted">{rate(a, nowMs)}</span>
              </span>
              <span className="truncate font-mono text-xs text-ink" title={a.template}>
                {a.template}
              </span>
            </Link>
          </li>
        ))}
      </ul>
      {alerts.length > ALERTS_SHOWN ? (
        <Link to="/logs/alerts" search={sinceSearch(since)} className="text-xs text-accent hover:underline">
          All {alerts.length} alerts
        </Link>
      ) : null}
    </>
  )
}
