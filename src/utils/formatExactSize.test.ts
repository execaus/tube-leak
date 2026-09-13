import { describe, expect, it } from 'vitest'

import { formatExactSize } from './formatExactSize'

describe('formatExactSize', () => {
  it('formats an exact size without the "≈" approximation marker (С-5)', () => {
    expect(formatExactSize(224_395_264)).toBe('214 МБ')
  })

  it('formats gigabytes with one decimal', () => {
    expect(formatExactSize(1_610_612_736)).toBe('1.5 ГБ')
  })

  it('formats kilobytes', () => {
    expect(formatExactSize(2048)).toBe('2 КБ')
  })
})
