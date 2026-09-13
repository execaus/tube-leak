import { describe, expect, it } from 'vitest'

import {
  getActiveQueueStatusText,
  getResumeBannerText,
  getStatusRowWaitingText,
  getWaitingStatusText,
  STATUS_ROW_YT_DLP_UPDATE_PAUSE_TEXT,
  YT_DLP_UPDATE_PAUSE_TEXT,
} from './queueTexts'

describe('getWaitingStatusText — обычный ход очереди (не awaitingContinue)', () => {
  it('zero ahead: "начнётся после текущей загрузки", без "и ещё N"', () => {
    expect(getWaitingStatusText(0, false)).toBe('В очереди — начнётся после текущей загрузки')
  })

  it('one ahead: genitive singular "задачи" (дизайн, пример «Урок кулинарии»)', () => {
    expect(getWaitingStatusText(1, false)).toBe(
      'В очереди — начнётся после текущей загрузки и ещё 1 задачи',
    )
  })

  it('two ahead: genitive plural "задач" (дизайн, пример «Летний влог»)', () => {
    expect(getWaitingStatusText(2, false)).toBe(
      'В очереди — начнётся после текущей загрузки и ещё 2 задач',
    )
  })
})

describe('getWaitingStatusText — очередь приостановлена после перезапуска (awaitingContinue)', () => {
  it('zero ahead: "начнётся первой, как только вы продолжите"', () => {
    expect(getWaitingStatusText(0, true)).toBe('В очереди — начнётся первой, как только вы продолжите')
  })

  it('one ahead: "после 1 задачи, как только вы продолжите"', () => {
    expect(getWaitingStatusText(1, true)).toBe(
      'В очереди — начнётся после 1 задачи, как только вы продолжите',
    )
  })

  it('two ahead: "после 2 задач, как только вы продолжите"', () => {
    expect(getWaitingStatusText(2, true)).toBe(
      'В очереди — начнётся после 2 задач, как только вы продолжите',
    )
  })
})

describe('getResumeBannerText', () => {
  it('states the exact count with nominative agreement and the no-network-yet invariant (Н-1)', () => {
    expect(getResumeBannerText(3)).toBe(
      'Очередь приостановлена после перезапуска — 3 задачи ждут. ' +
        'Сетевых обращений не будет, пока вы не нажмёте «Продолжить».',
    )
  })

  it('singular agreement for exactly one task', () => {
    expect(getResumeBannerText(1)).toBe(
      'Очередь приостановлена после перезапуска — 1 задача ждёт. ' +
        'Сетевых обращений не будет, пока вы не нажмёте «Продолжить».',
    )
  })

  it('"many" agreement for five or more', () => {
    expect(getResumeBannerText(5)).toBe(
      'Очередь приостановлена после перезапуска — 5 задач ждут. ' +
        'Сетевых обращений не будет, пока вы не нажмёте «Продолжить».',
    )
  })
})

describe('YT_DLP_UPDATE_PAUSE_TEXT', () => {
  it('is a non-empty, self-contained explanation — not borrowed from YtDlpUpdateBlock', () => {
    expect(YT_DLP_UPDATE_PAUSE_TEXT.length).toBeGreaterThan(0)
    expect(YT_DLP_UPDATE_PAUSE_TEXT).toContain('yt-dlp')
  })
})

describe('STATUS_ROW_YT_DLP_UPDATE_PAUSE_TEXT (TL-92, правки ревью, С-3)', () => {
  it('is exactly the design copy, without the "usually under a minute" tail that the full queue section carries', () => {
    expect(STATUS_ROW_YT_DLP_UPDATE_PAUSE_TEXT).toBe(
      'Между загрузками устанавливается обновлённый yt-dlp',
    )
  })

  it('is a different string object from the queue-section constant, not a re-export under a new name', () => {
    expect(STATUS_ROW_YT_DLP_UPDATE_PAUSE_TEXT).not.toBe(YT_DLP_UPDATE_PAUSE_TEXT)
    expect(YT_DLP_UPDATE_PAUSE_TEXT).toContain(STATUS_ROW_YT_DLP_UPDATE_PAUSE_TEXT)
  })
})

describe('getActiveQueueStatusText (TL-92, строка состояния очереди)', () => {
  it('fetching: title · phase label, no percent even if one was somehow passed', () => {
    expect(getActiveQueueStatusText('«Ролик A» — 1080p', 'fetching', 42)).toBe(
      '«Ролик A» — 1080p · Подготовка',
    )
  })

  it('merging: title · phase label, no percent even if one was somehow passed', () => {
    expect(getActiveQueueStatusText('«Ролик A» — 1080p', 'merging', 42)).toBe(
      '«Ролик A» — 1080p · Склейка',
    )
  })

  it('downloading with a known percent: title · phase label · rounded percent', () => {
    expect(getActiveQueueStatusText('«Как приручить дракона» — 1080p', 'downloading', 61.7)).toBe(
      '«Как приручить дракона» — 1080p · Скачивание · 62 %',
    )
  })

  it('downloading without a known percent yet: no percent segment', () => {
    expect(getActiveQueueStatusText('«Ролик A» — 1080p', 'downloading')).toBe(
      '«Ролик A» — 1080p · Скачивание',
    )
  })
})

describe('getStatusRowWaitingText (TL-92, строка состояния — очередь приостановлена)', () => {
  it('singular agreement for exactly one task', () => {
    expect(getStatusRowWaitingText(1)).toBe('Очередь приостановлена — 1 задача ждёт')
  })

  it('few agreement for 2-4', () => {
    expect(getStatusRowWaitingText(3)).toBe('Очередь приостановлена — 3 задачи ждут')
  })

  it('many agreement for 5+', () => {
    expect(getStatusRowWaitingText(5)).toBe('Очередь приостановлена — 5 задач ждут')
  })
})
