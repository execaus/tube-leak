import { describe, expect, it } from 'vitest'

import { pluralizeRu } from './pluralizeRu'

describe('pluralizeRu', () => {
  it('picks "one" for numbers ending in 1, except the 11-14 exception', () => {
    expect(pluralizeRu(1, 'one', 'few', 'many')).toBe('one')
    expect(pluralizeRu(21, 'one', 'few', 'many')).toBe('one')
    expect(pluralizeRu(101, 'one', 'few', 'many')).toBe('one')
  })

  it('picks "few" for 2-4, except the 12-14 exception', () => {
    expect(pluralizeRu(2, 'one', 'few', 'many')).toBe('few')
    expect(pluralizeRu(3, 'one', 'few', 'many')).toBe('few')
    expect(pluralizeRu(4, 'one', 'few', 'many')).toBe('few')
    expect(pluralizeRu(24, 'one', 'few', 'many')).toBe('few')
  })

  it('picks "many" for 0, 5-20 (including the 11-14 exception), and 25+', () => {
    expect(pluralizeRu(0, 'one', 'few', 'many')).toBe('many')
    expect(pluralizeRu(5, 'one', 'few', 'many')).toBe('many')
    expect(pluralizeRu(11, 'one', 'few', 'many')).toBe('many')
    expect(pluralizeRu(12, 'one', 'few', 'many')).toBe('many')
    expect(pluralizeRu(13, 'one', 'few', 'many')).toBe('many')
    expect(pluralizeRu(14, 'one', 'few', 'many')).toBe('many')
    expect(pluralizeRu(25, 'one', 'few', 'many')).toBe('many')
  })
})
