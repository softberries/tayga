import { describe, expect, it } from 'vitest'
import {
  customLabel,
  customRange,
  customRangeError,
  doubledRange,
  formatSince,
  fromLocalInput,
  presetRange,
  rangeBounds,
  rangeOf,
  rangeParams,
  rangePhrase,
  rangeSearch,
  toLocalInput,
  trendRange,
} from './range'

const at = (y: number, mo: number, d: number, h: number, mi = 0) => new Date(y, mo - 1, d, h, mi).getTime()
const NOW = at(2026, 10, 4, 15)
const HOUR = 3_600_000

describe('ranges', () => {
  it('a preset ends now and keeps the URL clean at the default', () => {
    const r = rangeOf({})
    expect(r).toEqual({ since: '1h', secs: 3600, label: '1h' })
    expect(rangeSearch(r)).toEqual({ since: undefined, until: undefined })
    expect(rangeParams(rangeOf({ since: '24h' }))).toEqual({ since: '24h' })
    expect(rangeBounds(r, NOW)).toEqual([NOW - HOUR, NOW])
    expect(rangePhrase(rangeOf({ since: '15m' }))).toBe('the last 15m')
  })

  it('a custom range has a fixed end, a local label and travels as since and until', () => {
    const r = customRange(at(2026, 10, 4, 12), at(2026, 10, 4, 14))
    expect(r.since).toBe('2h')
    expect(r.label).toBe('Oct 4 12:00 – 14:00')
    expect(rangeParams(r)).toEqual({ since: '2h', until: r.until })
    expect(rangeSearch(r)).toEqual({ since: '2h', until: r.until })
    expect(rangeOf(rangeSearch(r))).toEqual(r)
    // The end is fixed: the clock does not move the window.
    expect(rangeBounds(r, NOW)).toEqual([at(2026, 10, 4, 12), at(2026, 10, 4, 14)])
    expect(rangePhrase(r)).toBe('Oct 4 12:00 – 14:00')
  })

  it('labels a range over midnight with both dates, seconds only when present', () => {
    expect(customLabel(at(2026, 10, 3, 22), at(2026, 10, 4, 2, 30))).toBe('Oct 3 22:00 – Oct 4 02:30')
    expect(customLabel(at(2026, 10, 4, 12) + 5000, at(2026, 10, 4, 13))).toBe('Oct 4 12:00:05 – 13:00:00')
  })

  it('names durations in their largest whole unit', () => {
    expect([formatSince(86_400), formatSince(7200), formatSince(5400), formatSince(61)]).toEqual(['1d', '2h', '90m', '61s'])
  })

  it('doubles the window for deltas within 7 days and retention', () => {
    expect(doubledRange(presetRange('1h'), NOW)?.since).toBe('2h')
    expect(doubledRange(presetRange('24h'), NOW)?.since).toBe('2d')
    expect(doubledRange(presetRange('7d'), NOW)).toBeNull()
    const past = customRange(NOW - 2 * 86_400_000, NOW - 86_400_000)
    expect(rangeParams(doubledRange(past, NOW)!)).toEqual({ since: '2d', until: past.until })
    // Twice 4 days would start before the 7 days of retention.
    expect(doubledRange(customRange(NOW - 6 * 86_400_000, NOW - 2 * 86_400_000), NOW)).toBeNull()
  })

  it('keeps a story trend on the range that contains the story', () => {
    const r = customRange(at(2026, 10, 4, 12), at(2026, 10, 4, 14))
    expect(trendRange(r, at(2026, 10, 4, 13) * 1e6, NOW)).toBe(r)
    // Outside the custom range: the shortest preset ending now that covers it.
    expect(trendRange(r, at(2026, 10, 4, 10) * 1e6, NOW)).toEqual(presetRange('24h'))
    expect(trendRange(presetRange('1h'), (NOW - 10 * 60_000) * 1e6, NOW)).toEqual(presetRange('1h'))
    expect(trendRange(presetRange('1h'), (NOW - 3 * HOUR) * 1e6, NOW)).toEqual(presetRange('24h'))
  })
})

describe('custom range validation (mirrors the API)', () => {
  const err = (from: number, to: number) => customRangeError(from, to, NOW)
  const msg = (from: number, to: number) => {
    const e = err(from, to)
    return e ? `${e.field}: ${e.message}` : null
  }
  it('accepts a past range within retention', () => {
    expect(err(NOW - 3 * HOUR, NOW - HOUR)).toBeNull()
    expect(err(NOW - 7 * 86_400_000, NOW)).toBeNull()
    expect(err(NOW - HOUR, NOW + 60_000)).toBeNull()
  })
  it('rejects what the API would answer 400 to', () => {
    // Each error names the field it is about.
    expect(msg(NaN, NOW)).toBe('from: Enter a start.')
    expect(msg(NOW - HOUR, NaN)).toBe('to: Enter an end.')
    expect(msg(NOW - HOUR, NOW - 2 * HOUR)).toBe('to: The end must be after the start.')
    expect(msg(NOW - HOUR, NOW - HOUR)).toBe('to: The end must be after the start.')
    expect(msg(NOW - HOUR, NOW + 61_000)).toBe('to: The end must not be in the future.')
    expect(msg(NOW - 8 * 86_400_000, NOW - 1000)).toBe('from: A range can be at most 7 days long.')
    expect(msg(NOW - 7 * 86_400_000 - 1000, NOW - HOUR)).toBe('from: The start must be within the last 7 days (data retention).')
  })
  it('reads and writes datetime-local values in local time', () => {
    expect(toLocalInput(at(2026, 10, 4, 9, 5))).toBe('2026-10-04T09:05')
    expect(fromLocalInput('2026-10-04T09:05')).toBe(at(2026, 10, 4, 9, 5))
    expect(fromLocalInput('2026-10-04T09:05:30')).toBe(at(2026, 10, 4, 9, 5) + 30_000)
    expect(fromLocalInput('')).toBeNaN()
  })
})
