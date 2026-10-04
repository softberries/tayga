import { describe, expect, it } from 'vitest'
import { severityKind, severityLabel } from './severity'

describe('severity', () => {
  it('prefers the text, else names the OTLP range', () => {
    expect(severityLabel('Information', 9)).toBe('Information')
    expect(severityLabel('', 0)).toBe('—')
    expect(severityLabel('', 1)).toBe('TRACE')
    expect(severityLabel('', 9)).toBe('INFO')
    expect(severityLabel('', 13)).toBe('WARN')
    expect(severityLabel('', 17)).toBe('ERROR')
    expect(severityLabel('', 24)).toBe('FATAL')
    expect(severityLabel('', 99)).toBe('FATAL')
  })
  it('colors warn and error', () => {
    expect(severityKind(9)).toBe('neutral')
    expect(severityKind(13)).toBe('slow')
    expect(severityKind(17)).toBe('error')
  })
})
