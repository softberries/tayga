/**
 * Themed ECharts scatter on a time axis with an optional log y axis, rectangle brush selection
 * and point clicks. The selection is controlled: the brush always draws `selection`, and a
 * finished user brush (ECharts `brushEnd`, which programmatic brushing does not emit, so there
 * is no feedback loop) reports a new rectangle, or null when the user clears it.
 */
import type { EChartsType } from 'echarts/core'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { REDUCED_MOTION_QUERY, useMediaQuery } from '../../lib/useMediaQuery'
import { readChartTokens, withAlpha } from '../../theme/echartsTheme'
import { useAppliedTheme } from '../../theme/useAppliedTheme'
import { EChart } from './EChart'
import { timeAxisLabel } from './timeAxis'

export type ScatterTone = 'accent' | 'slow' | 'err'
/** Drawing order: errors last, so they sit on top. */
const TONES: readonly ScatterTone[] = ['accent', 'slow', 'err']

export interface ScatterPoint {
  /** Unix ms. */
  x: number
  y: number
  tone: ScatterTone
  /** Passed back to `onPointClick` and `tooltip`. */
  id: string
}

/** A brushed rectangle in data coordinates: `x` (unix ms) and `y` ranges, each `[min, max]`. */
export interface XYRect {
  x: [number, number]
  y: [number, number]
}

export interface ScatterProps {
  points: readonly ScatterPoint[]
  /** Series names by tone (legend-free; used by the tooltip and screen-reader summary). */
  names: Record<ScatterTone, string>
  logY?: boolean
  /** Fixed x-axis extent (unix ms), e.g. the selected time range. */
  xRange?: [number, number]
  formatY: (v: number) => string
  /** Tooltip for a point: a title and detail lines (plain text; escaped here). */
  tooltip: (id: string) => { title: string; lines: string[] }
  selection?: XYRect | null
  onSelect?: (rect: XYRect | null) => void
  onPointClick?: (id: string) => void
  height?: number
  summary: string
}

const pair = (v: unknown): [number, number] | null =>
  Array.isArray(v) && v.length === 2 && v.every((n) => typeof n === 'number' && Number.isFinite(n))
    ? [Math.min(v[0] as number, v[1] as number), Math.max(v[0] as number, v[1] as number)]
    : null

/**
 * The rectangle of an ECharts `brushEnd` (or `brushSelected` batch item) payload: the first
 * rect area's `coordRange` `[[xMin, xMax], [yMin, yMax]]`. Null when the brush was cleared
 * or the payload has no usable rect.
 */
export function rectFromBrush(params: unknown): XYRect | null {
  if (!params || typeof params !== 'object') return null
  const areas = (params as { areas?: unknown }).areas
  if (!Array.isArray(areas)) return null
  for (const a of areas as Array<{ brushType?: unknown; coordRange?: unknown }>) {
    if (a?.brushType !== 'rect' || !Array.isArray(a.coordRange)) continue
    const x = pair(a.coordRange[0])
    const y = pair(a.coordRange[1])
    if (x && y) return { x, y }
  }
  return null
}

const ESC: Record<string, string> = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }
/** Tooltip HTML is built from API strings (endpoint names): escape every one. */
export const escapeHtml = (s: string) => s.replace(/[&<>"']/g, (c) => ESC[c] ?? c)

export function Scatter({
  points,
  names,
  logY = false,
  xRange,
  formatY,
  tooltip,
  selection = null,
  onSelect,
  onPointClick,
  height = 260,
  summary,
}: ScatterProps) {
  const applied = useAppliedTheme()
  const reduceMotion = useMediaQuery(REDUCED_MOTION_QUERY)

  // Handlers through refs: the onEvents object stays stable, so ECharts never re-binds.
  const handlers = useRef({ onSelect, onPointClick, tooltip })
  useEffect(() => {
    handlers.current = { onSelect, onPointClick, tooltip }
  })

  const option = useMemo(() => {
    const tokens = applied ? readChartTokens() : null
    const color: Record<ScatterTone, string | undefined> = {
      accent: tokens?.accent,
      slow: tokens?.slow,
      err: tokens?.err,
    }
    return {
      animation: !reduceMotion,
      animationDuration: 300,
      grid: { left: 8, right: 16, top: 14, bottom: 4, containLabel: true },
      tooltip: {
        trigger: 'item',
        confine: true,
        // Long endpoint names wrap: a nowrap tooltip wider than a phone's chart overflows the page.
        extraCssText: 'max-width: min(320px, 80vw); white-space: normal; overflow-wrap: anywhere;',
        formatter: (p: { data?: { id?: string } }) => {
          const id = p.data?.id
          if (!id) return ''
          const t = handlers.current.tooltip(id)
          return [`<b>${escapeHtml(t.title)}</b>`, ...t.lines.map(escapeHtml)].join('<br/>')
        },
      },
      xAxis: {
        type: 'time',
        min: xRange?.[0],
        max: xRange?.[1],
        axisLabel: timeAxisLabel(),
      },
      yAxis: logY
        ? { type: 'log', logBase: 10, axisLabel: { formatter: formatY } }
        : { type: 'value', min: 0, axisLabel: { formatter: formatY } },
      // The brush component always adds toolbox buttons; the selection controls live in the page.
      toolbox: { show: false },
      brush: {
        xAxisIndex: 0,
        brushType: 'rect',
        brushMode: 'single',
        transformable: true,
        removeOnClick: true,
        outOfBrush: { opacity: 0.22 },
      },
      series: TONES.map((tone) => ({
        name: names[tone],
        type: 'scatter',
        symbolSize: tone === 'accent' ? 6 : 7,
        itemStyle: color[tone]
          ? { color: withAlpha(color[tone], tone === 'accent' ? 0.7 : 0.9), borderColor: color[tone], borderWidth: 0.5 }
          : undefined,
        emphasis: { scale: 1.6 },
        data: points.filter((p) => p.tone === tone).map((p) => ({ value: [p.x, p.y], id: p.id })),
      })),
    }
  }, [points, names, logY, xRange, formatY, applied, reduceMotion])

  const onEvents = useMemo(
    () => ({
      brushEnd: (p: unknown) => handlers.current.onSelect?.(rectFromBrush(p)),
      click: (p: unknown) => {
        const id = (p as { data?: { id?: unknown } }).data?.id
        if (typeof id === 'string') handlers.current.onPointClick?.(id)
      },
    }),
    [],
  )

  const [chart, setChart] = useState<EChartsType | null>(null)
  const onReady = useCallback((c: EChartsType) => setChart(c), [])
  // Every setOption (notMerge) and every new instance drops the brush state: re-arm the
  // rectangle brush and redraw the controlled selection.
  useEffect(() => {
    if (!chart || chart.isDisposed()) return
    if (onSelect) {
      chart.dispatchAction({ type: 'takeGlobalCursor', key: 'brush', brushOption: { brushType: 'rect', brushMode: 'single' } })
    }
    chart.dispatchAction({
      type: 'brush',
      areas: selection
        ? [{ brushType: 'rect', xAxisIndex: 0, yAxisIndex: 0, coordRange: [selection.x, selection.y] }]
        : [],
    })
  }, [chart, option, selection, onSelect])

  return <EChart option={option} height={height} onEvents={onEvents} onReady={onReady} summary={summary} />
}
