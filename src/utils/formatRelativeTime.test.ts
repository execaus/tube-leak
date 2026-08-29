import { describe, expect, it } from 'vitest'

import { formatRelativeTime } from './formatRelativeTime'

const NOW = new Date('2026-08-25T15:00:00.000Z')

describe('formatRelativeTime', () => {
  it('returns "только что" for anything under a minute, including exactly now', () => {
    expect(formatRelativeTime('2026-08-25T15:00:00.000Z', NOW)).toBe('только что')
    expect(formatRelativeTime('2026-08-25T14:59:31.000Z', NOW)).toBe('только что')
  })

  it('formats minutes with correct Russian pluralization', () => {
    expect(formatRelativeTime('2026-08-25T14:59:00.000Z', NOW)).toBe('1 минуту назад')
    expect(formatRelativeTime('2026-08-25T14:58:00.000Z', NOW)).toBe('2 минуты назад')
    expect(formatRelativeTime('2026-08-25T14:39:00.000Z', NOW)).toBe('21 минуту назад')
    expect(formatRelativeTime('2026-08-25T14:15:00.000Z', NOW)).toBe('45 минут назад')
    expect(formatRelativeTime('2026-08-25T14:49:00.000Z', NOW)).toBe('11 минут назад')
  })

  it('formats hours with correct Russian pluralization', () => {
    expect(formatRelativeTime('2026-08-25T14:00:00.000Z', NOW)).toBe('1 час назад')
    expect(formatRelativeTime('2026-08-25T12:00:00.000Z', NOW)).toBe('3 часа назад')
    expect(formatRelativeTime('2026-08-25T02:00:00.000Z', NOW)).toBe('13 часов назад')
    expect(formatRelativeTime('2026-08-24T18:01:00.000Z', NOW)).toBe('20 часов назад')
  })

  it('formats days with correct Russian pluralization once at least 24h have passed', () => {
    expect(formatRelativeTime('2026-08-24T15:00:00.000Z', NOW)).toBe('1 день назад')
    expect(formatRelativeTime('2026-08-23T15:00:00.000Z', NOW)).toBe('2 дня назад')
    expect(formatRelativeTime('2026-08-04T15:00:00.000Z', NOW)).toBe('21 день назад')
    expect(formatRelativeTime('2026-08-10T15:00:00.000Z', NOW)).toBe('15 дней назад')
  })

  it('clamps a timestamp reported as being in the future to "только что" instead of a negative duration', () => {
    expect(formatRelativeTime('2026-08-25T15:05:00.000Z', NOW)).toBe('только что')
  })

  it('defaults to the real current time when now is not provided', () => {
    const justNow = new Date().toISOString()
    expect(formatRelativeTime(justNow)).toBe('только что')
  })

  it('falls back to a neutral placeholder instead of "NaN дней назад" for an unparsable timestamp (Н-5)', () => {
    // Контракт обещает RFC 3339 (недостижимо сегодня настоящим `at` из
    // ядра), но `new Date('мусор').getTime()` -> `NaN` не должно молча
    // доходить до ветки суток и печатать `NaN` пользователю.
    expect(formatRelativeTime('мусор', NOW)).toBe('некоторое время назад')
    expect(formatRelativeTime('', NOW)).toBe('некоторое время назад')
  })
})
