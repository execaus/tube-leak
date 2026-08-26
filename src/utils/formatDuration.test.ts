import { describe, expect, it } from 'vitest'

import { formatDuration } from './formatDuration'

describe('formatDuration', () => {
  it('formats sub-hour durations as m:ss', () => {
    expect(formatDuration(0)).toBe('0:00')
    expect(formatDuration(5)).toBe('0:05')
    expect(formatDuration(65)).toBe('1:05')
    expect(formatDuration(599)).toBe('9:59')
    expect(formatDuration(3599)).toBe('59:59')
  })

  it('formats hour-or-longer durations as h:mm:ss', () => {
    expect(formatDuration(3600)).toBe('1:00:00')
    expect(formatDuration(3725)).toBe('1:02:05')
    expect(formatDuration(36_000)).toBe('10:00:00')
  })

  it('rounds fractional seconds and clamps negatives to zero', () => {
    expect(formatDuration(65.6)).toBe('1:06')
    expect(formatDuration(-5)).toBe('0:00')
  })
})
