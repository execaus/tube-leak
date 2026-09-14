import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import type { YtDlpUpdateSnapshot } from '@/types/generated/update'

import YtDlpUpdateBlock from './YtDlpUpdateBlock.vue'

const ACTIVE_VERSION = '2026.08.20'

function mountBlock(props: { snapshot?: YtDlpUpdateSnapshot; activeVersion?: string; pending?: boolean }) {
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

/** Кнопка «Вернуться» внутри инлайн-подтверждения (TL-60) — отличается от {@link rollbackButton} точным текстом без «к …». */
function confirmRollbackButton(wrapper: ReturnType<typeof mountBlock>) {
  return wrapper.findAll('button').find((btn) => btn.text() === 'Вернуться')
}

function cancelRollbackButton(wrapper: ReturnType<typeof mountBlock>) {
  return wrapper.findAll('button').find((btn) => btn.text() === 'Отмена')
}

function rollbackConfirmPanel(wrapper: ReturnType<typeof mountBlock>) {
  return wrapper.find('.ytdlp-update-block__rollback-confirm')
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

  it('clicking the toggle button does NOT emit "rollback" immediately (Р-3: it opens the inline confirmation instead)', async () => {
    const wrapper = mountBlock({
      snapshot: { busy: false, status: 'neverChecked', rollbackTarget: '2026.07.11' },
      activeVersion: ACTIVE_VERSION,
    })

    await rollbackButton(wrapper)?.trigger('click')

    expect(wrapper.emitted('rollback')).toBeUndefined()
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

/*
 * TL-60 (Р-3, issue execaus/tube-leak#62): инлайн-подтверждение отката.
 * «Клик по «Вернуться к {версия}» разворачивает инлайн-подтверждение на
 * месте блока (текст с версиями, кнопки «Вернуться»/«Отмена»), без
 * role="dialog"» — критерии приёмки задачи закрываются здесь: «Отмена»
 * не вызывает команду отката и не меняет статус-строку; во время
 * активной загрузки клик «Вернуться» переводит блок в состояние 14, не
 * в 13 немедленно.
 */
describe('YtDlpUpdateBlock — инлайн-подтверждение отката (TL-60, Р-3)', () => {
  const snapshotWithTarget: YtDlpUpdateSnapshot = {
    busy: false,
    status: 'upToDate',
    at: '2026-08-25T12:00:00Z',
    rollbackTarget: '2026.07.11',
  }

  it('is not rendered before the toggle button is clicked', () => {
    const wrapper = mountBlock({ snapshot: snapshotWithTarget, activeVersion: ACTIVE_VERSION })

    expect(rollbackConfirmPanel(wrapper).exists()).toBe(false)
  })

  it('clicking "Вернуться к …" reveals the confirmation with both versions named, and no role="dialog" anywhere in the block', async () => {
    const wrapper = mountBlock({ snapshot: snapshotWithTarget, activeVersion: ACTIVE_VERSION })

    await rollbackButton(wrapper)?.trigger('click')

    const panel = rollbackConfirmPanel(wrapper)
    expect(panel.exists()).toBe(true)
    expect(panel.text()).toContain('2026.07.11')
    expect(panel.text()).toContain(ACTIVE_VERSION)
    expect(confirmRollbackButton(wrapper)).toBeDefined()
    expect(cancelRollbackButton(wrapper)).toBeDefined()
    // Design, «Ручной откат — полный путь»: инлайн-раскрытие, не модалка.
    expect(wrapper.find('[role="dialog"]').exists()).toBe(false)
  })

  it('clicking the toggle button again collapses an already-open confirmation', async () => {
    const wrapper = mountBlock({ snapshot: snapshotWithTarget, activeVersion: ACTIVE_VERSION })

    await rollbackButton(wrapper)?.trigger('click')
    expect(rollbackConfirmPanel(wrapper).exists()).toBe(true)

    await rollbackButton(wrapper)?.trigger('click')
    expect(rollbackConfirmPanel(wrapper).exists()).toBe(false)
  })

  /* Критерий приёмки: «Отмена» не вызывает команду отката и не меняет статус-строку. */
  it('"Отмена" closes the confirmation without emitting "rollback" and without changing the status text', async () => {
    const wrapper = mountBlock({ snapshot: snapshotWithTarget, activeVersion: ACTIVE_VERSION })
    const textBefore = statusText(wrapper)

    await rollbackButton(wrapper)?.trigger('click')
    await cancelRollbackButton(wrapper)?.trigger('click')

    expect(rollbackConfirmPanel(wrapper).exists()).toBe(false)
    expect(wrapper.emitted('rollback')).toBeUndefined()
    expect(statusText(wrapper)).toBe(textBefore)
  })

  it('"Вернуться" inside the confirmation emits "rollback" exactly once and closes the panel', async () => {
    const wrapper = mountBlock({ snapshot: snapshotWithTarget, activeVersion: ACTIVE_VERSION })

    await rollbackButton(wrapper)?.trigger('click')
    await confirmRollbackButton(wrapper)?.trigger('click')

    expect(wrapper.emitted('rollback')).toHaveLength(1)
    expect(rollbackConfirmPanel(wrapper).exists()).toBe(false)
  })

  /*
   * Критерий приёмки: «во время активной загрузки клик «Вернуться»
   * переводит блок в состояние 14 (ожидание границы), не в 13 (уже
   * применено) немедленно». Компонент не решает это сам (doc компонента,
   * «Инлайн-подтверждение отката») — он только эмитит и ждёт снимок от
   * родителя. Доказывается в два шага: сразу после клика (до того, как
   * родитель успел бы что-то прислать) блок ещё не утверждает ничего
   * нового — старый статус-текст на месте, никакого локального прыжка
   * в «применено»; когда родитель присылает `rollbackWaiting` (то, что
   * вернула бы команда при активной загрузке — Ф-7), блок показывает
   * ровно строку 14, а не строку 13.
   */
  it('does not locally jump to "applied" (row 13) on click — reflects rollbackWaiting (row 14) only once that snapshot arrives via props', async () => {
    const wrapper = mountBlock({ snapshot: snapshotWithTarget, activeVersion: ACTIVE_VERSION })
    const textBeforeClick = statusText(wrapper)

    await rollbackButton(wrapper)?.trigger('click')
    await confirmRollbackButton(wrapper)?.trigger('click')

    // Синхронно после клика — ещё ничего не известно об исходе, никакого
    // локального предположения о «применено немедленно».
    expect(statusText(wrapper)).toBe(textBeforeClick)
    expect(statusText(wrapper)).not.toContain('Возврат выполнен')

    // Родитель получил ответ команды (busy: активная загрузка идёт,
    // Ф-7) и прислал новый снимок — ровно то, что делает `rollback()`
    // composable.
    await wrapper.setProps({
      snapshot: {
        busy: true,
        status: 'rollbackWaiting',
        version: '2026.07.11',
        rollbackTarget: ACTIVE_VERSION,
      },
      activeVersion: ACTIVE_VERSION,
    })

    expect(statusText(wrapper)).toBe(
      'Возврат к 2026.07.11 принят — применится, когда закончится текущая загрузка.',
    )
    expect(statusText(wrapper)).not.toContain('Возврат выполнен')
    expect(checkButton(wrapper)?.attributes('disabled')).toBeDefined()
    expect(rollbackButton(wrapper)?.attributes('disabled')).toBeDefined()
  })

  it('resets the confirmation if the rollback target disappears from a newly arrived snapshot', async () => {
    const wrapper = mountBlock({ snapshot: snapshotWithTarget, activeVersion: ACTIVE_VERSION })

    await rollbackButton(wrapper)?.trigger('click')
    expect(rollbackConfirmPanel(wrapper).exists()).toBe(true)

    await wrapper.setProps({
      snapshot: { busy: false, status: 'upToDate', at: '2026-08-25T12:05:00Z' },
      activeVersion: ACTIVE_VERSION,
    })

    expect(rollbackConfirmPanel(wrapper).exists()).toBe(false)
    expect(rollbackButton(wrapper)).toBeUndefined()
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

  it('reflects aria-busy="false" on the block while nothing is busy or pending', () => {
    const wrapper = mountBlock({
      snapshot: { busy: false, status: 'neverChecked' },
      activeVersion: ACTIVE_VERSION,
    })

    expect(wrapper.get('.ytdlp-update-block').attributes('aria-busy')).toBe('false')
  })

  it('reflects aria-busy="true" on the block while pending is true, same as the button-disabling busy flag', () => {
    const wrapper = mountBlock({
      snapshot: { busy: false, status: 'upToDate', at: '2026-08-25T12:00:00Z' },
      activeVersion: ACTIVE_VERSION,
      pending: true,
    })

    expect(wrapper.get('.ytdlp-update-block').attributes('aria-busy')).toBe('true')
  })
})

/*
 * TL-120 (issue #127): регрессия TL-66 — откат без активной загрузки
 * отвечает только после переключения (до 24 с на холодном дереве), а
 * `snapshot.busy` до этого момента ещё несёт старое значение. `pending`
 * — сигнал «ответ команды ещё не пришёл», отдельный от `snapshot`,
 * который держит обе кнопки неактивными вне зависимости от того, что
 * говорит текущий снимок.
 */
describe('YtDlpUpdateBlock — «команда в пути» (TL-120, issue #127)', () => {
  it('disables both buttons while pending is true, even though snapshot.busy is still false (stale value)', () => {
    const wrapper = mountBlock({
      snapshot: {
        busy: false,
        status: 'upToDate',
        at: '2026-08-25T12:00:00Z',
        rollbackTarget: '2026.07.11',
      },
      activeVersion: ACTIVE_VERSION,
      pending: true,
    })

    expect(checkButton(wrapper)?.attributes('disabled')).toBeDefined()
    expect(rollbackButton(wrapper)?.attributes('disabled')).toBeDefined()
  })

  it('does not disable the buttons when pending is false (default) and snapshot.busy is false', () => {
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
    expect(rollbackButton(wrapper)?.attributes('disabled')).toBeUndefined()
  })

  it('re-enables both buttons once pending flips back to false, snapshot unchanged (command refused/rejected)', async () => {
    const wrapper = mountBlock({
      snapshot: {
        busy: false,
        status: 'upToDate',
        at: '2026-08-25T12:00:00Z',
        rollbackTarget: '2026.07.11',
      },
      activeVersion: ACTIVE_VERSION,
      pending: true,
    })
    expect(checkButton(wrapper)?.attributes('disabled')).toBeDefined()

    await wrapper.setProps({ pending: false })

    expect(checkButton(wrapper)?.attributes('disabled')).toBeUndefined()
    expect(rollbackButton(wrapper)?.attributes('disabled')).toBeUndefined()
  })

  it('does not locally jump to "applied" while pending — status text is unchanged until a new snapshot arrives', async () => {
    const wrapper = mountBlock({
      snapshot: {
        busy: false,
        status: 'upToDate',
        at: '2026-08-25T12:00:00Z',
        rollbackTarget: '2026.07.11',
      },
      activeVersion: ACTIVE_VERSION,
    })
    const textBefore = statusText(wrapper)

    await wrapper.setProps({ pending: true })
    expect(statusText(wrapper)).toBe(textBefore)
    expect(statusText(wrapper)).not.toContain('Возврат выполнен')

    await wrapper.setProps({
      pending: false,
      snapshot: {
        busy: false,
        status: 'rolledBack',
        at: '2026-08-25T12:00:05Z',
        active: '2026.07.11',
        abandoned: '2026.08.20',
        rollbackTarget: '2026.08.20',
      },
    })

    expect(statusText(wrapper)).toContain('Возврат выполнен')
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
