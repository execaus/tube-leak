import { describe, expect, it } from 'vitest'

import { getCancelledText, getFailedPartialDataNote, getFolderDisplayText } from './downloadOutcomeTexts'

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

describe('getFolderDisplayText — папка готового файла называется настоящая (Ф-15, TL-95)', () => {
  it('renders the system Downloads folder in quotes, like the old hardcoded text', () => {
    expect(getFolderDisplayText({ kind: 'systemDownloads' })).toBe('«Загрузки»')
  })

  it('renders a custom folder as the raw path, unquoted and unshortened, including spaces and unicode', () => {
    const path = '/Users/исполнитель/Movies/Мой длинный путь с пробелами/YouTube Downloads'
    expect(getFolderDisplayText({ kind: 'custom', path })).toBe(path)
  })
})
