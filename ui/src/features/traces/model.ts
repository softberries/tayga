/** Pure helpers of the traces explorer: the API query, point tones, and the brush filter. */
import type { TraceHit } from '../../api/types'
import type { DurationRect, Since, TracesSearch } from '../../app/search'

/** Rows asked of /traces/search: the API's maximum, so the scatter shows the whole picture. */
export const TRACE_LIMIT = 500

/** Params for `api.traceSearch` from the URL search and the header's time range. */
export function traceQuery(search: TracesSearch, since: Since) {
  return {
    since,
    service: search.service,
    endpoint: search.endpoint,
    min_ms: search.min_ms,
    max_ms: search.max_ms,
    errors: search.errors,
    limit: TRACE_LIMIT,
  }
}

/**
 * Error traces are red; traces the pipeline made a story of without an error are slow
 * stories (a story is either an error or a slow one); the rest use the accent.
 */
export type TraceTone = 'err' | 'slow' | 'accent'

export function toneOf(t: TraceHit): TraceTone {
  if (t.is_error) return 'err'
  return t.story_id ? 'slow' : 'accent'
}

/**
 * Smallest duration plotted (ms): a log axis cannot show 0, so 0 ns traces sit on this floor.
 * The brush filter uses the same value, so what is inside the rectangle is what is selected.
 */
export const MS_FLOOR = 0.01

export function plotMs(t: TraceHit): number {
  return Math.max(MS_FLOOR, t.duration_ns / 1e6)
}

export function plotTime(t: TraceHit): number {
  return t.ts_ns / 1e6
}

export function inRect(t: TraceHit, r: DurationRect): boolean {
  const x = plotTime(t)
  const y = plotMs(t)
  return x >= r.t0 && x <= r.t1 && y >= r.d0 && y <= r.d1
}

/** The rows inside the brushed rectangle (all rows when nothing is selected). */
export function rowsInRect(rows: readonly TraceHit[], r: DurationRect | undefined): readonly TraceHit[] {
  return r ? rows.filter((t) => inRect(t, r)) : rows
}

export type SortKey = 'start' | 'endpoint' | 'duration' | 'spans'
export interface Sort {
  key: SortKey
  desc: boolean
}

const cmp: Record<SortKey, (a: TraceHit, b: TraceHit) => number> = {
  start: (a, b) => a.ts_ns - b.ts_ns,
  endpoint: (a, b) =>
    a.endpoint_service.localeCompare(b.endpoint_service) || a.endpoint_name.localeCompare(b.endpoint_name),
  duration: (a, b) => a.duration_ns - b.duration_ns,
  spans: (a, b) => a.span_count - b.span_count,
}

/** A sorted copy; ties fall back to newest first, then the trace id, so the order is stable. */
export function sortRows(rows: readonly TraceHit[], s: Sort): TraceHit[] {
  const dir = s.desc ? -1 : 1
  return [...rows].sort(
    (a, b) => dir * cmp[s.key](a, b) || b.ts_ns - a.ts_ns || a.trace_id.localeCompare(b.trace_id),
  )
}

/**
 * Width (0..1) of a row's duration bar against the plotted extent `lo..hi` (ms), on the same
 * scale as the chart's y axis: linear from 0, or logarithmic from `lo`. Never below 2 %, so
 * every row shows a bar.
 */
export function barFraction(ms: number, lo: number, hi: number, log: boolean): number {
  const MIN = 0.02
  if (!(hi > 0)) return MIN
  const v = Math.max(MS_FLOOR, ms)
  let f: number
  if (log) {
    const a = Math.log10(Math.max(MS_FLOOR, lo))
    const b = Math.log10(hi)
    f = b > a ? (Math.log10(v) - a) / (b - a) : 1
  } else {
    f = v / hi
  }
  return Math.min(1, Math.max(MIN, f))
}
