/**
 * ECharts theme built from the live CSS variables, so charts follow the active theme.
 * Call it again after a theme switch (key charts on `useTheme().resolved`). The object is
 * structurally an ECharts theme; echarts itself is added with the first chart (Task 8).
 */

function cssVar(style: CSSStyleDeclaration, name: string): string {
  return style.getPropertyValue(name).trim()
}

/** `#rgb`/`#rrggbb` plus alpha as an rgba() string ECharts' canvas understands. */
export function withAlpha(color: string, alpha: number): string {
  const m = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(color.trim())
  if (!m?.[1]) return color
  const hex = m[1].length === 3 ? [...m[1]].map((c) => c + c).join('') : m[1]
  const n = Number.parseInt(hex, 16)
  return `rgba(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}, ${alpha})`
}

export interface ChartTokens {
  palette: string[]
  ink: string
  muted: string
  faint: string
  line: string
  lineSoft: string
  panel: string
  panelLine: string
  err: string
  slow: string
  ok: string
  accent: string
  fontSans: string
  fontMono: string
}

export function readChartTokens(root: Element = document.documentElement): ChartTokens {
  const s = getComputedStyle(root)
  const v = (n: string) => cssVar(s, `--tg-${n}`)
  return {
    palette: [1, 2, 3, 4, 5, 6].map((i) => v(`chart-${i}`)),
    ink: v('ink'),
    muted: v('muted'),
    faint: v('faint'),
    line: v('line'),
    lineSoft: v('line-soft'),
    panel: v('panel'),
    panelLine: v('panel-line'),
    err: v('err'),
    slow: v('slow'),
    ok: v('ok'),
    accent: v('accent'),
    fontSans: v('font-sans'),
    fontMono: v('font-mono'),
  }
}

export function echartsTheme(root: Element = document.documentElement) {
  const t = readChartTokens(root)
  const axis = {
    axisLine: { show: true, lineStyle: { color: t.line } },
    axisTick: { show: false },
    axisLabel: { color: t.muted, fontFamily: t.fontMono, fontSize: 10.5 },
    splitLine: { show: true, lineStyle: { color: t.lineSoft } },
    nameTextStyle: { color: t.muted },
  }
  return {
    color: t.palette,
    backgroundColor: 'transparent',
    textStyle: { color: t.ink, fontFamily: t.fontSans, fontSize: 12 },
    title: { textStyle: { color: t.ink }, subtextStyle: { color: t.muted } },
    legend: { textStyle: { color: t.muted } },
    tooltip: {
      backgroundColor: t.panel,
      borderColor: t.panelLine,
      textStyle: { color: t.ink, fontFamily: t.fontSans, fontSize: 12 },
      axisPointer: { lineStyle: { color: t.faint }, crossStyle: { color: t.faint } },
    },
    categoryAxis: { ...axis, splitLine: { show: false } },
    valueAxis: axis,
    timeAxis: { ...axis, splitLine: { show: false } },
    logAxis: axis,
    line: { symbol: 'none', smooth: false, lineStyle: { width: 1.6 } },
    bar: { itemStyle: { borderRadius: [3, 3, 0, 0] } },
    scatter: { symbolSize: 6 },
    dataZoom: {
      borderColor: t.line,
      textStyle: { color: t.muted },
      fillerColor: withAlpha(t.accent, 0.12),
      handleStyle: { color: t.panel, borderColor: t.accent },
    },
    brush: { brushStyle: { borderColor: t.accent, color: withAlpha(t.accent, 0.12), borderWidth: 1 } },
    /** Semantic colors for series that mean error / slow / ok, not palette order. */
    semantic: { err: t.err, slow: t.slow, ok: t.ok, accent: t.accent },
  }
}

export type EChartsThemeObject = ReturnType<typeof echartsTheme>
