/** URL search params shared by every route. */

export const SINCE_VALUES = ['15m', '1h', '24h', '7d'] as const
export type Since = (typeof SINCE_VALUES)[number]
export const DEFAULT_SINCE: Since = '1h'

/** The API's `since` bounds: 1 s to 7 days. */
export const MAX_SINCE_SECS = 7 * 86_400

export interface RootSearch {
  /**
   * Time range length. A preset, absent for DEFAULT_SINCE so the default keeps URLs clean; with
   * `until`, any API duration (`<n>[smhd]`, 1 s to 7 d).
   */
  since?: string
  /** End of a custom range, RFC 3339 in UTC (`2026-10-04T12:00:00Z`); absent means now. */
  until?: string
}

function isSince(v: unknown): v is Since {
  return typeof v === 'string' && (SINCE_VALUES as readonly string[]).includes(v)
}

const UNIT_SECS: Record<string, number> = { s: 1, m: 60, h: 3600, d: 86_400 }

/** Seconds of an API duration (`<n>[smhd]`, 1 s to 7 d), else undefined. */
export function sinceSecs(v: unknown): number | undefined {
  const m = typeof v === 'string' ? /^(\d{1,9})([smhd])$/.exec(v) : null
  if (!m) return undefined
  const secs = Number(m[1]) * (UNIT_SECS[m[2] ?? ''] ?? 0)
  return secs >= 1 && secs <= MAX_SINCE_SECS ? secs : undefined
}

const RFC3339 = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})$/

/**
 * Unix ms of an `until` value: RFC 3339, or unix seconds (the router parses `?until=1791115200`
 * as a number). Whole seconds, as the API reads it.
 */
export function untilMs(v: unknown): number | undefined {
  const ms =
    typeof v === 'number' && Number.isSafeInteger(v) && v > 0 && v < 1e11
      ? v * 1000
      : typeof v === 'string' && RFC3339.test(v)
        ? Date.parse(v)
        : NaN
  return Number.isFinite(ms) ? Math.floor(ms / 1000) * 1000 : undefined
}

/** `until` as the URL and the API carry it: RFC 3339 in UTC, whole seconds. */
export function formatUntil(ms: number): string {
  return new Date(ms).toISOString().replace(/\.\d{3}Z$/, 'Z')
}

/**
 * Router `validateSearch` for the root route. The router merges the result over the raw
 * search, so an invalid or default value is overridden with an explicit `undefined`. A custom
 * range needs both a valid `until` and a valid `since`; otherwise only a preset `since` stays.
 * Whether the range is still within retention is the API's call (a 400 the page shows).
 */
export function validateRootSearch(search: Record<string, unknown>): RootSearch {
  const end = untilMs(search.until)
  if (end !== undefined && sinceSecs(search.since ?? DEFAULT_SINCE) !== undefined) {
    return { since: search.since === DEFAULT_SINCE ? undefined : (search.since as string | undefined), until: formatUntil(end) }
  }
  return { since: isSince(search.since) && search.since !== DEFAULT_SINCE ? search.since : undefined, until: undefined }
}

/** 32 lowercase-or-uppercase hex characters (trace and story ids). */
export const HEX32 = /^[0-9a-fA-F]{32}$/
/** A u64 as decimal digits (template ids, fingerprints). */
export const U64 = /^[0-9]{1,20}$/

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

/** `/map` search: the open drawer's service, the service search text, and whether infrastructure services are drawn. */
export interface MapSearch {
  service?: string
  q?: string
  /** Absent (false) hides the configured infrastructure services. */
  infra?: true
}

export function validateMapSearch(s: Record<string, unknown>): MapSearch {
  return { service: str(s.service, 200), q: str(s.q, 100), infra: flag(s.infra) }
}

export const SINCE_SECS: Record<Since, number> = { '15m': 900, '1h': 3600, '24h': 86_400, '7d': 604_800 }

/**
 * The smallest preset, no shorter than `atLeastSecs`, whose window still contains a moment
 * `tsNs` (unix ns) as seen at `nowMs`; `7d` when even that is too short.
 */
export function sinceCovering(tsNs: number, nowMs: number, atLeastSecs: number = SINCE_SECS['15m']): Since {
  const ageSecs = (nowMs - tsNs / 1e6) / 1000
  return SINCE_VALUES.find((s) => SINCE_SECS[s] >= atLeastSecs && SINCE_SECS[s] >= ageSecs) ?? '7d'
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
  /**
   * The service matches any span of the trace (the API's `touched=1`); absent, only the
   * trace's endpoint (root) service. Kept only with a service.
   */
  touched?: true
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
  const service = str(s.service, 200)
  return {
    service,
    touched: service !== undefined ? flag(s.touched) : undefined,
    endpoint: str(s.endpoint, 400),
    min_ms: min,
    // A hand-written URL with min above max would be a 400: keep the lower bound only.
    max_ms: max !== undefined && (min === undefined || max >= min) ? max : undefined,
    errors: flag(s.errors),
    log: flag(s.log),
    sel: sel ? formatRect(sel) : undefined,
  }
}

/** `/logs/alerts` search: the API's kind and service filters, and an active-only toggle. */
export interface LogAlertsSearch {
  kind?: 'new' | 'spike' | 'silence'
  service?: string
  /** Only alerts that are still firing. */
  active?: true
}

export function validateLogAlertsSearch(s: Record<string, unknown>): LogAlertsSearch {
  return { kind: s.kind === 'new' || s.kind === 'spike' || s.kind === 'silence' ? s.kind : undefined, service: str(s.service, 200), active: flag(s.active) }
}

/** `/logs/templates` search: the API's service filter and text search (`q`, at most 200 chars). */
export interface LogTemplatesSearch {
  service?: string
  q?: string
}

export function validateLogTemplatesSearch(s: Record<string, unknown>): LogTemplatesSearch {
  return { service: str(s.service, 200), q: str(s.q, 200) }
}
