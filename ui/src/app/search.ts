/** URL search params shared by every route. */

export const SINCE_VALUES = ['15m', '1h', '24h', '7d'] as const
export type Since = (typeof SINCE_VALUES)[number]
export const DEFAULT_SINCE: Since = '1h'

export interface RootSearch {
  /** Time range; absent means DEFAULT_SINCE, so the default keeps URLs clean. */
  since?: Since
}

function isSince(v: unknown): v is Since {
  return typeof v === 'string' && (SINCE_VALUES as readonly string[]).includes(v)
}

/**
 * Router `validateSearch` for the root route. The router merges the result over the raw
 * search, so an invalid or default value is overridden with an explicit `undefined`.
 */
export function validateRootSearch(search: Record<string, unknown>): RootSearch {
  return { since: isSince(search.since) && search.since !== DEFAULT_SINCE ? search.since : undefined }
}

/** 32 lowercase-or-uppercase hex characters (trace and story ids). */
export const HEX32 = /^[0-9a-fA-F]{32}$/
/** A u64 as decimal digits (template ids, fingerprints). */
export const U64 = /^[0-9]{1,20}$/

/**
 * Search for a cross-section link: only the time range travels, so one page's filters never
 * leak into another section. The default range stays out of the URL.
 */
export function sinceSearch(since: Since): RootSearch {
  return { since: since === DEFAULT_SINCE ? undefined : since }
}

/** 16 hex characters (span ids). */
export const HEX16 = /^[0-9a-fA-F]{16}$/

export type SpanFilter = 'errors' | 'critical'

/** `/traces/:id` search: the open span, waterfall search text and row filter. */
export interface TraceSearch {
  span?: string
  q?: string
  only?: SpanFilter
}

/** A non-empty string param, capped; the router parses `?q=42` as a number, so accept that too. */
const str = (v: unknown, max: number) => {
  const t = typeof v === 'number' && Number.isFinite(v) ? String(v) : v
  return typeof t === 'string' && t !== '' ? t.slice(0, max) : undefined
}

export function validateTraceSearch(s: Record<string, unknown>): TraceSearch {
  return {
    span: typeof s.span === 'string' && HEX16.test(s.span) ? s.span.toLowerCase() : undefined,
    q: str(s.q, 200),
    only: s.only === 'errors' || s.only === 'critical' ? s.only : undefined,
  }
}

/** Minimum log severity shown: info (9+), warn (13+), error (17+); absent shows all. */
export const SEVERITY_MIN = { info: 9, warn: 13, error: 17 } as const
export type SeverityFilter = keyof typeof SEVERITY_MIN

/** `/stories/:id` search: the trace params plus the log table's service and severity filters. */
export interface StorySearch extends TraceSearch {
  log_service?: string
  sev?: SeverityFilter
}

export function validateStorySearch(s: Record<string, unknown>): StorySearch {
  const sev = typeof s.sev === 'string' && Object.hasOwn(SEVERITY_MIN, s.sev) ? (s.sev as SeverityFilter) : undefined
  return { ...validateTraceSearch(s), log_service: str(s.log_service, 200), sev }
}

/** `/map` search: the service whose drawer is open. */
export interface MapSearch {
  service?: string
}

export function validateMapSearch(s: Record<string, unknown>): MapSearch {
  return { service: str(s.service, 200) }
}

const SINCE_SECS: Record<Since, number> = { '15m': 900, '1h': 3600, '24h': 86_400, '7d': 604_800 }

/**
 * The smallest range, no smaller than `atLeast`, whose window still contains a moment `tsNs`
 * (unix ns) as seen at `nowMs`; `7d` when even that is too short.
 */
export function sinceCovering(tsNs: number, nowMs: number, atLeast: Since = '15m'): Since {
  const ageSecs = (nowMs - tsNs / 1e6) / 1000
  const floor = SINCE_SECS[atLeast]
  return SINCE_VALUES.find((s) => SINCE_SECS[s] >= floor && SINCE_SECS[s] >= ageSecs) ?? '7d'
}

/**
 * `/` (stories home) search: the groups table's kind, service (root cause) and endpoint
 * filters, the text search, and the selected group (fingerprint).
 */
export interface HomeSearch {
  kind?: 'error' | 'slow'
  service?: string
  /** `"<endpoint service> <endpoint name>"`, as the table shows it. */
  endpoint?: string
  q?: string
  group?: string
}

export function validateHomeSearch(s: Record<string, unknown>): HomeSearch {
  const group = typeof s.group === 'number' && Number.isSafeInteger(s.group) ? String(s.group) : s.group
  return {
    kind: s.kind === 'error' || s.kind === 'slow' ? s.kind : undefined,
    service: str(s.service, 200),
    endpoint: str(s.endpoint, 400),
    q: str(s.q, 200),
    group: typeof group === 'string' && U64.test(group) ? group : undefined,
  }
}

/** A non-negative whole number of milliseconds (the API takes whole ms), capped at a day. */
const wholeMs = (v: unknown) => {
  const n = typeof v === 'string' && /^\d{1,9}$/.test(v) ? Number(v) : v
  return typeof n === 'number' && Number.isSafeInteger(n) && n >= 0 && n <= 86_400_000 ? n : undefined
}
/** `?flag=1`, `?flag=true`: the router parses either to a number or boolean. */
const flag = (v: unknown) => (v === true || v === 1 || v === '1' || v === 'true' ? true : undefined)

/**
 * A rectangle selected on the duration scatter: unix ms `t0..t1` and duration ms `d0..d1`,
 * written to the URL as `t0_t1_d0_d1`.
 */
export interface DurationRect {
  t0: number
  t1: number
  d0: number
  d1: number
}

export function formatRect(r: DurationRect): string {
  const n = (v: number) => String(Math.round(v * 1000) / 1000)
  return [Math.floor(r.t0), Math.ceil(r.t1), n(r.d0), n(r.d1)].join('_')
}

export function parseRect(v: unknown): DurationRect | undefined {
  if (typeof v !== 'string' || v.length > 80) return undefined
  const p = v.split('_').map(Number)
  if (p.length !== 4 || !p.every(Number.isFinite)) return undefined
  const [a, b, c, d] = p as [number, number, number, number]
  return { t0: Math.min(a, b), t1: Math.max(a, b), d0: Math.min(c, d), d1: Math.max(c, d) }
}

/**
 * `/traces` (explorer) search: the API filters (service, endpoint, duration bounds in whole
 * ms, errors only), the log-scale toggle, and the scatter's brush selection (`sel`).
 */
export interface TracesSearch {
  service?: string
  endpoint?: string
  min_ms?: number
  max_ms?: number
  errors?: true
  log?: true
  sel?: string
}

export function validateTracesSearch(s: Record<string, unknown>): TracesSearch {
  const min = wholeMs(s.min_ms)
  const max = wholeMs(s.max_ms)
  const sel = parseRect(s.sel)
  return {
    service: str(s.service, 200),
    endpoint: str(s.endpoint, 400),
    min_ms: min,
    // A hand-written URL with min above max would be a 400: keep the lower bound only.
    max_ms: max !== undefined && (min === undefined || max >= min) ? max : undefined,
    errors: flag(s.errors),
    log: flag(s.log),
    sel: sel ? formatRect(sel) : undefined,
  }
}
