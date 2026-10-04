import { Slot } from 'radix-ui'
import type { ButtonHTMLAttributes, Ref } from 'react'
import { cx } from '../../lib/cx'

export type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'dashed'
export type ButtonSize = 'sm' | 'md'

const base =
  'inline-flex items-center justify-center gap-2 whitespace-nowrap font-[inherit] select-none cursor-pointer ' +
  'transition-[background-color,color,box-shadow,transform] duration-150 disabled:cursor-not-allowed disabled:opacity-50'

const variants: Record<ButtonVariant, string> = {
  primary: 'tg-cta text-on-accent font-semibold shadow-cta hover:brightness-110 active:translate-y-px',
  secondary: 'border border-field-line bg-field text-ink hover:bg-rail-active',
  ghost: 'bg-transparent text-muted hover:bg-rail-active hover:text-ink',
  dashed: 'border border-dashed border-field-line bg-transparent text-muted hover:text-ink hover:border-accent',
}

const sizes: Record<ButtonSize, string> = {
  sm: 'h-[30px] px-2.5 rounded-control text-[13px]',
  md: 'h-9 px-3.5 rounded-field text-[13px]',
}

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant
  size?: ButtonSize
  /** Render the single child (e.g. a router Link) with the button styles. */
  asChild?: boolean
  ref?: Ref<HTMLButtonElement>
}

export function Button({ variant = 'secondary', size = 'md', asChild, className, type, ...rest }: ButtonProps) {
  const Comp = asChild ? Slot.Root : 'button'
  return (
    <Comp
      type={asChild ? undefined : (type ?? 'button')}
      className={cx(base, variants[variant], sizes[size], className)}
      {...rest}
    />
  )
}
