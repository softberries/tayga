/** Number of `--tg-svc-N` service colors in tokens.css. */
export const SERVICE_COLORS = 8

/** Stable color for a service name (FNV-1a hash into the service palette), as a CSS var. */
export function serviceColor(service: string): string {
  let h = 0x811c9dc5
  for (let i = 0; i < service.length; i++) {
    h ^= service.charCodeAt(i)
    h = Math.imul(h, 0x01000193)
  }
  return `var(--tg-svc-${((h >>> 0) % SERVICE_COLORS) + 1})`
}
