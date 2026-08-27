import { describe, expect, it } from 'vitest'

import { knownKindsOf } from './knownKinds'

type Sample = 'a' | 'b' | 'c'

describe('knownKindsOf — белый список, выведенный из типа (TL-52)', () => {
  it('returns the keys of a `satisfies`-checked whitelist as an array', () => {
    const kinds = knownKindsOf({ a: true, b: true, c: true } satisfies Record<Sample, true>)
    expect(kinds).toEqual(['a', 'b', 'c'])
  })

  /**
   * Доказательство мутацией на уровне типов, а не рантайма: пропуск
   * значения объединения в объекте, помеченном `satisfies Record<Sample,
   * true>`, — ошибка компиляции самого литерала (TS1360), не зависящая от
   * того, был ли явно указан generic-параметр вызова (ревью TL-52: первая
   * версия этой функции ловила пропуск, только если `<Sample>` был назван
   * явно, а без него компилировалась молча).
   */
  it('rejects a whitelist missing a variant at compile time', () => {
    // @ts-expect-error пропущен вариант 'c' — `satisfies Record<Sample, true>` требует все три.
    knownKindsOf({ a: true, b: true } satisfies Record<Sample, true>)
  })

  it('rejects a whitelist with an extra key that is not part of the union', () => {
    // @ts-expect-error 'd' не входит в Sample — избыточное свойство литерала.
    knownKindsOf({ a: true, b: true, c: true, d: true } satisfies Record<Sample, true>)
  })

  it('does not silently narrow when the generic is inferred from a plain object without `satisfies`', () => {
    // Без `satisfies` вызов остаётся синтаксически валидным (см. doc
    // `knownKindsOf`) — но это не тот путь, которым код проекта
    // пользуется: все настоящие вызовы держат проверку на `satisfies`, а
    // не на этом инфере. Тест документирует границу, а не одобряет её
    // использование.
    const kinds = knownKindsOf({ a: true, b: true })
    expect(kinds).toEqual(['a', 'b'])
  })
})
