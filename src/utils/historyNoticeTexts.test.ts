import { describe, expect, it } from 'vitest'

import type { HistoryNotice } from '@/types/generated/history'
import { getHistoryNoticeText } from './historyNoticeTexts'

describe('getHistoryNoticeText', () => {
  it('returns the exact design text for baseRecreated', () => {
    expect(getHistoryNoticeText({ kind: 'baseRecreated' })).toBe(
      'Файл истории был повреждён — он отложен в сторону с меткой времени, ' +
        'ничего не удалено. Начата новая, пустая история.',
    )
  })

  it('embeds the write-failure cause into the exact design sentence', () => {
    expect(getHistoryNoticeText({ kind: 'lastWriteFailed', cause: 'diskFull' })).toBe(
      'Последняя запись не сохранена: недостаточно места на диске. Загрузка ' +
        'завершена успешно, файл на месте — не сохранилась только строка в истории.',
    )
    expect(getHistoryNoticeText({ kind: 'lastWriteFailed', cause: 'noAccess' })).toContain(
      'Последняя запись не сохранена: нет доступа на запись.',
    )
    expect(getHistoryNoticeText({ kind: 'lastWriteFailed', cause: 'storageFailed' })).toContain(
      'Последняя запись не сохранена: не удалось записать файл истории.',
    )
  })

  it('mutation guard: an unhandled notice kind throws via assertNever', () => {
    expect(() => getHistoryNoticeText({ kind: 'bogus' } as unknown as HistoryNotice)).toThrow()
  })
})
