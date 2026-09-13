import { describe, expect, it } from 'vitest'

import type { DownloadProgress } from '@/types/generated/download'
import { getExitDialogText } from './exitDialogTexts'

const TITLE = '«Как приручить дракона» — 1080p'

function activeTask(progress: DownloadProgress) {
  return { displayTitle: TITLE, progress }
}

describe('getExitDialogText — диалог выхода для очереди (дизайн E4, TL-76, С-6)', () => {
  it('заголовок — «Очередь ещё не завершена», не про одну задачу', () => {
    const text = getExitDialogText({ activeTask: activeTask({ phase: 'queued' }), waitingCount: 0 })
    expect(text.heading).toBe('Очередь ещё не завершена')
  })

  it('без процента (Queued): не выдумывает число', () => {
    const text = getExitDialogText({ activeTask: activeTask({ phase: 'queued' }), waitingCount: 0 })
    expect(text.body).toContain('загрузка ещё готовится')
    expect(text.body).not.toMatch(/\d+ %/)
  })

  it('без процента (Fetching): тот же текст, что Queued', () => {
    const text = getExitDialogText({ activeTask: activeTask({ phase: 'fetching' }), waitingCount: 0 })
    expect(text.body).toContain('загрузка ещё готовится')
  })

  it('с процентом (Downloading/running): число совпадает с округлением панели', () => {
    const text = getExitDialogText({
      activeTask: activeTask({ phase: 'downloading', state: 'running', percent: 61.6 }),
      waitingCount: 0,
    })
    expect(text.body).toContain('скачивается (62 %)')
  })

  it('Downloading/running без известного процента — как «ещё готовится», не 0 %', () => {
    const text = getExitDialogText({
      activeTask: activeTask({ phase: 'downloading', state: 'running' }),
      waitingCount: 0,
    })
    expect(text.body).toContain('загрузка ещё готовится')
    expect(text.body).not.toMatch(/\d+ %/)
  })

  it('пауза перед повтором — процент заморожен и помечен «сохранено», как на панели', () => {
    const text = getExitDialogText({
      activeTask: activeTask({
        phase: 'downloading',
        state: 'waitingRetry',
        percent: 62,
        attempt: { number: 2, total: 6 },
        delaySecs: 10,
        remainingSecs: 8,
      }),
      waitingCount: 0,
    })
    expect(text.body).toContain('скачивается (62 %, сохранено)')
  })

  it('Merging — своя честная формулировка, не «ещё готовится» и не выдуманный процент', () => {
    const text = getExitDialogText({ activeTask: activeTask({ phase: 'merging' }), waitingCount: 0 })
    expect(text.body).toContain('идёт склейка видео и звука')
    expect(text.body).not.toContain('ещё готовится')
    expect(text.body).not.toMatch(/\d+ %/)
  })

  it('пауза между задачами на обновление yt-dlp (Р-7) — своя фраза вместо процента, без activeTask', () => {
    const text = getExitDialogText({ pauseReason: 'ytDlpUpdate', waitingCount: 1 })
    expect(text.body).toContain('Между загрузками устанавливается обновлённый yt-dlp.')
    expect(text.body).not.toMatch(/\d+ %/)
  })

  it('счётчик ожидающих: ноль — второе предложение не добавляется', () => {
    const text = getExitDialogText({ activeTask: activeTask({ phase: 'queued' }), waitingCount: 0 })
    expect(text.body).not.toContain('Ещё в очереди')
  })

  it('счётчик ожидающих: согласование числительного — 1 задача / 2 задачи / 5 задач', () => {
    const one = getExitDialogText({ activeTask: activeTask({ phase: 'queued' }), waitingCount: 1 })
    expect(one.body).toContain('Ещё в очереди: 1 задача.')

    const few = getExitDialogText({ activeTask: activeTask({ phase: 'queued' }), waitingCount: 2 })
    expect(few.body).toContain('Ещё в очереди: 2 задачи.')

    const many = getExitDialogText({ activeTask: activeTask({ phase: 'queued' }), waitingCount: 5 })
    expect(many.body).toContain('Ещё в очереди: 5 задач.')
  })

  it('общая часть текста (что останется на диске и как продолжить) — одна на все варианты', () => {
    const withPercent = getExitDialogText({
      activeTask: activeTask({ phase: 'downloading', state: 'running', percent: 10 }),
      waitingCount: 0,
    })
    const withoutPercent = getExitDialogText({ activeTask: activeTask({ phase: 'queued' }), waitingCount: 0 })
    for (const text of [withPercent, withoutPercent]) {
      expect(text.body).toContain('Если выйти сейчас, всё остановится.')
      expect(text.body).toContain('Уже скачанное останется на диске')
      expect(text.body).toContain('Продолжить очередь')
    }
  })

  it('не утверждает про «вставьте ссылку заново» — это стало ложью со снимком очереди (Ф-9)', () => {
    const text = getExitDialogText({ activeTask: activeTask({ phase: 'queued' }), waitingCount: 3 })
    expect(text.body).not.toMatch(/вставьте/i)
    expect(text.body).not.toMatch(/ссылку/i)
  })

  it('заголовок задачи (название + качество) подставляется как есть, без второго парсинга', () => {
    const text = getExitDialogText({ activeTask: activeTask({ phase: 'queued' }), waitingCount: 0 })
    expect(text.body.startsWith(TITLE)).toBe(true)
  })

  it('терминальные фазы не должны встречаться на практике (диалог не показывается), но не падают', () => {
    const done: DownloadProgress = { phase: 'done', fileName: 'x.mp4', folderDisplay: { kind: 'systemDownloads' } }
    expect(() => getExitDialogText({ activeTask: activeTask(done), waitingCount: 0 })).not.toThrow()
  })
})
