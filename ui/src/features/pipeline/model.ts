import type { SeriesView } from '../../api/types'
import type { Since } from '../../app/search'
import type { SeriesTone } from '../../components/charts/TimeSeries'
import { bytes, compact, duration } from '../../lib/format'

/** Scrape jobs, in pipeline order (recorder job names). */
export const JOBS = ['tayga-ingest', 'tayga-writer', 'tayga-assembler', 'tayga-logminer', 'tayga-notifier', 'tayga-api'] as const

/** A job whose newest `up` sample is older than this counts as not scraped (the recorder ticks every 15 s). */
export const STALE_MS = 3 * 60_000
/** Window of the status strip's `up` queries, independent of the page's time range. */
export const STATUS_SINCE: Since = '15m'

export type SeriesKind = 'rate' | 'gauge' | 'q50' | 'q99'

export interface SeriesSpec {
  name: string
  metric: string
  kind: SeriesKind
  /** Restrict to one scrape job. */
  job?: string
  /** Label filter, `k=v`; the API sums series that share the rest of their labels. */
  labels?: string
  /** Multiplies every value (rates ×60 for per-minute). */
  scale?: number
  tone?: SeriesTone
}

export interface ChartSpec {
  id: string
  title: string
  /** What the y-axis shows, for the subtitle and the screen-reader summary. */
  unit: string
  series: readonly SeriesSpec[]
  format: (v: number) => string
  type?: 'line' | 'area'
  minInterval?: number
}

/** Rates with just the digits that matter: 82, 4.3, 0.05, 0. */
export function rateNumber(v: number): string {
  if (!Number.isFinite(v)) return '—'
  const abs = Math.abs(v)
  if (abs >= 1000) return compact(v)
  const digits = abs >= 10 ? 0 : abs >= 1 ? 1 : 2
  const text = v.toFixed(digits)
  return text.includes('.') ? text.replace(/\.?0+$/, '') || '0' : text
}

const perSec = (v: number) => `${rateNumber(v)}/s`
const perMin = (v: number) => `${rateNumber(v)}/min`
const seconds = (v: number) => duration(v * 1e9)
const count = (v: number) => compact(v)

/** Every chart on the page; metric names are the ones the services expose on /metrics. */
export const CHARTS: readonly ChartSpec[] = [
  {
    id: 'ingest',
    title: 'Ingest records',
    unit: 'records published per second, by kind',
    format: perSec,
    minInterval: 0,
    series: [
      { name: 'traces', metric: 'tayga_ingest_records_published_total', kind: 'rate', job: 'tayga-ingest', labels: 'kind=traces' },
      { name: 'logs', metric: 'tayga_ingest_records_published_total', kind: 'rate', job: 'tayga-ingest', labels: 'kind=logs' },
    ],
  },
  {
    id: 'writer',
    title: 'Writer rows',
    unit: 'rows committed per second',
    format: perSec,
    minInterval: 0,
    type: 'area',
    series: [{ name: 'rows', metric: 'tayga_writer_rows_inserted_total', kind: 'rate', job: 'tayga-writer', tone: 'accent' }],
  },
  {
    id: 'assembler',
    title: 'Assembler output',
    unit: 'closed traces and stories per second, by kind',
    format: perSec,
    minInterval: 0,
    series: [
      { name: 'closed traces', metric: 'tayga_assembler_closed_traces_total', kind: 'rate', job: 'tayga-assembler', tone: 'accent' },
      { name: 'error stories', metric: 'tayga_assembler_stories_total', kind: 'rate', job: 'tayga-assembler', labels: 'kind=error', tone: 'err' },
      { name: 'slow stories', metric: 'tayga_assembler_stories_total', kind: 'rate', job: 'tayga-assembler', labels: 'kind=slow', tone: 'slow' },
    ],
  },
  {
    id: 'logminer',
    title: 'Logminer throughput',
    unit: 'logs mined per second',
    format: perSec,
    minInterval: 0,
    type: 'area',
    series: [{ name: 'logs', metric: 'tayga_logminer_logs_mined_total', kind: 'rate', job: 'tayga-logminer', tone: 'accent' }],
  },
  {
    id: 'open-traces',
    title: 'Open traces',
    unit: 'traces the assembler is still holding',
    format: count,
    series: [{ name: 'open traces', metric: 'tayga_assembler_open_traces', kind: 'gauge', job: 'tayga-assembler', tone: 'accent' }],
  },
  {
    id: 'buffered',
    title: 'Buffered bytes',
    unit: 'bytes held by the assembler',
    format: bytes,
    minInterval: 0,
    series: [{ name: 'buffered', metric: 'tayga_assembler_buffered_bytes', kind: 'gauge', job: 'tayga-assembler', tone: 'accent' }],
  },
  {
    id: 'batch',
    title: 'Writer batch latency',
    unit: 'seconds per batch insert, p50 and p99',
    format: seconds,
    minInterval: 0,
    series: [
      { name: 'p50', metric: 'tayga_writer_batch_seconds', kind: 'q50', job: 'tayga-writer', tone: 'accent' },
      { name: 'p99', metric: 'tayga_writer_batch_seconds', kind: 'q99', job: 'tayga-writer', tone: 'slow' },
    ],
  },
  {
    id: 'data-lag',
    title: 'Logminer data lag',
    unit: 'seconds between a log and its mining',
    format: seconds,
    minInterval: 0,
    series: [{ name: 'data lag', metric: 'tayga_logminer_data_lag_seconds', kind: 'gauge', job: 'tayga-logminer', tone: 'slow' }],
  },
  {
    id: 'commit-failures',
    title: 'Commit failures',
    unit: 'refused offset commits per minute (the records are read again)',
    format: perMin,
    minInterval: 0,
    series: [
      { name: 'writer commit failures', metric: 'tayga_writer_commit_failures_total', kind: 'rate', job: 'tayga-writer', scale: 60 },
      { name: 'logminer commit failures', metric: 'tayga_logminer_commit_failures_total', kind: 'rate', job: 'tayga-logminer', scale: 60 },
    ],
  },
  {
    id: 'errors',
    title: 'Errors',
    unit: 'failures per minute',
    format: perMin,
    minInterval: 0,
    series: [
      { name: 'insert failures', metric: 'tayga_writer_insert_failures_total', kind: 'rate', job: 'tayga-writer', scale: 60 },
      { name: 'assembler write failures', metric: 'tayga_assembler_write_failures_total', kind: 'rate', job: 'tayga-assembler', scale: 60 },
      { name: 'logminer write failures', metric: 'tayga_logminer_write_failures_total', kind: 'rate', job: 'tayga-logminer', scale: 60 },
      { name: 'publish failures', metric: 'tayga_ingest_publish_failures_total', kind: 'rate', job: 'tayga-ingest', scale: 60 },
      { name: 'analysis panics', metric: 'tayga_assembler_analysis_panics_total', kind: 'rate', job: 'tayga-assembler', scale: 60 },
      { name: 'scrape failures', metric: 'tayga_api_scrape_failures_total', kind: 'rate', job: 'tayga-api', scale: 60 },
    ],
  },
]

export type Point = readonly [number, number | null]

/**
 * A series response as chart points, values multiplied by `scale`; missing data is `[]`.
 * A rate's newest bucket is dropped while it is still filling (`nowMs` inside it): its last
 * sample is older than the bucket's end, so the rate would dip for no real reason.
 */
export function seriesPoints(view: SeriesView | undefined, scale = 1, nowMs?: number): Point[] {
  if (!view) return []
  const open = (t: number) => view.kind === 'rate' && nowMs !== undefined && t + view.bucket_secs * 1000 > nowMs
  return view.points
    .filter(([t], i, all) => !(i === all.length - 1 && open(t)))
    .map(([t, v]) => [t, v === null ? null : v * scale] as const)
}

export type JobState = 'up' | 'down' | 'unknown'

export interface JobStatus {
  job: string
  state: JobState
  /** Start of the newest `up` bucket (unix ms); null without samples. */
  at: number | null
}

/**
 * One job's state from its recent `up` series: the newest bucket decides. A job with no
 * sample inside `STALE_MS` is down (the recorder writes `up` = 0 on a failed scrape, so
 * silence means the API's recorder itself is not running); one with no history is unknown.
 */
export function jobStatus(job: string, view: SeriesView | undefined, nowMs: number): JobStatus {
  const last = view?.points.at(-1)
  if (!view || !last) return { job, state: 'unknown', at: null }
  const [at, value] = last
  const fresh = nowMs - (at + view.bucket_secs * 1000) <= STALE_MS
  return { job, state: fresh && (value ?? 0) >= 1 ? 'up' : 'down', at }
}

/**
 * An upper bound on the newest sample's age, from its bucket start (buckets are a minute
 * wide): "scraped within the last minute", "scraped within the last 4 min".
 */
export function scrapeAge(status: JobStatus, nowMs: number): string {
  if (status.at === null) return 'no samples yet'
  const min = Math.max(1, Math.ceil((nowMs - status.at) / 60_000))
  if (min === 1) return 'scraped within the last minute'
  if (min < 60) return `scraped within the last ${min} min`
  return `scraped within the last ${Math.ceil(min / 60)} h`
}

export function shortJob(job: string): string {
  return job.replace(/^tayga-/, '')
}
