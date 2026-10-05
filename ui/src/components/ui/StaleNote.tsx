/**
 * The app's one rule for a query that failed: with no data yet, the section shows its error
 * panel (`loadFailed`); with data from an earlier fetch, it keeps that data and shows
 * `StaleNote` ("Refresh failed · showing data from 14:03:07") with a retry.
 */
import { TriangleAlert } from 'lucide-react'
import type { ReactNode } from 'react'
import { cx } from '../../lib/cx'
import { Button } from './Button'

const pad = (n: number) => String(n).padStart(2, '0')

/** Local wall-clock time of a unix-ms stamp: "14:03:07". */
export function clockTime(ms: number): string {
  const d = new Date(ms)
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`
}

/** The parts of a query result the rule reads. */
export interface QueryLike {
  isError: boolean
  data: unknown
  dataUpdatedAt: number
  refetch: () => unknown
}

/** Failed with nothing to show: the section shows its error panel. */
export function loadFailed(q: Pick<QueryLike, 'isError' | 'data'>): boolean {
  return q.isError && q.data === undefined
}

/** A refresh failed but earlier data is kept: the section shows it with a `StaleNote`. */
export function isStale(q: Pick<QueryLike, 'isError' | 'data'>): boolean {
  return q.isError && q.data !== undefined
}

/** A one-line warning inside a section whose data is partly stale or missing, with an optional retry. */
export function SectionNote({ children, onRetry, className }: { children: ReactNode; onRetry?: () => void; className?: string }) {
  return (
    <div
      role="alert"
      className={cx(
        'flex flex-wrap items-center gap-x-2.5 gap-y-1.5 rounded-field border border-slow/40 bg-slow-soft px-3 py-1.5 text-xs',
        className,
      )}
    >
      <TriangleAlert aria-hidden size={14} className="shrink-0 text-slow" />
      <span className="min-w-0 flex-1 text-ink">{children}</span>
      {onRetry ? (
        <Button size="sm" onClick={onRetry}>
          Try again
        </Button>
      ) : null}
    </div>
  )
}

/** "Refresh failed · showing data from 14:03:07": the section kept its last good data. */
export function StaleNote({ updatedAt, onRetry, className }: { updatedAt: number; onRetry: () => void; className?: string }) {
  return (
    <SectionNote onRetry={onRetry} className={className}>
      Refresh failed · showing data from {clockTime(updatedAt)}
    </SectionNote>
  )
}

/**
 * The `StaleNote` for a section fed by `queries`: shown while any of them is stale, dated by
 * the oldest kept data; retry refetches the stale ones. Renders nothing otherwise.
 */
export function RefreshNote({ queries, className }: { queries: readonly QueryLike[]; className?: string }) {
  const stale = queries.filter(isStale)
  if (stale.length === 0) return null
  return (
    <StaleNote
      updatedAt={Math.min(...stale.map((q) => q.dataUpdatedAt))}
      onRetry={() => stale.forEach((q) => void q.refetch())}
      className={className}
    />
  )
}
