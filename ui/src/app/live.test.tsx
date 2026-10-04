import { act, renderHook } from '@testing-library/react'
import type { ReactNode } from 'react'
import { afterEach, describe, expect, it } from 'vitest'
import { LIVE_INTERVAL_MS, LiveProvider, useLive, useLiveInterval } from './live'

const wrapper = ({ children }: { children: ReactNode }) => <LiveProvider>{children}</LiveProvider>

function setHidden(hidden: boolean) {
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => (hidden ? 'hidden' : 'visible') })
  document.dispatchEvent(new Event('visibilitychange'))
}

afterEach(() => setHidden(false))

describe('live mode', () => {
  it('refetches every 10 s by default and stops when turned off', () => {
    const { result } = renderHook(() => ({ interval: useLiveInterval(), live: useLive() }), { wrapper })
    expect(result.current.interval).toBe(LIVE_INTERVAL_MS)
    act(() => result.current.live.setLive(false))
    expect(result.current.interval).toBe(false)
    expect(window.localStorage.getItem('tayga-live')).toBe('off')
  })

  it('pauses while the document is hidden', () => {
    const { result } = renderHook(() => useLiveInterval(), { wrapper })
    act(() => setHidden(true))
    expect(result.current).toBe(false)
    act(() => setHidden(false))
    expect(result.current).toBe(LIVE_INTERVAL_MS)
  })

  it('restores off from storage', () => {
    window.localStorage.setItem('tayga-live', 'off')
    const { result } = renderHook(() => useLiveInterval(), { wrapper })
    expect(result.current).toBe(false)
  })
})
