import { describe, expect, it } from 'vitest'

import type { HistoryCommandErrorKind } from '@/types/generated/history'
import { getHistoryCommandErrorText } from './historyCommandErrorTexts'

describe('getHistoryCommandErrorText', () => {
  it('returns a distinct title/explanation per kind', () => {
    expect(getHistoryCommandErrorText({ kind: 'unknownRecord' }).title).toBe('Запись не найдена')
    expect(getHistoryCommandErrorText({ kind: 'writeFailed' }).title).toBe('Не удалось сохранить изменение')
  })

  it('folds the unavailable reason into the same text as the full-page block', () => {
    const text = getHistoryCommandErrorText({ kind: 'unavailable', reason: 'noAccess' })
    expect(text.explanation).toContain('нет доступа на запись в папку данных')
  })

  it('mutation guard: an unhandled kind throws via assertNever', () => {
    expect(() =>
      getHistoryCommandErrorText({ kind: 'bogus' } as unknown as HistoryCommandErrorKind),
    ).toThrow()
  })
})
