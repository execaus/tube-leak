import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import type { YtDlpUpdateSnapshot } from '@/types/generated/update'

import YtDlpUpdateBlock from './YtDlpUpdateBlock.vue'

const ACTIVE_VERSION = '2026.08.20'

function mountBlock(props: { snapshot?: YtDlpUpdateSnapshot; activeVersion?: string }) {
  return mount(YtDlpUpdateBlock, { props })
}

function statusText(wrapper: ReturnType<typeof mountBlock>): string {
  return wrapper.get('.ytdlp-update-block__status').text()
}

function checkButton(wrapper: ReturnType<typeof mountBlock>) {
  return wrapper.findAll('button').find((btn) => btn.text() === 'Проверить сейчас')
}

function rollbackButton(wrapper: ReturnType<typeof mountBlock>) {
  return wrapper.findAll('button').find((btn) => btn.text().startsWith('Вернуться к'))
}

describe('YtDlpUpdateBlock — 14 состояний таблицы «Все состояния» дизайна E6', () => {
  it('row 1 — neverChecked: только «Проверить сейчас», активна', () => {
    const wrapper = mountBlock({
      snapshot: { busy: false, status: 'neverChecked' },
      activeVersion: ACTIVE_VERSION,
    })

    expect(statusText(wrapper)).toBe('Ещё не проверяли обновления.')
    expect(checkButton(wrapper)?.attributes('disabled')).toBeUndefined()
    expect(rollbackButton(wrapper)).toBeUndefined()
  })

  it('row 2 — checking: обе кнопки неактивны, «Вернуться» не рисуется без цели', () => {
    const wrapper = mountBlock({
      snapshot: { busy: true, status: 'checking' },
      activeVersion: ACTIVE_VERSION,
    })

    expect(statusText(wrapper)).toBe('Проверяем обновления…')
    expect(checkButton(wrapper)?.attributes('disabled')).toBeDefined()
    expect(rollbackButton(wrapper)).toBeUndefined()
  })

  it('row 3 — upToDate, без цели отката: «Проверить сейчас», без «Вернуться»', () => {
    const wrapper = mountBlock({
      snapshot: { busy: false, status: 'upToDate', at: '2026-08-25T12:00:00Z' },
      activeVersion: ACTIVE_VERSION,
    })

    expect(statusText(wrapper)).toContain('установлена последняя версия (2026.08.20)')
    expect(checkButton(wrapper)?.attributes('disabled')).toBeUndefined()
    expect(rollbackButton(wrapper)).toBeUndefined()
  })

  it('row 3 — upToDate, с целью отката: «Проверить сейчас» · «Вернуться к Y», если есть', () => {
    const wrapper = mountBlock({
      snapshot: {
        busy: false,
        status: 'upToDate',
        at: '2026-08-25T12:00:00Z',
        rollbackTarget: '2026.07.11',
      },
      activeVersion: ACTIVE_VERSION,
    })

    expect(checkButton(wrapper)?.attributes('disabled')).toBeUndefined()
    const rollback = rollbackButton(wrapper)
    expect(rollback?.text()).toBe('Вернуться к 2026.07.11')
    expect(rollback?.attributes('disabled')).toBeUndefined()
  })

  it('row 4 — downloading: обе кнопки неактивны, показывает версию и процент', () => {
    const wrapper = mountBlock({
      snapshot: { busy: true, status: 'downloading', version: '2026.08.25', percent: 42 },
      activeVersion: ACTIVE_VERSION,
    })

    expect(statusText(wrapper)).toBe('Скачиваем обновление 2026.08.25… 42 %.')
    expect(checkButton(wrapper)?.attributes('disabled')).toBeDefined()
    expect(rollbackButton(wrapper)).toBeUndefined()
  })

  it('row 5 — preparing: обе кнопки неактивны', () => {
    const wrapper = mountBlock({
      snapshot: { busy: true, status: 'preparing', version: '2026.08.25' },
      activeVersion: ACTIVE_VERSION,
    })

    expect(statusText(wrapper)).toBe('Готовим обновление 2026.08.25…')
    expect(checkButton(wrapper)?.attributes('disabled')).toBeDefined()
    expect(rollbackButton(wrapper)).toBeUndefined()
  })

  it('row 6 — readyWaiting: обе кнопки неактивны', () => {
    const wrapper = mountBlock({
      snapshot: { busy: true, status: 'readyWaiting', version: '2026.08.25' },
      activeVersion: ACTIVE_VERSION,
    })

    expect(statusText(wrapper)).toBe(
      'Обновление 2026.08.25 готово — применится, когда закончится текущая загрузка.',
    )
    expect(checkButton(wrapper)?.attributes('disabled')).toBeDefined()
    expect(rollbackButton(wrapper)).toBeUndefined()
  })

  it('row 7 — updated: «Проверить сейчас» · «Вернуться к X», обе активны', () => {
    const wrapper = mountBlock({
      snapshot: {
        busy: false,
        status: 'updated',
        at: '2026-08-25T12:00:00Z',
        version: '2026.08.25',
        rollbackTarget: '2026.08.20',
      },
      activeVersion: '2026.08.25',
    })

    expect(statusText(wrapper)).toContain('Обновлено до 2026.08.25,')
    expect(checkButton(wrapper)?.attributes('disabled')).toBeUndefined()
    const rollback = rollbackButton(wrapper)
    expect(rollback?.text()).toBe('Вернуться к 2026.08.20')
    expect(rollback?.attributes('disabled')).toBeUndefined()
  })

  it('row 8 — failed/networkUnavailable: только «Проверить сейчас», приглушённый цвет', () => {
    const wrapper = mountBlock({
      snapshot: {
        busy: false,
        status: 'failed',
        at: '2026-08-25T12:00:00Z',
        failure: { kind: 'networkUnavailable', message: 'connect ETIMEDOUT' },
      },
      activeVersion: ACTIVE_VERSION,
    })

    expect(statusText(wrapper)).toContain('нет соединения с интернетом')
    expect(statusText(wrapper)).toContain('работаем на 2026.08.20')
    expect(checkButton(wrapper)?.attributes('disabled')).toBeUndefined()
    expect(rollbackButton(wrapper)).toBeUndefined()
    expect(wrapper.get('.ytdlp-update-block__status').classes()).toContain(
      'ytdlp-update-block__status--muted',
    )
  })

  it('row 9 — failed/sourceUnavailable: только «Проверить сейчас»', () => {
    const wrapper = mountBlock({
      snapshot: {
        busy: false,
        status: 'failed',
        at: '2026-08-25T12:00:00Z',
        failure: { kind: 'sourceUnavailable', message: 'HTTP 503' },
      },
      activeVersion: ACTIVE_VERSION,
    })

    expect(statusText(wrapper)).toContain('GitHub не отвечает')
    expect(checkButton(wrapper)?.attributes('disabled')).toBeUndefined()
    expect(rollbackButton(wrapper)).toBeUndefined()
  })

  it('row 10 — failed/archiveCorrupted: «скачалось повреждённым», без «Вернуться»', () => {
    const wrapper = mountBlock({
      snapshot: {
        busy: false,
        status: 'failed',
        at: '2026-08-25T12:00:00Z',
        failure: { kind: 'archiveCorrupted', version: '2026.08.25', message: 'sha256 mismatch' },
      },
      activeVersion: ACTIVE_VERSION,
    })

    expect(statusText(wrapper)).toBe(
      'Обновление 2026.08.25 скачалось повреждённым и было отброшено — работаем на 2026.08.20. ' +
        'Попробуем ещё раз позже.',
    )
    expect(checkButton(wrapper)?.attributes('disabled')).toBeUndefined()
    expect(rollbackButton(wrapper)).toBeUndefined()
  })

  it('row 11 — failed/notEnoughSpace: «не хватает места на диске»', () => {
    const wrapper = mountBlock({
      snapshot: {
        busy: false,
        status: 'failed',
        at: '2026-08-25T12:00:00Z',
        failure: { kind: 'notEnoughSpace', version: '2026.08.25', message: 'need 130MiB' },
      },
      activeVersion: ACTIVE_VERSION,
    })

    expect(statusText(wrapper)).toBe(
      'Не удалось подготовить обновление 2026.08.25 — не хватает места на диске. Работаем на 2026.08.20.',
    )
    expect(checkButton(wrapper)?.attributes('disabled')).toBeUndefined()
    expect(rollbackButton(wrapper)).toBeUndefined()
  })

  it('row 12 — failed/smokeCheckFailed: «не прошло проверку запуска»', () => {
    const wrapper = mountBlock({
      snapshot: {
        busy: false,
        status: 'failed',
        at: '2026-08-25T12:00:00Z',
        failure: { kind: 'smokeCheckFailed', version: '2026.08.25', message: 'exit code 1' },
      },
      activeVersion: ACTIVE_VERSION,
    })

    expect(statusText(wrapper)).toBe(
      'Обновление 2026.08.25 не прошло проверку запуска и не было установлено — работаем на 2026.08.20.',
    )
    expect(checkButton(wrapper)?.attributes('disabled')).toBeUndefined()
    expect(rollbackButton(wrapper)).toBeUndefined()
  })

  it('row 13 — rolledBack: «Проверить сейчас» · «Вернуться к Y», обе активны', () => {
    const wrapper = mountBlock({
      snapshot: {
        busy: false,
        status: 'rolledBack',
        at: '2026-08-25T12:00:00Z',
        active: '2026.07.11',
        abandoned: '2026.08.20',
        rollbackTarget: '2026.08.20',
      },
      activeVersion: '2026.07.11',
    })

    expect(statusText(wrapper)).toBe(
      'Возврат выполнен: активна версия 2026.07.11. Автообновление до 2026.08.20 не предложится, ' +
        'пока апстрим не выпустит более новую версию.',
    )
    expect(checkButton(wrapper)?.attributes('disabled')).toBeUndefined()
    const rollback = rollbackButton(wrapper)
    expect(rollback?.text()).toBe('Вернуться к 2026.08.20')
    expect(rollback?.attributes('disabled')).toBeUndefined()
  })

  it('row 14 — rollbackWaiting: обе кнопки неактивны, версия — цель возврата, а не activeVersion', () => {
    const wrapper = mountBlock({
      snapshot: {
        busy: true,
        status: 'rollbackWaiting',
        version: '2026.07.11',
        rollbackTarget: '2026.08.20',
      },
      activeVersion: '2026.08.20',
    })

    expect(statusText(wrapper)).toBe(
      'Возврат к 2026.07.11 принят — применится, когда закончится текущая загрузка.',
    )
    expect(checkButton(wrapper)?.attributes('disabled')).toBeDefined()
    const rollback = rollbackButton(wrapper)
    expect(rollback?.text()).toBe('Вернуться к 2026.08.20')
    expect(rollback?.attributes('disabled')).toBeDefined()
  })
})

describe('YtDlpUpdateBlock — общее правило видимости «Вернуться к …»', () => {
  it('is rendered but disabled while busy, whenever a rollback target is present (not hidden)', () => {
    const wrapper = mountBlock({
      snapshot: { busy: true, status: 'checking', rollbackTarget: '2026.07.11' },
      activeVersion: ACTIVE_VERSION,
    })

    const rollback = rollbackButton(wrapper)
    expect(rollback).toBeDefined()
    expect(rollback?.attributes('disabled')).toBeDefined()
  })

  it('is never rendered when there is no rollback target, regardless of status', () => {
    const wrapper = mountBlock({
      snapshot: { busy: false, status: 'neverChecked' },
      activeVersion: ACTIVE_VERSION,
    })

    expect(rollbackButton(wrapper)).toBeUndefined()
  })
})

describe('YtDlpUpdateBlock — предзагрузочная пауза (снимок ещё не пришёл)', () => {
  it('shows a neutral placeholder and disables the check button until the first snapshot arrives', () => {
    const wrapper = mountBlock({ activeVersion: ACTIVE_VERSION })

    expect(statusText(wrapper)).toBe('Загружаем статус обновления…')
    expect(checkButton(wrapper)?.attributes('disabled')).toBeDefined()
    expect(rollbackButton(wrapper)).toBeUndefined()
  })
})

describe('YtDlpUpdateBlock — клики', () => {
  it('emits "check" when the check button is clicked', async () => {
    const wrapper = mountBlock({
      snapshot: { busy: false, status: 'neverChecked' },
      activeVersion: ACTIVE_VERSION,
    })

    await checkButton(wrapper)?.trigger('click')

    expect(wrapper.emitted('check')).toHaveLength(1)
  })

  it('emits "rollback" when the rollback button is clicked', async () => {
    const wrapper = mountBlock({
      snapshot: { busy: false, status: 'neverChecked', rollbackTarget: '2026.07.11' },
      activeVersion: ACTIVE_VERSION,
    })

    await rollbackButton(wrapper)?.trigger('click')

    expect(wrapper.emitted('rollback')).toHaveLength(1)
  })

  it('does not emit "check" when the button is disabled', async () => {
    const wrapper = mountBlock({
      snapshot: { busy: true, status: 'checking' },
      activeVersion: ACTIVE_VERSION,
    })

    await checkButton(wrapper)?.trigger('click')

    expect(wrapper.emitted('check')).toBeUndefined()
  })
})

describe('YtDlpUpdateBlock — доступность', () => {
  it('marks the whole block as an aria-live="polite" region (design E6: no interrupting modals)', () => {
    const wrapper = mountBlock({
      snapshot: { busy: false, status: 'neverChecked' },
      activeVersion: ACTIVE_VERSION,
    })

    expect(wrapper.get('.ytdlp-update-block').attributes('aria-live')).toBe('polite')
  })
})

/*
 * Сторож задачи TL-59 (issue execaus/tube-leak#61): удаление декларации
 * `color` из правила `.ytdlp-update-block__status--muted` не должно
 * проходить незамеченным. jsdom не применяет scoped-стили SFC, поэтому
 * «смонтируй компонент и проверь вычисленный цвет» здесь не работает —
 * этот тест вместо этого разбирает **исходный** блок `<style>` файла
 * `YtDlpUpdateBlock.vue` и проверяет, что правило `--muted` содержит
 * непустую декларацию `color`.
 *
 * От какого дефекта стережёт: пустое CSS-правило (`.foo {}`) вырезается
 * сборщиком из итогового бандла целиком (проверено сборкой, issue #61) —
 * тогда пять классов отказа (строки 8–12 таблицы «Все состояния»)
 * становятся пиксель-в-пиксель равны обычным состояниям, что нарушает
 * дизайн E6. До этого теста такая регрессия не роняла ни один из
 * существующих тестов компонента, потому что они проверяют только имя
 * класса на элементе (например, row 8 выше), а не содержимое правила.
 */
describe('YtDlpUpdateBlock — CSS-сторож правила --muted (TL-59)', () => {
  it('rule .ytdlp-update-block__status--muted has a non-empty "color" declaration', () => {
    // `new URL('./file.vue', import.meta.url)` не годится: Vite перехватывает
    // этот паттерн как asset-импорт и переписывает его в dev-server URL
    // (`http://localhost:.../...vue`), а не в файловый путь — отсюда путь
    // собирается через `node:path`, а не через `URL`.
    const componentDir = dirname(fileURLToPath(import.meta.url))
    const componentPath = join(componentDir, 'YtDlpUpdateBlock.vue')
    const source = readFileSync(componentPath, 'utf-8')

    const styleMatch = source.match(/<style[^>]*>([\s\S]*?)<\/style>/)
    expect(styleMatch, 'component must have a <style> block').not.toBeNull()
    const styleBlock = styleMatch?.[1] ?? ''

    const ruleMatch = styleBlock.match(
      /\.ytdlp-update-block__status--muted\s*\{([^}]*)\}/,
    )
    expect(ruleMatch, 'rule .ytdlp-update-block__status--muted must exist').not.toBeNull()
    const ruleBody = ruleMatch?.[1] ?? ''

    const colorMatch = ruleBody.match(/color\s*:\s*([^;]+);/)
    expect(colorMatch, 'rule must declare a non-empty "color"').not.toBeNull()
    expect((colorMatch?.[1] ?? '').trim().length).toBeGreaterThan(0)
  })
})
