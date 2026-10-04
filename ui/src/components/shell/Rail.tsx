import { Link, useLocation } from '@tanstack/react-router'
import { Activity, ChartGantt, ScrollText, TextAlignStart, Waypoints } from 'lucide-react'
import type { LucideIcon } from 'lucide-react'
import { sinceSearch } from '../../app/search'
import { cx } from '../../lib/cx'
import { Tooltip } from '../ui/Tooltip'
import { useSince } from './TimeRange'

interface Section {
  to: '/' | '/traces' | '/map' | '/logs' | '/pipeline'
  label: string
  icon: LucideIcon
  /** Path prefixes that mark the section active. */
  match: (path: string) => boolean
}

export const SECTIONS: readonly Section[] = [
  { to: '/', label: 'Stories', icon: TextAlignStart, match: (p) => p === '/' || p.startsWith('/stories') },
  { to: '/traces', label: 'Traces', icon: ChartGantt, match: (p) => p.startsWith('/traces') },
  { to: '/map', label: 'Service map', icon: Waypoints, match: (p) => p.startsWith('/map') },
  { to: '/logs', label: 'Logs and templates', icon: ScrollText, match: (p) => p.startsWith('/logs') },
  { to: '/pipeline', label: 'Pipeline health', icon: Activity, match: (p) => p.startsWith('/pipeline') },
]

export function Rail() {
  const path = useLocation({ select: (l) => l.pathname })
  const search = sinceSearch(useSince())
  // The wrapper stretches to the page height and carries the rail's background and border;
  // the nav inside stays in view (sticky). Phones: a bottom bar fixed to the viewport (main gets matching bottom padding).
  return (
    <div className="shrink-0 border-r border-line bg-rail max-sm:fixed max-sm:inset-x-0 max-sm:bottom-0 max-sm:z-30 max-sm:border-r-0 max-sm:border-t max-sm:pb-[env(safe-area-inset-bottom,0px)]">
      <nav
        aria-label="Main"
        className="sticky top-0 flex h-dvh w-[68px] flex-col items-center gap-1.5 py-4 max-sm:static max-sm:h-auto max-sm:w-full max-sm:flex-row max-sm:justify-around max-sm:px-2 max-sm:py-1.5"
      >
        <img
          src="/logo-mark.png"
          alt=""
          width={34}
          height={34}
          className="mb-3.5 size-[34px] rounded-field object-cover shadow-brand max-sm:hidden"
        />
        {SECTIONS.map(({ to, label, icon: Icon, match }) => {
          const active = match(path)
          return (
            <Tooltip key={to} content={label} side="right">
              <Link
                to={to}
                search={search}
                aria-label={label}
                aria-current={active ? 'page' : undefined}
                className={cx(
                  'flex size-11 items-center justify-center rounded-rail transition-colors duration-150',
                  active ? 'bg-rail-active text-accent shadow-rail' : 'text-muted hover:bg-rail-active hover:text-ink',
                )}
              >
                <Icon size={18} strokeWidth={1.8} aria-hidden />
              </Link>
            </Tooltip>
          )
        })}
      </nav>
    </div>
  )
}
