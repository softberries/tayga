/**
 * Call edge: a bezier whose width follows calls/min and whose color follows the error rate.
 * Failing calls are dashed and flow (CSS `stroke-dashoffset`; static under reduced motion).
 * Hovering shows calls, error % and average latency; clicking pins that label.
 */
import { BaseEdge, EdgeLabelRenderer, getBezierPath, useStore } from '@xyflow/react'
import type { Edge, EdgeProps } from '@xyflow/react'
import { memo, useState } from 'react'
import type { EdgeView } from '../../api/types'
import { cx } from '../../lib/cx'
import { compact, duration } from '../../lib/format'
import { TONE_STROKE, errText } from './model'
import type { EdgeTone } from './model'

export interface ServiceEdgeData extends Record<string, unknown> {
  edge: EdgeView
  perMin: number
  width: number
  tone: EdgeTone
  /** Label pinned open by a click. */
  pinned: boolean
  /** A search is active and neither end matches it. */
  dimmed: boolean
}

export type ServiceEdgeType = Edge<ServiceEdgeData, 'service'>

export function edgeLabel(e: EdgeView, perMin: number): string {
  const rate = perMin >= 1 ? compact(perMin) : '<1'
  return `${compact(e.calls)} calls · ${rate}/min · ${errText(e.error_rate)} err · avg ${duration(e.avg_duration_ns)}`
}

function ServiceEdgeImpl({ id, sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, data }: EdgeProps<ServiceEdgeType>) {
  const [hover, setHover] = useState(false)
  // The label keeps its size at any zoom.
  const zoom = useStore((s) => s.transform[2])
  const [path, labelX, labelY] = getBezierPath({ sourceX, sourceY, sourcePosition, targetX, targetY, targetPosition })
  if (!data) return null
  const failing = data.tone === 'err'
  const show = hover || data.pinned
  return (
    <>
      <g
        onMouseEnter={() => setHover(true)}
        onMouseLeave={() => setHover(false)}
        className={cx('transition-opacity duration-200', data.dimmed && 'opacity-25')}
        data-tone={data.tone}
      >
        <BaseEdge
          id={id}
          path={path}
          interactionWidth={18}
          className={cx(failing && 'tg-flow')}
          style={{
            stroke: TONE_STROKE[data.tone],
            strokeWidth: show ? data.width + 1 : data.width,
            strokeDasharray: failing ? '7 5' : undefined,
            strokeLinecap: 'round',
          }}
        />
      </g>
      {show ? (
        <EdgeLabelRenderer>
          <div
            role="tooltip"
            className="tg-map-label pointer-events-none absolute z-10 whitespace-nowrap rounded-control border border-panel-line bg-panel px-2 py-1 font-mono text-[11px] text-ink shadow-panel"
            style={{ transform: `translate(${labelX}px, ${labelY}px) translate(-50%, -50%) scale(${1 / zoom})` }}
          >
            <span className="text-muted">
              {data.edge.parent} → {data.edge.child}
            </span>
            <br />
            <span className={cx(data.tone !== 'ok' && 'text-err')}>{edgeLabel(data.edge, data.perMin)}</span>
          </div>
        </EdgeLabelRenderer>
      ) : null}
    </>
  )
}

export const ServiceEdge = memo(ServiceEdgeImpl)
