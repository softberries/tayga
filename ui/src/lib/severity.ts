import type { BadgeKind } from '../components/ui/Badge'

const NAMES = ['TRACE', 'DEBUG', 'INFO', 'WARN', 'ERROR', 'FATAL'] as const

/** The log's severity text, else the OTLP range name for its number ("—" for 0, unspecified). */
export function severityLabel(text: string, num: number): string {
  if (text !== '') return text
  if (num <= 0 || !Number.isFinite(num)) return '—'
  return NAMES[Math.min(NAMES.length - 1, Math.floor((num - 1) / 4))]!
}

/** Badge color: error and fatal in err, warn in slow, the rest neutral. */
export function severityKind(num: number): BadgeKind {
  if (num >= 17) return 'error'
  if (num >= 13) return 'slow'
  return 'neutral'
}
