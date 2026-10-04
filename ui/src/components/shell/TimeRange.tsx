import { useNavigate, useSearch } from '@tanstack/react-router'
import { ToggleGroup } from '../ui/ToggleGroup'
import { DEFAULT_SINCE, SINCE_VALUES } from '../../app/search'
import type { Since } from '../../app/search'

const options = SINCE_VALUES.map((v) => ({ value: v, label: v }))

/** The validated time range from the URL (`since`), defaulting to 1h. */
export function useSince(): Since {
  // From the root match: its search is validated, while location.search keeps raw values.
  return useSearch({ from: '__root__', select: (s) => s.since ?? DEFAULT_SINCE })
}

export function TimeRange() {
  const since = useSince()
  const navigate = useNavigate()
  return (
    <ToggleGroup
      label="Time range"
      options={options}
      value={since}
      onValueChange={(v) =>
        void navigate({
          to: '.',
          search: (prev) => ({ ...prev, since: v === DEFAULT_SINCE ? undefined : v }),
          replace: true,
        })
      }
    />
  )
}
