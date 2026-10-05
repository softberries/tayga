/**
 * `/logs/templates/:id`: the template with its hits over the window as a bar chart, a sample
 * log line, the latest hits with links to their traces and stories, and the template's alerts.
 * Template text and samples are log data and render as plain text only.
 */
import { useQuery } from '@tanstack/react-query'
import { Link, useParams } from '@tanstack/react-router'
import { SearchX } from 'lucide-react'
import { useMemo } from 'react'
import { isApiError } from '../../api/client'
import { api } from '../../api/queries'
import type { TemplateHit } from '../../api/types'
import { useLiveInterval } from '../../app/live'
import { sinceSearch, SINCE_SECS } from '../../app/search'
import type { Since } from '../../app/search'
import { TimeSeries } from '../../components/charts/TimeSeries'
import { useSince } from '../../components/shell/TimeRange'
import { Badge } from '../../components/ui/Badge'
import { Button } from '../../components/ui/Button'
import { Card, PanelTitle } from '../../components/ui/Card'
import { EmptyState } from '../../components/ui/EmptyState'
import { ErrorState } from '../../components/ui/ErrorState'
import { Skeleton } from '../../components/ui/Skeleton'
import { RefreshNote } from '../../components/ui/StaleNote'
import { AlertsTable } from '../../features/logs/AlertsTable'
import { bucketPoints, stepWord } from '../../features/logs/model'
import { ago, compact, dateTime, shortId } from '../../lib/format'
import { serviceColor } from '../../lib/serviceColor'
import { severityKind, severityLabel } from '../../lib/severity'

function PageSkeleton() {
  return (
    <div className="flex flex-col gap-3.5" aria-busy="true" aria-label="Loading log template">
      <Card className="flex flex-col gap-3 p-5">
        <Skeleton className="h-4 w-1/4" />
        <Skeleton className="h-5 w-3/4" />
      </Card>
      <Card className="p-5">
        <Skeleton className="h-[190px]" />
      </Card>
      <Card className="flex flex-col gap-2 p-5">
        <Skeleton className="h-4" />
        <Skeleton className="h-4 w-2/3" />
      </Card>
    </div>
  )
}

function Stat({ label, value, title }: { label: string; value: string; title?: string }) {
  return (
    <div className="flex flex-col gap-0.5" title={title}>
      <dt className="text-[11px] uppercase tracking-[0.06em] text-muted">{label}</dt>
      <dd className="tabular m-0 font-mono text-[15px] text-ink">{value}</dd>
    </div>
  )
}

function RecentHits({ hits, since, nowMs }: { hits: readonly TemplateHit[]; since: Since; nowMs: number }) {
  if (hits.length === 0) return <EmptyState title="No hits recorded" description="No log has matched this template yet." />
  const th = 'whitespace-nowrap px-3 py-2 text-left text-[11px] font-normal uppercase tracking-[0.06em] text-muted first:pl-4 last:pr-4'
  const td = 'px-3 py-2 align-middle first:pl-4 last:pr-4'
  return (
    <div className="overflow-x-auto">
      <table aria-label="Recent hits" className="w-full border-collapse text-[13px]">
        <thead>
          <tr className="border-b border-line">
            <th scope="col" className={th}>
              Time
            </th>
            <th scope="col" className={th}>
              Severity
            </th>
            <th scope="col" className={th}>
              Trace
            </th>
            <th scope="col" className={th}>
              Story
            </th>
          </tr>
        </thead>
        <tbody>
          {hits.map((h, i) => (
            <tr key={`${h.trace_id}-${h.span_id}-${h.ts_ns}-${i}`} className="tg-row border-b border-line-soft last:border-b-0 hover:bg-inner">
              <td className={td}>
                <time title={dateTime(h.ts_ns)} dateTime={new Date(h.ts_ns / 1e6).toISOString()} className="tabular whitespace-nowrap font-mono text-xs text-muted">
                  {dateTime(h.ts_ns)} <span className="font-sans">({ago(h.ts_ns, nowMs)})</span>
                </time>
              </td>
              <td className={td}>
                <Badge kind={severityKind(h.severity_number)}>{severityLabel('', h.severity_number)}</Badge>
              </td>
              <td className={td}>
                <Link
                  to="/traces/$traceId"
                  params={{ traceId: h.trace_id }}
                  search={sinceSearch(since)}
                  title={h.trace_id}
                  className="font-mono text-xs text-accent hover:underline"
                >
                  {shortId(h.trace_id)}
                </Link>
              </td>
              <td className={td}>
                {h.story_id ? (
                  <Link
                    to="/stories/$storyId"
                    params={{ storyId: h.story_id }}
                    search={sinceSearch(since)}
                    aria-label={`Story of trace ${h.trace_id}`}
                    className="text-xs text-accent hover:underline"
                  >
                    story
                  </Link>
                ) : (
                  <span className="text-xs text-faint">none</span>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

export function LogTemplatePage() {
  const { templateId } = useParams({ from: '/logs/templates/$templateId' })
  const since = useSince()
  const refetchInterval = useLiveInterval()
  const detail = useQuery({ ...api.logTemplate(templateId, since), refetchInterval })
  const nowMs = detail.dataUpdatedAt
  const d = detail.data
  const points = useMemo(
    () => (d ? bucketPoints(d.buckets, d.bucket_secs, SINCE_SECS[since], nowMs) : []),
    [d, since, nowMs],
  )
  const series = useMemo(() => [{ name: 'hits', type: 'bar' as const, tone: 'accent' as const, points }], [points])
  const xRange = useMemo(() => [nowMs - SINCE_SECS[since] * 1000, nowMs + ((d?.bucket_secs ?? 60) * 1000) / 2] as const, [nowMs, since, d?.bucket_secs])

  if (detail.isPending) return <PageSkeleton />
  if (!detail.data) {
    if (isApiError(detail.error) && detail.error.status === 404)
      return (
        <Card>
          <EmptyState
            icon={<SearchX size={18} />}
            title="Template not found"
            description={`No log template ${templateId} is stored. It may have expired.`}
            action={
              <Button asChild size="sm">
                <Link to="/logs/templates" search={sinceSearch(since)}>
                  All templates
                </Link>
              </Button>
            }
          />
        </Card>
      )
    return (
      <Card>
        <ErrorState error={detail.error} onRetry={() => void detail.refetch()} />
      </Card>
    )
  }
  const t = detail.data.template
  const summary = `Hits of this template per ${stepWord(detail.data.bucket_secs)} over the last ${since}: ${t.count} in total.`

  return (
    <div className="flex flex-col gap-3.5">
      <RefreshNote queries={[detail]} />
      <Card className="tg-in flex flex-col gap-4 px-5 py-4">
        <div className="flex flex-wrap items-center gap-2">
          <span className="flex items-center gap-1.5 text-[13px]">
            <span aria-hidden className="size-2 rounded-full" style={{ backgroundColor: serviceColor(t.service) }} />
            {t.service}
          </span>
          {t.alerting ? <Badge kind="spike">alerting</Badge> : null}
          <Link to="/logs/templates" search={sinceSearch(since)} className="ml-auto text-xs text-accent hover:underline">
            All templates
          </Link>
        </div>
        <h2 className="m-0 whitespace-pre-wrap break-words font-mono text-[13px] font-medium leading-relaxed text-ink [overflow-wrap:anywhere]">
          {t.template}
        </h2>
        <dl className="m-0 grid grid-cols-[repeat(auto-fit,minmax(120px,1fr))] gap-4">
          <Stat label={`Hits in ${since}`} value={compact(t.count)} title={String(t.count)} />
          <Stat label="First seen" value={ago(t.first_seen_ns, nowMs)} title={dateTime(t.first_seen_ns)} />
          <Stat label="Last seen" value={ago(t.last_seen_ns, nowMs)} title={dateTime(t.last_seen_ns)} />
          <Stat label="Alerts" value={String(detail.data.alerts.length)} />
        </dl>
      </Card>

      <Card className="flex min-w-0 flex-col gap-2 px-4 py-3.5">
        <PanelTitle>Hits per {stepWord(detail.data.bucket_secs)}</PanelTitle>
        {detail.data.buckets.length === 0 ? (
          <EmptyState title="No hits in this window" description={`This template did not match any log in the last ${since}. A longer time range may show older hits.`} />
        ) : (
          <TimeSeries series={series} height={190} summary={summary} xRange={xRange} />
        )}
      </Card>

      <Card className="flex min-w-0 flex-col gap-2 px-4 py-3.5">
        <PanelTitle>Sample</PanelTitle>
        <pre className="m-0 max-h-60 overflow-auto whitespace-pre-wrap break-words rounded-field border border-panel-line bg-inner p-3 font-mono text-xs leading-relaxed text-ink [overflow-wrap:anywhere]">
          {detail.data.sample.replace(/\n+$/, '') || '(empty)'}
        </pre>
      </Card>

      <Card className="overflow-hidden">
        <div className="border-b border-line px-4 py-2.5">
          <div className="flex flex-wrap items-baseline gap-x-3">
            <PanelTitle>Latest {detail.data.recent.length} hits</PanelTitle>
            <span className="text-xs text-muted">the newest, whatever the time range</span>
          </div>
        </div>
        <RecentHits hits={detail.data.recent} since={since} nowMs={nowMs} />
      </Card>

      <Card className="overflow-hidden">
        <div className="border-b border-line px-4 py-2.5">
          <PanelTitle>Alerts</PanelTitle>
        </div>
        {detail.data.alerts.length === 0 ? (
          <EmptyState title="No alerts for this template" description="It has not spiked and was not flagged as new." />
        ) : (
          <AlertsTable alerts={detail.data.alerts} since={since} nowMs={nowMs} showTemplate={false} label="Alerts of this template" />
        )}
      </Card>
    </div>
  )
}
