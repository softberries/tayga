import { describe, expect, it } from 'vitest'
import { safeNext } from './auth'

describe('safeNext', () => {
  it.each(['/', '/map?since=1h', '/traces/abc', '/logs/templates?q=payment%20declined'])('keeps same-origin %s', (p) =>
    expect(safeNext(p)).toBe(p),
  )
  it.each([
    '//evil.com',
    '/\\evil.com',
    'https://evil.com',
    'javascript:alert(1)',
    '',
    42,
    undefined,
    null,
    '/%2F%2Fevil.com',
    '/%5Cevil.com',
    '/\t/evil.com',
    '/\n/evil.com',
    'evil.com',
    '/%E0%A4%A',
  ])('rejects %s', (p) => expect(safeNext(p)).toBe('/'))
})
