import { TriangleAlert } from 'lucide-react'
import { isApiError } from '../../api/client'
import { Button } from '../../components/ui/Button'
import { EXPIRED_TITLE } from '../../components/ui/ErrorState'
import { ExpiredRangeReset, isExpiredRange } from '../../components/ui/ExpiredRangeReset'

/** One-line failure notice with a retry, for a section that could not load. */
export function ErrorBanner({ what, error, onRetry }: { what: string; error: unknown; onRetry: () => void }) {
  const detail = isApiError(error)
    ? `${error.status ? `${error.status} · ` : ''}${error.message}`
    : error instanceof Error
      ? error.message
      : 'Something went wrong.'
  return (
    <div role="alert" className="flex flex-wrap items-center gap-x-3 gap-y-2 rounded-field border border-err/40 bg-err-soft px-4 py-2.5">
      <TriangleAlert aria-hidden size={16} className="shrink-0 text-err" />
      <span className="font-medium text-ink">{isExpiredRange(error) ? `${EXPIRED_TITLE}.` : `Could not load ${what}.`}</span>
      <span className="min-w-0 flex-1 truncate font-mono text-xs text-muted" title={detail}>
        {detail}
      </span>
      <ExpiredRangeReset error={error} />
      <Button size="sm" onClick={onRetry}>
        Try again
      </Button>
    </div>
  )
}
