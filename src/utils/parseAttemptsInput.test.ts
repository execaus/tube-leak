import { describe, expect, it } from 'vitest'

import { clampAttempts, parseAttemptsInput } from './parseAttemptsInput'

describe('parseAttemptsInput', () => {
  it('parses a plain integer', () => {
    expect(parseAttemptsInput('8')).toBe(8)
    expect(parseAttemptsInput(' 8 ')).toBe(8)
  })

  it('returns undefined for an empty or whitespace-only value (does not send to the core)', () => {
    expect(parseAttemptsInput('')).toBeUndefined()
    expect(parseAttemptsInput('   ')).toBeUndefined()
  })

  it('returns undefined for non-numeric input (does not send to the core)', () => {
    expect(parseAttemptsInput('abc')).toBeUndefined()
    expect(parseAttemptsInput('8abc')).toBeUndefined()
  })

  it('returns undefined for a fractional value (does not send to the core)', () => {
    expect(parseAttemptsInput('8.5')).toBeUndefined()
    expect(parseAttemptsInput('8,5')).toBeUndefined()
  })

  it('returns undefined for scientific notation and other non-plain-integer forms', () => {
    expect(parseAttemptsInput('1e2')).toBeUndefined()
  })

  it('parses out-of-range integers as-is — the range is the core’s job (К-6: 0/21 rejected by settings_set)', () => {
    expect(parseAttemptsInput('0')).toBe(0)
    expect(parseAttemptsInput('21')).toBe(21)
    expect(parseAttemptsInput('-1')).toBe(-1)
  })
})

describe('clampAttempts', () => {
  it('clamps to the 1..20 range', () => {
    expect(clampAttempts(0)).toBe(1)
    expect(clampAttempts(21)).toBe(20)
    expect(clampAttempts(8)).toBe(8)
  })
})
