import { act, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import type { EChartProps } from './EChart'
import { Scatter, escapeHtml, rectFromBrush } from './Scatter'
import type { ScatterPoint } from './Scatter'

let last: EChartProps | undefined
vi.mock('./EChartImpl', () => ({
  default: (p: EChartProps) => {
    last = p
    return <div data-testid="chart" />
  },
}))

const points: ScatterPoint[] = [
  { x: 1000, y: 50, tone: 'accent', id: 'a' },
  { x: 2000, y: 5000, tone: 'err', id: 'b' },
]
const names = { accent: 'Traces', slow: 'Slow', err: 'Errors' }

function fakeChart() {
  return { dispatchAction: vi.fn(), isDisposed: () => false }
}

describe('rectFromBrush', () => {
  it('reads the first rect coordRange, normalized', () => {
    expect(rectFromBrush({ areas: [{ brushType: 'rect', coordRange: [[2000, 1000], [60, 40]] }] })).toEqual({ x: [1000, 2000], y: [40, 60] })
  })
  it('is null for a cleared brush or unusable payloads', () => {
    expect(rectFromBrush({ areas: [] })).toBeNull()
    expect(rectFromBrush({ areas: [{ brushType: 'polygon', coordRange: [[1, 2], [3, 4]] }] })).toBeNull()
    expect(rectFromBrush({ areas: [{ brushType: 'rect', coordRange: [[1, Number.NaN], [3, 4]] }] })).toBeNull()
    expect(rectFromBrush(null)).toBeNull()
  })
})

describe('Scatter', () => {
  it('splits points into series by tone, with a log axis on request', async () => {
    render(<Scatter points={points} names={names} logY formatY={String} tooltip={() => ({ title: '', lines: [] })} summary="Two traces" />)
    await screen.findByTestId('chart')
    expect(screen.getByText('Two traces')).toBeInTheDocument()
    const opt = last!.option as { yAxis: { type: string }; series: Array<{ name: string; data: Array<{ value: number[]; id: string }> }>; toolbox: { show: boolean } }
    expect(opt.yAxis.type).toBe('log')
    expect(opt.toolbox.show).toBe(false)
    expect(opt.series.map((s) => [s.name, s.data.map((d) => d.id)])).toEqual([
      ['Traces', ['a']],
      ['Slow', []],
      ['Errors', ['b']],
    ])
  })

  it('reports a finished brush and a point click; draws the controlled selection', async () => {
    const onSelect = vi.fn()
    const onPointClick = vi.fn()
    const { rerender } = render(
      <Scatter points={points} names={names} formatY={String} tooltip={() => ({ title: '', lines: [] })} onSelect={onSelect} onPointClick={onPointClick} summary="s" />,
    )
    await screen.findByTestId('chart')
    const chart = fakeChart()
    act(() => last!.onReady!(chart as never))
    // Armed for rect brushing, nothing selected yet.
    expect(chart.dispatchAction).toHaveBeenCalledWith(expect.objectContaining({ type: 'takeGlobalCursor', key: 'brush' }))
    expect(chart.dispatchAction).toHaveBeenLastCalledWith({ type: 'brush', areas: [] })

    last!.onEvents!.brushEnd!({ type: 'brushEnd', areas: [{ brushType: 'rect', coordRange: [[900, 2100], [10, 100]] }] })
    expect(onSelect).toHaveBeenLastCalledWith({ x: [900, 2100], y: [10, 100] })
    last!.onEvents!.brushEnd!({ type: 'brushEnd', areas: [] })
    expect(onSelect).toHaveBeenLastCalledWith(null)
    last!.onEvents!.click!({ componentType: 'series', data: { value: [2000, 5000], id: 'b' } })
    expect(onPointClick).toHaveBeenCalledWith('b')

    rerender(
      <Scatter
        points={points}
        names={names}
        formatY={String}
        tooltip={() => ({ title: '', lines: [] })}
        selection={{ x: [900, 2100], y: [10, 100] }}
        onSelect={onSelect}
        onPointClick={onPointClick}
        summary="s"
      />,
    )
    expect(chart.dispatchAction).toHaveBeenLastCalledWith({
      type: 'brush',
      areas: [{ brushType: 'rect', xAxisIndex: 0, yAxisIndex: 0, coordRange: [[900, 2100], [10, 100]] }],
    })
  })

  it('escapes API text in the tooltip', async () => {
    render(
      <Scatter points={points} names={names} formatY={String} tooltip={(id) => ({ title: `<img src=x>${id}`, lines: ['a & b'] })} summary="s" />,
    )
    await screen.findByTestId('chart')
    const fmt = (last!.option as { tooltip: { formatter: (p: unknown) => string } }).tooltip.formatter
    expect(fmt({ data: { id: 'b' } })).toBe('<b>&lt;img src=x&gt;b</b><br/>a &amp; b')
    expect(escapeHtml(`"'`)).toBe('&quot;&#39;')
  })
})
