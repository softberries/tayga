import { Monitor, Moon, Sun } from 'lucide-react'
import { IconButton } from '../ui/IconButton'
import { useTheme } from '../../theme/ThemeProvider'
import { NEXT_MODE } from '../../theme/theme'
import type { ThemeMode } from '../../theme/theme'

const icons: Record<ThemeMode, typeof Sun> = { light: Sun, dark: Moon, system: Monitor }
const names: Record<ThemeMode, string> = { light: 'light', dark: 'dark', system: 'system' }

/** Cycles light → dark → system. The label says the current mode and the next one. */
export function ThemeSwitch() {
  const { mode, cycle } = useTheme()
  const Icon = icons[mode]
  return (
    <IconButton
      label={`Theme: ${names[mode]}. Switch to ${names[NEXT_MODE[mode]]}`}
      data-mode={mode}
      icon={<Icon size={16} strokeWidth={1.8} aria-hidden />}
      onClick={cycle}
    />
  )
}
