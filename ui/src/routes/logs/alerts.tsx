/**
 * `/logs/alerts`: a timeline of alerts per bucket and kind, and the alerts table. Kind and
 * service filters live in the URL; both queries refresh in live mode.
 */
import { useQuery } from '@tanstack/react-query'
import { Link, useNavigate, useSearch } from '@tanstack/react-router'
import { useCallback, useMemo } from 'react'
import { api } from '../../api/queries'
import { useLiveInterval } from '../../app/live'
import type { LogAlertsSearch } from '../../app/search'
import { useSince } from '../../components/shell/TimeRange'
import { Button } from '../../components/ui/Button'
import { Card, PanelTitle } from '../../components/ui/Card'
import { Combobox } from '../../components/ui/Combobox'
import { EmptyState } from '../../components/ui/EmptyState'
import { Skeleton } from '../../components/ui/Skeleton'
import { RefreshNote, loadFailed } from '../../components/ui/StaleNote'
import { ToggleGroup } from '../../components/ui/ToggleGroup'
import { ErrorBanner } from '../../features/stories/ErrorBanner'
import { AlertsTable } from '../../features/logs/AlertsTable'
import { AlertsTimeline } from '../../features/logs/AlertsTimeline'
import { sinceSearch } from '../../app/search'
import { TIMELINE_STEP, sortAlerts, stepWord } from '../../features/logs/model'
import { Reveal } from '../../components/ui/Reveal'

const KINDS = [
  { value: 'all', label: 'All' },
  { value: 'new', label: 'New' },
  { value: 'spike', label: 'Spike' },
] as const

function TableSkeleton() {
  return (
    <div aria-busy="true" aria-label="Loading log alerts">
      {Array.from({ length: 5 }, (_, i) => (
        <div key={i} className="flex items-center gap-4 border-b border-line-soft px-4 py-3 last:border-b-0">
          <Skeleton className="h-4 w-12" />
          <Skeleton className="h-4 w-24" />
          <Skeleton className="h-4 flex-1" />
          <Skeleton className="hidden h-4 w-28 sm:block" />
        </div>
      ))}
    </div>
  )
}

export function LogAlertsPage() {
  const since = useSince()
  const search = useSearch({ from: '/logs/alerts' })
  const navigate = useNavigate({ from: '/logs/alerts' })
  const refetchInterval = useLiveInterval()
  const services = useQuery(api.services())
  const alerts = useQuery({ ...api.logAlerts({ since, kind: search.kind, service: search.service }), refetchInterval })

  const onSearch = useCallback(
    (patch: Partial<LogAlertsSearch>) =>
      void navigate({ search: (prev) => ({ ...prev, ...patch }), replace: true, resetScroll: false }),
    [navigate],
  )
  const filtered = Boolean(search.kind || search.service || search.active)
  // Active alerts first, then the most recently seen; "Active only" hides the rest.
  const data = useMemo(() => {
    const sorted = alerts.data ? sortAlerts(alerts.data) : undefined
    return search.active ? sorted?.filter((a) => a.active) : sorted
  }, [alerts.data, search.active])
  const active = data?.filter((a) => a.active).length ?? 0

  return (
    <div className="flex flex-col gap-3.5">
      <Card className="overflow-visible">
        <div className="flex flex-wrap items-center gap-2 px-4 py-3">
          <ToggleGroup
            label="Alert kind"
            options={KINDS}
            value={search.kind ?? 'all'}
            onValueChange={(v) => onSearch({ kind: v === 'all' ? undefined : v })}
          />
          <Combobox
            label="Service"
            value={search.service}
            options={services.data ?? []}
            onChange={(service) => onSearch({ service })}
            emptyText="No services"
          />
          <Button
            size="sm"
            aria-pressed={Boolean(search.active)}
            onClick={() => onSearch({ active: search.active ? undefined : true })}
            className={search.active ? 'border-err/60 bg-err-soft text-err hover:bg-err-soft' : undefined}
          >
            <span aria-hidden className="size-2 rounded-full bg-err" />
            Active only
          </Button>
          {filtered ? (
            <Button size="sm" variant="ghost" onClick={() => onSearch({ kind: undefined, service: undefined, active: undefined })}>
              Clear filters
            </Button>
          ) : null}
          <Link
            to="/logs/templates"
            search={{ ...sinceSearch(since), service: search.service }}
            className="ml-auto text-xs text-accent hover:underline"
          >
            Browse templates
          </Link>
        </div>
      </Card>

      {loadFailed(alerts) ? <ErrorBanner what="log alerts" error={alerts.error} onRetry={() => void alerts.refetch()} /> : null}
      <RefreshNote queries={[alerts]} />

      <Card className="flex min-w-0 flex-col gap-2 px-4 py-3.5">
        <div className="flex flex-wrap items-baseline gap-x-4 gap-y-1">
          <PanelTitle>Alerts per {stepWord(TIMELINE_STEP[since])}</PanelTitle>
          <span className="text-xs text-muted">alerts started in this window, by kind</span>
          <ul aria-label="Legend" className="m-0 flex list-none gap-3 p-0 text-xs text-muted">
            <li className="flex items-center gap-1.5">
              <span aria-hidden className="size-2 rounded-full bg-accent" />
              new
            </li>
            <li className="flex items-center gap-1.5">
              <span aria-hidden className="size-2 rounded-full bg-slow" />
              spike
            </li>
          </ul>
        </div>
        {alerts.data ? <Reveal>
            <AlertsTimeline alerts={alerts.data} since={since} nowMs={alerts.dataUpdatedAt} />
          </Reveal> : <Skeleton className="h-[170px]" />}
      </Card>

      <Card className="overflow-hidden">
        <div className="flex flex-wrap items-baseline gap-x-3 border-b border-line px-4 py-2.5 text-xs text-muted" aria-live="polite">
          <PanelTitle>Alerts</PanelTitle>
          {data ? (
            <span>
              {data.length} {data.length === 1 ? 'alert' : 'alerts'}
              {data.length > 0 ? ` · ${active} active` : ''}
            </span>
          ) : null}
        </div>
        {alerts.isPending ? (
          <TableSkeleton />
        ) : data === undefined ? null : data.length === 0 ? (
          <EmptyState
            title={filtered ? 'No alerts match these filters' : 'No log alerts in this window'}
            description={filtered ? 'Clear a filter to see more.' : `No template was new or spiked in the last ${since}. A longer time range may show older alerts.`}
            action={
              filtered ? (
                <Button size="sm" onClick={() => onSearch({ kind: undefined, service: undefined, active: undefined })}>
                  Clear filters
                </Button>
              ) : null
            }
          />
        ) : (
          <Reveal>
            <AlertsTable alerts={data} since={since} nowMs={alerts.dataUpdatedAt} />
          </Reveal>
        )}
      </Card>
    </div>
  )
}
