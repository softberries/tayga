import { TriangleAlert } from 'lucide-react'
import type { ReactNode } from 'react'
import { Button } from '../../components/ui/Button'

const pad = (n: number) => String(n).padStart(2, '0')

/** Local wall-clock time of a unix-ms stamp: "14:03:07". */
export function clockTime(ms: number): string {
  const d = new Date(ms)
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`
}

/** A one-line warning inside a section whose data is partly stale or missing, with an optional retry. */
export function SectionNote({ children, onRetry }: { children: ReactNode; onRetry?: () => void }) {
  return (
    <div role="alert" className="flex flex-wrap items-center gap-x-2.5 gap-y-1.5 rounded-field border border-slow/40 bg-slow-soft px-3 py-1.5 text-xs">
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
export function StaleNote({ updatedAt, onRetry }: { updatedAt: number; onRetry: () => void }) {
  return <SectionNote onRetry={onRetry}>Refresh failed · showing data from {clockTime(updatedAt)}</SectionNote>
}
