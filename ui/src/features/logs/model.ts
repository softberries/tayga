import type { ExampleTrace, LogAlertView, LogTemplateListItem } from '../../api/types'
import type { Range } from '../../app/range'
import { denseSeries, gridStart } from '../stories/model'

/** Seconds per bar of the alerts timeline: hourly for long ranges (over 1h), finer for short ones. */
export function timelineStep(secs: number): number {
  return secs <= 900 ? 60 : secs <= 3600 ? 300 : 3600
}

export function stepWord(secs: number): string {
  return secs === 3600 ? 'hour' : secs === 60 ? 'minute' : `${secs / 60} minutes`
}

export interface TimelineBar {
  /** Bucket start, unix ms. */
  t: number
  new: number
  spike: number
  silence: number
}

/**
 * The time an alert is drawn at, in ns, or `null` when it falls outside the window
 * `[startNs, endNs)`.
 *
 * - `new` and `spike` alerts sit at `started_at`, when they fired.
 * - A `silence` alert's `started_at` is the template's last hit, `minutes` (or days) before the
 *   alert fired, so it sits at `last_at` instead: the latest pass that detected it. One that
 *   began before the window and was still detected inside it counts, at that detection; one
 *   still detected after the window end but quiet since before it is clamped to the window end.
 */
export function timelineAt(a: Pick<LogAlertView, 'kind' | 'started_at_ns' | 'last_at_ns'>, startNs: number, endNs: number): number | null {
  if (a.kind !== 'silence') return a.started_at_ns >= startNs && a.started_at_ns < endNs ? a.started_at_ns : null
  if (a.last_at_ns < startNs || a.started_at_ns >= endNs) return null
  // 1 ms before the end: ns times are past 2^53, so `endNs - 1` would round back to `endNs`.
  return Math.min(a.last_at_ns, endNs - 1e6)
}

/**
 * Alerts per bucket and kind over the window ending at `endMs`, each at [`timelineAt`]; every
 * bucket of the window is present (zeros included) so the bars sit on a regular grid.
 */
export function timeline(alerts: readonly LogAlertView[], range: Range, endMs: number): TimelineBar[] {
  const step = timelineStep(range.secs)
  const last = Math.floor(endMs / 1000 / step) * step
  const first = Math.floor((endMs / 1000 - range.secs) / step) * step
  const bars: TimelineBar[] = []
  for (let t = first; t <= last; t += step) bars.push({ t: t * 1000, new: 0, spike: 0, silence: 0 })
  // The bucket grid: from the start of the first bucket to the end of the last one.
  const startNs = first * 1e9
  const endNs = (last + step) * 1e9
  for (const a of alerts) {
    const at = timelineAt(a, startNs, endNs)
    if (at === null) continue
    const bar = bars[Math.round((Math.floor(at / 1e9 / step) * step - first) / step)]
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

/**
 * "35 vs 1.8 / window" for a spike; "first seen" for a new template (no baseline exists);
 * "silent 12 min" for a silence: the quiet time from the template's last hit (`started_at`) to
 * the latest detection pass (`last_at`).
 */
export function countVsBaseline(
  a: Pick<LogAlertView, 'kind' | 'peak_count' | 'baseline_per_window' | 'started_at_ns' | 'last_at_ns'>,
): string {
  if (a.kind === 'new') return 'first seen'
  if (a.kind === 'silence') {
    const min = Math.max(0, Math.round((a.last_at_ns - a.started_at_ns) / 60e9))
    return `silent ${min} min`
  }
  const b = a.baseline_per_window
  return `${a.peak_count} vs ${b < 10 ? b.toFixed(1) : Math.round(b)} / window`
}

export type TemplateSortKey = 'template' | 'count' | 'first'
export interface TemplateSort {
  key: TemplateSortKey
  desc: boolean
}

export function sortTemplates(rows: readonly LogTemplateListItem[], { key, desc }: TemplateSort): LogTemplateListItem[] {
  const dir = desc ? -1 : 1
  const val = (t: LogTemplateListItem) => (key === 'count' ? t.count : key === 'first' ? t.first_seen_ns : t.template)
  return [...rows].sort((a, b) => {
    const x = val(a)
    const y = val(b)
    const c = typeof x === 'string' && typeof y === 'string' ? x.localeCompare(y) : Number(x) - Number(y)
    return c * dir || a.template_id.localeCompare(b.template_id)
  })
}

/**
 * A template's sparse `[bucket start s, hits]` pairs as `[unix ms, hits]` points for every
 * bucket of the window ending at `endMs`, empty buckets as 0 (see `denseSeries`).
 */
export function bucketPoints(
  buckets: ReadonlyArray<readonly [number, number]>,
  bucketSecs: number,
  windowSecs: number,
  endMs: number,
): Array<readonly [number, number]> {
  const values = denseSeries(buckets, bucketSecs, windowSecs, endMs)
  const step = Math.max(1, Math.round(bucketSecs))
  const first = gridStart(bucketSecs, windowSecs, endMs)
  return values.map((v, i) => [(first + i * step) * 1000, v] as const)
}

/** Active alerts first, then the most recently seen first. */
export function sortAlerts(alerts: readonly LogAlertView[]): LogAlertView[] {
  return [...alerts].sort((a, b) => Number(b.active) - Number(a.active) || b.last_at_ns - a.last_at_ns)
}
