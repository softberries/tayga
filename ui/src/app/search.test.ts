import { describe, expect, it } from 'vitest'
import { HEX32, U64, formatRect, formatUntil, sinceSecs, untilMs, parseRect, sinceCovering, validateHomeSearch, validateMapSearch, validateRootSearch, validateStorySearch, validateTraceSearch, validateTracesSearch } from './search'

describe('validateRootSearch', () => {
  it('keeps a valid non-default since', () => {
    expect(validateRootSearch({ since: '15m' })).toEqual({ since: '15m' })
    expect(validateRootSearch({ since: '7d' })).toEqual({ since: '7d' })
  })
  it('clears the default and invalid values', () => {
    // Explicit undefined, so the router's merge overrides the raw value.
    const none = { since: undefined, until: undefined }
    expect(validateRootSearch({ since: '1h' })).toStrictEqual(none)
    expect(validateRootSearch({ since: '99y' })).toStrictEqual(none)
    expect(validateRootSearch({ since: 5 })).toStrictEqual(none)
    expect(validateRootSearch({ other: 'x' })).toStrictEqual(none)
    // Without until, only a preset is a range.
    expect(validateRootSearch({ since: '2h' })).toStrictEqual(none)
  })
  it('keeps a custom range: any API since with a valid until, normalized to UTC', () => {
    expect(validateRootSearch({ since: '2h', until: '2026-10-04T14:00:00Z' })).toStrictEqual({ since: '2h', until: '2026-10-04T14:00:00Z' })
    expect(validateRootSearch({ since: '7201s', until: '2026-10-04T16:00:00.250+02:00' })).toStrictEqual({
      since: '7201s',
      until: '2026-10-04T14:00:00Z',
    })
    // The router parses `?until=1791115200` as a number: unix seconds.
    expect(validateRootSearch({ until: 1_791_115_200 })).toStrictEqual({ since: undefined, until: '2026-10-04T12:00:00Z' })
    expect(validateRootSearch({ since: '1h', until: '2026-10-04T14:00:00Z' })).toStrictEqual({ since: undefined, until: '2026-10-04T14:00:00Z' })
  })
  it('drops a custom range with a bad since or until', () => {
    const none = { since: undefined, until: undefined }
    expect(validateRootSearch({ since: '8d', until: '2026-10-04T14:00:00Z' })).toStrictEqual(none)
    expect(validateRootSearch({ since: '0s', until: '2026-10-04T14:00:00Z' })).toStrictEqual(none)
    expect(validateRootSearch({ since: '15m', until: 'yesterday' })).toStrictEqual({ since: '15m', until: undefined })
    expect(validateRootSearch({ since: '15m', until: '2026-10-04 14:00' })).toStrictEqual({ since: '15m', until: undefined })
    expect(validateRootSearch({ until: -5 })).toStrictEqual(none)
    expect(validateRootSearch({ until: 1.5 })).toStrictEqual(none)
  })
  it('reads API durations', () => {
    expect([sinceSecs('30s'), sinceSecs('90m'), sinceSecs('2h'), sinceSecs('7d')]).toEqual([30, 5400, 7200, 604_800])
    expect([sinceSecs('8d'), sinceSecs('0m'), sinceSecs('1w'), sinceSecs('h'), sinceSecs(5)]).toEqual([undefined, undefined, undefined, undefined, undefined])
    expect(untilMs('2026-10-04T12:00:00Z')).toBe(Date.UTC(2026, 9, 4, 12))
    expect(formatUntil(Date.UTC(2026, 9, 4, 12, 0, 0, 999))).toBe('2026-10-04T12:00:00Z')
  })
  it('id patterns', () => {
    expect(HEX32.test('334c8a31ddeaa3304a4e4e7f219bebbc')).toBe(true)
    expect(HEX32.test('334c8a31')).toBe(false)
    expect(U64.test('11039615203255878215')).toBe(true)
    expect(U64.test('12a')).toBe(false)
  })
})

describe('page search params', () => {
  it('trace: span id, search text and filter', () => {
    expect(validateTraceSearch({ span: '910F3FCFB9A67DDC', q: 'pay', only: 'errors' })).toEqual({
      span: '910f3fcfb9a67ddc',
      q: 'pay',
      only: 'errors',
    })
    expect(validateTraceSearch({ span: 'nope', q: '', only: 'all' })).toStrictEqual({
      span: undefined,
      q: undefined,
      only: undefined,
    })
    expect(validateTraceSearch({ q: 42 }).q).toBe('42')
    expect(validateTraceSearch({ q: 'x'.repeat(500) }).q).toHaveLength(200)
  })
  it('story: adds log service and severity', () => {
    expect(validateStorySearch({ log_service: 'payment', sev: 'warn', only: 'critical' })).toMatchObject({
      log_service: 'payment',
      sev: 'warn',
      only: 'critical',
    })
    expect(validateStorySearch({ sev: 'loud' }).sev).toBeUndefined()
    expect(validateStorySearch({ sev: 'toString' }).sev).toBeUndefined()
  })
  it('map: service', () => {
    expect(validateMapSearch({ service: 'checkout' })).toEqual({ service: 'checkout' })
    expect(validateMapSearch({})).toStrictEqual({ service: undefined, q: undefined })
    expect(validateMapSearch({ q: 'pay', service: '' })).toStrictEqual({ service: undefined, q: 'pay' })
  })
})

describe('sinceCovering', () => {
  const now = 1_791_000_000_000
  const ago = (secs: number) => (now - secs * 1000) * 1e6
  it('picks the smallest range that still contains the moment', () => {
    expect(sinceCovering(ago(60), now)).toBe('15m')
    expect(sinceCovering(ago(1800), now)).toBe('1h')
    expect(sinceCovering(ago(5 * 3600), now)).toBe('24h')
    expect(sinceCovering(ago(3 * 86_400), now)).toBe('7d')
    expect(sinceCovering(ago(30 * 86_400), now)).toBe('7d')
  })
  it('never goes below the requested range', () => {
    expect(sinceCovering(ago(60), now, 86_400)).toBe('24h')
    expect(sinceCovering(ago(5 * 3600), now, 3600)).toBe('24h')
  })
})

describe('validateHomeSearch', () => {
  it('keeps valid filters and the selected group', () => {
    expect(
      validateHomeSearch({ kind: 'slow', service: 'payment', endpoint: 'frontend GET /api/data', q: 'charge', group: '3637772083875833685' }),
    ).toEqual({ kind: 'slow', service: 'payment', endpoint: 'frontend GET /api/data', q: 'charge', group: '3637772083875833685' })
  })
  it('drops invalid values', () => {
    expect(validateHomeSearch({ kind: 'warn', service: '', q: 7, group: 'abc' })).toEqual({
      kind: undefined,
      service: undefined,
      endpoint: undefined,
      q: '7',
      group: undefined,
    })
    expect(validateHomeSearch({ group: 42 }).group).toBe('42')
    expect(validateHomeSearch({ group: 2 ** 60 }).group).toBeUndefined()
  })
})

describe('traces explorer search', () => {
  it('keeps valid filters and parses router numbers and flags', () => {
    expect(
      validateTracesSearch({ service: 'payment', endpoint: 'POST /api/checkout', min_ms: 5, max_ms: '100', errors: true, log: 1, sel: '1000_2000_40_60' }),
    ).toEqual({ service: 'payment', endpoint: 'POST /api/checkout', min_ms: 5, max_ms: 100, errors: true, log: true, sel: '1000_2000_40_60' })
  })
  it('drops invalid values: fractional or negative ms, max below min, bad flags and rects', () => {
    expect(validateTracesSearch({ min_ms: 1.5, max_ms: -3, errors: 'maybe', log: 0, sel: 'a_b_c_d' })).toStrictEqual({
      service: undefined,
      touched: undefined,
      endpoint: undefined,
      min_ms: undefined,
      max_ms: undefined,
      errors: undefined,
      log: undefined,
      sel: undefined,
    })
    expect(validateTracesSearch({ min_ms: 100, max_ms: 5 })).toMatchObject({ min_ms: 100, max_ms: undefined })
  })
  it('keeps touched only with a service', () => {
    expect(validateTracesSearch({ service: 'payment', touched: 1 }).touched).toBe(true)
    expect(validateTracesSearch({ service: 'payment', touched: 'true' }).touched).toBe(true)
    expect(validateTracesSearch({ service: 'payment', touched: 'yes' }).touched).toBeUndefined()
    expect(validateTracesSearch({ touched: 1 }).touched).toBeUndefined()
  })
  it('a rect round-trips through the URL, normalized to min before max', () => {
    expect(parseRect('2000_1000_60_40')).toEqual({ t0: 1000, t1: 2000, d0: 40, d1: 60 })
    expect(formatRect({ t0: 1000.4, t1: 1999.2, d0: 40.12345, d1: 60 })).toBe('1000_2000_40.123_60')
    expect(validateTracesSearch({ sel: '2000_1000_60_40' }).sel).toBe('1000_2000_40_60')
    expect(parseRect('1_2_3')).toBeUndefined()
  })
})
