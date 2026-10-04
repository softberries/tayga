/**
 * The only module that imports ECharts. It is loaded lazily by ./EChart, so ECharts lands in
 * its own chunk and stays out of the initial bundle (spec §10). Register further chart types
 * and components here as pages need them.
 */
import ReactEChartsCore from 'echarts-for-react/esm/core'
import { BarChart, LineChart, ScatterChart } from 'echarts/charts'
import { BrushComponent, GridComponent, LegendComponent, MarkLineComponent, ToolboxComponent, TooltipComponent } from 'echarts/components'
import * as echarts from 'echarts/core'
import { CanvasRenderer } from 'echarts/renderers'
import type { EChartProps } from './EChart'

echarts.use([
  LineChart,
  BarChart,
  ScatterChart,
  GridComponent,
  LegendComponent,
  TooltipComponent,
  MarkLineComponent,
  BrushComponent,
  // The brush preprocessor always adds a toolbox to the option; charts hide it (`show: false`).
  ToolboxComponent,
  CanvasRenderer,
])

export default function EChartImpl({ option, theme, height, onEvents, onReady }: EChartProps & { theme: object }) {
  return (
    <ReactEChartsCore
      echarts={echarts}
      option={option}
      theme={theme}
      notMerge
      onEvents={onEvents}
      onChartReady={onReady}
      style={{ height, width: '100%' }}
      opts={{ renderer: 'canvas' }}
    />
  )
}
