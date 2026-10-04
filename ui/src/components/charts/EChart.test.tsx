import { act, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { TimeSeries } from './TimeSeries'

const themes: object[] = []
vi.mock('./EChartImpl', () => ({
  default: ({ theme }: { theme: object }) => {
    themes.push(theme)
    return <div data-testid="chart" />
  },
}))

describe('TimeSeries', () => {
  it('has a text summary and rebuilds the ECharts theme when the theme switches', async () => {
    document.documentElement.dataset.theme = 'light'
    render(<TimeSeries series={[{ name: 's', points: [[0, 1]], tone: 'err' }]} summary="Two stories" />)
    expect(await screen.findByTestId('chart')).toBeInTheDocument()
    expect(screen.getByText('Two stories')).toBeInTheDocument()
    const before = themes.at(-1)
    await act(async () => {
      document.documentElement.dataset.theme = 'dark'
      await new Promise((r) => setTimeout(r, 0))
    })
    expect(themes.at(-1)).not.toBe(before)
  })
})
