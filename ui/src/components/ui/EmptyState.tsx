import { Inbox } from 'lucide-react'
import type { ReactNode } from 'react'
import { cx } from '../../lib/cx'

export interface EmptyStateProps {
  title: ReactNode
  description?: ReactNode
  icon?: ReactNode
  action?: ReactNode
  className?: string
}

export function EmptyState({ title, description, icon, action, className }: EmptyStateProps) {
  return (
    <div className={cx('flex flex-col items-center justify-center gap-2 px-6 py-10 text-center', className)}>
      <div aria-hidden className="mb-1 flex size-10 items-center justify-center rounded-field bg-inner text-muted">
        {icon ?? <Inbox size={18} />}
      </div>
      <p className="m-0 text-[14px] font-medium text-ink">{title}</p>
      {description ? <p className="m-0 max-w-[48ch] text-muted">{description}</p> : null}
      {action ? <div className="mt-2">{action}</div> : null}
    </div>
  )
}
