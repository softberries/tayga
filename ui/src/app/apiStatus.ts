/**
 * Global "storage unavailable" state: set when any query fails with 503, cleared by the next
 * successful query. The shell shows a banner while it is set.
 */
import { useSyncExternalStore } from 'react'

export interface ApiOutage {
  message: string
  at: number
}

let outage: ApiOutage | null = null
const listeners = new Set<() => void>()

function emit() {
  for (const l of listeners) l()
}

export function reportOutage(message: string): void {
  outage = { message, at: Date.now() }
  emit()
}

export function clearOutage(): void {
  if (outage === null) return
  outage = null
  emit()
}

export function getOutage(): ApiOutage | null {
  return outage
}

function subscribe(cb: () => void): () => void {
  listeners.add(cb)
  return () => listeners.delete(cb)
}

export function useOutage(): ApiOutage | null {
  return useSyncExternalStore(subscribe, getOutage, getOutage)
}
