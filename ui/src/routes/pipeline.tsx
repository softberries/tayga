/**
 * `/pipeline`: pipeline health. A status chip per scrape job from the recorder's `up` series,
 * metric charts from the recorded history (`/pipeline/series`) and live consumer lag
 * (`/pipeline/lag`). Every section loads and fails on its own.
 */
import { useQueries, useQuery } from '@tanstack/react-query'
import { Activity } from 'lucide-react'
import { api } from '../api/queries'
import { useLiveInterval } from '../app/live'
import { useSince } from '../components/shell/TimeRange'
import { Card, PanelTitle } from '../components/ui/Card'
import { EmptyState } from '../components/ui/EmptyState'
import { Skeleton } from '../components/ui/Skeleton'
import { StaleNote } from '../features/pipeline/Notes'
import { LagList } from '../features/pipeline/LagList'
import { COLLECTING, PipelineChart } from '../features/pipeline/PipelineChart'
import { StatusStrip } from '../features/pipeline/StatusStrip'
import { CHARTS, JOBS, STATUS_SINCE } from '../features/pipeline/model'
import { ErrorBanner } from '../features/stories/ErrorBanner'

export function PipelinePage() {
  const since = useSince()
  const refetchInterval = useLiveInterval()
  const up = useQueries({
    queries: JOBS.map((job) => ({
      ...api.pipelineSeries({ since: STATUS_SINCE, metric: 'up', kind: 'gauge', job }),
      refetchInterval,
    })),
  })
  // A Kafka failure (503) is not a storage outage: keep it out of the shell banner.
  const lag = useQuery({ ...api.pipelineLag(), refetchInterval, meta: { outage: false } })

  const loaded = up.every((r) => r.data !== undefined)
  // Charts wait for the job statuses, so an empty history shows one notice, not nine.
  const upPending = up.some((r) => r.isPending)
  const staleUp = up.some((r) => r.isError && r.data !== undefined)
  const upError = up.find((r) => r.isError)
  // Nothing recorded yet (the recorder writes its first row on its first tick, 15 s apart).
  const collecting = loaded && up.every((r) => (r.data?.points.length ?? 0) === 0)

  return (
    <div className="flex flex-col gap-3.5">
      {upError && !loaded ? (
        <ErrorBanner what="job status" error={upError.error} onRetry={() => up.forEach((r) => void r.refetch())} />
      ) : (
        <StatusStrip views={loaded ? up.map((r) => r.data) : undefined} />
      )}

      {staleUp ? <StaleNote updatedAt={Math.min(...up.filter((r) => r.isError && r.data !== undefined).map((r) => r.dataUpdatedAt))} onRetry={() => up.forEach((r) => void r.refetch())} /> : null}

      {collecting ? (
        <Card>
          <EmptyState icon={<Activity size={18} />} title={COLLECTING} description="The API records every job's metrics every 15 s; charts fill in as history builds up." />
        </Card>
      ) : (
        <div className="grid grid-cols-1 gap-3.5 lg:grid-cols-2">
          {CHARTS.map((spec) =>
            upPending ? (
              <Card key={spec.id} role="status" aria-busy="true" aria-label={`Loading ${spec.title}`} className="px-4 py-3.5">
                <Skeleton className="h-[222px]" />
              </Card>
            ) : (
              <PipelineChart key={spec.id} spec={spec} since={since} refetchInterval={refetchInterval} />
            ),
          )}
        </div>
      )}

      <Card className="flex flex-col gap-3 px-4 py-3.5">
        <div className="flex flex-wrap items-baseline gap-x-3 gap-y-0.5">
          <PanelTitle>Consumer lag</PanelTitle>
          <span className="text-xs text-muted">messages each group has yet to commit, live from Kafka</span>
        </div>
        {lag.isError && lag.data ? <StaleNote updatedAt={lag.dataUpdatedAt} onRetry={() => void lag.refetch()} /> : null}
        {lag.isError && !lag.data ? (
          <ErrorBanner what="consumer lag" error={lag.error} onRetry={() => void lag.refetch()} />
        ) : (
          <LagList groups={lag.data} />
        )}
      </Card>
    </div>
  )
}
