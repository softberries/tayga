import { ToggleGroup as TG } from 'radix-ui'
import { cx } from '../../lib/cx'

export interface ToggleOption<V extends string> {
  value: V
  label: string
}

export interface ToggleGroupProps<V extends string> {
  /** Accessible name of the group, e.g. "Time range". */
  label: string
  options: readonly ToggleOption<V>[]
  value: V
  onValueChange: (value: V) => void
  className?: string
}

/**
 * Single-choice segmented control (the header time range). Clicking the active item keeps it
 * selected: Radix would otherwise emit an empty value.
 */
export function ToggleGroup<V extends string>({ label, options, value, onValueChange, className }: ToggleGroupProps<V>) {
  return (
    <TG.Root
      type="single"
      aria-label={label}
      value={value}
      onValueChange={(v) => {
        const next = options.find((o) => o.value === v)
        if (next) onValueChange(next.value)
      }}
      className={cx('inline-flex rounded-field border border-field-line bg-field p-0.5', className)}
    >
      {options.map((o) => (
        <TG.Item
          key={o.value}
          value={o.value}
          className={cx(
            'h-[30px] cursor-pointer rounded-control px-2.5 text-[13px] text-muted transition-[background-color,color,box-shadow] duration-150',
            'hover:text-ink data-[state=on]:bg-accent-strong data-[state=on]:text-on-accent data-[state=on]:shadow-accent',
          )}
        >
          {o.label}
        </TG.Item>
      ))}
    </TG.Root>
  )
}
