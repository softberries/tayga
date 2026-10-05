import { useSyncExternalStore } from 'react'

/**
 * Whether the header's custom-range popover is open. A tiny store rather than component state,
 * so the ⌘K palette's "Custom range…" action can open it from outside the header.
 */
let open = false
const listeners = new Set<() => void>()

export function setCustomRangeOpen(next: boolean): void {
  if (open === next) return
  open = next
  for (const l of listeners) l()
}

function subscribe(cb: () => void): () => void {
  listeners.add(cb)
  return () => listeners.delete(cb)
}

export function useCustomRangeOpen(): boolean {
  return useSyncExternalStore(
    subscribe,
    () => open,
    () => false,
  )
}
