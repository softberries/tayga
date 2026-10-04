import type { ConsumerLag } from '../../api/types'
import { Skeleton } from '../../components/ui/Skeleton'
import { compact } from '../../lib/format'

/** Consumer lag per group as bars scaled to the largest lag; zero lag shows an empty track. */
export function LagList({ groups }: { groups: readonly ConsumerLag[] | undefined }) {
  if (!groups) {
    return (
      <div aria-busy="true" aria-label="Loading consumer lag" className="flex flex-col gap-3">
        {Array.from({ length: 3 }, (_, i) => (
          <Skeleton key={i} className="h-7" />
        ))}
      </div>
    )
  }
  if (groups.length === 0) return <p className="m-0 text-muted">No consumer groups.</p>
  const max = Math.max(...groups.map((g) => g.lag), 1)
  return (
    <ul aria-label="Consumer lag" className="m-0 flex list-none flex-col gap-3 p-0">
      {groups.map((g) => (
        <li
          key={g.group}
          className="grid grid-cols-[minmax(7rem,10rem)_1fr_auto] items-center gap-x-3 gap-y-1 max-sm:grid-cols-[1fr_auto]"
          title={`committed ${g.committed.toLocaleString('en-US')} of ${g.end.toLocaleString('en-US')}`}
        >
          <span className="truncate font-mono text-[13px] text-ink">{g.group}</span>
          <div aria-hidden className="h-2.5 overflow-hidden rounded-full bg-inner max-sm:order-last max-sm:col-span-2">
            <div className="h-full rounded-full bg-accent" style={{ width: `${(g.lag / max) * 100}%`, minWidth: g.lag > 0 ? 4 : 0 }} />
          </div>
          <span className="text-right font-mono text-[13px] tabular-nums text-ink">
            {compact(g.lag)} <span className="text-xs text-muted">messages behind</span>
          </span>
        </li>
      ))}
    </ul>
  )
}
