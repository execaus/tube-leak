import { describe, expect, it } from 'vitest'

import { validateNameTemplateDraft } from './nameTemplateClientCheck'

describe('validateNameTemplateDraft', () => {
  it('accepts a template built only from known variables and literals', () => {
    expect(validateNameTemplateDraft('{title} [{quality}]')).toBeUndefined()
    expect(validateNameTemplateDraft('{id} — {title}')).toBeUndefined()
    expect(validateNameTemplateDraft('{title}')).toBeUndefined()
  })

  it('rejects an empty template as noVariables', () => {
    expect(validateNameTemplateDraft('')).toStrictEqual({ kind: 'noVariables' })
  })

  it('rejects a template made only of literals as noVariables', () => {
    expect(validateNameTemplateDraft('video')).toStrictEqual({ kind: 'noVariables' })
  })

  it('reports an unknown variable name with the position of its opening brace', () => {
    expect(validateNameTemplateDraft('{title} [{channel}]')).toStrictEqual({
      kind: 'unknownVariable',
      position: 10,
      name: 'channel',
    })
  })

  it('reports an unclosed brace at the position of the opening brace', () => {
    expect(validateNameTemplateDraft('{title} [{quality')).toStrictEqual({
      kind: 'unclosedBrace',
      position: 10,
    })
  })

  it('reports a stray closing brace at its own position', () => {
    expect(validateNameTemplateDraft('{title}}')).toStrictEqual({
      kind: 'strayClosingBrace',
      position: 8,
    })
  })

  it('reports the first problem when several are present, left to right', () => {
    expect(validateNameTemplateDraft('}{title}')).toStrictEqual({
      kind: 'strayClosingBrace',
      position: 1,
    })
  })

  it('counts positions in Unicode characters, not UTF-16 units (surrogate pair before the brace)', () => {
    // '😀' — суррогатная пара в UTF-16, но один символ Unicode/`Array.from`.
    expect(validateNameTemplateDraft('😀{channel}')).toStrictEqual({
      kind: 'unknownVariable',
      position: 2,
      name: 'channel',
    })
  })
})
