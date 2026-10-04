import { describe, expect, it } from 'vitest'
import { compact, shortId } from './format'

describe('format', () => {
  it('compacts numbers', () => {
    expect(compact(0)).toBe('0')
    expect(compact(7)).toBe('7')
    expect(compact(1.25)).toBe('1.3')
    expect(compact(950)).toBe('950')
    expect(compact(1900)).toBe('1.9k')
    expect(compact(2000)).toBe('2k')
    expect(compact(1_250_000)).toBe('1.3M')
  })
  it('shortens ids', () => {
    expect(shortId('9b3f0000000000000000000000c21e')).toBe('9b3f…c21e')
    expect(shortId('abc')).toBe('abc')
  })
})
