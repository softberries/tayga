import type { ButtonHTMLAttributes, ReactNode, Ref } from 'react'
import { cx } from '../../lib/cx'
import { Tooltip } from './Tooltip'

export interface IconButtonProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'aria-label' | 'children'> {
  /** Accessible name; also the tooltip text unless `tooltip` is false. */
  label: string
  icon: ReactNode
  tooltip?: boolean
  ref?: Ref<HTMLButtonElement>
}

/** Icon-only button: always has an aria-label (spec §5). */
export function IconButton({ label, icon, tooltip = true, className, type, ...rest }: IconButtonProps) {
  const button = (
    <button
      type={type ?? 'button'}
      aria-label={label}
      className={cx(
        'inline-flex h-9 w-11 cursor-pointer items-center justify-center rounded-field border border-field-line bg-field text-ink',
        'transition-colors duration-150 hover:bg-rail-active disabled:cursor-not-allowed disabled:opacity-50',
        className,
      )}
      {...rest}
    >
      {icon}
    </button>
  )
  return tooltip ? <Tooltip content={label}>{button}</Tooltip> : button
}
