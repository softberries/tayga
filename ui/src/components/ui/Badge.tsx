import type { HTMLAttributes, ReactNode } from 'react'
import { cx } from '../../lib/cx'

export type BadgeKind = 'error' | 'slow' | 'new' | 'spike' | 'silence' | 'ok' | 'neutral'

const tag: Record<BadgeKind, string> = {
  error: 'bg-err-soft text-err',
  slow: 'bg-slow-soft text-slow',
  new: 'bg-accent-strong text-on-accent',
  spike: 'bg-slow text-on-slow',
  silence: 'bg-silence-soft text-silence',
  ok: 'bg-ok-soft text-ok',
  neutral: 'bg-inner text-muted',
}

const dot: Record<BadgeKind, string> = {
  error: 'bg-err',
  slow: 'bg-slow',
  new: 'bg-on-accent',
  spike: 'bg-on-slow',
  silence: 'bg-silence',
  ok: 'bg-ok',
  neutral: 'bg-muted',
}

export interface BadgeProps extends HTMLAttributes<HTMLSpanElement> {
  kind: BadgeKind
  /** `tag`: small mono label (kind chips). `pill`: rounded status pill with a dot. */
  shape?: 'tag' | 'pill'
  /** Pulsing dot (pill only), e.g. degraded services. Off under reduced motion. */
  pulse?: boolean
  /** Colored glow for the selected/root-cause emphasis. */
  glow?: boolean
  children: ReactNode
}

export function Badge({ kind, shape = 'tag', pulse, glow, className, children, ...rest }: BadgeProps) {
  if (shape === 'pill') {
    return (
      <span
        data-kind={kind}
        className={cx(
          'inline-flex items-center gap-2 rounded-full px-[11px] py-[5px] text-xs font-semibold',
          tag[kind],
          glow && (kind === 'slow' ? 'shadow-glow-slow' : 'shadow-glow-err'),
          className,
        )}
        {...rest}
      >
        <span aria-hidden className={cx('size-2 rounded-full', dot[kind], pulse && 'tg-pulse')} />
        {children}
      </span>
    )
  }
  return (
    <span
      data-kind={kind}
      className={cx(
        'inline-flex items-center rounded-badge px-1.5 py-px font-mono text-[10.5px] leading-[1.5]',
        tag[kind],
        glow && (kind === 'slow' ? 'shadow-glow-slow' : 'shadow-glow-err'),
        className,
      )}
      {...rest}
    >
      {children}
    </span>
  )
}
