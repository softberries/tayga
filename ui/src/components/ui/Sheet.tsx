import { Dialog as D } from 'radix-ui'
import { X } from 'lucide-react'
import { AnimatePresence, motion } from 'motion/react'
import { useCallback, useRef, useState } from 'react'
import type { KeyboardEvent, PointerEvent, ReactNode } from 'react'
import { cx } from '../../lib/cx'

export const SHEET_MIN = 360
const SHEET_STEP = 32

function maxWidth(): number {
  return Math.max(SHEET_MIN, Math.round(window.innerWidth * 0.9))
}

function clamp(w: number): number {
  return Math.min(maxWidth(), Math.max(SHEET_MIN, Math.round(w)))
}

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

export interface SheetProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: ReactNode
  /** Optional line under the title. */
  subtitle?: ReactNode
  children?: ReactNode
  /** Initial width in px (default 520). */
  defaultWidth?: number
  /** localStorage key that remembers the width the user dragged to. */
  storageKey?: string
  /** Modal drawers trap focus and dim the page; inspectors default to non-modal. */
  modal?: boolean
  className?: string
}

/**
 * Right-side drawer that slides in (Motion; reduced motion skips the slide via MotionConfig)
 * and resizes from its left edge by pointer or keyboard (arrow keys, Home/End).
 */
export function Sheet({
  open,
  onOpenChange,
  title,
  subtitle,
  children,
  defaultWidth = 520,
  storageKey,
  modal = false,
  className,
}: SheetProps) {
  const [width, setWidth] = useState(() => readWidth(storageKey, defaultWidth))
  const drag = useRef<{ x: number; w: number } | null>(null)

  const commit = useCallback(
    (w: number) => {
      const c = clamp(w)
      setWidth(c)
      writeWidth(storageKey, c)
    },
    [storageKey],
  )

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    e.preventDefault()
    e.currentTarget.setPointerCapture(e.pointerId)
    drag.current = { x: e.clientX, w: width }
  }
  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    if (!drag.current) return
    // The sheet is anchored right, so dragging left (smaller x) widens it.
    setWidth(clamp(drag.current.w + (drag.current.x - e.clientX)))
  }
  const onPointerUp = (e: PointerEvent<HTMLDivElement>) => {
    if (!drag.current) return
    drag.current = null
    e.currentTarget.releasePointerCapture(e.pointerId)
    commit(width)
  }
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const step = e.shiftKey ? SHEET_STEP * 4 : SHEET_STEP
    if (e.key === 'ArrowLeft') commit(width + step)
    else if (e.key === 'ArrowRight') commit(width - step)
    else if (e.key === 'Home') commit(maxWidth())
    else if (e.key === 'End') commit(SHEET_MIN)
    else return
    e.preventDefault()
  }

  return (
    <D.Root open={open} onOpenChange={onOpenChange} modal={modal}>
      <AnimatePresence>
        {open ? (
          <D.Portal forceMount>
            {modal ? (
              <D.Overlay forceMount asChild>
                <motion.div
                  className="fixed inset-0 z-40 bg-scrim"
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  exit={{ opacity: 0 }}
                  transition={{ duration: 0.15 }}
                />
              </D.Overlay>
            ) : null}
            <D.Content
              forceMount
              asChild
              aria-describedby={undefined}
              onInteractOutside={modal ? undefined : (e) => e.preventDefault()}
            >
              <motion.aside
                className={cx(
                  'fixed inset-y-0 right-0 z-50 flex max-w-[100vw] flex-col border-l border-panel-line bg-panel text-ink shadow-panel-lg focus:outline-none',
                  className,
                )}
                style={{ width }}
                initial={{ x: '100%' }}
                animate={{ x: 0 }}
                exit={{ x: '100%' }}
                transition={{ type: 'tween', duration: 0.22, ease: [0.2, 0.8, 0.2, 1] }}
              >
                <div
                  role="separator"
                  aria-orientation="vertical"
                  aria-label="Resize panel"
                  aria-valuemin={SHEET_MIN}
                  aria-valuemax={maxWidth()}
                  aria-valuenow={width}
                  tabIndex={0}
                  onPointerDown={onPointerDown}
                  onPointerMove={onPointerMove}
                  onPointerUp={onPointerUp}
                  onPointerCancel={onPointerUp}
                  onKeyDown={onKeyDown}
                  className="absolute inset-y-0 -left-1 w-2 cursor-col-resize touch-none after:absolute after:inset-y-0 after:left-[3px] after:w-0.5 after:bg-transparent hover:after:bg-accent focus-visible:after:bg-accent"
                />
                <header className="flex items-start gap-3 border-b border-line px-5 py-4">
                  <div className="flex min-w-0 flex-1 flex-col gap-1">
                    <D.Title className="m-0 truncate text-[15px] font-semibold">{title}</D.Title>
                    {subtitle ? <div className="text-xs text-muted">{subtitle}</div> : null}
                  </div>
                  <D.Close
                    aria-label="Close panel"
                    className="inline-flex size-8 cursor-pointer items-center justify-center rounded-control text-muted hover:bg-rail-active hover:text-ink"
                  >
                    <X size={16} aria-hidden />
                  </D.Close>
                </header>
                <div className="min-h-0 flex-1 overflow-auto px-5 py-4">{children}</div>
              </motion.aside>
            </D.Content>
          </D.Portal>
        ) : null}
      </AnimatePresence>
    </D.Root>
  )
}
