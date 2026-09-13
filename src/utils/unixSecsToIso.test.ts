import { describe, expect, it } from 'vitest'

import { unixSecsToIso } from './unixSecsToIso'

describe('unixSecsToIso', () => {
  it('converts Unix seconds UTC to an RFC 3339 string accepted by formatRelativeTime', () => {
    expect(unixSecsToIso(0)).toBe('1970-01-01T00:00:00.000Z')
    expect(unixSecsToIso(1_756_130_400)).toBe('2025-08-25T14:00:00.000Z')
  })
})
