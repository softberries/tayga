import { useNavigate } from '@tanstack/react-router'
import { CalendarClock } from 'lucide-react'
import { useEffect, useId, useState } from 'react'
import type { FormEvent } from 'react'
import { setCustomRangeOpen, useCustomRangeOpen } from '../../app/customRangeDialog'
import { MAX_UNTIL_AHEAD_SECS, RETENTION_SECS, customRange, customRangeError, fromLocalInput, rangeSearch, toLocalInput } from '../../app/range'
import type { Range } from '../../app/range'
import { DEFAULT_SINCE, SINCE_VALUES } from '../../app/search'
import { useRange } from '../../app/useRange'
import { cx } from '../../lib/cx'
import { Button } from '../ui/Button'
import { Popover, PopoverContent, PopoverTrigger } from '../ui/Popover'
import { ToggleGroup } from '../ui/ToggleGroup'

const options = SINCE_VALUES.map((v) => ({ value: v as string, label: v }))

const field =
  'h-9 w-full min-w-0 rounded-control border border-field-line bg-field px-2 font-[inherit] text-[13px] text-ink shadow-inset outline-none focus-visible:border-accent'

/** The custom range form: local `datetime-local` fields, applied to the URL as UTC. */
function CustomRangeForm({ range, onApply, onCancel }: { range: Range; onApply: (r: Range) => void; onCancel: () => void }) {
  // The clock when the popover opened: the fields' defaults and bounds.
  const [now] = useState(() => Date.now())
  // Start from the range on screen: a custom one as it is, a preset as its window so far.
  const end = range.untilMs ?? Math.floor(now / 60_000) * 60_000
  const [from, setFrom] = useState(() => toLocalInput(end - range.secs * 1000))
  const [to, setTo] = useState(() => toLocalInput(end))
  // The reason Apply was refused; cleared by the next edit.
  const [shown, setShown] = useState<string | null>(null)
  const id = useId()

  const submit = (e: FormEvent) => {
    e.preventDefault()
    const fromMs = fromLocalInput(from)
    const toMs = fromLocalInput(to)
    // Checked against the clock at submit, as the API will.
    const error = customRangeError(fromMs, toMs, Date.now())
    setShown(error)
    if (error === null) onApply(customRange(fromMs, toMs))
  }

  const min = toLocalInput(now - RETENTION_SECS * 1000)
  const max = toLocalInput(now + MAX_UNTIL_AHEAD_SECS * 1000)
  return (
    <form onSubmit={submit} noValidate className="flex w-[min(280px,calc(100vw-48px))] flex-col gap-3" aria-label="Custom time range">
      <div className="flex flex-col gap-1">
        <label htmlFor={`${id}-from`} className="text-[11px] uppercase tracking-[0.06em] text-muted">
          From
        </label>
        <input
          id={`${id}-from`}
          type="datetime-local"
          value={from}
          min={min}
          max={max}
          onChange={(e) => {
            setFrom(e.target.value)
            setShown(null)
          }}
          aria-invalid={shown !== null}
          aria-describedby={shown ? `${id}-error` : `${id}-hint`}
          className={cx(field, shown && 'border-err')}
        />
      </div>
      <div className="flex flex-col gap-1">
        <label htmlFor={`${id}-to`} className="text-[11px] uppercase tracking-[0.06em] text-muted">
          To
        </label>
        <input
          id={`${id}-to`}
          type="datetime-local"
          value={to}
          min={min}
          max={max}
          onChange={(e) => {
            setTo(e.target.value)
            setShown(null)
          }}
          aria-invalid={shown !== null}
          aria-describedby={shown ? `${id}-error` : `${id}-hint`}
          className={cx(field, shown && 'border-err')}
        />
      </div>
      {shown ? (
        <p id={`${id}-error`} role="alert" className="text-xs text-err">
          {shown}
        </p>
      ) : (
        <p id={`${id}-hint`} className="text-xs text-muted">
          Local time. Up to 7 days long, within the last 7 days.
        </p>
      )}
      <div className="flex justify-end gap-2">
        <Button size="sm" variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
        <Button size="sm" variant="primary" type="submit">
          Apply
        </Button>
      </div>
    </form>
  )
}

/**
 * The header time range: the presets (which end now) and "Custom…", a range with a fixed end.
 * Both live in the URL (`since`, `until`); choosing a preset clears `until`.
 */
export function TimeRange() {
  const range = useRange()
  const navigate = useNavigate()
  const open = useCustomRangeOpen()
  const custom = range.until !== undefined
  // The open state is shared (the palette opens it): reset it when the header goes away.
  useEffect(() => () => setCustomRangeOpen(false), [])

  const go = (next: Range) => void navigate({ to: '.', search: (prev) => ({ ...prev, ...rangeSearch(next) }), replace: true })
  return (
    <div className="flex min-w-0 flex-wrap items-center gap-1.5">
      <ToggleGroup
        label="Time range"
        options={options}
        value={custom ? '' : range.since}
        onValueChange={(v) =>
          void navigate({
            to: '.',
            search: (prev) => ({ ...prev, since: v === DEFAULT_SINCE ? undefined : v, until: undefined }),
            replace: true,
          })
        }
      />
      <Popover open={open} onOpenChange={setCustomRangeOpen}>
        <PopoverTrigger asChild>
          <button
            type="button"
            aria-label={custom ? `Custom time range: ${range.label}` : 'Custom time range'}
            className={cx(
              'inline-flex h-9 min-w-0 cursor-pointer items-center gap-2 rounded-field border px-2.5 text-[13px] transition-colors duration-150',
              custom
                ? 'border-transparent bg-accent-strong text-on-accent shadow-accent'
                : 'border-field-line bg-field text-muted hover:bg-rail-active hover:text-ink',
            )}
          >
            <CalendarClock size={14} strokeWidth={2} aria-hidden className="shrink-0" />
            <span className="tabular truncate">{custom ? range.label : 'Custom…'}</span>
          </button>
        </PopoverTrigger>
        <PopoverContent align="end">
          {/* Remounted per opening, so the fields start from the range on screen. */}
          <CustomRangeForm
            range={range}
            onApply={(r) => {
              setCustomRangeOpen(false)
              go(r)
            }}
            onCancel={() => setCustomRangeOpen(false)}
          />
        </PopoverContent>
      </Popover>
    </div>
  )
}
