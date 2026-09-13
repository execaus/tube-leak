import { describe, expect, it } from 'vitest'

import { getClearHistoryConfirmBody } from './historyClearDialogTexts'

describe('getClearHistoryConfirmBody', () => {
  it('names the exact count with correct Russian pluralization when it is known to be complete', () => {
    expect(getClearHistoryConfirmBody(1)).toBe(
      'Будут удалены все 1 запись истории. Файлы на диске не тронет ничего — это очищает только список, не содержимое папок.',
    )
    expect(getClearHistoryConfirmBody(3)).toContain('Будут удалены все 3 записи истории.')
    expect(getClearHistoryConfirmBody(42)).toContain('Будут удалены все 42 записи истории.')
    expect(getClearHistoryConfirmBody(5)).toContain('Будут удалены все 5 записей истории.')
  })

  it('omits the number when the loaded pages are not known to be the complete list', () => {
    expect(getClearHistoryConfirmBody(undefined)).toBe(
      'Будут удалены все записи истории. Файлы на диске не тронет ничего — это очищает только список, не содержимое папок.',
    )
  })
})
