import type { HTMLAttributes } from 'react'
import { cx } from '../../lib/cx'

/** Loading placeholder with a shimmer (static under reduced motion). Hidden from AT. */
export function Skeleton({ className, ...rest }: HTMLAttributes<HTMLDivElement>) {
  return <div aria-hidden className={cx('tg-skeleton rounded-control', className)} {...rest} />
}
