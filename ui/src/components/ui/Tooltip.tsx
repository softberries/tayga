import { Tooltip as T } from 'radix-ui'
import { cloneElement, useRef, useState } from 'react'
import type { ReactElement, ReactNode, Ref } from 'react'

export const TooltipProvider = T.Provider

const CONTENT =
  'z-50 rounded-control border border-panel-line bg-panel px-2 py-1 text-xs text-ink shadow-panel motion-safe:data-[state=delayed-open]:animate-[tg-in_.15s_ease]'

export interface TooltipProps {
  content: ReactNode
  side?: 'top' | 'right' | 'bottom' | 'left'
  /** One focusable element; it becomes the trigger. */
  children: ReactElement
}

export function Tooltip({ content, side = 'bottom', children }: TooltipProps) {
  return (
    <T.Root>
      <T.Trigger asChild>{children}</T.Trigger>
      <T.Portal>
        <T.Content side={side} sideOffset={6} className={CONTENT}>
          {content}
        </T.Content>
      </T.Portal>
    </T.Root>
  )
}

/** True when the element's text is cut off by `truncate` (ellipsis showing). */
export function isTruncated(el: HTMLElement | null): boolean {
  return el !== null && el.scrollWidth > el.clientWidth
}

/**
 * Tooltip with the full text of a `truncate` element, shown only while the text is actually cut
 * off (checked when the tooltip tries to open). `children` is the one truncating element and
 * must accept a `ref`.
 */
export function TruncationTooltip({
  content,
  children,
}: {
  content: ReactNode
  children: ReactElement<{ ref?: Ref<HTMLElement> }>
}) {
  const ref = useRef<HTMLElement>(null)
  const [open, setOpen] = useState(false)
  return (
    <T.Root open={open} onOpenChange={(o) => setOpen(o && isTruncated(ref.current))}>
      <T.Trigger asChild>{cloneElement(children, { ref })}</T.Trigger>
      <T.Portal>
        <T.Content side="bottom" align="start" sideOffset={6} className={CONTENT}>
          <span className="block max-w-[min(80vw,640px)] whitespace-pre-wrap break-words">{content}</span>
        </T.Content>
      </T.Portal>
    </T.Root>
  )
}
