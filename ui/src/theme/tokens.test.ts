/// <reference types="node" />
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join, relative } from 'node:path'
import { describe, expect, it } from 'vitest'

const SRC = join(import.meta.dirname, '..')
const THEME = join(SRC, 'theme')

function walk(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const p = join(dir, name)
    if (statSync(p).isDirectory()) return name === '__fixtures__' ? [] : walk(p)
    return /\.(ts|tsx|css)$/.test(name) ? [p] : []
  })
}

/** Raw colors outside src/theme/ bypass the tokens and break one of the two themes. */
const COLOR_LITERAL = /#[0-9a-fA-F]{3,8}\b|\brgba?\(|\bhsla?\(/

describe('token lint', () => {
  it('pattern catches color literals but not ids', () => {
    for (const bad of ['color: #fff;', "'#1F6FD1'", 'rgb(0 0 0)', 'rgba(1,2,3,.5)', 'hsl(10 50% 50%)'])
      expect(COLOR_LITERAL.test(bad), bad).toBe(true)
    for (const ok of ["href='#main'", 'var(--tg-err)', 'trace 334c8a31'])
      expect(COLOR_LITERAL.test(ok), ok).toBe(false)
  })

  it('no hex, rgb() or hsl() color literals in src outside theme/', () => {
    const offenders: string[] = []
    for (const file of walk(SRC)) {
      if (file.startsWith(THEME)) continue
      readFileSync(file, 'utf8')
        .split('\n')
        .forEach((line, i) => {
          if (COLOR_LITERAL.test(line)) offenders.push(`${relative(SRC, file)}:${i + 1}: ${line.trim()}`)
        })
    }
    expect(offenders).toEqual([])
  })
})

const css = readFileSync(join(THEME, 'tokens.css'), 'utf8')

function block(selector: string): Map<string, string> {
  const start = css.indexOf(`${selector} {`)
  expect(start, selector).toBeGreaterThanOrEqual(0)
  const body = css.slice(start, css.indexOf('\n}', start))
  return new Map([...body.matchAll(/(--tg-[\w-]+):\s*([^;]+);/g)].map((m) => [m[1]!, m[2]!.trim()]))
}

describe('tokens.css', () => {
  const light = block(':root')
  const dark = block("[data-theme='dark']")

  it('dark only overrides tokens that light defines', () => {
    for (const name of dark.keys()) expect(light.has(name), name).toBe(true)
  })

  it('every theme-dependent color is set in both themes', () => {
    const shared = /^--tg-(brand|cta|radius|font|chart-[1-4]|glow|shadow-(accent|cta|brand))/
    const missing = [...light.keys()].filter((n) => !shared.test(n) && !dark.has(n))
    // on-accent is white in both themes and still listed in dark for clarity.
    expect(missing).toEqual([])
  })

  it('matches the spec §5 table', () => {
    const table: Array<[string, string, string]> = [
      ['ground', '#0a1020', '#eef2f8'],
      ['panel', '#0e162a', '#ffffff'],
      ['ink', '#d8e1f0', '#0e1a2e'],
      ['muted', '#8e9bb3', '#55627a'],
      ['accent', '#7cc4ff', '#1c69c7'],
      ['err', '#ff6b6b', '#bf342d'],
      ['slow', '#f4b740', '#8f5c06'],
    ]
    for (const [name, d, l] of table) {
      expect(dark.get(`--tg-${name}`), name).toBe(d)
      expect(light.get(`--tg-${name}`), name).toBe(l)
    }
  })

  it('maps Tailwind utilities to the variables', () => {
    expect(css).toMatch(/@theme inline \{[\s\S]*--color-panel: var\(--tg-panel\)/)
    expect(css).toMatch(/--shadow-panel: var\(--tg-shadow\)/)
    expect(css).toMatch(/--font-mono: var\(--tg-font-mono\)/)
  })

  it('disables the animations under reduced motion', () => {
    expect(css).toMatch(/prefers-reduced-motion: reduce\)[\s\S]*\.tg-pulse[\s\S]*animation: none/)
    expect(css).toMatch(/prefers-reduced-motion: no-preference\)[\s\S]*tg-theme-switching/)
  })
})

/** WCAG 2.x relative luminance of a #rrggbb color. */
function luminance(hex: string): number {
  const n = Number.parseInt(hex.slice(1), 16)
  const lin = (c: number) => {
    const v = c / 255
    return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4
  }
  return 0.2126 * lin((n >> 16) & 255) + 0.7152 * lin((n >> 8) & 255) + 0.0722 * lin(n & 255)
}

function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [number, number]
  return (hi + 0.05) / (lo + 0.05)
}

describe('contrast (spec §5: text at least 4.5:1 in both themes)', () => {
  const light = block(':root')
  const themes = { light, dark: new Map([...light, ...block("[data-theme='dark']")]) }
  const hex = (t: Map<string, string>, name: string) => {
    const v = t.get(`--tg-${name}`)
    expect(v, name).toMatch(/^#[0-9a-f]{6}$/)
    return v!
  }

  /** Text colors the components use, on every surface they sit on. */
  const surfaces = ['ground', 'panel', 'inner', 'field', 'row-selected', 'rail-active']
  const pairs: Array<[string, string]> = [
    ...['ink', 'ink-2', 'muted', 'accent', 'err', 'slow', 'ok', 'silence'].flatMap((fg) =>
      surfaces.map((bg): [string, string] => [fg, bg]),
    ),
    ['err', 'err-soft'],
    ['slow', 'slow-soft'],
    ['ok', 'ok-soft'],
    ['silence', 'silence-soft'],
    ['on-accent', 'accent-strong'],
    ['on-slow', 'slow'],
    ['muted', 'inner'],
  ]

  for (const [theme, t] of Object.entries(themes)) {
    it(`${theme}: every text/surface pair reaches 4.5:1`, () => {
      const failing = pairs
        .map(([fg, bg]) => ({ pair: `${fg} on ${bg}`, ratio: Math.round(contrast(hex(t, fg), hex(t, bg)) * 100) / 100 }))
        .filter((p) => p.ratio < 4.5)
      expect(failing).toEqual([])
    })
  }

  it('contrast() matches known values', () => {
    expect(contrast('#000000', '#ffffff')).toBeCloseTo(21, 5)
    expect(contrast('#777777', '#ffffff')).toBeCloseTo(4.48, 2)
  })
})

describe('index.html pre-paint background', () => {
  const html = readFileSync(join(SRC, '..', 'index.html'), 'utf8')
  it('uses the ground token of each theme', () => {
    const light = block(':root').get('--tg-ground')
    const dark = block("[data-theme='dark']").get('--tg-ground')
    expect(html).toContain(`html[data-theme='light']{background:${light}`)
    expect(html).toContain(`html[data-theme='dark']{background:${dark}`)
  })
})
