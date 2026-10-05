import { DropdownMenu as M } from 'radix-ui'
import type { ComponentProps } from 'react'
import { cx } from '../../lib/cx'
import { floatingPanel } from './Popover'

export const DropdownMenu = M.Root
export const DropdownMenuTrigger = M.Trigger

export function DropdownMenuContent({ className, sideOffset = 6, align = 'start', ...rest }: ComponentProps<typeof M.Content>) {
  return (
    <M.Portal>
      <M.Content sideOffset={sideOffset} align={align} className={cx(floatingPanel, 'min-w-44 p-1', className)} {...rest} />
    </M.Portal>
  )
}

export function DropdownMenuItem({ className, ...rest }: ComponentProps<typeof M.Item>) {
  return (
    <M.Item
      className={cx(
        'flex h-8 cursor-pointer select-none items-center gap-2 rounded-control px-2 text-[13px] outline-none',
        'data-[highlighted]:bg-rail-active data-[disabled]:cursor-not-allowed data-[disabled]:opacity-50',
        className,
      )}
      {...rest}
    />
  )
}

export function DropdownMenuSeparator({ className, ...rest }: ComponentProps<typeof M.Separator>) {
  return <M.Separator className={cx('my-1 h-px bg-line', className)} {...rest} />
}

export function DropdownMenuLabel({ className, ...rest }: ComponentProps<typeof M.Label>) {
  return <M.Label className={cx('px-2 py-1 text-[11px] uppercase tracking-[0.06em] text-muted', className)} {...rest} />
}
