import { describe, expect, it } from 'vitest'

import { getCancelledText, getFailedPartialDataNote } from './downloadOutcomeTexts'

describe('getFailedPartialDataNote — судьба частичных файлов в панели Failed (Ф-8)', () => {
  it('differs for kept vs removed', () => {
    expect(getFailedPartialDataNote('kept')).toContain('осталось на диске')
    expect(getFailedPartialDataNote('removed')).toContain('удалены')
  })
})

describe('getCancelledText — Cancelled: подчистка всегда полная (Ф-4)', () => {
  it('differs whether nothing was created yet (Queued) or data was removed (later phases)', () => {
    expect(getCancelledText('nothingCreated')).toBe('Загрузка отменена до начала скачивания.')
    expect(getCancelledText('removed')).toContain('удалены')
  })
})
