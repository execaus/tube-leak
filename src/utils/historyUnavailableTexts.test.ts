import { describe, expect, it } from 'vitest'

import type { HistoryUnavailableReason } from '@/types/generated/history'
import { getHistoryUnavailableText } from './historyUnavailableTexts'

describe('getHistoryUnavailableText', () => {
  it('returns the exact design paragraph for each reason', () => {
    expect(getHistoryUnavailableText('newerVersion')).toBe(
      'История недоступна в этом сеансе: файл базы данных создан более новой ' +
        'версией tube-leak — эта версия прочитать его не может. Файл не тронут: ' +
        'откроется снова, когда вы обновите приложение до этой или более новой ' +
        'версии.',
    )
    expect(getHistoryUnavailableText('noAccess')).toBe(
      'История недоступна в этом сеансе: нет доступа на запись в папку данных ' +
        'приложения. Проверьте права доступа к ней и запустите tube-leak снова.',
    )
    expect(getHistoryUnavailableText('migrationFailed')).toBe(
      'История недоступна в этом сеансе: не удалось обновить формат базы данных ' +
        'до текущей версии приложения. Файл остался на прежней версии и не ' +
        'повреждён — сообщите об этом, если увидите снова.',
    )
  })

  it('mutation guard: an unhandled reason throws via assertNever instead of silently returning undefined', () => {
    expect(() => getHistoryUnavailableText('bogus' as HistoryUnavailableReason)).toThrow()
  })
})

describe('getHistoryUnavailableText — storageFailed (TL-91)', () => {
  it('names a read failure of an opened base, distinct from the other reasons', () => {
    const text = getHistoryUnavailableText('storageFailed')
    expect(text).toContain('не удалось прочитать базу данных')
    // Отказ одного ответа, а не сеанса (контракт StorageFailed): текст не обещает недоступность до перезапуска.
    expect(text).not.toContain('в этом сеансе')
    expect(text).not.toContain('перезапустите')
    for (const other of ['newerVersion', 'noAccess', 'migrationFailed'] as const) {
      expect(text).not.toBe(getHistoryUnavailableText(other))
    }
  })
})
