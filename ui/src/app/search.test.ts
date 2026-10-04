import { describe, expect, it } from 'vitest'
import { HEX32, U64, validateRootSearch } from './search'

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
