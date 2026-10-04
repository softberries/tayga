import { useMemo } from 'react'
import type { LogAlertView } from '../../api/types'
import type { Since } from '../../app/search'
import { TimeSeries } from '../../components/charts/TimeSeries'
import { SINCE_SECS } from '../stories/model'
import { TIMELINE_STEP, stepWord, timeline } from './model'

/** Alerts started per bucket, stacked by kind (new in the accent color, spike in the slow one). */
export function AlertsTimeline({ alerts, since, nowMs }: { alerts: readonly LogAlertView[]; since: Since; nowMs: number }) {
  const step = TIMELINE_STEP[since]
  const bars = useMemo(() => timeline(alerts, since, nowMs), [alerts, since, nowMs])
  const series = useMemo(
    () => [
      { name: 'new', type: 'bar' as const, tone: 'accent' as const, points: bars.map((b) => [b.t, b.new] as const) },
      { name: 'spike', type: 'bar' as const, tone: 'slow' as const, points: bars.map((b) => [b.t, b.spike] as const) },
    ],
    [bars],
  )
  const xRange = useMemo(() => [nowMs - SINCE_SECS[since] * 1000, nowMs + step * 500] as const, [nowMs, since, step])
  const spikes = bars.reduce((n, b) => n + b.spike, 0)
  const fresh = bars.reduce((n, b) => n + b.new, 0)
  const summary = `Log alerts started per ${stepWord(step)} over the last ${since}: ${fresh} new, ${spikes} spike.`
  return <TimeSeries series={series} stack height={170} summary={summary} xRange={xRange} splitNumber={3} />
}
