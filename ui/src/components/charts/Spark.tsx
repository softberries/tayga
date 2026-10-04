import { cx } from '../../lib/cx'

export type SparkTone = 'err' | 'slow' | 'accent' | 'ok'

const toneClass: Record<SparkTone, string> = {
  err: 'text-err',
  slow: 'text-slow',
  accent: 'text-accent',
  ok: 'text-ok',
}

/**
 * Polyline points for `values` across a `w`×`h` box, the largest value 2 px from the top and
 * zero 2 px above the bottom (the design board's scale). One value draws a flat line.
 */
export function sparkPoints(values: readonly number[], w: number, h: number): string {
  const clean = (v: number | undefined) => (v !== undefined && Number.isFinite(v) ? Math.max(0, v) : 0)
  const vs = values.length === 0 ? [0, 0] : values.length === 1 ? [values[0], values[0]] : values
  const max = Math.max(1, ...vs.map(clean))
  const dx = w / (vs.length - 1)
  return vs
    .map((v, i) => {
      const y = h - 2 - (clean(v) / max) * (h - 4)
      return `${(i * dx).toFixed(1)},${y.toFixed(1)}`
    })
    .join(' ')
}

export interface SparkProps {
  values: readonly number[]
  tone: SparkTone
  /** Drawing box (viewBox); the SVG stretches to its container's width. */
  width?: number
  height?: number
  /** Screen-reader summary; without it the sparkline is decorative and hidden. */
  label?: string
  className?: string
}

/** Area sparkline (KPI tiles, table rows): a soft fill under a 1.5 px line, in token colors. */
export function Spark({ values, tone, width = 120, height = 28, label, className }: SparkProps) {
  const line = sparkPoints(values, width, height)
  const area = `0,${height} ${line} ${width},${height}`
  return (
    <svg
      className={cx('block', toneClass[tone], className)}
      width="100%"
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      preserveAspectRatio="none"
      role={label ? 'img' : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
      focusable="false"
    >
      <polygon points={area} fill="currentColor" fillOpacity={0.14} />
      <polyline
        points={line}
        fill="none"
        stroke="currentColor"
        strokeWidth={1.5}
        strokeLinejoin="round"
        vectorEffect="non-scaling-stroke"
      />
    </svg>
  )
}
