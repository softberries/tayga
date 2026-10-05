import { describe, expect, it } from 'vitest'
import type { LogAlertView } from '../../api/types'
import {
  activeAlertsText,
  alertActivity,
  bucketWord,
  deltaText,
  denseSeries,
  endpointOf,
  previousCount,
  splitSummary,
} from './model'

describe('splitSummary', () => {
  it('splits a slow summary into the operation and its p99 and hot spot', () => {
    expect(
      splitSummary(
        'frontend-web GET /api/data took 2073.0 ms (p99 375.7 ms); most time in frontend-web GET (2066.2 ms on the critical path)',
      ),
    ).toEqual({
      title: 'frontend-web GET /api/data took 2073.0 ms',
      detail: 'p99 375.7 ms · most time in frontend-web GET (2066.2 ms on the critical path)',
    })
  })
  it('splits an error summary at "failed: "', () => {
    expect(splitSummary('payment charge failed: Payment request failed. Invalid token.')).toEqual({
      title: 'payment charge failed',
      detail: 'Payment request failed. Invalid token.',
    })
  })
  it('splits other summaries at the first ": "', () => {
    expect(splitSummary("load-generator could not reach agent (POST): ConnectionError: HTTPConnectionPool(host='agent')")).toEqual({
      title: 'load-generator could not reach agent (POST)',
      detail: "ConnectionError: HTTPConnectionPool(host='agent')",
    })
    expect(splitSummary('just a title')).toEqual({ title: 'just a title', detail: '' })
  })
})

describe('denseSeries', () => {
  const now = 600_000 // 600 s
  it('fills missing buckets with zero over the window', () => {
    expect(denseSeries([[360, 2], [480, 5]], 60, 300, now)).toEqual([0, 2, 0, 5, 0])
  })
  it('ignores buckets outside the window and bad values', () => {
    expect(denseSeries([[0, 9], [600, Number.NaN], [540, 1]], 60, 120, now)).toEqual([0, 1])
  })
  it('starts the grid at the window start, as the API buckets it', () => {
    // A window of 300 s ending at 610 s starts at 310 s, not on a minute.
    expect(denseSeries([[370, 2], [550, 4]], 60, 300, 610_000)).toEqual([0, 2, 0, 0, 4])
    // A live window fetched a moment after the API computed its start still lines up.
    expect(denseSeries([[370, 2]], 60, 300, 610_800)).toEqual([0, 2, 0, 0, 0])
  })
  it('keeps the newest points of a very long window', () => {
    const s = denseSeries([], 1, 10_000, now)
    expect(s).toHaveLength(240)
  })
})

describe('window deltas', () => {
  it('derives the previous count and describes the change', () => {
    expect(previousCount(96, 154)).toBe(58)
    expect(previousCount(10, 8)).toBe(0)
    expect(deltaText(96, 58)).toBe('+38 vs prev')
    expect(deltaText(3, 8)).toBe('−5 vs prev')
    expect(deltaText(4, 4)).toBe('same as prev')
  })
})

const alert = (kind: 'new' | 'spike', active: boolean, startS: number, lastS: number): LogAlertView => ({
  alert_id: `${kind}${startS}`,
  kind,
  template_id: '1',
  service: 'payment',
  template: 't',
  started_at_ns: startS * 1e9,
  last_at_ns: lastS * 1e9,
  window_count: 1,
  peak_count: 1,
  baseline_per_window: 0,
  active,
  example_traces: [],
})

describe('alerts', () => {
  it('counts active alerts by kind', () => {
    expect(activeAlertsText([alert('spike', true, 0, 1), alert('spike', true, 0, 1), alert('new', true, 0, 1)])).toBe(
      '2 spike · 1 new',
    )
    expect(activeAlertsText([alert('new', false, 0, 1)])).toBe('none active')
  })
  it('counts open alerts per bucket', () => {
    // Window 300 s ending at 600 s: buckets 300..600.
    expect(alertActivity([alert('spike', true, 350, 470), alert('new', false, 100, 310)], 60, 300, 600_000)).toEqual([
      2, 1, 1, 0, 0,
    ])
  })
})

describe('labels', () => {
  it('names endpoints and bucket widths', () => {
    expect(endpointOf({ endpoint_service: 'frontend', endpoint_name: 'GET /api/data' })).toBe('frontend GET /api/data')
    expect(bucketWord(60)).toBe('minute')
    expect(bucketWord(720)).toBe('12 minutes')
    expect(bucketWord(3600)).toBe('hour')
    expect(bucketWord(30)).toBe('30 seconds')
  })
})
