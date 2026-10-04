/** URL search params shared by every route. */

export const SINCE_VALUES = ['15m', '1h', '24h', '7d'] as const
export type Since = (typeof SINCE_VALUES)[number]
export const DEFAULT_SINCE: Since = '1h'

export interface RootSearch {
  /** Time range; absent means DEFAULT_SINCE, so the default keeps URLs clean. */
  since?: Since
}

function isSince(v: unknown): v is Since {
  return typeof v === 'string' && (SINCE_VALUES as readonly string[]).includes(v)
}

/**
 * Router `validateSearch` for the root route. The router merges the result over the raw
 * search, so an invalid or default value is overridden with an explicit `undefined`.
 */
export function validateRootSearch(search: Record<string, unknown>): RootSearch {
  return { since: isSince(search.since) && search.since !== DEFAULT_SINCE ? search.since : undefined }
}

/** 32 lowercase-or-uppercase hex characters (trace and story ids). */
export const HEX32 = /^[0-9a-fA-F]{32}$/
/** A u64 as decimal digits (template ids, fingerprints). */
export const U64 = /^[0-9]{1,20}$/

/**
 * Search for a cross-section link: only the time range travels, so one page's filters never
 * leak into another section. The default range stays out of the URL.
 */
export function sinceSearch(since: Since): RootSearch {
  return { since: since === DEFAULT_SINCE ? undefined : since }
}
