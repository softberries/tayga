import { describe, expect, it } from 'vitest'
import type { SeriesView } from '../../api/types'
import { bytes } from '../../lib/format'
import { CHARTS, JOBS, jobStatus, rateNumber, scrapeAge, seriesPoints, shortJob } from './model'

const NOW = 1_791_138_700_000
const view = (points: SeriesView['points']): SeriesView => ({ metric: 'up', kind: 'gauge', bucket_secs: 60, points })

describe('seriesPoints', () => {
  it('keeps times, scales values and passes gaps through', () => {
    const v: SeriesView = { metric: 'm', kind: 'rate', bucket_secs: 60, points: [[1000, 0.5], [61_000, null], [121_000, 2]] }
    expect(seriesPoints(v)).toEqual([[1000, 0.5], [61_000, null], [121_000, 2]])
    expect(seriesPoints(v, 60)).toEqual([[1000, 30], [61_000, null], [121_000, 120]])
  })

  it('drops a rate bucket that is still filling, but not a gauge', () => {
    const points: SeriesView['points'] = [[NOW - 120_000, 5], [NOW - 60_000, 6], [NOW - 20_000, 1]]
    const rate: SeriesView = { metric: 'm', kind: 'rate', bucket_secs: 60, points }
    expect(seriesPoints(rate, 1, NOW)).toEqual([[NOW - 120_000, 5], [NOW - 60_000, 6]])
    expect(seriesPoints(rate)).toHaveLength(3)
    expect(seriesPoints({ ...rate, kind: 'gauge' }, 1, NOW)).toHaveLength(3)
  })

  it('is empty without a response or points', () => {
    expect(seriesPoints(undefined)).toEqual([])
    expect(seriesPoints(view([]))).toEqual([])
  })
})

describe('jobStatus', () => {
  it('is up for a fresh up=1 sample', () => {
    expect(jobStatus('tayga-writer', view([[NOW - 90_000, 0], [NOW - 30_000, 1]]), NOW)).toEqual({ job: 'tayga-writer', state: 'up', at: NOW - 30_000 })
  })

  it('is down when the newest sample is 0', () => {
    expect(jobStatus('tayga-writer', view([[NOW - 30_000, 1], [NOW - 20_000, 0]]), NOW).state).toBe('down')
  })

  it('is down when the newest sample is stale, even if it was 1', () => {
    expect(jobStatus('tayga-writer', view([[NOW - 20 * 60_000, 1]]), NOW).state).toBe('down')
  })

  it('is unknown without history', () => {
    expect(jobStatus('tayga-writer', view([]), NOW)).toEqual({ job: 'tayga-writer', state: 'unknown', at: null })
    expect(jobStatus('tayga-writer', undefined, NOW).state).toBe('unknown')
  })
})

describe('labels', () => {
  it('scrapeAge is an upper bound from the bucket start', () => {
    const at = (ms: number) => ({ job: 'j', state: 'up' as const, at: NOW - ms })
    expect(scrapeAge(at(10_000), NOW)).toBe('scraped within the last minute')
    expect(scrapeAge(at(61_000), NOW)).toBe('scraped within the last 2 min')
    expect(scrapeAge({ job: 'j', state: 'unknown', at: null }, NOW)).toBe('no samples yet')
  })

  it('shortJob drops the tayga- prefix', () => {
    expect(shortJob('tayga-ingest')).toBe('ingest')
    expect(JOBS.map(shortJob)).toEqual(['ingest', 'writer', 'assembler', 'logminer', 'notifier', 'api'])
  })

  it('rateNumber keeps only the digits that matter', () => {
    expect([0, 0.05, 0.5, 4.25, 82.4, 1234].map(rateNumber)).toEqual(['0', '0.05', '0.5', '4.3', '82', '1.2k'])
  })

  it('bytes uses binary units', () => {
    expect([bytes(0), bytes(512), bytes(1536), bytes(1_680_969), bytes(-1)]).toEqual(['0 B', '512 B', '1.50 KiB', '1.60 MiB', '—'])
  })
})

describe('chart specs', () => {
  it('query only valid metric names and k=v labels', () => {
    for (const c of CHARTS) {
      for (const s of c.series) {
        expect(s.metric).toMatch(/^[a-z_:][a-z0-9_:]*$/)
        if (s.labels) expect(s.labels).toMatch(/^[a-z_]+=[^,]+$/)
      }
    }
  })

  it('charts the writer and logminer commit failures', () => {
    const metrics = CHARTS.find((c) => c.id === 'commit-failures')?.series.map((x) => x.metric)
    expect(metrics).toEqual(['tayga_writer_commit_failures_total', 'tayga_logminer_commit_failures_total'])
    expect(CHARTS.find((c) => c.id === 'writer')?.unit).toBe('rows committed per second')
  })

  it('give each chart at most six lines (the theme palette size)', () => {
    for (const c of CHARTS) expect(c.series.length).toBeLessThanOrEqual(6)
  })
})
