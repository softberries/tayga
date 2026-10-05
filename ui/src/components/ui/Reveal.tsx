import { m, useReducedMotionConfig } from 'motion/react'
import type { HTMLMotionProps } from 'motion/react'

/**
 * Content that replaces a skeleton: it mounts at opacity 0 and fades to 1 in 180 ms. Render it
 * only in the loaded branch, so a refetch (same element, no remount) never replays the fade.
 * Under reduced motion it renders at full opacity at once.
 */
export function Reveal(props: HTMLMotionProps<'div'>) {
  const reduce = useReducedMotionConfig() ?? false
  return <m.div initial={reduce ? false : { opacity: 0 }} animate={{ opacity: 1 }} transition={{ duration: 0.18, ease: 'easeOut' }} {...props} />
}
