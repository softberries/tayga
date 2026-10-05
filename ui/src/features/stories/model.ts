/** Pure helpers for the stories home: titles, dense sparkline series, deltas, alert activity. */
import type { CountBucket, LogAlertView, StoryGroup } from '../../api/types'
import type { Since } from '../../app/search'

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
 * Sparse `[bucket start s, value]` pairs as one value per bucket over the window ending at
 * `nowMs`, missing buckets as 0. Buckets are aligned to multiples of `bucketSecs` (as the API's
 * `toStartOfInterval` does); values outside the window are ignored.
 */
export function denseSeries(
  buckets: ReadonlyArray<readonly [number, number]>,
  bucketSecs: number,
  windowSecs: number,
  nowMs: number,
): number[] {
  const step = Math.max(1, Math.round(bucketSecs))
  const last = Math.floor(nowMs / 1000 / step) * step
  const first = Math.floor((nowMs / 1000 - windowSecs) / step) * step
  const n = Math.min(MAX_POINTS, Math.max(1, (last - first) / step + 1))
  const start = last - (n - 1) * step
  const out = new Array<number>(n).fill(0)
  for (const [t, v] of buckets) {
    const i = Math.round((t - start) / step)
    if (i >= 0 && i < n && Number.isFinite(v)) out[i] = (out[i] ?? 0) + v
  }
  return out
}

/** The `since` that covers the current and the previous window together; null for 7d (the API's limit). */
export function doubleSince(since: Since): string | null {
  return { '15m': '30m', '1h': '2h', '24h': '48h', '7d': null }[since]
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
 * How many alerts were open in each bucket of the window ending at `nowMs`: an alert counts
 * in every bucket that its `[started_at, last_at]` span touches.
 */
export function alertActivity(
  alerts: readonly LogAlertView[],
  bucketSecs: number,
  windowSecs: number,
  nowMs: number,
): number[] {
  const step = Math.max(1, Math.round(bucketSecs))
  const pairs: CountBucket[] = []
  const last = Math.floor(nowMs / 1000 / step) * step
  const first = Math.max(Math.floor((nowMs / 1000 - windowSecs) / step) * step, last - (MAX_POINTS - 1) * step)
  for (const a of alerts) {
    const from = Math.max(first, Math.floor(a.started_at_ns / 1e9 / step) * step)
    const to = Math.min(last, Math.floor(a.last_at_ns / 1e9 / step) * step)
    for (let t = from; t <= to; t += step) pairs.push([t, 1])
  }
  return denseSeries(pairs, step, windowSecs, nowMs)
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
