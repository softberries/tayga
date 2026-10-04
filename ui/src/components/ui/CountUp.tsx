import { animate, useReducedMotionConfig } from 'motion/react'
import { useEffect, useRef, useState } from 'react'

export interface CountUpProps {
  value: number
  /** Formats every frame's value; default rounds to an integer. */
  format?: (n: number) => string
  /** Seconds (default 0.6). */
  duration?: number
  className?: string
}

const round = (n: number) => String(Math.round(n))

/**
 * Animates from the previously shown value to `value` (KPI numbers). Reduced motion (OS setting or MotionConfig) shows the
 * final value at once. Screen readers get only the final value.
 */
export function CountUp({ value, format = round, duration = 0.6, className }: CountUpProps) {
  const reduce = useReducedMotionConfig() ?? false
  const [shown, setShown] = useState(0)
  const from = useRef(0)

  useEffect(() => {
    if (reduce) {
      from.current = value
      return
    }
    const controls = animate(from.current, value, {
      duration,
      ease: 'easeOut',
      onUpdate: (v) => {
        from.current = v
        setShown(v)
      },
    })
    return () => controls.stop()
  }, [value, duration, reduce])

  return (
    <span className={className}>
      <span className="sr-only">{format(value)}</span>
      <span aria-hidden>{format(reduce ? value : shown)}</span>
    </span>
  )
}
