import { describe, expect, it } from 'vitest'
import { apiUrl } from '../../api/client'
import type { TraceHit } from '../../api/types'
import { MS_FLOOR, TRACE_LIMIT, barFraction, plotMs, rowsInRect, sortRows, toneOf, traceQuery } from './model'

const hit = (o: Partial<TraceHit>): TraceHit => ({
  trace_id: 'a'.repeat(32),
  ts_ns: 1_000_000_000_000,
  endpoint_service: 'frontend',
  endpoint_name: 'GET /',
  duration_ns: 50_000_000,
  is_error: false,
  span_count: 3,
  story_id: null,
  ...o,
})

describe('traceQuery', () => {
  it('builds the API query string from the URL filters and time range', () => {
    const q = traceQuery({ service: 'payment', endpoint: 'POST /api/checkout', min_ms: 5, max_ms: 100, errors: true }, '15m')
    expect(apiUrl('/traces/search', q)).toBe(
      '/api/v1/traces/search?since=15m&service=payment&endpoint=POST+%2Fapi%2Fcheckout&min_ms=5&max_ms=100&errors=true&limit=500',
    )
  })
  it('leaves unset filters out and ignores view-only params', () => {
    expect(apiUrl('/traces/search', traceQuery({ log: true, sel: '1_2_3_4' }, '1h'))).toBe(
      `/api/v1/traces/search?since=1h&limit=${TRACE_LIMIT}`,
    )
    expect(apiUrl('/traces/search', traceQuery({ min_ms: 0 }, '7d'))).toBe('/api/v1/traces/search?since=7d&min_ms=0&limit=500')
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
  it('errors win over stories; a story without an error is a slow one', () => {
    expect(toneOf(hit({ is_error: true, story_id: 'b'.repeat(32) }))).toBe('err')
    expect(toneOf(hit({ story_id: 'b'.repeat(32) }))).toBe('slow')
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
