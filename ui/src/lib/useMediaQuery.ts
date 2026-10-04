import { useCallback, useSyncExternalStore } from 'react'

/** Live `matchMedia(query).matches`; false where matchMedia is unavailable. */
export function useMediaQuery(query: string): boolean {
  const subscribe = useCallback(
    (cb: () => void) => {
      if (typeof window.matchMedia !== 'function') return () => {}
      const mql = window.matchMedia(query)
      mql.addEventListener('change', cb)
      return () => mql.removeEventListener('change', cb)
    },
    [query],
  )
  return useSyncExternalStore(
    subscribe,
    () => typeof window.matchMedia === 'function' && window.matchMedia(query).matches,
    () => false,
  )
}

/** Below Tailwind's `sm` breakpoint (640 px). */
export const NARROW_QUERY = '(max-width: 639.98px)'
export const REDUCED_MOTION_QUERY = '(prefers-reduced-motion: reduce)'
