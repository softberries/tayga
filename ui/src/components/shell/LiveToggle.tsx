import { useLive, useDocumentVisible } from '../../app/live'
import { cx } from '../../lib/cx'
import { Tooltip } from '../ui/Tooltip'

/** Auto-refresh every 10 s; paused while the tab is hidden. */
export function LiveToggle() {
  const { live, setLive } = useLive()
  const visible = useDocumentVisible()
  const hint = live ? (visible ? 'Refreshing every 10 s' : 'Paused while the tab is hidden') : 'Auto-refresh is off'
  return (
    <Tooltip content={hint}>
      <button
        type="button"
        aria-pressed={live}
        onClick={() => setLive(!live)}
        className={cx(
          'inline-flex h-9 cursor-pointer items-center gap-2 rounded-field border border-field-line bg-field px-3 text-[13px] transition-colors duration-150 hover:bg-rail-active',
          live ? 'text-ink' : 'text-muted',
        )}
      >
        <span aria-hidden className={cx('size-2 rounded-full', live ? 'tg-pulse-accent bg-accent' : 'bg-faint')} />
        Live
      </button>
    </Tooltip>
  )
}
