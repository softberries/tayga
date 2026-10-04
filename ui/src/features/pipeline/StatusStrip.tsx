import { Badge } from '../../components/ui/Badge'
import type { BadgeKind } from '../../components/ui/Badge'
import { Skeleton } from '../../components/ui/Skeleton'
import { scrapeAge, shortJob } from './model'
import type { JobState, JobStatus } from './model'

const KIND: Record<JobState, BadgeKind> = { up: 'ok', down: 'error', unknown: 'neutral' }
const LABEL: Record<JobState, string> = { up: 'up', down: 'down', unknown: 'no data' }

/** One chip per scrape job: up or down from the latest `up` sample, a glow when down. */
export function StatusStrip({ statuses, nowMs }: { statuses: readonly JobStatus[] | undefined; nowMs: number }) {
  if (!statuses) {
    return (
      <div aria-busy="true" aria-label="Loading job status" className="flex flex-wrap gap-3">
        {Array.from({ length: 5 }, (_, i) => (
          <Skeleton key={i} className="h-[58px] w-[176px] rounded-field" />
        ))}
      </div>
    )
  }
  return (
    <ul aria-label="Job status" className="m-0 flex list-none flex-wrap gap-3 p-0">
      {statuses.map((s) => (
        <li
          key={s.job}
          data-state={s.state}
          className="flex min-w-[10.5rem] flex-1 flex-col gap-1.5 rounded-field border border-panel-line bg-panel px-3.5 py-2.5 shadow-panel"
        >
          <div className="flex items-center justify-between gap-2">
            <span className="font-mono text-[13px] font-medium text-ink">{shortJob(s.job)}</span>
            <Badge kind={KIND[s.state]} shape="pill" glow={s.state === 'down'} pulse={s.state === 'down'}>
              {LABEL[s.state]}
            </Badge>
          </div>
          <span className="text-xs text-muted">{scrapeAge(s, nowMs)}</span>
        </li>
      ))}
    </ul>
  )
}
