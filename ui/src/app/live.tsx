import { createContext, useCallback, useContext, useMemo, useState, useSyncExternalStore } from 'react'
import type { ReactNode } from 'react'

/** Live mode refetch interval (spec §10). */
export const LIVE_INTERVAL_MS = 10_000
const LIVE_KEY = 'tayga-live'

interface LiveContextValue {
  live: boolean
  setLive: (live: boolean) => void
}

const LiveContext = createContext<LiveContextValue | null>(null)

function readLive(): boolean {
  try {
    return window.localStorage.getItem(LIVE_KEY) !== 'off'
  } catch {
    return true
  }
}

export function LiveProvider({ children }: { children: ReactNode }) {
  const [live, setLiveState] = useState(readLive)
  const setLive = useCallback((v: boolean) => {
    try {
      window.localStorage.setItem(LIVE_KEY, v ? 'on' : 'off')
    } catch {
      // Storage blocked: the choice lasts for this page only.
    }
    setLiveState(v)
  }, [])
  const value = useMemo(() => ({ live, setLive }), [live, setLive])
  return <LiveContext value={value}>{children}</LiveContext>
}

export function useLive(): LiveContextValue {
  const ctx = useContext(LiveContext)
  if (!ctx) throw new Error('useLive must be used inside <LiveProvider>')
  return ctx
}

function subscribeVisibility(cb: () => void): () => void {
  document.addEventListener('visibilitychange', cb)
  return () => document.removeEventListener('visibilitychange', cb)
}

export function useDocumentVisible(): boolean {
  return useSyncExternalStore(
    subscribeVisibility,
    () => document.visibilityState !== 'hidden',
    () => true,
  )
}

/** `refetchInterval` for live queries: 10 s while live and the tab is visible, else off. */
export function useLiveInterval(): number | false {
  const { live } = useLive()
  const visible = useDocumentVisible()
  return live && visible ? LIVE_INTERVAL_MS : false
}
