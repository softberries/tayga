import { ArrowDown, ArrowUp } from 'lucide-react'
import type { ReactNode } from 'react'
import { cx } from '../../lib/cx'

export type SortDirection = 'ascending' | 'descending' | 'none'

/** A sortable column header: a button with the direction arrow, and `aria-sort` on the cell. */
export function SortHeader({
  dir,
  onSort,
  className,
  children,
}: {
  dir: SortDirection
  onSort: () => void
  className?: string
  children: ReactNode
}) {
  return (
    <div role="columnheader" aria-sort={dir} className={className}>
      <button
        type="button"
        onClick={onSort}
        className={cx(
          'inline-flex cursor-pointer items-center gap-1 rounded-badge text-[11px] uppercase tracking-[0.06em] hover:text-ink',
          dir === 'none' ? 'text-muted' : 'text-ink',
        )}
      >
        {children}
        {dir === 'descending' ? <ArrowDown aria-hidden size={11} /> : dir === 'ascending' ? <ArrowUp aria-hidden size={11} /> : null}
      </button>
    </div>
  )
}
