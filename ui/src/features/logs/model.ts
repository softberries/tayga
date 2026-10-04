import type { ExampleTrace, LogAlertView, LogTemplateView } from '../../api/types'
import type { Since } from '../../app/search'
import { SINCE_SECS, denseSeries } from '../stories/model'

/** Seconds per bar of the alerts timeline: hourly for the long ranges, finer for short ones. */
export const TIMELINE_STEP: Record<Since, number> = { '15m': 60, '1h': 300, '24h': 3600, '7d': 3600 }

export function stepWord(secs: number): string {
  return secs === 3600 ? 'hour' : secs === 60 ? 'minute' : `${secs / 60} minutes`
}

export interface TimelineBar {
  /** Bucket start, unix ms. */
  t: number
  new: number
  spike: number
}

/**
 * Alerts started per bucket and kind over the window ending at `nowMs`; every bucket of the
 * window is present (zeros included) so the bars sit on a regular grid.
 */
export function timeline(alerts: readonly LogAlertView[], since: Since, nowMs: number): TimelineBar[] {
  const step = TIMELINE_STEP[since]
  const last = Math.floor(nowMs / 1000 / step) * step
  const first = Math.floor((nowMs / 1000 - SINCE_SECS[since]) / step) * step
  const bars: TimelineBar[] = []
  for (let t = first; t <= last; t += step) bars.push({ t: t * 1000, new: 0, spike: 0 })
  for (const a of alerts) {
    const i = Math.round((Math.floor(a.started_at_ns / 1e9 / step) * step - first) / step)
    const bar = bars[i]
    if (bar) bar[a.kind]++
  }
  return bars
}

/** Where an example trace links to: its story when it has one, else the trace itself. */
export function exampleLink(e: ExampleTrace):
  | { to: '/stories/$storyId'; params: { storyId: string } }
  | { to: '/traces/$traceId'; params: { traceId: string } } {
  return e.story_id
    ? { to: '/stories/$storyId', params: { storyId: e.story_id } }
    : { to: '/traces/$traceId', params: { traceId: e.trace_id } }
}

/** "35 vs 1.8 / window" for a spike; "first seen" for a new template (no baseline exists). */
export function countVsBaseline(a: Pick<LogAlertView, 'kind' | 'peak_count' | 'baseline_per_window'>): string {
  if (a.kind === 'new') return 'first seen'
  const b = a.baseline_per_window
  return `${a.peak_count} vs ${b < 10 ? b.toFixed(1) : Math.round(b)} / window`
}

export type TemplateSortKey = 'template' | 'count' | 'first'
export interface TemplateSort {
  key: TemplateSortKey
  desc: boolean
}

export function sortTemplates(rows: readonly LogTemplateView[], { key, desc }: TemplateSort): LogTemplateView[] {
  const dir = desc ? -1 : 1
  const val = (t: LogTemplateView) => (key === 'count' ? t.count : key === 'first' ? t.first_seen_ns : t.template)
  return [...rows].sort((a, b) => {
    const x = val(a)
    const y = val(b)
    const c = typeof x === 'string' && typeof y === 'string' ? x.localeCompare(y) : Number(x) - Number(y)
    return c * dir || a.template_id.localeCompare(b.template_id)
  })
}

/**
 * A template's sparse `[bucket start s, hits]` pairs as `[unix ms, hits]` points for every
 * bucket of the window ending at `nowMs`, empty buckets as 0.
 */
export function bucketPoints(
  buckets: ReadonlyArray<readonly [number, number]>,
  bucketSecs: number,
  windowSecs: number,
  nowMs: number,
): Array<readonly [number, number]> {
  const values = denseSeries(buckets, bucketSecs, windowSecs, nowMs)
  const step = Math.max(1, Math.round(bucketSecs))
  const last = Math.floor(nowMs / 1000 / step) * step
  const start = last - (values.length - 1) * step
  return values.map((v, i) => [(start + i * step) * 1000, v] as const)
}
