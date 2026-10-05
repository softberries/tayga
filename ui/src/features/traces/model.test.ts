import { describe, expect, it } from 'vitest'
import { apiUrl } from '../../api/client'
import { customRange, presetRange } from '../../app/range'
import type { TraceHit } from '../../api/types'
import { MS_FLOOR, TRACE_LIMIT, axisMs, barFraction, plotMs, rowsInRect, sortRows, toneOf, traceQuery } from './model'

const hit = (o: Partial<TraceHit>): TraceHit => ({
  trace_id: 'a'.repeat(32),
  ts_ns: 1_000_000_000_000,
  endpoint_service: 'frontend',
  endpoint_name: 'GET /',
  duration_ns: 50_000_000,
  is_error: false,
  span_count: 3,
  story_id: null,
  story_kind: null,
  ...o,
})

describe('traceQuery', () => {
  it('builds the API query string from the URL filters and time range', () => {
    const q = traceQuery({ service: 'payment', endpoint: 'POST /api/checkout', min_ms: 5, max_ms: 100, errors: true }, presetRange('15m'))
    expect(apiUrl('/traces/search', q)).toBe(
      '/api/v1/traces/search?since=15m&service=payment&endpoint=POST+%2Fapi%2Fcheckout&min_ms=5&max_ms=100&errors=true&limit=500',
    )
  })
  it('sends touched=1 to match the service on any span, and only with a service', () => {
    expect(apiUrl('/traces/search', traceQuery({ service: 'payment', touched: true }, presetRange('1h')))).toBe(
      '/api/v1/traces/search?since=1h&service=payment&touched=1&limit=500',
    )
    expect(apiUrl('/traces/search', traceQuery({ touched: true }, presetRange('1h')))).toBe('/api/v1/traces/search?since=1h&limit=500')
  })
  it('leaves unset filters out and ignores view-only params', () => {
    expect(apiUrl('/traces/search', traceQuery({ log: true, sel: '1_2_3_4' }, presetRange('1h')))).toBe(
      `/api/v1/traces/search?since=1h&limit=${TRACE_LIMIT}`,
    )
    expect(apiUrl('/traces/search', traceQuery({ min_ms: 0 }, presetRange('7d')))).toBe('/api/v1/traces/search?since=7d&min_ms=0&limit=500')
  })
  it('sends the end of a custom range as until', () => {
    const r = customRange(Date.UTC(2026, 9, 4, 12), Date.UTC(2026, 9, 4, 14))
    expect(apiUrl('/traces/search', traceQuery({ errors: true }, r))).toBe(
      '/api/v1/traces/search?since=2h&until=2026-10-04T14%3A00%3A00Z&errors=true&limit=500',
    )
  })
})

describe('brush filter', () => {
  const rows = [
    hit({ trace_id: '1'.repeat(32), ts_ns: 1_000e6, duration_ns: 50e6 }),
    hit({ trace_id: '2'.repeat(32), ts_ns: 2_000e6, duration_ns: 5_000e6 }),
    hit({ trace_id: '3'.repeat(32), ts_ns: 3_000e6, duration_ns: 60e6 }),
    hit({ trace_id: '4'.repeat(32), ts_ns: 2_500e6, duration_ns: 0 }),
  ]
  const ids = (r: readonly TraceHit[]) => r.map((t) => t.trace_id[0])

  it('keeps rows inside the rectangle, edges included', () => {
    expect(ids(rowsInRect(rows, { t0: 1_000, t1: 2_500, d0: 40, d1: 6_000 }))).toEqual(['1', '2'])
    expect(ids(rowsInRect(rows, { t0: 0, t1: 4_000, d0: 45, d1: 70 }))).toEqual(['1', '3'])
  })
  it('places 0 ns traces on the plotted floor, so a brush down to the floor selects them', () => {
    expect(plotMs(rows[3]!)).toBe(MS_FLOOR)
    expect(ids(rowsInRect(rows, { t0: 2_400, t1: 2_600, d0: 0, d1: 1 }))).toEqual(['4'])
  })
  it('returns every row without a selection', () => {
    expect(rowsInRect(rows, undefined)).toBe(rows)
  })
})

describe('tones, sorting and bars', () => {
  it('colors by error flag and story kind', () => {
    const story = 'b'.repeat(32)
    expect(toneOf(hit({ is_error: true }))).toBe('err')
    // An error story on a trace whose summary is not an error is still an error.
    expect(toneOf(hit({ story_id: story, story_kind: 'error' }))).toBe('err')
    expect(toneOf(hit({ story_id: story, story_kind: 'slow' }))).toBe('slow')
    expect(toneOf(hit({ is_error: true, story_id: story, story_kind: 'slow' }))).toBe('err')
    expect(toneOf(hit({}))).toBe('accent')
  })
  it('sorts by a column with newest-first ties', () => {
    const rows = [
      hit({ trace_id: 'a'.repeat(32), duration_ns: 5, ts_ns: 1 }),
      hit({ trace_id: 'b'.repeat(32), duration_ns: 9, ts_ns: 2 }),
      hit({ trace_id: 'c'.repeat(32), duration_ns: 5, ts_ns: 3 }),
    ]
    expect(sortRows(rows, { key: 'duration', desc: true }).map((t) => t.trace_id[0])).toEqual(['b', 'c', 'a'])
    expect(sortRows(rows, { key: 'duration', desc: false }).map((t) => t.trace_id[0])).toEqual(['c', 'a', 'b'])
    expect(sortRows(rows, { key: 'start', desc: true }).map((t) => t.trace_id[0])).toEqual(['c', 'b', 'a'])
  })
  it('bar widths follow the chart scale', () => {
    expect(barFraction(50, 1, 100, false)).toBe(0.5)
    expect(barFraction(10, 1, 100, true)).toBe(0.5)
    expect(barFraction(0, 1, 100, false)).toBe(0.02)
    expect(barFraction(5, 5, 5, true)).toBe(1)
  })
})

describe('axisMs', () => {
  it('labels every tick with a unit', () => {
    expect([0, 0.01, 0.1, 1, 100, 250, 1000, 1500, 10_000].map(axisMs)).toEqual([
      '0',
      '0.01 ms',
      '0.1 ms',
      '1 ms',
      '100 ms',
      '250 ms',
      '1 s',
      '1.5 s',
      '10 s',
    ])
  })
})
