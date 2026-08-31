import { describe, expect, it } from 'vitest'

import { getResumeBannerText, getWaitingStatusText, YT_DLP_UPDATE_PAUSE_TEXT } from './queueTexts'

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
