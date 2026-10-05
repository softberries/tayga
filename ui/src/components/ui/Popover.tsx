import { Popover as P } from 'radix-ui'
import type { ComponentProps } from 'react'
import { cx } from '../../lib/cx'

export const Popover = P.Root
export const PopoverTrigger = P.Trigger
export const PopoverClose = P.Close

export const floatingPanel =
  'z-50 rounded-field border border-panel-line bg-panel text-ink shadow-panel-lg motion-safe:data-[state=open]:animate-[tg-in_.15s_ease]'

export function PopoverContent({ className, sideOffset = 6, align = 'start', ...rest }: ComponentProps<typeof P.Content>) {
  return (
    <P.Portal>
      <P.Content sideOffset={sideOffset} align={align} className={cx(floatingPanel, 'p-3', className)} {...rest} />
    </P.Portal>
  )
}
