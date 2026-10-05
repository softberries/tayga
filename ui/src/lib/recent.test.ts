import { describe, expect, it } from 'vitest'
import { RECENT_KEY, RECENT_MAX, pushRecent, readRecent } from './recent'

describe('recent', () => {
  it('puts the newest first and drops duplicates', () => {
    pushRecent({ kind: 'service', id: 'a', label: 'a' })
    pushRecent({ kind: 'service', id: 'b', label: 'b' })
    pushRecent({ kind: 'service', id: 'a', label: 'a' })
    expect(readRecent().map((r) => r.id)).toEqual(['a', 'b'])
  })

  it('keeps at most ten', () => {
    for (let i = 0; i < 15; i++) pushRecent({ kind: 'template', id: String(i), label: `t${i}` })
    const r = readRecent()
    expect(r).toHaveLength(RECENT_MAX)
    expect(r[0]?.id).toBe('14')
  })

  it('ignores corrupt storage', () => {
    window.localStorage.setItem(RECENT_KEY, '{nope')
    expect(readRecent()).toEqual([])
    window.localStorage.setItem(RECENT_KEY, JSON.stringify([{ kind: 'x', id: 1 }, { kind: 'trace', id: 't', label: 'T' }]))
    expect(readRecent()).toEqual([{ kind: 'trace', id: 't', label: 'T' }])
  })
})
