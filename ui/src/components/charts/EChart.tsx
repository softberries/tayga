import { Suspense, lazy, useMemo } from 'react'
import { echartsTheme } from '../../theme/echartsTheme'
import { useAppliedTheme } from '../../theme/useAppliedTheme'
import { Skeleton } from '../ui/Skeleton'

const Impl = lazy(() => import('./EChartImpl'))

export interface EChartProps {
  /** An ECharts option object (echarts-for-react types it as `any`). */
  option: Record<string, unknown>
  height: number
  /** ECharts event handlers, e.g. `{ brushSelected: fn }`. */
  onEvents?: Record<string, (params: unknown) => void>
  /** Text alternative for screen readers (spec §5: every chart has a text summary). */
  summary: string
}

/**
 * Lazily loaded, themed ECharts canvas. The theme object is rebuilt from the CSS variables
 * whenever `<html data-theme>` changes; echarts-for-react disposes and re-creates the chart
 * when the theme prop changes, so colors follow the theme switch.
 */
export function EChart({ option, height, onEvents, summary }: EChartProps) {
  const applied = useAppliedTheme()
  // `applied` is the cache key: the theme object reads the variables of the applied theme.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const theme = useMemo(() => echartsTheme(), [applied])
  return (
    <figure className="m-0">
      <figcaption className="sr-only">{summary}</figcaption>
      <div aria-hidden>
        <Suspense fallback={<Skeleton style={{ height }} />}>
          <Impl option={option} theme={theme} height={height} onEvents={onEvents} summary={summary} />
        </Suspense>
      </div>
    </figure>
  )
}
