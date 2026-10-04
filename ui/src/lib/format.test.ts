import { describe, expect, it } from 'vitest'
import { clockMs, compact, dateTime, duration, msValue, percent, shortId } from './format'
import { SERVICE_COLORS, serviceColor } from './serviceColor'

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
  it('formats durations', () => {
    expect(duration(210_000)).toBe('0.21 ms')
    expect(duration(12_000)).toBe('0.012 ms')
    expect(duration(2_900_000)).toBe('2.90 ms')
    expect(duration(65_100_000)).toBe('65.1 ms')
    expect(duration(5_120_000_000)).toBe('5.12 s')
    expect(duration(192e9)).toBe('3.2 min')
    expect(duration(Number.NaN)).toBe('—')
    expect(duration(0)).toBe('0 ms')
    expect(duration(2 * 3600e9)).toBe('2.0 h')
    expect(duration(3 * 86400e9)).toBe('3.0 d')
    expect(msValue(210_000)).toBe('0.21')
    expect(msValue(65_100_000)).toBe('65.1')
    expect(msValue(5_120_000_000)).toBe('5120')
    expect(msValue(600e9)).toBe('10.0 min')
  })
  it('formats times and percents', () => {
    expect(clockMs(1.791114992498e18)).toMatch(/^\d\d:\d\d:\d\d\.\d{3}$/)
    expect(dateTime(1.791114992498e18)).toMatch(/^2026-\d\d-\d\d \d\d:\d\d:\d\d$/)
    expect(percent(0.004)).toBe('0.4 %')
    expect(percent(0.37)).toBe('37 %')
  })
  it('hashes services to a stable palette color', () => {
    expect(serviceColor('checkout')).toBe(serviceColor('checkout'))
    expect(serviceColor('checkout')).toMatch(new RegExp(`^var\\(--tg-svc-[1-${SERVICE_COLORS}]\\)$`))
    const used = new Set(['frontend', 'checkout', 'payment', 'cart', 'ad', 'quote', 'email', 'shipping'].map(serviceColor))
    expect(used.size).toBeGreaterThan(3)
  })
})
