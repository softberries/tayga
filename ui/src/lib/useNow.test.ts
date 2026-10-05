import { act, renderHook } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { useNow } from './useNow'

afterEach(() => vi.useRealTimers())

describe('useNow', () => {
  it('advances on its own clock', () => {
    vi.useFakeTimers({ now: 1_000_000 })
    const { result } = renderHook(() => useNow(15_000))
    expect(result.current).toBe(1_000_000)
    act(() => void vi.advanceTimersByTime(15_000))
    expect(result.current).toBe(1_015_000)
  })
})
