import { createContext, useCallback, useContext, useLayoutEffect, useMemo, useState, useSyncExternalStore } from 'react'
import type { ReactNode } from 'react'
import {
  DARK_QUERY,
  NEXT_MODE,
  applyTheme,
  readMode,
  resolveTheme,
  systemPrefersDark,
  writeMode,
} from './theme'
import type { ResolvedTheme, ThemeMode } from './theme'

interface ThemeContextValue {
  mode: ThemeMode
  resolved: ResolvedTheme
  setMode: (mode: ThemeMode) => void
  /** light → dark → system → light */
  cycle: () => void
}

const ThemeContext = createContext<ThemeContextValue | null>(null)

/** Cross-fade length plus a little slack; matches the 200 ms transition in tokens.css. */
const SWITCH_MS = 250

function subscribeSystem(onChange: () => void): () => void {
  if (typeof window.matchMedia !== 'function') return () => {}
  const mql = window.matchMedia(DARK_QUERY)
  mql.addEventListener('change', onChange)
  return () => mql.removeEventListener('change', onChange)
}

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [mode, setModeState] = useState<ThemeMode>(readMode)
  const prefersDark = useSyncExternalStore(subscribeSystem, systemPrefersDark, () => false)
  const resolved = resolveTheme(mode, prefersDark)

  // The inline script in index.html already applied the first theme, so a change here is a
  // switch and cross-fades. Without a prior theme (tests) it applies without the fade.
  useLayoutEffect(() => {
    const root = document.documentElement
    if (root.dataset.theme === resolved) return
    const switching = root.dataset.theme !== undefined
    applyTheme(resolved, root)
    if (!switching) return
    root.classList.add('tg-theme-switching')
    const t = window.setTimeout(() => root.classList.remove('tg-theme-switching'), SWITCH_MS)
    return () => {
      window.clearTimeout(t)
      root.classList.remove('tg-theme-switching')
    }
  }, [resolved])

  const setMode = useCallback((m: ThemeMode) => {
    writeMode(m)
    setModeState(m)
  }, [])
  const cycle = useCallback(() => {
    setModeState((m) => {
      const next = NEXT_MODE[m]
      writeMode(next)
      return next
    })
  }, [])

  const value = useMemo(() => ({ mode, resolved, setMode, cycle }), [mode, resolved, setMode, cycle])
  return <ThemeContext value={value}>{children}</ThemeContext>
}

export function useTheme(): ThemeContextValue {
  const ctx = useContext(ThemeContext)
  if (!ctx) throw new Error('useTheme must be used inside <ThemeProvider>')
  return ctx
}
