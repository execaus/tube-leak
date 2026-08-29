import { describe, expect, it } from 'vitest'

import type { YtDlpUpdateStatus } from '@/types/generated/update'

import { getYtDlpUpdateStatusText, isYtDlpUpdateStatusFailure } from './ytDlpUpdateStatusText'

/** Фиктивный форматтер «когда» — проверяет только интерполяцию, не формат времени (тот — отдельно, `formatRelativeTime.test.ts`). */
const formatWhen = (iso: string): string => `<${iso}>`

const ACTIVE_VERSION = '2026.08.20'

describe('getYtDlpUpdateStatusText — 14 строк таблицы «Все состояния» дизайна E6', () => {
  it('row 1 — neverChecked', () => {
    const status: YtDlpUpdateStatus = { status: 'neverChecked' }
    expect(getYtDlpUpdateStatusText(status, ACTIVE_VERSION, formatWhen)).toBe(
      'Ещё не проверяли обновления.',
    )
  })

  it('row 2 — checking', () => {
    const status: YtDlpUpdateStatus = { status: 'checking' }
    expect(getYtDlpUpdateStatusText(status, ACTIVE_VERSION, formatWhen)).toBe('Проверяем обновления…')
  })

  it('row 3 — upToDate', () => {
    const status: YtDlpUpdateStatus = { status: 'upToDate', at: '2026-08-25T12:00:00Z' }
    expect(getYtDlpUpdateStatusText(status, ACTIVE_VERSION, formatWhen)).toBe(
      'Проверено <2026-08-25T12:00:00Z> — установлена последняя версия (2026.08.20).',
    )
  })

  it('row 4 — downloading', () => {
    const status: YtDlpUpdateStatus = { status: 'downloading', version: '2026.08.25', percent: 42 }
    expect(getYtDlpUpdateStatusText(status, ACTIVE_VERSION, formatWhen)).toBe(
      'Скачиваем обновление 2026.08.25… 42 %.',
    )
  })

  it('row 5 — preparing', () => {
    const status: YtDlpUpdateStatus = { status: 'preparing', version: '2026.08.25' }
    expect(getYtDlpUpdateStatusText(status, ACTIVE_VERSION, formatWhen)).toBe(
      'Готовим обновление 2026.08.25…',
    )
  })

  it('row 6 — readyWaiting', () => {
    const status: YtDlpUpdateStatus = { status: 'readyWaiting', version: '2026.08.25' }
    expect(getYtDlpUpdateStatusText(status, ACTIVE_VERSION, formatWhen)).toBe(
      'Обновление 2026.08.25 готово — применится, когда закончится текущая загрузка.',
    )
  })

  it('row 7 — updated', () => {
    const status: YtDlpUpdateStatus = {
      status: 'updated',
      at: '2026-08-25T12:00:00Z',
      version: '2026.08.25',
    }
    expect(getYtDlpUpdateStatusText(status, ACTIVE_VERSION, formatWhen)).toBe(
      'Обновлено до 2026.08.25, <2026-08-25T12:00:00Z>.',
    )
  })

  it('row 8 — failed/networkUnavailable', () => {
    const status: YtDlpUpdateStatus = {
      status: 'failed',
      at: '2026-08-25T12:00:00Z',
      failure: { kind: 'networkUnavailable', message: 'connect ETIMEDOUT' },
    }
    expect(getYtDlpUpdateStatusText(status, ACTIVE_VERSION, formatWhen)).toBe(
      'Проверено <2026-08-25T12:00:00Z> (нет соединения с интернетом) — работаем на 2026.08.20.',
    )
  })

  it('row 9 — failed/sourceUnavailable', () => {
    const status: YtDlpUpdateStatus = {
      status: 'failed',
      at: '2026-08-25T12:00:00Z',
      failure: { kind: 'sourceUnavailable', message: 'HTTP 503 from api.github.com' },
    }
    expect(getYtDlpUpdateStatusText(status, ACTIVE_VERSION, formatWhen)).toBe(
      'Проверено <2026-08-25T12:00:00Z> (GitHub не отвечает) — работаем на 2026.08.20.',
    )
  })

  it('row 10 — failed/archiveCorrupted', () => {
    const status: YtDlpUpdateStatus = {
      status: 'failed',
      at: '2026-08-25T12:00:00Z',
      failure: { kind: 'archiveCorrupted', version: '2026.08.25', message: 'sha256 mismatch' },
    }
    expect(getYtDlpUpdateStatusText(status, ACTIVE_VERSION, formatWhen)).toBe(
      'Обновление 2026.08.25 скачалось повреждённым и было отброшено — работаем на 2026.08.20. ' +
        'Попробуем ещё раз позже.',
    )
  })

  it('row 11 — failed/notEnoughSpace', () => {
    const status: YtDlpUpdateStatus = {
      status: 'failed',
      at: '2026-08-25T12:00:00Z',
      failure: { kind: 'notEnoughSpace', version: '2026.08.25', message: 'need 130MiB, have 12MiB' },
    }
    expect(getYtDlpUpdateStatusText(status, ACTIVE_VERSION, formatWhen)).toBe(
      'Не удалось подготовить обновление 2026.08.25 — не хватает места на диске. Работаем на 2026.08.20.',
    )
  })

  it('row 12 — failed/smokeCheckFailed', () => {
    const status: YtDlpUpdateStatus = {
      status: 'failed',
      at: '2026-08-25T12:00:00Z',
      failure: { kind: 'smokeCheckFailed', version: '2026.08.25', message: 'exit code 1' },
    }
    expect(getYtDlpUpdateStatusText(status, ACTIVE_VERSION, formatWhen)).toBe(
      'Обновление 2026.08.25 не прошло проверку запуска и не было установлено — работаем на 2026.08.20.',
    )
  })

  it('row 13 — rolledBack', () => {
    const status: YtDlpUpdateStatus = {
      status: 'rolledBack',
      at: '2026-08-25T12:00:00Z',
      active: '2026.07.11',
      abandoned: '2026.08.20',
    }
    expect(getYtDlpUpdateStatusText(status, ACTIVE_VERSION, formatWhen)).toBe(
      'Возврат выполнен: активна версия 2026.07.11. Автообновление до 2026.08.20 не предложится, ' +
        'пока апстрим не выпустит более новую версию.',
    )
  })

  it('row 14 — rollbackWaiting (uses the target version from the status, not the activeVersion parameter)', () => {
    const status: YtDlpUpdateStatus = { status: 'rollbackWaiting', version: '2026.07.11' }
    expect(getYtDlpUpdateStatusText(status, ACTIVE_VERSION, formatWhen)).toBe(
      'Возврат к 2026.07.11 принят — применится, когда закончится текущая загрузка.',
    )
  })
})

describe('getYtDlpUpdateStatusText — активная версия отсутствует (sidecar ещё не ok)', () => {
  it('falls back to a placeholder instead of rendering "undefined" in the sentence', () => {
    const status: YtDlpUpdateStatus = { status: 'upToDate', at: '2026-08-25T12:00:00Z' }
    expect(getYtDlpUpdateStatusText(status, undefined, formatWhen)).toBe(
      'Проверено <2026-08-25T12:00:00Z> — установлена последняя версия (—).',
    )
  })
})

describe('isYtDlpUpdateStatusFailure', () => {
  it('is true only for the failed status (rows 8–12) — the one muted-color case', () => {
    expect(
      isYtDlpUpdateStatusFailure({
        status: 'failed',
        at: '2026-08-25T12:00:00Z',
        failure: { kind: 'networkUnavailable', message: 'x' },
      }),
    ).toBe(true)
    expect(isYtDlpUpdateStatusFailure({ status: 'neverChecked' })).toBe(false)
    expect(isYtDlpUpdateStatusFailure({ status: 'upToDate', at: '2026-08-25T12:00:00Z' })).toBe(false)
  })
})
