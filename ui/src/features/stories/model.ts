/** Pure helpers for the stories home: titles, dense sparkline series, deltas, alert activity. */
import type { CountBucket, LogAlertView, StoryGroup } from '../../api/types'

/** The endpoint as the table shows and filters it: `"<service> <operation>"`. */
export function endpointOf(g: Pick<StoryGroup, 'endpoint_service' | 'endpoint_name'>): string {
  return `${g.endpoint_service} ${g.endpoint_name}`
}

/**
 * A story summary split into a row title and a detail line.
 * - Slow: `"<op> took 491.0 ms (p99 327.0 ms); most time in …"` → `"<op> took 491.0 ms"` and
 *   `"p99 327.0 ms · most time in …"`.
 * - Error: `"<svc> <op> failed: <message>"` → `"<svc> <op> failed"` and `"<message>"`; other
 *   error summaries split at their first `": "`.
 * Anything else is all title.
 */
export function splitSummary(summary: string): { title: string; detail: string } {
  const slow = /^(.*?) \((p99 [^)]*)\); (.*)$/s.exec(summary)
  if (slow) return { title: slow[1] ?? '', detail: `${slow[2] ?? ''} · ${slow[3] ?? ''}` }
  const failed = summary.indexOf(' failed: ')
  if (failed > 0) return { title: summary.slice(0, failed + 7), detail: summary.slice(failed + 9) }
  const semi = summary.indexOf('; ')
  if (semi > 0) return { title: summary.slice(0, semi), detail: summary.slice(semi + 2) }
  const colon = summary.indexOf(': ')
  if (colon > 0) return { title: summary.slice(0, colon), detail: summary.slice(colon + 2) }
  return { title: summary, detail: '' }
}


/** Most points a sparkline is given; longer series keep their newest points. */
const MAX_POINTS = 240

/**
 * The sparkline grid of the window `[endMs - windowSecs, endMs)`: buckets `bucketSecs` wide on
 * the epoch grid (multiples of the width, as the API's `toStartOfInterval`), so a live window
 * that moves keeps its bucket edges. Every bucket that intersects the window is there, the
 * first and last possibly partial; the last one holds `endMs - 1 ms`. Capped at MAX_POINTS, keeping
 * the newest. `first` is the first bucket's start in unix seconds.
 */
function grid(bucketSecs: number, windowSecs: number, endMs: number): { step: number; first: number; n: number } {
  const step = Math.max(1, Math.round(bucketSecs))
  const floor = (s: number) => Math.floor(s / step) * step
  const last = floor((endMs - 1) / 1000)
  const all = Math.max(1, (last - floor(endMs / 1000 - windowSecs)) / step + 1)
  const n = Math.min(MAX_POINTS, all)
  return { step, first: last - (n - 1) * step, n }
}

/** Start (unix s) of the first bucket `denseSeries` returns for the same arguments. */
export function gridStart(bucketSecs: number, windowSecs: number, endMs: number): number {
  return grid(bucketSecs, windowSecs, endMs).first
}

/**
 * Sparse `[bucket start s, value]` pairs as one value per bucket of the window ending at
 * `endMs` (see `grid`), missing buckets as 0. Values outside the window are ignored.
 */
export function denseSeries(
  buckets: ReadonlyArray<readonly [number, number]>,
  bucketSecs: number,
  windowSecs: number,
  endMs: number,
): number[] {
  const { step, first, n } = grid(bucketSecs, windowSecs, endMs)
  const out = new Array<number>(n).fill(0)
  for (const [t, v] of buckets) {
    const i = Math.round((t - first) / step)
    if (i >= 0 && i < n && Number.isFinite(v)) out[i] = (out[i] ?? 0) + v
  }
  return out
}

/** Previous window's count from the current one and the doubled window's: never negative. */
export function previousCount(current: number, doubled: number): number {
  return Math.max(0, doubled - current)
}

/** `"+38 vs prev"`, `"−5 vs prev"` or `"same as prev"`. */
export function deltaText(current: number, previous: number): string {
  const d = Math.round(current - previous)
  if (d === 0) return 'same as prev'
  return `${d > 0 ? '+' : '−'}${Math.abs(d)} vs prev`
}

/** `"2 spike · 1 new"` over the active alerts, or `"none active"`. */
export function activeAlertsText(alerts: readonly LogAlertView[]): string {
  const spike = alerts.filter((a) => a.active && a.kind === 'spike').length
  const fresh = alerts.filter((a) => a.active && a.kind === 'new').length
  if (spike + fresh === 0) return 'none active'
  return [spike ? `${spike} spike` : '', fresh ? `${fresh} new` : ''].filter(Boolean).join(' · ')
}

/**
 * How many alerts were open in each bucket of the window ending at `endMs`: an alert counts
 * in every bucket that its `[started_at, last_at]` span touches.
 */
export function alertActivity(
  alerts: readonly LogAlertView[],
  bucketSecs: number,
  windowSecs: number,
  endMs: number,
): number[] {
  const { step, first, n } = grid(bucketSecs, windowSecs, endMs)
  const slot = (ns: number) => Math.floor((ns / 1e9 - first) / step)
  const pairs: CountBucket[] = []
  for (const a of alerts) {
    const from = Math.max(0, slot(a.started_at_ns))
    const to = Math.min(n - 1, slot(a.last_at_ns))
    for (let i = from; i <= to; i++) pairs.push([first + i * step, 1])
  }
  return denseSeries(pairs, step, windowSecs, endMs)
}

/** The largest value of a series (0 when empty). */
export function peak(values: readonly number[]): number {
  return values.reduce((m, v) => Math.max(m, v), 0)
}

/** "minute" for 60 s buckets, else "N minutes" / "N seconds". */
export function bucketWord(secs: number): string {
  if (secs === 60) return 'minute'
  if (secs % 3600 === 0) return secs === 3600 ? 'hour' : `${secs / 3600} hours`
  if (secs % 60 === 0) return `${secs / 60} minutes`
  return `${secs} seconds`
}
