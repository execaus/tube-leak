import { describe, expect, it } from 'vitest'

import { knownKindsOf } from './knownKinds'

type Sample = 'a' | 'b' | 'c'

describe('knownKindsOf — белый список, выведенный из типа (TL-52)', () => {
  it('returns the keys of the whitelist record as an array', () => {
    const kinds = knownKindsOf<Sample>({ a: true, b: true, c: true })
    expect(kinds).toEqual(['a', 'b', 'c'])
  })

  /**
   * Доказательство мутацией на уровне типов, а не рантайма: пропуск
   * значения объединения — недостающее свойство `Record<Sample, true>`, и
   * без `@ts-expect-error` эта строка не прошла бы `npm run type-check`.
   * Это тот самый механизм, который заменяет собой ручной массив
   * `KNOWN_ERROR_KINDS`.
   */
  it('rejects a whitelist missing a variant at compile time', () => {
    // @ts-expect-error пропущен вариант 'c' — Record<Sample, true> требует все три.
    knownKindsOf<Sample>({ a: true, b: true })
  })

  it('rejects a whitelist with an extra key that is not part of the union', () => {
    // @ts-expect-error 'd' не входит в Sample — избыточное свойство литерала.
    knownKindsOf<Sample>({ a: true, b: true, c: true, d: true })
  })
})
