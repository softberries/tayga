import { act, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import { ThemeSwitch } from '../components/shell/ThemeSwitch'
import { TooltipProvider } from '../components/ui/Tooltip'
import { setSystemDark } from '../test/setup'
import { THEME_KEY } from './theme'
import { ThemeProvider, useTheme } from './ThemeProvider'

function Probe() {
  const { mode, resolved } = useTheme()
  return <output data-testid="probe">{`${mode}:${resolved}`}</output>
}

function renderTheme() {
  return render(
    <ThemeProvider>
      <TooltipProvider>
        <ThemeSwitch />
        <Probe />
      </TooltipProvider>
    </ThemeProvider>,
  )
}

const html = () => document.documentElement

describe('ThemeProvider', () => {
  it('defaults to system and applies data-theme', () => {
    renderTheme()
    expect(screen.getByTestId('probe')).toHaveTextContent('system:light')
    expect(html().dataset.theme).toBe('light')
  })

  it('restores the stored mode', () => {
    window.localStorage.setItem(THEME_KEY, 'dark')
    renderTheme()
    expect(screen.getByTestId('probe')).toHaveTextContent('dark:dark')
    expect(html().dataset.theme).toBe('dark')
  })

  it('ignores an invalid stored value', () => {
    window.localStorage.setItem(THEME_KEY, 'purple')
    renderTheme()
    expect(screen.getByTestId('probe')).toHaveTextContent('system:light')
  })

  it('persists the choice and survives a remount', async () => {
    const user = userEvent.setup()
    const { unmount } = renderTheme()
    await user.click(screen.getByRole('button', { name: /^Theme: system/ }))
    expect(window.localStorage.getItem(THEME_KEY)).toBe('light')
    await user.click(screen.getByRole('button', { name: /^Theme: light/ }))
    expect(window.localStorage.getItem(THEME_KEY)).toBe('dark')
    unmount()
    renderTheme()
    expect(screen.getByTestId('probe')).toHaveTextContent('dark:dark')
  })

  it('follows the system preference while in system mode', () => {
    renderTheme()
    expect(html().dataset.theme).toBe('light')
    act(() => setSystemDark(true))
    expect(screen.getByTestId('probe')).toHaveTextContent('system:dark')
    expect(html().dataset.theme).toBe('dark')
    act(() => setSystemDark(false))
    expect(html().dataset.theme).toBe('light')
  })

  it('ignores the system preference in an explicit mode', () => {
    window.localStorage.setItem(THEME_KEY, 'light')
    renderTheme()
    act(() => setSystemDark(true))
    expect(html().dataset.theme).toBe('light')
  })

  it('cross-fades only on a switch', async () => {
    const user = userEvent.setup()
    html().dataset.theme = 'light' // as set by the inline script in index.html
    renderTheme()
    expect(html().classList.contains('tg-theme-switching')).toBe(false)
    await user.click(screen.getByRole('button', { name: /^Theme:/ }))
    // system (light) → light: no change; → dark: switch.
    await user.click(screen.getByRole('button', { name: /^Theme:/ }))
    expect(html().dataset.theme).toBe('dark')
    expect(html().classList.contains('tg-theme-switching')).toBe(true)
  })

  it('survives blocked storage', () => {
    const get = Storage.prototype.getItem
    const set = Storage.prototype.setItem
    Storage.prototype.getItem = () => {
      throw new Error('blocked')
    }
    Storage.prototype.setItem = () => {
      throw new Error('blocked')
    }
    try {
      renderTheme()
      expect(screen.getByTestId('probe')).toHaveTextContent('system:light')
      act(() => screen.getByRole('button', { name: /^Theme:/ }).click())
      expect(screen.getByTestId('probe')).toHaveTextContent('light:light')
    } finally {
      Storage.prototype.getItem = get
      Storage.prototype.setItem = set
    }
  })
})

describe('ThemeSwitch', () => {
  it('cycles light → dark → system with a descriptive aria-label', async () => {
    const user = userEvent.setup()
    window.localStorage.setItem(THEME_KEY, 'light')
    renderTheme()
    const button = () => screen.getByRole('button', { name: /^Theme:/ })
    expect(button()).toHaveAccessibleName('Theme: light. Switch to dark')
    await user.click(button())
    expect(button()).toHaveAccessibleName('Theme: dark. Switch to system')
    await user.click(button())
    expect(button()).toHaveAccessibleName('Theme: system. Switch to light')
    await user.click(button())
    expect(button()).toHaveAccessibleName('Theme: light. Switch to dark')
  })
})
