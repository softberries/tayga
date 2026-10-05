import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { Spark, sparkPoints } from './Spark'

describe('sparkPoints', () => {
  it('scales the largest value to the top and zero to the bottom', () => {
    expect(sparkPoints([0, 5, 10], 100, 30)).toBe('0.0,28.0 50.0,15.0 100.0,2.0')
  })
  it('draws a flat line for one value, none, or bad values', () => {
    expect(sparkPoints([3], 10, 10)).toBe('0.0,2.0 10.0,2.0')
    expect(sparkPoints([], 10, 10)).toBe('0.0,8.0 10.0,8.0')
    expect(sparkPoints([Number.NaN, -1], 10, 10)).toBe('0.0,8.0 10.0,8.0')
  })
})

describe('Spark', () => {
  it('is an image with a label, or hidden without one', () => {
    const { container, rerender } = render(<Spark values={[1, 2]} tone="err" label="Error stories per minute, peak 2" />)
    expect(screen.getByRole('img', { name: 'Error stories per minute, peak 2' })).toHaveClass('text-err')
    rerender(<Spark values={[1, 2]} tone="slow" />)
    expect(screen.queryByRole('img')).toBeNull()
    expect(container.querySelector('svg')).toHaveAttribute('aria-hidden', 'true')
    expect(container.querySelector('polygon')).toHaveAttribute('points', '0,28 0.0,14.0 120.0,2.0 120,28')
  })
})
