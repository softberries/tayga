import { Link, useLocation } from '@tanstack/react-router'
import { Activity, ChartGantt, ScrollText, TextAlignStart, Waypoints } from 'lucide-react'
import type { LucideIcon } from 'lucide-react'
import { cx } from '../../lib/cx'
import { Tooltip } from '../ui/Tooltip'

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
  return (
    <nav
      aria-label="Main"
      className="sticky top-0 flex h-dvh w-[68px] shrink-0 flex-col items-center gap-1.5 border-r border-line bg-rail py-4 max-sm:h-auto max-sm:w-full max-sm:flex-row max-sm:justify-center max-sm:border-r-0 max-sm:border-b max-sm:py-2"
    >
      <div
        aria-hidden
        className="tg-brand mb-3.5 flex size-[34px] items-center justify-center rounded-field font-bold text-on-accent shadow-brand max-sm:mb-0 max-sm:mr-3"
      >
        T
      </div>
      {SECTIONS.map(({ to, label, icon: Icon, match }) => {
        const active = match(path)
        return (
          <Tooltip key={to} content={label} side="right">
            <Link
              to={to}
              search={(prev) => prev}
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
  )
}
