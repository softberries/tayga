import type { ConsumerLag } from '../../api/types'
import { Reveal } from '../../components/ui/Reveal'
import { Skeleton } from '../../components/ui/Skeleton'
import { compact } from '../../lib/format'

/** Bars are scaled to at least this many messages, so a trickle of lag does not fill the track. */
export const LAG_SCALE_FLOOR = 1000
/** The topic the logminer publishes alerts to. Its records are alerts, tens per hour, not batches. */
export const ALERTS_TOPIC = 'tayga.alerts'
/** The scale floor on `ALERTS_TOPIC`: tens of alerts held back by a stuck notifier fill the bar. */
export const ALERTS_LAG_SCALE_FLOOR = 10

/**
 * The bar scale of each topic: its largest lag, at least its floor. Groups on different topics
 * count different records, so one topic's lag never shrinks another's bars.
 */
export function lagScales(groups: readonly ConsumerLag[]): Map<string, number> {
  const scales = new Map<string, number>()
  for (const g of groups) {
    const floor = g.topic === ALERTS_TOPIC ? ALERTS_LAG_SCALE_FLOOR : LAG_SCALE_FLOOR
    scales.set(g.topic, Math.max(scales.get(g.topic) ?? floor, g.lag))
  }
  return scales
}

/** Consumer lag per group, with its topic, as bars scaled per topic (see `lagScales`); zero lag shows an empty track. */
export function LagList({ groups }: { groups: readonly ConsumerLag[] | undefined }) {
  if (!groups) {
    return (
      <div role="status" aria-busy="true" aria-label="Loading consumer lag" className="flex flex-col gap-3">
        {Array.from({ length: 3 }, (_, i) => (
          <Skeleton key={i} className="h-7" />
        ))}
      </div>
    )
  }
  if (groups.length === 0) return <p className="m-0 text-muted">No consumer groups.</p>
  const scales = lagScales(groups)
  return (
    <Reveal>
      <ul aria-label="Consumer lag" className="m-0 flex list-none flex-col gap-3 p-0">
        {groups.map((g) => (
          <li
            key={`${g.topic}/${g.group}`}
            className="grid grid-cols-[minmax(14rem,17rem)_1fr_auto] items-center gap-x-3 gap-y-1 max-sm:grid-cols-[1fr_auto]"
          >
            <div className="min-w-0">
              <div className="truncate font-mono text-[13px] text-ink">{g.group}</div>
              <div className="text-[11px] tabular-nums text-muted">
                <span className="font-mono">{g.topic}</span> · committed {g.committed.toLocaleString('en-US')} · end {g.end.toLocaleString('en-US')}
              </div>
            </div>
            <div aria-hidden className="h-2.5 overflow-hidden rounded-full bg-inner max-sm:order-last max-sm:col-span-2">
              <div className="h-full rounded-full bg-accent" style={{ width: `${(g.lag / (scales.get(g.topic) ?? LAG_SCALE_FLOOR)) * 100}%`, minWidth: g.lag > 0 ? 4 : 0 }} />
            </div>
            <span className="text-right font-mono text-[13px] tabular-nums text-ink">
              {compact(g.lag)} <span className="text-xs text-muted">messages behind</span>
            </span>
          </li>
        ))}
      </ul>
    </Reveal>
  )
}
