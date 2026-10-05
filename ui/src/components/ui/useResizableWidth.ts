import { useCallback, useRef, useState } from 'react'
import type { HTMLAttributes, KeyboardEvent, PointerEvent } from 'react'

const STEP = 32

function readWidth(key: string | undefined, fallback: number): number {
  if (!key) return fallback
  try {
    const v = Number(window.localStorage.getItem(key))
    return Number.isFinite(v) && v > 0 ? v : fallback
  } catch {
    return fallback
  }
}

function writeWidth(key: string | undefined, w: number): void {
  if (!key) return
  try {
    window.localStorage.setItem(key, String(w))
  } catch {
    // Storage blocked: the width lasts for this page only.
  }
}

export interface ResizableWidthOptions {
  defaultWidth: number
  min: number
  /** Called on every change, so it can follow the window size. */
  max: () => number
  /** localStorage key that remembers the width the user chose. */
  storageKey?: string
}

/**
 * Width of a right-anchored panel resized from its left edge: dragging left (or ArrowLeft)
 * widens it, ArrowRight narrows it, Home/End jump to the max/min, Shift takes bigger steps.
 * Returns the width and the props for a `role="separator"` handle.
 */
export function useResizableWidth({ defaultWidth, min, max, storageKey }: ResizableWidthOptions) {
  const clamp = useCallback((w: number) => Math.min(Math.max(min, max()), Math.max(min, Math.round(w))), [min, max])
  const [width, setWidth] = useState(() => clamp(readWidth(storageKey, defaultWidth)))
  const drag = useRef<{ x: number; w: number } | null>(null)

  const commit = useCallback(
    (w: number) => {
      const c = clamp(w)
      setWidth(c)
      writeWidth(storageKey, c)
    },
    [clamp, storageKey],
  )

  const handleProps: HTMLAttributes<HTMLDivElement> & { 'aria-valuenow': number } = {
    role: 'separator',
    'aria-orientation': 'vertical',
    'aria-valuemin': min,
    'aria-valuemax': Math.max(min, max()),
    'aria-valuenow': width,
    tabIndex: 0,
    onPointerDown: (e: PointerEvent<HTMLDivElement>) => {
      e.preventDefault()
      e.currentTarget.setPointerCapture(e.pointerId)
      drag.current = { x: e.clientX, w: width }
    },
    onPointerMove: (e: PointerEvent<HTMLDivElement>) => {
      if (!drag.current) return
      setWidth(clamp(drag.current.w + (drag.current.x - e.clientX)))
    },
    onPointerUp: (e: PointerEvent<HTMLDivElement>) => {
      if (!drag.current) return
      drag.current = null
      e.currentTarget.releasePointerCapture(e.pointerId)
      commit(width)
    },
    onKeyDown: (e: KeyboardEvent<HTMLDivElement>) => {
      const step = e.shiftKey ? STEP * 4 : STEP
      if (e.key === 'ArrowLeft') commit(width + step)
      else if (e.key === 'ArrowRight') commit(width - step)
      else if (e.key === 'Home') commit(max())
      else if (e.key === 'End') commit(min)
      else return
      e.preventDefault()
    },
  }
  handleProps.onPointerCancel = handleProps.onPointerUp
  return { width, handleProps }
}
