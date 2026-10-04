/** Compact number: 1234 → "1.2k", 1_250_000 → "1.3M". */
export function compact(n: number): string {
  const abs = Math.abs(n)
  if (abs >= 1e9) return `${trim(n / 1e9)}B`
  if (abs >= 1e6) return `${trim(n / 1e6)}M`
  if (abs >= 1e3) return `${trim(n / 1e3)}k`
  return abs >= 100 || Number.isInteger(n) ? String(Math.round(n)) : trim(n)
}

function trim(n: number): string {
  return n.toFixed(1).replace(/\.0$/, '')
}

/** A 32-hex trace or story id shortened for display: "9b3f…c21e". */
export function shortId(id: string): string {
  return id.length > 12 ? `${id.slice(0, 4)}…${id.slice(-4)}` : id
}
