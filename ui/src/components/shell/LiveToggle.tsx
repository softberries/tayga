import { useLive, useDocumentVisible } from '../../app/live'
import { useRange } from '../../app/useRange'
import { cx } from '../../lib/cx'
import { Tooltip } from '../ui/Tooltip'

/** Auto-refresh every 10 s; paused while the tab is hidden, and off for a custom (past) range. */
export function LiveToggle() {
  const { live, setLive } = useLive()
  const visible = useDocumentVisible()
  const past = useRange().until !== undefined
  const on = live && !past
  const hint = past ? 'Live is off for a past range' : live ? (visible ? 'Refreshing every 10 s' : 'Paused while the tab is hidden') : 'Auto-refresh is off'
  return (
    <Tooltip content={hint}>
      {/* aria-disabled, not disabled: the button stays focusable so the tooltip explains why. */}
      <button
        type="button"
        aria-pressed={on}
        aria-disabled={past || undefined}
        onClick={() => {
          if (!past) setLive(!live)
        }}
        className={cx(
          'inline-flex h-9 items-center gap-2 rounded-field border border-field-line bg-field px-3 text-[13px] transition-colors duration-150',
          past ? 'cursor-not-allowed opacity-60' : 'cursor-pointer hover:bg-rail-active',
          on ? 'text-ink' : 'text-muted',
        )}
      >
        <span aria-hidden className={cx('size-2 rounded-full', on ? 'tg-pulse-accent bg-accent' : 'bg-faint')} />
        Live
      </button>
    </Tooltip>
  )
}
