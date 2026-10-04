import { Tabs as T } from 'radix-ui'
import type { ComponentProps } from 'react'
import { cx } from '../../lib/cx'

export const Tabs = T.Root

export function TabsList({ className, ...rest }: ComponentProps<typeof T.List>) {
  return <T.List className={cx('flex items-center gap-1 border-b border-line', className)} {...rest} />
}

export function TabsTrigger({ className, ...rest }: ComponentProps<typeof T.Trigger>) {
  return (
    <T.Trigger
      className={cx(
        '-mb-px inline-flex h-9 cursor-pointer items-center gap-2 border-b-2 border-transparent px-3 text-[13px] text-muted',
        'transition-colors duration-150 hover:text-ink data-[state=active]:border-accent data-[state=active]:text-ink',
        className,
      )}
      {...rest}
    />
  )
}

export function TabsContent({ className, ...rest }: ComponentProps<typeof T.Content>) {
  return <T.Content className={cx('pt-4 focus-visible:outline-none', className)} {...rest} />
}
