import { useSyncExternalStore } from 'react'

/**
 * The theme currently applied to `<html data-theme>`. Unlike `useTheme().resolved`, this
 * changes only after ThemeProvider has written the attribute, so code that reads the CSS
 * variables (canvas drawing, the ECharts theme) sees the new values when it re-renders.
 */
function subscribe(cb: () => void): () => void {
  const mo = new MutationObserver(cb)
  mo.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] })
  return () => mo.disconnect()
}

const read = () => document.documentElement.dataset.theme ?? 'light'

export function useAppliedTheme(): string {
  return useSyncExternalStore(subscribe, read, () => 'light')
}
