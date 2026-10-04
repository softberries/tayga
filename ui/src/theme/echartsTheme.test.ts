import { describe, expect, it } from 'vitest'
import { echartsTheme, withAlpha } from './echartsTheme'

describe('echartsTheme', () => {
  it('reads colors from CSS variables', () => {
    const el = document.createElement('div')
    el.style.setProperty('--tg-chart-1', '#7cc4ff')
    el.style.setProperty('--tg-chart-2', '#ff6b6b')
    el.style.setProperty('--tg-ink', '#d8e1f0')
    el.style.setProperty('--tg-muted', '#8e9bb3')
    el.style.setProperty('--tg-accent', '#7cc4ff')
    document.body.append(el)
    const t = echartsTheme(el)
    expect(t.color.slice(0, 2)).toEqual(['#7cc4ff', '#ff6b6b'])
    expect(t.textStyle.color).toBe('#d8e1f0')
    expect(t.valueAxis.axisLabel.color).toBe('#8e9bb3')
    expect(t.brush.brushStyle.color).toBe('rgba(124, 196, 255, 0.12)')
    el.remove()
  })

  it('withAlpha converts hex and leaves other values alone', () => {
    expect(withAlpha('#fff', 0.5)).toBe('rgba(255, 255, 255, 0.5)')
    expect(withAlpha('#1f6fd1', 1)).toBe('rgba(31, 111, 209, 1)')
    expect(withAlpha('var(--x)', 0.5)).toBe('var(--x)')
  })
})
