import { useSearch } from '@tanstack/react-router'
import { useMemo } from 'react'
import { useLiveInterval } from './live'
import { rangeOf } from './range'
import type { Range } from './range'

/** The validated time range from the URL (`since` and `until`), defaulting to the last 1h. */
export function useRange(): Range {
  // From the root match: its search is validated, while location.search keeps raw values.
  const since = useSearch({ from: '__root__', select: (s) => s.since })
  const until = useSearch({ from: '__root__', select: (s) => s.until })
  return useMemo(() => rangeOf({ since, until }), [since, until])
}

/**
 * `refetchInterval` for the pages' queries: the live interval, and off for a custom range (its
 * window has a fixed end, so there is nothing new to fetch).
 */
export function useAutoRefresh(): number | false {
  const past = useRange().until !== undefined
  return useLiveInterval(past)
}
