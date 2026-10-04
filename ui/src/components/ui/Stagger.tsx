import { m, stagger } from 'motion/react'
import type { HTMLMotionProps, Variants } from 'motion/react'

const list: Variants = {
  hidden: {},
  shown: { transition: { delayChildren: stagger(0.06) } },
}

const item: Variants = {
  hidden: { opacity: 0, y: 6 },
  shown: { opacity: 1, y: 0, transition: { duration: 0.45, ease: 'easeOut' } },
}

/** Container whose StaggerItem children fade and rise in one after another (60 ms apart). */
export function StaggerList(props: HTMLMotionProps<'div'>) {
  return <m.div variants={list} initial="hidden" animate="shown" {...props} />
}

export function StaggerItem(props: HTMLMotionProps<'div'>) {
  return <m.div variants={item} {...props} />
}
