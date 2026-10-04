import { m } from 'motion/react'
import type { HTMLMotionProps } from 'motion/react'
import type { HTMLAttributes } from 'react'
import { cx } from '../../lib/cx'

export interface CardProps extends HTMLMotionProps<'div'> {
  /** `panel`: 16 px radius sections; `tile`: 14 px KPI tiles; `inner`: nested boxes. */
  variant?: 'panel' | 'tile' | 'inner'
  /** Larger shadow, for the inspector and drawers. */
  elevated?: boolean
  /** Hover lift (2 px); MotionConfig reducedMotion="user" turns it off under reduced motion. */
  lift?: boolean
  className?: string
}

const variants = {
  panel: 'rounded-panel border border-panel-line bg-panel',
  tile: 'rounded-card border border-panel-line bg-panel',
  inner: 'rounded-field border border-panel-line bg-inner',
} as const

export function Card({ variant = 'panel', elevated, lift, className, ...rest }: CardProps) {
  return (
    <m.div
      data-variant={variant}
      className={cx(
        variants[variant],
        variant !== 'inner' && (elevated ? 'shadow-panel-lg' : 'shadow-panel'),
        className,
      )}
      whileHover={lift ? { y: -2 } : undefined}
      transition={{ duration: 0.2, ease: 'easeOut' }}
      {...rest}
    />
  )
}

/** Uppercase section label used at the top of panels ("SERVICE MAP", "LOG ALERTS"). */
export function PanelTitle({ className, ...rest }: HTMLAttributes<HTMLHeadingElement>) {
  return (
    <h2
      className={cx('m-0 text-xs font-medium uppercase tracking-[0.06em] text-muted', className)}
      {...rest}
    />
  )
}
