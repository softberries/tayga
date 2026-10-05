import { Fragment } from 'react'
import { cx } from '../../lib/cx'

export interface PathChipsProps {
  /** Services along the request path, entry first; the last one is where it went wrong. */
  services: readonly string[]
  /** Colors the last chip: err for error stories, slow for slow ones. */
  kind?: 'error' | 'slow'
  className?: string
}

/** Request path as chips joined by arrows; the root-cause service is tinted and glows. */
export function PathChips({ services, kind = 'error', className }: PathChipsProps) {
  if (services.length === 0) return null
  return (
    <ol aria-label="Request path" className={cx('m-0 flex list-none flex-wrap items-center gap-1.5 p-0 text-xs', className)}>
      {services.map((s, i) => {
        const last = i === services.length - 1
        return (
          <Fragment key={`${i}-${s}`}>
            <li
              className={cx(
                'rounded-chip px-2 py-[3px]',
                !last && 'bg-inner text-ink-2',
                last && kind === 'error' && 'bg-err-soft text-err shadow-glow-err',
                last && kind === 'slow' && 'bg-slow-soft text-slow shadow-glow-slow',
              )}
              aria-current={last ? 'step' : undefined}
            >
              {s}
            </li>
            {last ? null : (
              <li aria-hidden className="text-faint">
                →
              </li>
            )}
          </Fragment>
        )
      })}
    </ol>
  )
}
