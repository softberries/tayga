import type { HTMLAttributes } from 'react'
import { cx } from '../../lib/cx'

export function Kbd({ className, ...rest }: HTMLAttributes<HTMLElement>) {
  return (
    <kbd
      className={cx(
        'inline-flex items-center rounded-badge border border-field-line px-[5px] py-px font-mono text-[11px] leading-none text-muted',
        className,
      )}
      {...rest}
    />
  )
}
