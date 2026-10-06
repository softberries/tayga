import { useMemo } from 'react'
import { readChartTokens, withAlpha } from '../../theme/echartsTheme'
import { REDUCED_MOTION_QUERY, useMediaQuery } from '../../lib/useMediaQuery'
import { useAppliedTheme } from '../../theme/useAppliedTheme'
import { EChart } from './EChart'
import { timeAxisLabel } from './timeAxis'

export type SeriesTone = 'accent' | 'err' | 'slow' | 'ok' | 'silence'

export interface TimeSeriesSeries {
  name: string
  /** `[time in unix ms, value]`; null values leave a gap. */
  points: ReadonlyArray<readonly [number, number | null]>
  /** Semantic color; defaults to the palette order. */
  tone?: SeriesTone
  /** `line` (default), `area` (line with a soft fill) or `bar`. */
  type?: 'line' | 'area' | 'bar'
}

export interface TimeSeriesProps {
  series: readonly TimeSeriesSeries[]
  height?: number
  /** Vertical marker, e.g. the story's own time (unix ms). */
  markAt?: number
  markLabel?: string
  /** Y-axis and tooltip value formatter. */
  format?: (v: number) => string
  /** Screen-reader summary of what the chart shows. */
  summary: string
  /** Smallest y-axis step (default 1, for counts); 0 lets fractional values get their own ticks. */
  minInterval?: number
  /** Approximate number of y-axis steps (ECharts default 5); fewer suits small charts. */
  splitNumber?: number
  /** Stack the series on top of each other (bars; the first series sits at the bottom). */
  stack?: boolean
  /** Smallest x extent as `[from, to]` unix ms, so a sparse series still spans its window. */
  xRange?: readonly [number, number]
  /** Shows a legend (colored dots with series names) above the plot, for charts with several lines. */
  legend?: boolean
}

/** Rough number of legend lines (each name is a dot, a gap and ~6.5 px per character), for the room above the plot. */
function legendRows(series: readonly TimeSeriesSeries[], viewport: number): number {
  const width = series.reduce((w, s) => w + 27 + s.name.length * 6.5, 0)
  const room = viewport < 640 ? viewport - 80 : 600
  return Math.max(1, Math.ceil(width / room))
}

/** Shared time-series chart (line, area or bar) on a time axis, themed via echartsTheme. */
export function TimeSeries({ series, height = 180, markAt, markLabel, format, summary, minInterval = 1, splitNumber, stack, xRange, legend }: TimeSeriesProps) {
  const applied = useAppliedTheme()
  const reduceMotion = useMediaQuery(REDUCED_MOTION_QUERY)
  const option = useMemo(() => {
    // Canvas needs real colors: resolve semantic tones from the applied theme's variables.
    const tokens = applied ? readChartTokens() : null
    const fmt = format ?? ((v: number) => String(v))
    return {
      // Spec §5: reduced motion turns every animation off, charts included.
      animation: !reduceMotion,
      animationDuration: 300,
      grid: { left: 8, right: 12, top: legend ? 14 + 20 * legendRows(series, window.innerWidth) : 16, bottom: 4, containLabel: true },
      ...(legend
        ? { legend: { show: true, top: 0, left: 0, icon: 'circle', itemWidth: 8, itemHeight: 8, itemGap: 14, selectedMode: false, textStyle: { fontSize: 11.5 } } }
        : {}),
      tooltip: { trigger: 'axis', valueFormatter: (v: unknown) => (typeof v === 'number' ? fmt(v) : '—') },
      xAxis: { type: 'time', axisLabel: timeAxisLabel(), ...(xRange ? { min: xRange[0], max: xRange[1] } : {}) },
      yAxis: {
        type: 'value',
        minInterval,
        ...(splitNumber ? { splitNumber } : {}),
        axisLabel: { formatter: (v: number) => fmt(v) },
      },
      series: series.map((s, i) => {
        const color = s.tone && tokens ? tokens[s.tone] : undefined
        return {
          name: s.name,
          type: s.type === 'bar' ? 'bar' : 'line',
          data: s.points.map(([t, v]) => [t, v]),
          ...(color ? { itemStyle: { color }, lineStyle: { color } } : {}),
          showSymbol: false,
          ...(stack ? { stack: 'total' } : {}),
          barMaxWidth: 18,
          areaStyle: s.type === 'area' ? (color ? { color: withAlpha(color, 0.16) } : { opacity: 0.16 }) : undefined,
          markLine:
            i === 0 && markAt !== undefined
              ? {
                  symbol: 'none',
                  silent: true,
                  label: { formatter: markLabel ?? '', position: 'insideEndTop', color: tokens?.muted },
                  lineStyle: { type: 'dashed', color: tokens?.faint },
                  data: [{ xAxis: markAt }],
                }
              : undefined,
        }
      }),
    }
  }, [series, markAt, markLabel, format, applied, reduceMotion, minInterval, splitNumber, stack, xRange, legend])
  return <EChart option={option} height={height} summary={summary} />
}
