import { ScrollArea as S } from 'radix-ui'
import type { ReactNode } from 'react'
import { cx } from '../../lib/cx'

export interface ScrollAreaProps {
  children: ReactNode
  className?: string
  /** Class for the scrolling viewport (set a max height here). */
  viewportClassName?: string
  /** Accessible name; the viewport is focusable so keyboard users can scroll it. */
  label?: string
}

function Bar({ orientation }: { orientation: 'vertical' | 'horizontal' }) {
  return (
    <S.Scrollbar
      orientation={orientation}
      className={cx(
        'flex touch-none select-none p-0.5 transition-colors',
        orientation === 'vertical' ? 'w-2.5' : 'h-2.5 flex-col',
      )}
    >
      <S.Thumb className="relative flex-1 rounded-full bg-field-line hover:bg-faint" />
    </S.Scrollbar>
  )
}

export function ScrollArea({ children, className, viewportClassName, label }: ScrollAreaProps) {
  return (
    <S.Root type="hover" className={cx('relative overflow-hidden', className)}>
      <S.Viewport
        tabIndex={0}
        aria-label={label}
        className={cx('size-full rounded-[inherit] focus-visible:outline-none', viewportClassName)}
      >
        {children}
      </S.Viewport>
      <Bar orientation="vertical" />
      <Bar orientation="horizontal" />
      <S.Corner />
    </S.Root>
  )
}
