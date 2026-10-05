/**
 * Searchable single-value picker: a trigger button that opens a cmdk list in a popover.
 * The first item clears the value ("any"). Options are plain strings shown in mono.
 */
import { Command } from 'cmdk'
import { Check, ChevronDown, X } from 'lucide-react'
import { useState } from 'react'
import { cx } from '../../lib/cx'
import { Button } from './Button'
import { Popover, PopoverContent, PopoverTrigger } from './Popover'

export interface ComboboxProps {
  /** Field name, e.g. "Service": the trigger reads "Service: any" or "Service: payment". */
  label: string
  value: string | undefined
  options: readonly string[]
  onChange: (value: string | undefined) => void
  /** Per-option count shown at the right, e.g. rows per endpoint. */
  counts?: ReadonlyMap<string, number>
  /** Shown when the options are still loading or none exist. */
  emptyText?: string
  className?: string
}

const item =
  'flex min-h-8 cursor-pointer items-center gap-2 rounded-control px-2 text-[13px] text-ink data-[selected=true]:bg-rail-active'

export function Combobox({ label, value, options, onChange, counts, emptyText = 'No matches', className }: ComboboxProps) {
  const [open, setOpen] = useState(false)
  const pick = (v: string | undefined) => {
    onChange(v)
    setOpen(false)
  }
  return (
    <span className={cx('inline-flex max-w-full items-center', className)}>
      <Popover open={open} onOpenChange={setOpen}>
        <PopoverTrigger asChild>
          <Button
            size="sm"
            aria-label={`${label}: ${value ?? 'any'}`}
            className={cx('min-w-0 max-w-[min(100%,320px)]', value && 'rounded-r-none')}
          >
            <span className="text-muted">{label}:</span>
            <span className={cx('min-w-0 truncate', value ? 'font-mono text-xs' : 'text-muted')} title={value}>
              {value ?? 'any'}
            </span>
            <ChevronDown aria-hidden size={14} className="shrink-0 text-muted" />
          </Button>
        </PopoverTrigger>
        <PopoverContent className="w-[min(92vw,360px)] p-1.5">
          <Command label={label} loop>
            <Command.Input
              autoFocus
              placeholder={`Search ${label.toLowerCase()}`}
              className="mb-1 h-8 w-full rounded-control border border-field-line bg-field px-2 text-[13px] text-ink outline-none placeholder:text-faint focus-visible:border-accent"
            />
            <Command.List className="max-h-72 overflow-y-auto overscroll-contain">
              <Command.Empty className="px-2 py-3 text-center text-xs text-muted">{emptyText}</Command.Empty>
              <Command.Item value={`any ${label}`} onSelect={() => pick(undefined)} className={item}>
                <Check aria-hidden size={13} className={value ? 'invisible' : 'text-accent'} />
                <span className="text-muted">Any {label.toLowerCase()}</span>
              </Command.Item>
              {options.map((o) => (
                <Command.Item key={o} value={o} onSelect={() => pick(o)} className={item}>
                  <Check aria-hidden size={13} className={o === value ? 'shrink-0 text-accent' : 'invisible shrink-0'} />
                  <span className="min-w-0 truncate font-mono text-xs" title={o}>
                    {o}
                  </span>
                  {counts?.has(o) ? <span className="tabular ml-auto pl-3 text-xs text-muted">{counts.get(o)}</span> : null}
                </Command.Item>
              ))}
            </Command.List>
          </Command>
        </PopoverContent>
      </Popover>
      {value ? (
        <button
          type="button"
          aria-label={`Clear ${label.toLowerCase()} filter`}
          onClick={() => onChange(undefined)}
          className="inline-flex h-[30px] w-7 shrink-0 cursor-pointer items-center justify-center rounded-r-control border border-l-0 border-field-line bg-field text-muted hover:bg-rail-active hover:text-ink"
        >
          <X aria-hidden size={13} />
        </button>
      ) : null}
    </span>
  )
}
