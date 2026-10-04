/**
 * The only module that imports ECharts. It is loaded lazily by ./EChart, so ECharts lands in
 * its own chunk and stays out of the initial bundle (spec §10). Register further chart types
 * and components here as pages need them.
 */
import ReactEChartsCore from 'echarts-for-react/esm/core'
import { BarChart, LineChart } from 'echarts/charts'
import { GridComponent, MarkLineComponent, TooltipComponent } from 'echarts/components'
import * as echarts from 'echarts/core'
import { CanvasRenderer } from 'echarts/renderers'
import type { EChartProps } from './EChart'

echarts.use([LineChart, BarChart, GridComponent, TooltipComponent, MarkLineComponent, CanvasRenderer])

export default function EChartImpl({ option, theme, height, onEvents }: EChartProps & { theme: object }) {
  return (
    <ReactEChartsCore
      echarts={echarts}
      option={option}
      theme={theme}
      notMerge
      onEvents={onEvents}
      style={{ height, width: '100%' }}
      opts={{ renderer: 'canvas' }}
    />
  )
}
