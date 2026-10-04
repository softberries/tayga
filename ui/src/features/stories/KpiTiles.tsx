/** KPI tiles from /overview: count-up numbers, area sparklines and a delta to the previous window. */
import type { UseQueryResult } from '@tanstack/react-query'
import type { LogAlertView, OverviewView } from '../../api/types'
import type { Since } from '../../app/search'
import { Spark } from '../../components/charts/Spark'
import type { SparkTone } from '../../components/charts/Spark'
import { Card } from '../../components/ui/Card'
import { CountUp } from '../../components/ui/CountUp'
import { Skeleton } from '../../components/ui/Skeleton'
import { StaggerItem, StaggerList } from '../../components/ui/Stagger'
import { compact } from '../../lib/format'
import {
  SINCE_SECS,
  activeAlertsText,
  alertActivity,
  bucketWord,
  deltaText,
  denseSeries,
  peak,
  previousCount,
} from './model'

interface Tile {
  label: string
  value: number
  format?: (n: number) => string
  delta: string
  tone: SparkTone
  values: number[]
  summary: string
}

const valueTone: Record<SparkTone, string> = {
  err: 'text-err',
  slow: 'text-slow',
  accent: 'text-accent',
  ok: 'text-ok',
}

function lagText(secs: number | null): string {
  if (secs === null || !Number.isFinite(secs)) return 'lag —'
  return secs < 10 ? `lag ${secs.toFixed(1)} s` : `lag ${Math.round(secs)} s`
}

export function buildTiles(
  o: OverviewView,
  doubled: OverviewView | undefined,
  alerts: readonly LogAlertView[] | undefined,
  since: Since,
  nowMs: number,
): Tile[] {
  const win = SINCE_SECS[since]
  const per = bucketWord(o.bucket_secs)
  const err = denseSeries(o.stories.error, o.stories.bucket_secs, win, nowMs)
  const slow = denseSeries(o.stories.slow, o.stories.bucket_secs, win, nowMs)
  const spans = denseSeries(o.spans, o.bucket_secs, win, nowMs)
  const alertSeries = alerts ? alertActivity(alerts, o.bucket_secs, win, nowMs) : []
  const delta = (cur: number, pick: (v: OverviewView) => number) =>
    doubled ? deltaText(cur, previousCount(cur, pick(doubled))) : since === '7d' ? 'no earlier window' : '…'
  return [
    {
      label: `Error stories · ${since}`,
      value: o.error_stories,
      delta: delta(o.error_stories, (v) => v.error_stories),
      tone: 'err',
      values: err,
      summary: `Error stories per ${per} over ${since}, at most ${peak(err)} in one ${per}`,
    },
    {
      label: `Slow stories · ${since}`,
      value: o.slow_stories,
      delta: delta(o.slow_stories, (v) => v.slow_stories),
      tone: 'slow',
      values: slow,
      summary: `Slow stories per ${per} over ${since}, at most ${peak(slow)} in one ${per}`,
    },
    {
      label: 'Active log alerts',
      value: o.active_alerts,
      delta: alerts ? activeAlertsText(alerts) : '…',
      tone: 'accent',
      values: alertSeries,
      summary: `Open log alerts per ${per} over ${since}, at most ${peak(alertSeries)} at once`,
    },
    {
      label: 'Spans / s',
      value: o.spans_per_sec,
      format: compact,
      delta: lagText(o.data_lag_secs),
      tone: 'ok',
      values: spans,
      summary: `Spans per second over ${since}, peak ${compact(peak(spans))}`,
    },
  ]
}

export function KpiTilesSkeleton() {
  return (
    <div className="grid grid-cols-2 gap-3.5 lg:grid-cols-4" aria-busy="true" aria-label="Loading summary">
      {Array.from({ length: 4 }, (_, i) => (
        <Card key={i} variant="tile" className="flex flex-col gap-2 px-4 py-3.5">
          <Skeleton className="h-3 w-28" />
          <Skeleton className="h-7 w-16" />
          <Skeleton className="h-[30px]" />
        </Card>
      ))}
    </div>
  )
}

export function KpiTiles({
  overview,
  doubled,
  alerts,
  since,
  nowMs,
}: {
  overview: OverviewView
  doubled: UseQueryResult<OverviewView>
  alerts: readonly LogAlertView[] | undefined
  since: Since
  /** End of the window: when the overview was fetched. */
  nowMs: number
}) {
  const tiles = buildTiles(overview, doubled.data, alerts, since, nowMs)
  return (
    <StaggerList role="list" aria-label="Summary" className="grid grid-cols-2 gap-3.5 lg:grid-cols-4">
      {tiles.map((t) => (
        <StaggerItem key={t.label} role="listitem" className="min-w-0">
          <Card variant="tile" lift className="flex h-full min-w-0 flex-col gap-1.5 px-4 py-3.5">
            <span className="truncate text-[11px] uppercase tracking-[0.06em] text-muted">{t.label}</span>
            <div className="flex min-w-0 flex-wrap items-baseline gap-x-2.5">
              <CountUp
                value={t.value}
                format={t.format}
                className={`tabular text-[26px] font-semibold tracking-[-0.02em] ${valueTone[t.tone]}`}
              />
              <span className="truncate text-xs text-muted">{t.delta}</span>
            </div>
            <Spark values={t.values} tone={t.tone} width={220} height={30} label={t.summary} />
          </Card>
        </StaggerItem>
      ))}
    </StaggerList>
  )
}
