import { describe, expect, it } from 'vitest'

import type { DownloadProgress } from '@/types/download'
import { getExitDialogText } from './exitDialogTexts'

const TITLE = '«Как приручить дракона» — 1080p'

describe('getExitDialogText — диалог подтверждения выхода (Р-2, дизайн E3, TL-46)', () => {
  it('без процента (Queued): не выдумывает число', () => {
    const text = getExitDialogText(TITLE, { phase: 'queued' })
    expect(text.heading).toBe('Загрузка ещё не завершена')
    expect(text.body).toContain('загрузка ещё готовится')
    expect(text.body).not.toMatch(/\d+ %/)
  })

  it('без процента (Fetching): тот же текст, что Queued', () => {
    const text = getExitDialogText(TITLE, { phase: 'fetching' })
    expect(text.body).toContain('загрузка ещё готовится')
  })

  it('с процентом (Downloading/running): число совпадает с округлением панели', () => {
    const text = getExitDialogText(TITLE, { phase: 'downloading', state: 'running', percent: 61.6 })
    expect(text.body).toContain('скачивается (62 %)')
  })

  it('Downloading/running без известного процента — как «ещё готовится», не 0 %', () => {
    const text = getExitDialogText(TITLE, { phase: 'downloading', state: 'running' })
    expect(text.body).toContain('загрузка ещё готовится')
    expect(text.body).not.toMatch(/\d+ %/)
  })

  it('пауза перед повтором — процент заморожен и помечен «сохранено», как на панели', () => {
    const text = getExitDialogText(TITLE, {
      phase: 'downloading',
      state: 'waitingRetry',
      percent: 62,
      attempt: { number: 2, total: 6 },
      delaySecs: 10,
      remainingSecs: 8,
    })
    expect(text.body).toContain('скачивается (62 %, сохранено)')
  })

  it('Merging — своя честная формулировка, не «ещё готовится» и не выдуманный процент', () => {
    const text = getExitDialogText(TITLE, { phase: 'merging' })
    expect(text.body).toContain('идёт склейка видео и звука')
    expect(text.body).not.toContain('ещё готовится')
    expect(text.body).not.toMatch(/\d+ %/)
  })

  it('общая часть текста (что останется на диске и как продолжить) — одна на все варианты', () => {
    const withPercent = getExitDialogText(TITLE, { phase: 'downloading', state: 'running', percent: 10 })
    const withoutPercent = getExitDialogText(TITLE, { phase: 'queued' })
    for (const text of [withPercent, withoutPercent]) {
      expect(text.body).toContain('Если выйти сейчас, загрузка остановится.')
      expect(text.body).toContain('Уже скачанное останется на диске в папке «Загрузки»')
      expect(text.body).toContain('вставьте ту же ссылку ещё раз')
    }
  })

  it('заголовок задачи (название + качество) подставляется как есть, без второго парсинга', () => {
    const text = getExitDialogText(TITLE, { phase: 'queued' })
    expect(text.body.startsWith(TITLE)).toBe(true)
  })

  it('терминальные фазы не должны встречаться на практике (диалог не показывается), но не падают', () => {
    const done: DownloadProgress = { phase: 'done', fileName: 'x.mp4' }
    expect(() => getExitDialogText(TITLE, done)).not.toThrow()
  })
})
