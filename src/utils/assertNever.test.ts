import { describe, expect, it } from 'vitest'

import { assertNever } from './assertNever'

describe('assertNever — сторож исчерпывающего switch по объединению контракта (TL-52)', () => {
  it('throws at runtime if a value somehow reaches an exhausted branch', () => {
    // Единственный способ вызвать функцию, не нарушив типы, — привести
    // значение к `never` явно (как в защите теста ниже): в проде до этой
    // точки по контракту дойти невозможно, а рантайм-сообщение — на
    // случай, если контракт всё же нарушен (например, устаревший
    // сгенерированный файл).
    expect(() => assertNever('unexpectedKind' as never)).toThrow(/unexpectedKind/)
  })
})
