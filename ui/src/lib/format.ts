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

/** A duration in ns for display: "0.21 ms", "65.1 ms", "5.12 s", "3.2 min", "2.4 d". */
export function duration(ns: number): string {
  if (!Number.isFinite(ns) || ns < 0) return '—'
  if (ns === 0) return '0 ms'
  const ms = ns / 1e6
  if (ms < 1) return `${ms.toFixed(ms < 0.1 ? 3 : 2)} ms`
  if (ms < 10) return `${ms.toFixed(2)} ms`
  if (ms < 1000) return `${ms.toFixed(1)} ms`
  if (ms < 60_000) return `${(ms / 1000).toFixed(2)} s`
  if (ms < 3_600_000) return `${(ms / 60_000).toFixed(1)} min`
  if (ms < 86_400_000) return `${(ms / 3_600_000).toFixed(1)} h`
  return `${(ms / 86_400_000).toFixed(1)} d`
}

/** Bare milliseconds for a ms column: "0.21", "65.1", "5120". */
export function msValue(ns: number): string {
  if (!Number.isFinite(ns) || ns < 0) return '—'
  const ms = ns / 1e6
  return ms < 10 ? ms.toFixed(2) : ms < 1000 ? ms.toFixed(1) : String(Math.round(ms))
}

const pad = (n: number, w = 2) => String(n).padStart(w, '0')

/** Local wall-clock time with milliseconds: "14:03:07.512". */
export function clockMs(ns: number): string {
  const d = new Date(ns / 1e6)
  if (Number.isNaN(d.getTime())) return '—'
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}.${pad(d.getMilliseconds(), 3)}`
}

/** Local date and time: "2026-10-04 14:03:07". */
export function dateTime(ns: number): string {
  const d = new Date(ns / 1e6)
  if (Number.isNaN(d.getTime())) return '—'
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`
}

/** Percent with one decimal below 10 %: "0.4 %", "37 %". */
export function percent(fraction: number): string {
  if (!Number.isFinite(fraction)) return '—'
  const p = fraction * 100
  return `${p < 10 ? p.toFixed(1) : Math.round(p)} %`
}
