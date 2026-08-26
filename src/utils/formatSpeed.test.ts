import { describe, expect, it } from 'vitest'

import { formatSpeed } from './formatSpeed'

describe('formatSpeed', () => {
  it('formats bytes/sec below 1 KB', () => {
    expect(formatSpeed(512)).toBe('512 Б/с')
  })

  it('formats KB/s', () => {
    expect(formatSpeed(45 * 1024)).toBe('45 КБ/с')
  })

  it('formats MB/s with one decimal', () => {
    expect(formatSpeed(4.2 * 1024 * 1024)).toBe('4.2 МБ/с')
  })

  it('formats GB/s with one decimal', () => {
    expect(formatSpeed(1.5 * 1024 * 1024 * 1024)).toBe('1.5 ГБ/с')
  })
})
