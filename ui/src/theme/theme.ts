/** Theme mode handling shared by ThemeProvider and mirrored by the inline script in index.html. */

export type ThemeMode = 'light' | 'dark' | 'system'
export type ResolvedTheme = 'light' | 'dark'

/** localStorage key; index.html reads the same key before first paint. */
export const THEME_KEY = 'tayga-theme'
export const DARK_QUERY = '(prefers-color-scheme: dark)'

/** The switch cycles light → dark → system → light. */
export const NEXT_MODE: Record<ThemeMode, ThemeMode> = {
  light: 'dark',
  dark: 'system',
  system: 'light',
}

function isMode(v: unknown): v is ThemeMode {
  return v === 'light' || v === 'dark' || v === 'system'
}

/** The stored mode, or `system` when nothing valid is stored or storage is unavailable. */
export function readMode(): ThemeMode {
  try {
    const v = window.localStorage.getItem(THEME_KEY)
    return isMode(v) ? v : 'system'
  } catch {
    return 'system'
  }
}

export function writeMode(mode: ThemeMode): void {
  try {
    window.localStorage.setItem(THEME_KEY, mode)
  } catch {
    // Storage blocked (private mode, quota): the choice lasts for this page only.
  }
}

export function systemPrefersDark(): boolean {
  return typeof window.matchMedia === 'function' && window.matchMedia(DARK_QUERY).matches
}

export function resolveTheme(mode: ThemeMode, prefersDark: boolean): ResolvedTheme {
  if (mode === 'system') return prefersDark ? 'dark' : 'light'
  return mode
}

export function applyTheme(theme: ResolvedTheme, root: HTMLElement = document.documentElement): void {
  root.dataset.theme = theme
}
