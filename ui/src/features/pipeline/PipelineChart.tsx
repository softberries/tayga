import { SINCE_SECS } from '../../app/search'
import { useQueries } from '@tanstack/react-query'
import { Activity } from 'lucide-react'
import { useMemo } from 'react'
import { api } from '../../api/queries'
import type { Since } from '../../app/search'
import { TimeSeries } from '../../components/charts/TimeSeries'
import { Card, PanelTitle } from '../../components/ui/Card'
import { EmptyState } from '../../components/ui/EmptyState'
import { Reveal } from '../../components/ui/Reveal'
import { Skeleton } from '../../components/ui/Skeleton'
import { SectionNote, StaleNote } from '../../components/ui/StaleNote'
import { ErrorBanner } from '../stories/ErrorBanner'
import { seriesPoints } from './model'
import type { ChartSpec } from './model'

export const COLLECTING = 'Collecting… first points in 15 s'

/** One metric chart: a request per series line, drawn together on a themed time axis. */
export function PipelineChart({ spec, since, refetchInterval }: { spec: ChartSpec; since: Since; refetchInterval: number | false }) {
  const results = useQueries({
    queries: spec.series.map((s) => ({
      ...api.pipelineSeries({ since, metric: s.metric, kind: s.kind, job: s.job, labels: s.labels }),
      refetchInterval,
    })),
  })
  const updated = Math.max(...results.map((r) => r.dataUpdatedAt))
  const lines = useMemo(
    () => spec.series.map((s, i) => ({ name: s.name, tone: s.tone, type: spec.type, points: seriesPoints(results[i]?.data, s.scale, updated) })),
    // `results` is a fresh array each render; the data only changes with the update stamps.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [spec, updated],
  )
  const xRange = useMemo(() => [updated - SINCE_SECS[since] * 1000, updated] as const, [updated, since])
  const failed = results.find((r) => r.isError)
  // A failed refetch keeps the last good data (stale); a first load that failed has none (missing).
  const stale = results.filter((r) => r.isError && r.data !== undefined)
  const missing = spec.series.filter((_, i) => results[i]?.isError && results[i]?.data === undefined).map((s) => s.name)
  const staleAt = stale.length ? Math.min(...stale.map((r) => r.dataUpdatedAt)) : 0
  const pending = results.some((r) => r.isPending)
  const noData = lines.every((l) => l.points.every(([, v]) => v === null))
  const empty = !pending && noData
  const latest = lines
    .map((l) => {
      const v = l.points.findLast(([, v]) => v !== null)?.[1]
      return v === undefined || v === null ? null : `${l.name} ${spec.format(v)}`
    })
    .filter((v): v is string => v !== null)
  const retry = () => results.forEach((r) => void r.refetch())
  const summary = `${spec.title}, ${spec.unit}, last ${since}.${latest.length ? ` Latest: ${latest.join(', ')}.` : ''}`

  return (
    <Card className="flex min-w-0 flex-col gap-2 px-4 py-3.5">
      <div className="flex flex-wrap items-baseline gap-x-3 gap-y-0.5">
        <PanelTitle>{spec.title}</PanelTitle>
        <span className="text-xs text-muted">{spec.unit}</span>
      </div>
      {stale.length > 0 ? <StaleNote updatedAt={staleAt} onRetry={retry} /> : null}
      {missing.length > 0 && !(failed && noData) ? (
        <SectionNote onRetry={retry}>
          Could not load {missing.join(', ')}; {missing.length === 1 ? 'it is' : 'they are'} missing from this chart, not zero.
        </SectionNote>
      ) : null}
      {failed && noData && stale.length === 0 ? (
        <ErrorBanner what={`${spec.title.toLowerCase()} chart`} error={failed.error} onRetry={retry} />
      ) : pending ? (
        <Skeleton className="h-[190px]" />
      ) : empty ? (
        <EmptyState className="h-[190px] py-4" icon={<Activity size={18} />} title={COLLECTING} />
      ) : (
        <Reveal>
          <TimeSeries
            series={lines}
            height={190}
            format={spec.format}
            minInterval={spec.minInterval}
            summary={summary}
            xRange={xRange}
            splitNumber={4}
            legend={spec.series.length > 1}
          />
        </Reveal>
      )}
    </Card>
  )
}
