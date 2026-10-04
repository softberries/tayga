/**
 * Service card on the map: a health ring, the name, and rate / error % / p99 for the window.
 * Degraded services glow and show a pulsing dot (static under reduced motion). The card is a
 * real button that opens the service drawer, so the map is keyboard reachable.
 */
import { Handle, Position, useStore } from '@xyflow/react'
import type { Node, NodeProps } from '@xyflow/react'
import { memo } from 'react'
import type { Health, NodeView } from '../../api/types'
import { cx } from '../../lib/cx'
import { duration } from '../../lib/format'
import { NODE_H, NODE_W } from './layout'
import { HEALTH_COLOR, errText, rateText } from './model'

export interface ServiceNodeData extends Record<string, unknown> {
  service: string
  /** Absent for services seen only as callers (no server spans of their own). */
  view: NodeView | null
  /** Matches the search box. */
  match: boolean
  /** A search is active and this service does not match it. */
  dimmed: boolean
  /** Its drawer is open. */
  active: boolean
  onOpen: (service: string) => void
  /** Keyboard focus reached the card: bring it into view. */
  onFocusCard: (service: string) => void
}

export type ServiceNodeType = Node<ServiceNodeData, 'service'>

const HEALTH_WORD: Record<Health, string> = { ok: 'healthy', slow: 'slow', error: 'errors' }

export function nodeLabel(service: string, view: NodeView | null): string {
  if (!view) return `${service}: no server spans in this window. Open details.`
  return (
    `${service}, ${HEALTH_WORD[view.health]}: ${rateText(view.rate)} calls, ` +
    `${errText(view.error_ratio)} errors, p99 ${duration(view.p99_ns)}. Open details.`
  )
}

/**
 * Ring: the health color, with an err arc for the share of failed calls. On a failing service
 * the ring itself is the neutral track, so the arc stays visible.
 */
export function HealthRing({ health, errorRatio, size = 30 }: { health: Health; errorRatio: number; size?: number }) {
  const r = size / 2 - 3
  const c = 2 * Math.PI * r
  const arc = errorRatio > 0 ? Math.max(0.04, Math.min(1, errorRatio)) * c : 0
  return (
    <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} aria-hidden className="shrink-0">
      <circle
        cx={size / 2}
        cy={size / 2}
        r={r}
        strokeWidth={health === 'ok' ? 2 : health === 'error' ? 3.5 : 2.5}
        style={{ stroke: health === 'error' ? 'var(--tg-track)' : HEALTH_COLOR[health] }}
        className={cx(health === 'error' ? 'fill-err-soft' : health === 'slow' ? 'fill-slow-soft' : 'fill-node')}
      />
      {arc > 0 ? (
        <circle
          cx={size / 2}
          cy={size / 2}
          r={r}
          fill="none"
          strokeWidth={3.5}
          strokeLinecap="round"
          strokeDasharray={`${arc} ${c}`}
          transform={`rotate(-90 ${size / 2} ${size / 2})`}
          style={{ stroke: 'var(--tg-err)' }}
        />
      ) : null}
    </svg>
  )
}

/** Below this zoom the card switches to its compact form: bigger name, one telling metric. */
export const COMPACT_ZOOM = 0.85

/**
 * Compact name size in px, from its longest hyphen-delimited word: names wrap only at
 * hyphens, so the longest word must fit one line (18 px up to 12 characters, 15 px up to 15,
 * else 13 px).
 */
export function compactNameSize(service: string): number {
  const longest = Math.max(0, ...service.split('-').map((w) => w.length))
  return longest <= 12 ? 18 : longest <= 15 ? 15 : 13
}

/** The metric that explains a degraded service's health. */
function keyMetric(view: NodeView): string | null {
  if (view.health === 'error') return `${errText(view.error_ratio)} err`
  if (view.health === 'slow') return `p99 ${duration(view.p99_ns)}`
  return null
}

function ServiceNodeImpl({ data }: NodeProps<ServiceNodeType>) {
  const { service, view, match, dimmed, active, onOpen, onFocusCard } = data
  const health: Health = view?.health ?? 'ok'
  const degraded = health !== 'ok'
  // A boolean selector: cards re-render only when the zoom crosses the threshold.
  const compact = useStore((s) => s.transform[2] < COMPACT_ZOOM)
  const metric = view ? keyMetric(view) : null
  return (
    <>
      <Handle type="target" position={Position.Left} isConnectable={false} className="tg-map-handle" />
      <button
        type="button"
        data-service={service}
        data-health={health}
        data-compact={compact || undefined}
        aria-label={nodeLabel(service, view)}
        aria-current={active ? 'true' : undefined}
        onClick={() => onOpen(service)}
        onFocus={() => onFocusCard(service)}
        style={{ width: NODE_W, height: NODE_H }}
        className={cx(
          'tg-map-node relative flex cursor-pointer items-center rounded-card border bg-panel text-left text-ink shadow-panel',
          'transition-[opacity,box-shadow,border-color] duration-200 hover:border-accent',
          compact ? 'gap-2 px-2.5' : 'gap-2.5 px-2.5',
          health === 'error' && 'tg-map-node-err border-err',
          health === 'slow' && 'tg-map-node-slow border-slow',
          health === 'ok' && 'border-panel-line',
          match && 'tg-map-node-match border-accent',
          active && 'outline-2 outline-offset-2 outline-accent',
          dimmed && 'opacity-35',
        )}
      >
        <HealthRing health={health} errorRatio={view?.error_ratio ?? 0} size={compact ? 24 : 28} />
        {compact ? (
          <span className="flex min-w-0 flex-1 flex-col gap-0.5">
            <span
              className="line-clamp-2 pr-3 font-semibold leading-[1.15]"
              style={{ fontSize: compactNameSize(service) }}
              title={service}
            >
              {service}
            </span>
            {metric ? (
              <span className={cx('tabular truncate font-mono text-[15px] leading-tight', health === 'error' ? 'text-err' : 'text-slow')}>
                {metric}
              </span>
            ) : null}
          </span>
        ) : (
          <span className="flex min-w-0 flex-1 flex-col gap-0.5">
            <span className="truncate text-[13px] font-semibold leading-tight" title={service}>
              {service}
            </span>
            {view ? (
              <span className="tabular flex flex-col font-mono text-[11px] leading-[1.35] text-muted">
                <span className="truncate">
                  <span title="Calls per second">{rateText(view.rate)}</span>
                  {' · '}
                  <span title="Error ratio" className={cx(view.error_ratio > 0 && 'text-err')}>
                    {errText(view.error_ratio)} err
                  </span>
                </span>
                <span title="p99 latency" className={cx('truncate', health === 'slow' && 'text-slow')}>
                  p99 {duration(view.p99_ns)}
                </span>
              </span>
            ) : (
              <span className="text-[11px] leading-tight text-muted">caller only · no server spans</span>
            )}
          </span>
        )}
        {degraded ? (
          <span
            aria-hidden
            className={cx(
              'absolute right-2 top-2 size-2 rounded-full',
              health === 'error' ? 'tg-pulse bg-err' : 'tg-pulse-slow bg-slow',
            )}
          />
        ) : null}
      </button>
      <Handle type="source" position={Position.Right} isConnectable={false} className="tg-map-handle" />
    </>
  )
}

export const ServiceNode = memo(ServiceNodeImpl)
