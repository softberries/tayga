/**
 * Axis label options for a time axis. ECharts' default prints the first tick of a day as the
 * bare day number ("4"); these levels print "Oct 4" there, in bold, and "HH:mm" elsewhere.
 * `primary` marks the tick that starts a larger unit (a new day on an hourly axis).
 */
const DAY = '{MMM} {d}'
const BOLD_DAY = `{primary|${DAY}}`

export function timeAxisLabel() {
  return {
    hideOverlap: true,
    formatter: {
      year: '{yyyy}',
      month: ['{MMM}', '{primary|{yyyy}}'],
      day: [DAY, BOLD_DAY],
      hour: ['{HH}:{mm}', BOLD_DAY],
      minute: ['{HH}:{mm}', BOLD_DAY],
      second: ['{HH}:{mm}:{ss}', BOLD_DAY],
      millisecond: ['{HH}:{mm}:{ss}', BOLD_DAY],
      none: '{HH}:{mm}:{ss}.{SSS}',
    },
    rich: { primary: { fontWeight: 'bold' } },
  }
}
