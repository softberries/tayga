/**
 * Static SVG preview of /service-map: services in columns by call depth, failing services
 * glowing and failing calls as flowing dashed red edges (still under reduced motion). The
 * whole preview links to the interactive map.
 */
import { Link } from '@tanstack/react-router'
import { ArrowRight } from 'lucide-react'
import { useId, useMemo } from 'react'
import type { ServiceMapView } from '../../api/types'
import { sinceSearch } from '../../app/search'
import type { Since } from '../../app/search'
import { cx } from '../../lib/cx'
import { MINI, isFailingEdge, layoutMiniMap } from './mapLayout'

/** Longest label drawn; longer service names are cut with an ellipsis (full name in <title>). */
const LABEL_CHARS = 16

const label = (s: string) => (s.length > LABEL_CHARS ? `${s.slice(0, LABEL_CHARS - 1)}…` : s)

export function describeMap(map: ServiceMapView): string {
  const bad = map.nodes.filter((n) => n.health !== 'ok')
  const failing = map.edges.filter((e) => isFailingEdge(e) && e.parent !== e.child)
  const parts = [`${map.nodes.length} services`]
  parts.push(bad.length ? `degraded: ${bad.map((n) => `${n.service} (${n.health})`).join(', ')}` : 'all healthy')
  if (failing.length) parts.push(`failing calls: ${failing.map((e) => `${e.parent} to ${e.child}`).join(', ')}`)
  return `Service map: ${parts.join('; ')}.`
}

export function MiniMap({ map, since }: { map: ServiceMapView; since: Since }) {
  const layout = useMemo(() => layoutMiniMap(map), [map])
  const glowId = `${useId()}-glow`
  const slowGlowId = `${glowId}-slow`
  if (layout.nodes.length === 0) return <p className="m-0 text-muted">No service calls in this window.</p>
  return (
    <Link
      to="/map"
      search={sinceSearch(since)}
      className="group -mx-1 block overflow-x-auto rounded-field px-1 focus-visible:outline-offset-0"
      aria-label={`${describeMap(map)} Open the service map.`}
    >
      <svg
        role="presentation"
        width="100%"
        viewBox={`0 0 ${layout.width} ${layout.height}`}
        className="block min-w-[320px]"
        style={{ maxHeight: 320 }}
      >
        <defs>
          <radialGradient id={glowId}>
            <stop offset="0%" stopColor="var(--tg-err)" stopOpacity={0.45} />
            <stop offset="100%" stopColor="var(--tg-err)" stopOpacity={0} />
          </radialGradient>
          <radialGradient id={slowGlowId}>
            <stop offset="0%" stopColor="var(--tg-slow)" stopOpacity={0.35} />
            <stop offset="100%" stopColor="var(--tg-slow)" stopOpacity={0} />
          </radialGradient>
        </defs>
        {layout.edges.map((e) => (
          <path
            key={`${e.parent}>${e.child}`}
            d={e.d}
            fill="none"
            className={e.failing ? 'tg-flow stroke-err' : 'stroke-edge'}
            strokeWidth={e.failing ? 2.2 : 1.4}
            strokeDasharray={e.failing ? '6 5' : undefined}
          />
        ))}
        {layout.nodes
          .filter((n) => n.health !== 'ok')
          .map((n) => (
            <circle
              key={`glow-${n.service}`}
              cx={n.x}
              cy={n.y}
              r={MINI.r * 3.6}
              fill={`url(#${n.health === 'error' ? glowId : slowGlowId})`}
            />
          ))}
        {layout.nodes.map((n) => (
          <g key={n.service}>
            <title>{`${n.service}${n.health === 'ok' ? '' : ` (${n.health})`}`}</title>
            <circle
              cx={n.x}
              cy={n.y}
              r={MINI.r}
              strokeWidth={n.health === 'ok' ? 1.5 : 2}
              className={cx(
                n.health === 'error' && 'fill-err-soft stroke-err',
                n.health === 'slow' && 'fill-slow-soft stroke-slow',
                n.health === 'ok' && 'fill-node stroke-node-line',
              )}
            />
            <text
              x={n.x}
              y={n.y + MINI.labelGap}
              textAnchor="middle"
              fontSize={10.5}
              className={cx(
                'font-sans',
                n.health === 'error' ? 'fill-err' : n.health === 'slow' ? 'fill-slow' : 'fill-ink-2',
              )}
            >
              {label(n.service)}
            </text>
          </g>
        ))}
      </svg>
      <span className="mt-1 inline-flex items-center gap-1 text-xs text-accent group-hover:underline">
        Open service map <ArrowRight aria-hidden size={12} />
      </span>
    </Link>
  )
}
