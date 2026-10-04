import { Dialog as D } from 'radix-ui'
import { X } from 'lucide-react'
import type { ReactNode } from 'react'
import { cx } from '../../lib/cx'

export const DialogRoot = D.Root
export const DialogTrigger = D.Trigger
export const DialogClose = D.Close

export interface DialogContentProps {
  title: ReactNode
  /** Visually hidden when false-y; Radix requires a description or aria-describedby={undefined}. */
  description?: ReactNode
  children?: ReactNode
  className?: string
  /** Hide the visible title (kept for screen readers). */
  hideTitle?: boolean
}

/** Centered modal: scrim, panel, close button, enter animation (off under reduced motion). */
export function DialogContent({ title, description, children, className, hideTitle }: DialogContentProps) {
  return (
    <D.Portal>
      <D.Overlay className="fixed inset-0 z-40 bg-scrim backdrop-blur-[2px] data-[state=open]:animate-[tg-fade_.15s_ease] motion-reduce:animate-none" />
      <D.Content
        {...(description ? {} : { 'aria-describedby': undefined })}
        className={cx(
          'fixed left-1/2 top-[12vh] z-50 w-[min(640px,calc(100vw-32px))] -translate-x-1/2',
          'rounded-panel border border-panel-line bg-panel p-5 text-ink shadow-panel-lg',
          'data-[state=open]:animate-[tg-in_.2s_ease] motion-reduce:animate-none focus:outline-none',
          className,
        )}
      >
        <div className="mb-3 flex items-start gap-3">
          <D.Title className={cx('m-0 flex-1 text-[15px] font-semibold', hideTitle && 'sr-only')}>{title}</D.Title>
          <D.Close
            aria-label="Close"
            className="inline-flex size-8 cursor-pointer items-center justify-center rounded-control text-muted hover:bg-rail-active hover:text-ink"
          >
            <X size={16} aria-hidden />
          </D.Close>
        </div>
        {description ? (
          <D.Description className="m-0 mb-3 text-muted">{description}</D.Description>
        ) : null}
        {children}
      </D.Content>
    </D.Portal>
  )
}
