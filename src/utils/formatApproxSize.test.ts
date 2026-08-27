import { describe, expect, it } from 'vitest'

import type { QualitySize } from '@/types/generated/probe'

import { formatApproxSize } from './formatApproxSize'

describe('formatApproxSize', () => {
  it('renders "размер неизвестен" for the unknown variant, not a dash or zero', () => {
    const size: QualitySize = { kind: 'unknown' }
    expect(formatApproxSize(size)).toBe('размер неизвестен')
  })

  it('formats gigabyte-scale sizes with one decimal', () => {
    const size: QualitySize = { kind: 'known', bytes: 1.8 * 1024 ** 3 }
    expect(formatApproxSize(size)).toBe('≈ 1.8 ГБ')
  })

  it('formats megabyte-scale sizes rounded to a whole number', () => {
    expect(formatApproxSize({ kind: 'known', bytes: 980 * 1024 ** 2 })).toBe('≈ 980 МБ')
    expect(formatApproxSize({ kind: 'known', bytes: 14 * 1024 ** 2 })).toBe('≈ 14 МБ')
  })

  it('formats sub-megabyte sizes in kilobytes', () => {
    expect(formatApproxSize({ kind: 'known', bytes: 500 * 1024 })).toBe('≈ 500 КБ')
  })
})
