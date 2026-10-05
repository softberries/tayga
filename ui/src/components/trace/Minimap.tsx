import { useEffect, useRef, useState } from 'react'
import type { KeyboardEvent, PointerEvent } from 'react'
import { useAppliedTheme } from '../../theme/useAppliedTheme'
import type { LayoutRow } from './layout'

export type ZoomWindow = readonly [from: number, to: number]

export const FULL_VIEW: ZoomWindow = [0, 1]
/** Narrowest zoom window, as a fraction of the trace. */
const MIN_ZOOM = 0.002

export interface MinimapProps {
  rows: readonly LayoutRow[]
  view: ZoomWindow
  onViewChange: (view: ZoomWindow) => void
  height?: number
}

const clamp01 = (v: number) => Math.min(1, Math.max(0, v))

/**
 * Overview of every bar (canvas) with a brush: drag across it to zoom the waterfall to that
 * time window. Arrow keys pan the window, +/- zoom, Escape resets.
 */
export function Minimap({ rows, view, onViewChange, height = 44 }: MinimapProps) {
  const wrap = useRef<HTMLDivElement>(null)
  const canvas = useRef<HTMLCanvasElement>(null)
  const theme = useAppliedTheme()
  const [width, setWidth] = useState(0)
  const [brush, setBrush] = useState<{ a: number; b: number } | null>(null)

  useEffect(() => {
    const el = wrap.current
    if (!el) return
    setWidth(el.clientWidth)
    const ro = new ResizeObserver(() => setWidth(el.clientWidth))
    ro.observe(el)
    return () => ro.disconnect()
  }, [])

  useEffect(() => {
    const c = canvas.current
    const ctx = width > 0 ? c?.getContext('2d') : null
    if (!c || !ctx) return
    const dpr = window.devicePixelRatio || 1
    c.width = Math.round(width * dpr)
    c.height = Math.round(height * dpr)
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
    const css = getComputedStyle(document.documentElement)
    const color = (n: string) => css.getPropertyValue(`--tg-${n}`).trim()
    ctx.clearRect(0, 0, width, height)
    const n = Math.max(1, rows.length)
    const rowH = (height - 4) / n
    const lineH = Math.max(1, Math.min(4, rowH - (rowH > 3 ? 1 : 0)))
    const draw = (pick: (r: LayoutRow) => boolean, fill: string) => {
      ctx.fillStyle = fill
      for (const r of rows) {
        if (!pick(r)) continue
        ctx.fillRect(r.left * width, 2 + r.index * rowH, Math.max(1, r.width * width), lineH)
      }
    }
    // Layer by importance so errors and the critical path stay visible when rows overlap.
    draw((r) => !r.critical && !r.error, color('node-line'))
    draw((r) => r.critical && !r.error, color('accent'))
    draw((r) => r.error, color('err'))
  }, [rows, width, height, theme])

  const toFrac = (clientX: number) => {
    const rect = wrap.current?.getBoundingClientRect()
    return rect && rect.width > 0 ? clamp01((clientX - rect.left) / rect.width) : 0
  }
  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return
    e.currentTarget.setPointerCapture(e.pointerId)
    const f = toFrac(e.clientX)
    setBrush({ a: f, b: f })
  }
  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    if (brush) setBrush({ a: brush.a, b: toFrac(e.clientX) })
  }
  const onPointerUp = (e: PointerEvent<HTMLDivElement>) => {
    if (!brush) return
    e.currentTarget.releasePointerCapture(e.pointerId)
    const from = Math.min(brush.a, brush.b)
    const to = Math.max(brush.a, brush.b)
    setBrush(null)
    // A click (no drag) is not a zoom.
    if (to - from >= 0.005) onViewChange([from, Math.max(to, from + MIN_ZOOM)])
  }
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const [from, to] = view
    const w = to - from
    const pan = (d: number) => {
      const f = Math.min(1 - w, Math.max(0, from + d))
      onViewChange([f, f + w])
    }
    const zoom = (k: number) => {
      const mid = (from + to) / 2
      const nw = Math.min(1, Math.max(MIN_ZOOM, w * k))
      const f = Math.min(1 - nw, Math.max(0, mid - nw / 2))
      onViewChange([f, f + nw])
    }
    if (e.key === 'ArrowLeft') pan(-w / 4)
    else if (e.key === 'ArrowRight') pan(w / 4)
    else if (e.key === '+' || e.key === '=') zoom(0.5)
    else if (e.key === '-' || e.key === '_') zoom(2)
    else if (e.key === 'Escape' || e.key === '0') onViewChange(FULL_VIEW)
    else return
    e.preventDefault()
  }

  const zoomed = view[0] > 0 || view[1] < 1
  const sel: [number, number] | null = brush ? [Math.min(brush.a, brush.b), Math.max(brush.a, brush.b)] : null
  return (
    <div
      ref={wrap}
      role="group"
      tabIndex={0}
      aria-label="Trace overview. Drag to zoom the time axis; arrow keys pan, plus and minus zoom, Escape resets."
      className="relative cursor-crosshair touch-none select-none overflow-hidden rounded-control border border-panel-line bg-inner"
      style={{ height }}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={() => setBrush(null)}
      onKeyDown={onKeyDown}
    >
      <canvas ref={canvas} aria-hidden className="absolute inset-0 h-full w-full" />
      {zoomed ? (
        <>
          {/* Dim what lies outside the zoom window. */}
          <div aria-hidden className="absolute inset-y-0 left-0 bg-ground/60" style={{ width: `${view[0] * 100}%` }} />
          <div aria-hidden className="absolute inset-y-0 right-0 bg-ground/60" style={{ width: `${(1 - view[1]) * 100}%` }} />
          <div
            aria-hidden
            className="absolute inset-y-0 rounded-[3px] border border-accent"
            style={{ left: `${view[0] * 100}%`, width: `${(view[1] - view[0]) * 100}%` }}
          />
        </>
      ) : null}
      {sel ? (
        <div
          aria-hidden
          className="absolute inset-y-0 border-x border-accent bg-accent/15"
          style={{ left: `${sel[0] * 100}%`, width: `${(sel[1] - sel[0]) * 100}%` }}
        />
      ) : null}
    </div>
  )
}
