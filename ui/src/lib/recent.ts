/** Recently opened palette items, kept per browser. Storage may be missing or full: never throw. */
export type RecentKind = 'page' | 'service' | 'group' | 'template' | 'trace'

export interface RecentItem {
  kind: RecentKind
  /** Path for pages, otherwise the service name, fingerprint, template id or trace id. */
  id: string
  label: string
  hint?: string
}

export const RECENT_KEY = 'tayga.recent'
export const RECENT_MAX = 10

const KINDS: readonly string[] = ['page', 'service', 'group', 'template', 'trace']

function isItem(v: unknown): v is RecentItem {
  if (typeof v !== 'object' || v === null) return false
  const o = v as Record<string, unknown>
  return (
    typeof o.kind === 'string' &&
    KINDS.includes(o.kind) &&
    typeof o.id === 'string' &&
    typeof o.label === 'string' &&
    (o.hint === undefined || typeof o.hint === 'string')
  )
}

export function readRecent(): RecentItem[] {
  try {
    const raw = window.localStorage.getItem(RECENT_KEY)
    const v: unknown = raw ? JSON.parse(raw) : []
    return Array.isArray(v) ? v.filter(isItem).slice(0, RECENT_MAX) : []
  } catch {
    return []
  }
}

/** Puts `item` first (removing an earlier copy), keeps at most RECENT_MAX, returns the list. */
export function pushRecent(item: RecentItem): RecentItem[] {
  const next = [item, ...readRecent().filter((r) => !(r.kind === item.kind && r.id === item.id))].slice(0, RECENT_MAX)
  try {
    window.localStorage.setItem(RECENT_KEY, JSON.stringify(next))
  } catch {
    // Private mode or quota: the list just does not persist.
  }
  return next
}
