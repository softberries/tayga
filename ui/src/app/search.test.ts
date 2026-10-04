import { describe, expect, it } from 'vitest'
import { HEX32, U64, sinceCovering, validateMapSearch, validateRootSearch, validateStorySearch, validateTraceSearch } from './search'

describe('validateRootSearch', () => {
  it('keeps a valid non-default since', () => {
    expect(validateRootSearch({ since: '15m' })).toEqual({ since: '15m' })
    expect(validateRootSearch({ since: '7d' })).toEqual({ since: '7d' })
  })
  it('clears the default and invalid values', () => {
    // Explicit undefined, so the router's merge overrides the raw value.
    expect(validateRootSearch({ since: '1h' })).toStrictEqual({ since: undefined })
    expect(validateRootSearch({ since: '99y' })).toStrictEqual({ since: undefined })
    expect(validateRootSearch({ since: 5 })).toStrictEqual({ since: undefined })
    expect(validateRootSearch({ other: 'x' })).toStrictEqual({ since: undefined })
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
    expect(validateMapSearch({})).toStrictEqual({ service: undefined })
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
    expect(sinceCovering(ago(60), now, '24h')).toBe('24h')
    expect(sinceCovering(ago(5 * 3600), now, '1h')).toBe('24h')
  })
})
