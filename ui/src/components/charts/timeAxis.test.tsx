import { render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import type { EChartProps } from './EChart'
import { Scatter } from './Scatter'
import { TimeSeries } from './TimeSeries'
import { timeAxisLabel } from './timeAxis'

let last: EChartProps | undefined
vi.mock('./EChartImpl', () => ({
  default: (p: EChartProps) => {
    last = p
    return <div data-testid="chart" />
  },
}))

describe('timeAxisLabel', () => {
  it('names the day and prints clock times, with the day start in bold', () => {
    const l = timeAxisLabel()
    expect(l.hideOverlap).toBe(true)
    expect(l.formatter.day).toEqual(['{MMM} {d}', '{primary|{MMM} {d}}'])
    expect(l.formatter.hour[0]).toBe('{HH}:{mm}')
    // A day boundary on an hourly axis is the emphasised variant, never a bare number.
    expect(l.formatter.hour[1]).toBe('{primary|{MMM} {d}}')
    expect(l.formatter.second[0]).toBe('{HH}:{mm}:{ss}')
    expect(l.rich.primary.fontWeight).toBe('bold')
  })

  it('is used by the time series and the scatter', async () => {
    const axis = (o: unknown) => (o as { xAxis: { axisLabel: unknown } }).xAxis.axisLabel
    render(<TimeSeries series={[{ name: 's', points: [[1, 1]] }]} summary="series" />)
    await screen.findByTestId('chart')
    expect(axis(last!.option)).toEqual(timeAxisLabel())
    last = undefined
    render(
      <Scatter
        points={[{ x: 1, y: 1, tone: 'accent', id: 'a' }]}
        names={{ accent: 'a', slow: 's', err: 'e' }}
        formatY={String}
        tooltip={() => ({ title: '', lines: [] })}
        summary="scatter"
      />,
    )
    await screen.findAllByTestId('chart')
    expect(axis(last!.option)).toEqual(timeAxisLabel())
  })
})
