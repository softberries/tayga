/**
 * The time range every page reads: a preset ending now ("the last 1h"), or a custom range with
 * a fixed end (`until`). Both live in the URL as `since` and `until` (see `RootSearch`).
 */
import { DEFAULT_SINCE, MAX_SINCE_SECS, SINCE_SECS, formatUntil, sinceCovering, sinceSecs, untilMs } from './search'
import type { RootSearch, Since } from './search'

/**
 * How far back a range may start: the API's data retention (7 days, the longest TTL of the
 * tables its windowed queries read).
 */
export const RETENTION_SECS = 7 * 86_400
/** How far ahead of the clock `until` may lie (the API allows 60 s of clock skew). */
export const MAX_UNTIL_AHEAD_SECS = 60

export interface Range {
  /** The API's `since`: a preset, or `<secs>s` for a custom range. */
  since: string
  /** The API's `until` (RFC 3339, UTC) for a custom range; absent means now. */
  until?: string
  /** Length of the window in seconds. */
  secs: number
  /** End of a custom range in unix ms; absent for a range that ends now. */
  untilMs?: number
  /** Short name: "1h", or "Oct 4 12:00 – 14:00" for a custom range. */
  label: string
}

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']
const pad = (n: number) => String(n).padStart(2, '0')

/** "Oct 4 12:00 – 14:00" in local time; the end carries its date when it is another day. */
export function customLabel(startMs: number, endMs: number): string {
  const a = new Date(startMs)
  const b = new Date(endMs)
  const day = (d: Date) => `${MONTHS[d.getMonth()] ?? ''} ${d.getDate()}`
  // Seconds only when either end has them.
  const secs = a.getSeconds() !== 0 || b.getSeconds() !== 0
  const time = (d: Date) => `${pad(d.getHours())}:${pad(d.getMinutes())}${secs ? `:${pad(d.getSeconds())}` : ''}`
  const sameDay = a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate()
  return `${day(a)} ${time(a)} – ${sameDay ? '' : `${day(b)} `}${time(b)}`
}

/** The range a validated root search describes. */
export function rangeOf(search: RootSearch): Range {
  const since = search.since ?? DEFAULT_SINCE
  const secs = sinceSecs(since) ?? SINCE_SECS[DEFAULT_SINCE]
  const end = untilMs(search.until)
  if (end === undefined) return { since, secs, label: since }
  return { since, until: formatUntil(end), secs, untilMs: end, label: customLabel(end - secs * 1000, end) }
}

/** A preset range ending now. */
export function presetRange(since: Since): Range {
  return { since, secs: SINCE_SECS[since], label: since }
}

/** A window length as the API's `since`, in the largest whole unit: "2h", "90m", "7201s". */
export function formatSince(secs: number): string {
  if (secs % 86_400 === 0) return `${secs / 86_400}d`
  if (secs % 3600 === 0) return `${secs / 3600}h`
  if (secs % 60 === 0) return `${secs / 60}m`
  return `${secs}s`
}

/** The custom range between two moments (unix ms), to the second. */
export function customRange(fromMs: number, toMs: number): Range {
  const end = Math.floor(toMs / 1000) * 1000
  const secs = Math.round((end - fromMs) / 1000)
  return { since: formatSince(secs), until: formatUntil(end), secs, untilMs: end, label: customLabel(end - secs * 1000, end) }
}

/**
 * Search for a link or a navigation: only the time range travels, so one page's filters never
 * leak into another section. The default range stays out of the URL.
 */
export function rangeSearch(r: Range): RootSearch {
  return { since: r.since === DEFAULT_SINCE ? undefined : r.since, until: r.until }
}

/** The API's window params. */
export function rangeParams(r: Range): { since: string; until?: string } {
  return r.until === undefined ? { since: r.since } : { since: r.since, until: r.until }
}

/** End of the window in unix ms: the custom end, else `nowMs` (when the data was fetched). */
export function rangeEnd(r: Range, nowMs: number): number {
  return r.untilMs ?? nowMs
}

/** `[start, end]` of the window in unix ms, for chart axes. */
export function rangeBounds(r: Range, nowMs: number): readonly [number, number] {
  const end = rangeEnd(r, nowMs)
  return [end - r.secs * 1000, end]
}

/** How prose names the window: "the last 1h", or "Oct 4 12:00 – 14:00". */
export function rangePhrase(r: Range): string {
  return r.until === undefined ? `the last ${r.label}` : r.label
}

/** "A longer time range" hint for empty states; a custom range has no such advice. */
export function widerHint(r: Range, what: string): string {
  return r.until === undefined ? ` A longer time range may show older ${what}.` : ''
}

/**
 * The window covering this range and the one before it (for "vs previous" deltas); null when
 * it would exceed the API's 7 days or reach past retention.
 */
export function doubledRange(r: Range, nowMs: number): Range | null {
  const secs = r.secs * 2
  if (secs > MAX_SINCE_SECS) return null
  if (r.untilMs !== undefined && r.untilMs - secs * 1000 < nowMs - RETENTION_SECS * 1000) return null
  return { ...r, since: formatSince(secs), secs }
}

/**
 * Why a custom range is not valid, mirroring the API's rules (so Apply never produces a 400), or
 * null when it is fine. Times are unix ms; NaN means the field is empty or unparsable.
 */
export function customRangeError(fromMs: number, toMs: number, nowMs: number): string | null {
  if (!Number.isFinite(fromMs) || !Number.isFinite(toMs)) return 'Enter both a start and an end.'
  if (toMs <= fromMs) return 'The end must be after the start.'
  if (toMs > nowMs + MAX_UNTIL_AHEAD_SECS * 1000) return 'The end must not be in the future.'
  if (toMs - fromMs > MAX_SINCE_SECS * 1000) return 'A range can be at most 7 days long.'
  if (toMs - fromMs < 1000) return 'A range must be at least 1 second long.'
  if (fromMs < nowMs - RETENTION_SECS * 1000) return 'The start must be within the last 7 days (data retention).'
  return null
}

/** A unix ms moment as a `datetime-local` value in local time: "2026-10-04T12:00". */
export function toLocalInput(ms: number): string {
  const d = new Date(ms)
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`
}

/** Unix ms of a `datetime-local` value read as local time; NaN when empty or malformed. */
export function fromLocalInput(v: string): number {
  const m = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})(?::(\d{2}))?/.exec(v)
  if (!m) return NaN
  const [y, mo, d, h, mi, s] = m.slice(1).map((x) => Number(x ?? 0)) as [number, number, number, number, number, number]
  return new Date(y, mo - 1, d, h, mi, s).getTime()
}

/**
 * The range of a story's group trend: the header range when it contains the story (`tsNs`,
 * unix ns), else the shortest preset ending now that does (at least as long as the header's
 * own preset), so an older story still shows its group.
 */
export function trendRange(r: Range, tsNs: number, nowMs: number): Range {
  const tsMs = tsNs / 1e6
  const [start, end] = rangeBounds(r, nowMs)
  if (tsMs > start && tsMs <= end) return r
  return presetRange(sinceCovering(tsNs, nowMs, r.until === undefined ? r.secs : SINCE_SECS['15m']))
}
