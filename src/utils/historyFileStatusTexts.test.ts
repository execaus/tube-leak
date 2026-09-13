import { describe, expect, it } from 'vitest'

import { getHistoryFileStatusText } from './historyFileStatusTexts'

describe('getHistoryFileStatusText', () => {
  it('returns undefined for present — the folder line covers that row instead', () => {
    expect(getHistoryFileStatusText({ kind: 'present' }, { kind: 'systemDownloads' })).toBeUndefined()
  })

  it('returns the exact design sentence when missing but the folder still exists, without naming the folder', () => {
    expect(getHistoryFileStatusText({ kind: 'missing', folderExists: true }, { kind: 'custom', path: '/tmp/x' })).toBe(
      'Файл сейчас не на месте — папка существует.',
    )
  })

  it('names the folder when missing and the folder is gone too — quoted for the system folder, bare for a custom path', () => {
    expect(
      getHistoryFileStatusText({ kind: 'missing', folderExists: false }, { kind: 'systemDownloads' }),
    ).toBe('Файл сейчас не на месте, папка «Загрузки» тоже не существует.')
    expect(
      getHistoryFileStatusText({ kind: 'missing', folderExists: false }, { kind: 'custom', path: '/Volumes/Ext/Videos' }),
    ).toBe('Файл сейчас не на месте, папка /Volumes/Ext/Videos тоже не существует.')
  })
})
