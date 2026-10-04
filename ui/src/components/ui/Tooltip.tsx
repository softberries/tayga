import { Tooltip as T } from 'radix-ui'
import type { ReactElement, ReactNode } from 'react'

export const TooltipProvider = T.Provider

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
        <T.Content
          side={side}
          sideOffset={6}
          className="z-50 rounded-control border border-panel-line bg-panel px-2 py-1 text-xs text-ink shadow-panel data-[state=delayed-open]:animate-[tg-in_.15s_ease] motion-reduce:animate-none"
        >
          {content}
        </T.Content>
      </T.Portal>
    </T.Root>
  )
}
