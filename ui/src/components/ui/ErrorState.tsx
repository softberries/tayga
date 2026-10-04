import { TriangleAlert } from 'lucide-react'
import type { ReactNode } from 'react'
import { isApiError } from '../../api/client'
import { cx } from '../../lib/cx'
import { Button } from './Button'

export interface ErrorStateProps {
  error: unknown
  title?: ReactNode
  onRetry?: () => void
  className?: string
}

function describe(error: unknown): { status?: number; message: string } {
  if (isApiError(error)) return { status: error.status, message: error.message }
  if (error instanceof Error) return { message: error.message }
  return { message: 'Something went wrong.' }
}

function defaultTitle(status?: number): string {
  if (status === 503) return 'Storage is unavailable'
  if (status === 404) return 'Not found'
  if (status === 400) return 'The request was not valid'
  if (status === 0) return 'Cannot reach the Tayga API'
  return 'Something went wrong'
}

export function ErrorState({ error, title, onRetry, className }: ErrorStateProps) {
  const { status, message } = describe(error)
  return (
    <div role="alert" className={cx('flex flex-col items-center justify-center gap-2 px-6 py-10 text-center', className)}>
      <div aria-hidden className="mb-1 flex size-10 items-center justify-center rounded-field bg-err-soft text-err">
        <TriangleAlert size={18} />
      </div>
      <p className="m-0 text-[14px] font-medium text-ink">{title ?? defaultTitle(status)}</p>
      <p className="m-0 max-w-[60ch] break-words font-mono text-xs text-muted">
        {status ? `${status} · ` : ''}
        {message}
      </p>
      {onRetry ? (
        <Button size="sm" className="mt-2" onClick={onRetry}>
          Try again
        </Button>
      ) : null}
    </div>
  )
}
