import '@testing-library/jest-dom/vitest'
import { cleanup } from '@testing-library/react'
import { afterEach, vi } from 'vitest'

// jsdom lacks these browser APIs that Radix and Motion use.
class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
globalThis.ResizeObserver ??= ResizeObserverStub as unknown as typeof ResizeObserver

if (!Element.prototype.hasPointerCapture) {
  Element.prototype.hasPointerCapture = () => false
  Element.prototype.setPointerCapture = () => {}
  Element.prototype.releasePointerCapture = () => {}
}
Element.prototype.scrollIntoView ??= () => {}
window.scrollTo = () => {}

/** A controllable matchMedia: tests flip `prefers-color-scheme` via setSystemDark(). */
type Listener = (e: MediaQueryListEvent) => void
const media = { dark: false, reduce: false, listeners: new Set<Listener>() }

export function setSystemDark(dark: boolean): void {
  media.dark = dark
  for (const l of media.listeners) l({ matches: dark, media: '(prefers-color-scheme: dark)' } as MediaQueryListEvent)
}

export function setReducedMotion(reduce: boolean): void {
  media.reduce = reduce
}

window.matchMedia = vi.fn((query: string) => {
  const isDark = query.includes('prefers-color-scheme: dark')
  const isReduce = query.includes('prefers-reduced-motion')
  return {
    get matches() {
      if (isDark) return media.dark
      if (isReduce) return media.reduce && query.includes('reduce')
      return false
    },
    media: query,
    onchange: null,
    addEventListener: (_: string, l: Listener) => {
      if (isDark) media.listeners.add(l)
    },
    removeEventListener: (_: string, l: Listener) => {
      media.listeners.delete(l)
    },
    addListener: (l: Listener) => {
      if (isDark) media.listeners.add(l)
    },
    removeListener: (l: Listener) => {
      media.listeners.delete(l)
    },
    dispatchEvent: () => false,
  } as unknown as MediaQueryList
})

afterEach(() => {
  cleanup()
  media.dark = false
  media.reduce = false
  media.listeners.clear()
  window.localStorage.clear()
  delete document.documentElement.dataset.theme
  document.documentElement.className = ''
})
