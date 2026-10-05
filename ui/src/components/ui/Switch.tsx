import { Switch as S } from 'radix-ui'
import type { ComponentProps } from 'react'
import { cx } from '../../lib/cx'

/** A labelled on/off setting (pair it with a `<label htmlFor>`). */
export function Switch({ className, ...props }: ComponentProps<typeof S.Root>) {
  return (
    <S.Root
      className={cx(
        'inline-flex h-[18px] w-8 shrink-0 cursor-pointer items-center rounded-full border border-field-line bg-field p-px transition-colors',
        'data-[state=checked]:border-accent-strong data-[state=checked]:bg-accent-strong',
        'disabled:cursor-not-allowed disabled:opacity-50',
        className,
      )}
      {...props}
    >
      <S.Thumb className="block size-3.5 rounded-full bg-muted transition-transform motion-reduce:transition-none data-[state=checked]:translate-x-3.5 data-[state=checked]:bg-on-accent" />
    </S.Root>
  )
}
