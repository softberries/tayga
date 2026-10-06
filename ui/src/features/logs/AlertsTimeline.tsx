import { useMemo } from 'react'
import type { LogAlertView } from '../../api/types'
import { rangeBounds, rangePhrase } from '../../app/range'
import type { Range } from '../../app/range'
import { TimeSeries } from '../../components/charts/TimeSeries'
import { stepWord, timeline, timelineStep } from './model'

/** Alerts started per bucket, stacked by kind (new in the accent color, spike in the slow one, silence in its own). */
export function AlertsTimeline({ alerts, range, nowMs }: { alerts: readonly LogAlertView[]; range: Range; nowMs: number }) {
  const step = timelineStep(range.secs)
  const [start, end] = rangeBounds(range, nowMs)
  const bars = useMemo(() => timeline(alerts, range, end), [alerts, range, end])
  const series = useMemo(
    () => [
      { name: 'new', type: 'bar' as const, tone: 'accent' as const, points: bars.map((b) => [b.t, b.new] as const) },
      { name: 'spike', type: 'bar' as const, tone: 'slow' as const, points: bars.map((b) => [b.t, b.spike] as const) },
      { name: 'silence', type: 'bar' as const, tone: 'silence' as const, points: bars.map((b) => [b.t, b.silence] as const) },
    ],
    [bars],
  )
  const xRange = useMemo(() => [start, end + step * 500] as const, [start, end, step])
  const spikes = bars.reduce((n, b) => n + b.spike, 0)
  const fresh = bars.reduce((n, b) => n + b.new, 0)
  const silent = bars.reduce((n, b) => n + b.silence, 0)
  const summary = `Log alerts started per ${stepWord(step)} over ${rangePhrase(range)}: ${fresh} new, ${spikes} spike, ${silent} silence.`
  return <TimeSeries series={series} stack height={170} summary={summary} xRange={xRange} splitNumber={3} />
}
