import { describe, expect, it } from 'vitest'

import { formatEtaSecs } from './formatEtaSecs'

describe('formatEtaSecs', () => {
  it('formats seconds only, below a minute', () => {
    expect(formatEtaSecs(45)).toBe('≈ 45 с')
  })

  it('formats minutes and seconds, per the design example', () => {
    expect(formatEtaSecs(100)).toBe('≈ 1 мин 40 с')
  })

  it('formats hours, minutes and seconds', () => {
    expect(formatEtaSecs(3725)).toBe('≈ 1 ч 2 мин 5 с')
  })
})
