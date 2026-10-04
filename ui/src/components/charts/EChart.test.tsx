import { act, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { setReducedMotion } from '../../test/setup'
import { TimeSeries } from './TimeSeries'

const calls: Array<{ theme: object; option: Record<string, unknown> }> = []
vi.mock('./EChartImpl', () => ({
  default: (p: { theme: object; option: Record<string, unknown> }) => {
    calls.push({ theme: p.theme, option: p.option })
    return <div data-testid="chart" />
  },
}))

const series = [{ name: 's', points: [[0, 1]] as const, tone: 'err' as const }]

describe('TimeSeries', () => {
  it('has a text summary and rebuilds the ECharts theme when the theme switches', async () => {
    document.documentElement.dataset.theme = 'light'
    render(<TimeSeries series={series} summary="Two stories" />)
    expect(await screen.findByTestId('chart')).toBeInTheDocument()
    expect(screen.getByText('Two stories')).toBeInTheDocument()
    const before = calls.at(-1)!.theme
    await act(async () => {
      document.documentElement.dataset.theme = 'dark'
      await new Promise((r) => setTimeout(r, 0))
    })
    expect(calls.at(-1)!.theme).not.toBe(before)
  })

  it('animates normally, and not at all under reduced motion', async () => {
    const { unmount } = render(<TimeSeries series={series} summary="s" />)
    await screen.findByTestId('chart')
    expect(calls.at(-1)!.option.animation).toBe(true)
    unmount()
    setReducedMotion(true)
    render(<TimeSeries series={series} summary="s" />)
    await screen.findByTestId('chart')
    expect(calls.at(-1)!.option.animation).toBe(false)
  })
})
