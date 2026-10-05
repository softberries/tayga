import { Tooltip as T } from 'radix-ui'
import { cloneElement, useEffect, useRef, useState } from 'react'
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
 *
 * Keyboard and touch: the cell is not itself a tab stop (big tables would get one per cell).
 * With `openOnHostFocus` the tooltip also opens while the host the cell sits in has focus: the
 * nearest row (`role=row`, whatever its tabindex, so roving-tabindex rows work), link or button.
 * Only one tooltip per host may opt in, or they would open on top of each other. A pointer press
 * on the host closes it. The text stays whole in the DOM, so screen readers
 * read it in full either way.
 */
export function TruncationTooltip({
  content,
  children,
  openOnHostFocus = false,
  side = 'bottom',
}: {
  content: ReactNode
  children: ReactElement<{ ref?: Ref<HTMLElement> }>
  /** `'any-cut'`: open when any truncated text inside the host is cut, not just this cell's. */
  openOnHostFocus?: boolean | 'any-cut'
  side?: 'top' | 'bottom'
}) {
  const ref = useRef<HTMLElement>(null)
  const [open, setOpen] = useState(false)
  useEffect(() => {
    if (!openOnHostFocus) return
    const host = ref.current?.closest<HTMLElement>('[role=row], a[href], button')
    if (!host) return
    const show = (e: FocusEvent) => {
      // Only the host's own focus, not a control inside it.
      if (e.target !== host) return
      const cut = openOnHostFocus === 'any-cut' ? [...host.querySelectorAll<HTMLElement>('.truncate')].some(isTruncated) : isTruncated(ref.current)
      setOpen(cut)
    }
    const hide = () => setOpen(false)
    host.addEventListener('focus', show)
    host.addEventListener('blur', hide)
    host.addEventListener('pointerdown', hide)
    return () => {
      host.removeEventListener('focus', show)
      host.removeEventListener('blur', hide)
      host.removeEventListener('pointerdown', hide)
    }
  }, [openOnHostFocus])
  return (
    <T.Root open={open} onOpenChange={(o) => setOpen(o && isTruncated(ref.current))}>
      <T.Trigger asChild>{cloneElement(children, { ref })}</T.Trigger>
      <T.Portal>
        <T.Content side={side} align="start" sideOffset={6} className={CONTENT}>
          <span className="block max-w-[min(80vw,640px)] whitespace-pre-wrap break-words">{content}</span>
        </T.Content>
      </T.Portal>
    </T.Root>
  )
}
