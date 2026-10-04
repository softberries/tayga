import { Dialog as D } from 'radix-ui'
import { X } from 'lucide-react'
import { AnimatePresence, m } from 'motion/react'
import type { ReactNode } from 'react'
import { cx } from '../../lib/cx'
import { useResizableWidth } from './useResizableWidth'

export const SHEET_MIN = 360

function maxWidth(): number {
  return Math.max(SHEET_MIN, Math.round(window.innerWidth * 0.9))
}

export interface SheetProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: ReactNode
  /** Optional line under the title. */
  subtitle?: ReactNode
  children?: ReactNode
  /** Initial width in px (default 520). */
  defaultWidth?: number
  /** localStorage key that remembers the width the user dragged to. */
  storageKey?: string
  /** Modal drawers trap focus and dim the page; inspectors default to non-modal. */
  modal?: boolean
  /** Where focus goes on close; call `e.preventDefault()` and focus your element. Without a
   *  Radix trigger (drawers opened from a list), focus would otherwise fall to <body>. */
  onCloseAutoFocus?: (e: Event) => void
  /** Where focus goes on open (Radix default: the first tabbable, the resize handle). */
  onOpenAutoFocus?: (e: Event) => void
  className?: string
}

/**
 * Right-side drawer that slides in (Motion; reduced motion skips the slide via MotionConfig)
 * and resizes from its left edge by pointer or keyboard (arrow keys, Home/End).
 */
export function Sheet({
  open,
  onOpenChange,
  title,
  subtitle,
  children,
  defaultWidth = 520,
  storageKey,
  modal = false,
  onCloseAutoFocus,
  onOpenAutoFocus,
  className,
}: SheetProps) {
  const { width, handleProps } = useResizableWidth({ defaultWidth, min: SHEET_MIN, max: maxWidth, storageKey })

  return (
    <D.Root open={open} onOpenChange={onOpenChange} modal={modal}>
      <AnimatePresence>
        {open ? (
          <D.Portal forceMount>
            {modal ? (
              <D.Overlay forceMount asChild>
                <m.div
                  className="fixed inset-0 z-40 bg-scrim"
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  exit={{ opacity: 0 }}
                  transition={{ duration: 0.15 }}
                />
              </D.Overlay>
            ) : null}
            <D.Content
              forceMount
              asChild
              aria-describedby={undefined}
              onInteractOutside={modal ? undefined : (e) => e.preventDefault()}
              onCloseAutoFocus={onCloseAutoFocus}
              onOpenAutoFocus={onOpenAutoFocus}
            >
              <m.aside
                className={cx(
                  'fixed inset-y-0 right-0 z-50 flex max-w-[100vw] flex-col border-l border-panel-line bg-panel text-ink shadow-panel-lg focus:outline-none',
                  className,
                )}
                style={{ width }}
                initial={{ x: '100%' }}
                animate={{ x: 0 }}
                exit={{ x: '100%' }}
                transition={{ type: 'tween', duration: 0.22, ease: [0.2, 0.8, 0.2, 1] }}
              >
                <div
                  {...handleProps}
                  aria-label="Resize panel"
                  className="absolute inset-y-0 -left-1 w-2 cursor-col-resize touch-none after:absolute after:inset-y-0 after:left-[3px] after:w-0.5 after:bg-transparent hover:after:bg-accent focus-visible:after:bg-accent"
                />
                <header className="flex items-start gap-3 border-b border-line px-5 py-4">
                  <div className="flex min-w-0 flex-1 flex-col gap-1">
                    <D.Title className="m-0 truncate text-[15px] font-semibold">{title}</D.Title>
                    {subtitle ? <div className="text-xs text-muted">{subtitle}</div> : null}
                  </div>
                  <D.Close
                    aria-label="Close panel"
                    className="inline-flex size-8 cursor-pointer items-center justify-center rounded-control text-muted hover:bg-rail-active hover:text-ink"
                  >
                    <X size={16} aria-hidden />
                  </D.Close>
                </header>
                <div className="min-h-0 flex-1 overflow-auto px-5 py-4">{children}</div>
              </m.aside>
            </D.Content>
          </D.Portal>
        ) : null}
      </AnimatePresence>
    </D.Root>
  )
}
