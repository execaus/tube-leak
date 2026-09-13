import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { HistoryEntry, HistoryPage } from '@/types/generated/history'

const invokeMock = vi.fn()
const unlistenMock = vi.fn()
type Handler = (event: { payload: unknown }) => void
const handlers = new Map<string, Handler>()
const listenMock = vi.fn((eventName: string, handler: Handler) => {
  handlers.set(eventName, handler)
  return Promise.resolve(unlistenMock)
})

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: (...args: [string, Handler]) => listenMock(...args),
}))

const { default: HistoryScreen } = await import('./HistoryScreen.vue')

function entry(overrides: Partial<HistoryEntry> = {}): HistoryEntry {
  return {
    id: '1',
    videoId: 'dQw4w9WgXcQ',
    url: 'https://youtu.be/dQw4w9WgXcQ',
    title: 'Как приручить дракона',
    quality: { kind: 'standard', heightPx: 1080 },
    fileName: 'Как приручить дракона.mp4',
    folderDisplay: { kind: 'custom', path: '/Users/execaus/Movies/YouTube' },
    sizeBytes: 224_395_264, // 214 МБ
    finishedAtUnixSecs: Math.floor(Date.now() / 1000) - 3 * 3600,
    fileStatus: { kind: 'present' },
    ...overrides,
  }
}

let host: HTMLElement

beforeEach(() => {
  invokeMock.mockReset()
  listenMock.mockClear()
  unlistenMock.mockClear()
  handlers.clear()
  setActivePinia(createPinia())
  host = document.createElement('div')
  document.body.appendChild(host)
})

afterEach(() => {
  host.remove()
})

async function mountScreen() {
  const wrapper = mount(HistoryScreen, { attachTo: host })
  await flushPromises()
  return wrapper
}

describe('HistoryScreen — пустая история', () => {
  it('shows the exact empty-state text and no «Очистить» button', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [], notices: [] } satisfies HistoryPage)
    const wrapper = await mountScreen()

    expect(wrapper.text()).toContain('История пуста. Здесь появятся ролики после первой завершённой загрузки.')
    expect(wrapper.findAll('button').find((b) => b.text() === 'Очистить')).toBeUndefined()
  })
})

describe('HistoryScreen — список и статус файла (Ф-5, таблица трёх случаев)', () => {
  it('renders title, meta line and folder path for a present entry, with both action buttons', async () => {
    invokeMock.mockResolvedValueOnce({
      entries: [entry()],
      notices: [],
    } satisfies HistoryPage)
    const wrapper = await mountScreen()

    expect(wrapper.text()).toContain('«Как приручить дракона» — 1080p')
    expect(wrapper.text()).toContain('mp4')
    expect(wrapper.text()).toContain('214 МБ')
    expect(wrapper.text()).toContain('/Users/execaus/Movies/YouTube')
    expect(wrapper.findAll('button').find((b) => b.text() === 'Показать в папке')).toBeDefined()
    const deleteButton = wrapper.findAll('button').find((b) => b.text() === 'Удалить')
    expect(deleteButton?.attributes('aria-label')).toBe(
      'Удалить запись «Как приручить дракона» — 1080p из истории',
    )
  })

  it('С-5 (правки ревью TL-93, второй раунд): sizeBytes is exact, shown without the "≈" approximation marker', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry()], notices: [] } satisfies HistoryPage)
    const wrapper = await mountScreen()

    expect(wrapper.get('.history-screen__entry-meta').text()).not.toContain('≈')
    expect(wrapper.get('.history-screen__entry-meta').text()).toContain('214 МБ')
  })

  it('missing file with an existing folder: exact sentence, «Показать в папке» stays', async () => {
    invokeMock.mockResolvedValueOnce({
      entries: [entry({ fileStatus: { kind: 'missing', folderExists: true } })],
      notices: [],
    } satisfies HistoryPage)
    const wrapper = await mountScreen()

    expect(wrapper.text()).toContain('Файл сейчас не на месте — папка существует.')
    expect(wrapper.findAll('button').find((b) => b.text() === 'Показать в папке')).toBeDefined()
  })

  it('missing file with no folder either: names the folder, no «Показать в папке» button', async () => {
    invokeMock.mockResolvedValueOnce({
      entries: [
        entry({
          fileStatus: { kind: 'missing', folderExists: false },
          folderDisplay: { kind: 'systemDownloads' },
        }),
      ],
      notices: [],
    } satisfies HistoryPage)
    const wrapper = await mountScreen()

    expect(wrapper.text()).toContain('Файл сейчас не на месте, папка «Загрузки» тоже не существует.')
    expect(wrapper.findAll('button').find((b) => b.text() === 'Показать в папке')).toBeUndefined()
    expect(wrapper.findAll('button').find((b) => b.text() === 'Удалить')).toBeDefined()
  })
})

describe('HistoryScreen — «Показать ещё» (курсорная пагинация)', () => {
  it('requests the next page with the exact cursor received, appends entries, hides the button once exhausted', async () => {
    invokeMock.mockResolvedValueOnce({
      entries: [entry({ id: '1' })],
      nextCursor: { finishedAtUnixSecs: 1, id: '1' },
      notices: [],
    } satisfies HistoryPage)
    const wrapper = await mountScreen()

    const loadMoreButton = () => wrapper.findAll('button').find((b) => b.text() === 'Показать ещё')
    expect(loadMoreButton()).toBeDefined()

    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '2' })], notices: [] } satisfies HistoryPage)
    await loadMoreButton()!.trigger('click')
    await flushPromises()

    expect(invokeMock).toHaveBeenLastCalledWith('history_page', { cursor: { finishedAtUnixSecs: 1, id: '1' } })
    expect(wrapper.findAll('.history-screen__entry')).toHaveLength(2)
    expect(loadMoreButton()).toBeUndefined()
  })
})

describe('HistoryScreen — история недоступна (Ф-1 б/в/д)', () => {
  it('replaces the whole screen with the exact newerVersion paragraph, no list/actions', async () => {
    invokeMock.mockRejectedValueOnce({ reason: 'newerVersion', message: 'diag' })
    const wrapper = await mountScreen()

    expect(wrapper.text()).toContain('файл базы данных создан более новой версией tube-leak')
    expect(wrapper.find('.history-screen__list').exists()).toBe(false)
    expect(wrapper.findAll('button').find((b) => b.text() === 'Очистить')).toBeUndefined()
  })

  it('renders the noAccess paragraph', async () => {
    invokeMock.mockRejectedValueOnce({ reason: 'noAccess', message: 'diag' })
    const wrapper = await mountScreen()
    expect(wrapper.text()).toContain('нет доступа на запись в папку данных приложения')
  })

  it('renders the migrationFailed paragraph', async () => {
    invokeMock.mockRejectedValueOnce({ reason: 'migrationFailed', message: 'diag' })
    const wrapper = await mountScreen()
    expect(wrapper.text()).toContain('не удалось обновить формат базы данных')
  })
})

describe('HistoryScreen — С-6 (правки ревью TL-93, второй раунд): отказ IPC на первой странице', () => {
  it('renders the neutral unavailable paragraph, not "История пуста" — an uncaught IPC exception is not the same fact as an empty history', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {})
    invokeMock.mockRejectedValueOnce(new Error('ipc broken'))
    const wrapper = await mountScreen()

    expect(wrapper.text()).not.toContain('История пуста')
    expect(wrapper.text()).toContain('не удалось получить данные')
    expect(wrapper.find('.history-screen__list').exists()).toBe(false)
  })
})

describe('HistoryScreen — пометки (С-10 порча базы, Ф-3 последняя запись не сохранена)', () => {
  it('shows both notices at once with a working «Скрыть» each (mutation: only-the-first would fail this)', async () => {
    invokeMock.mockResolvedValueOnce({
      entries: [],
      notices: [{ kind: 'baseRecreated' }, { kind: 'lastWriteFailed', cause: 'diskFull' }],
    } satisfies HistoryPage)
    const wrapper = await mountScreen()

    expect(wrapper.text()).toContain('Файл истории был повреждён')
    expect(wrapper.text()).toContain('Последняя запись не сохранена: недостаточно места на диске.')

    const hideButtons = wrapper.findAll('.history-screen__banner button')
    expect(hideButtons).toHaveLength(2)
    await hideButtons[0]!.trigger('click')

    expect(wrapper.text()).not.toContain('Файл истории был повреждён')
    expect(wrapper.text()).toContain('Последняя запись не сохранена')
  })

  it('does not show a notice in a genuinely fresh session (new store) whose very first response carries none', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [], notices: [{ kind: 'baseRecreated' }] } satisfies HistoryPage)
    const wrapper = await mountScreen()
    expect(wrapper.text()).toContain('Файл истории был повреждён')
    wrapper.unmount()

    // Новая сессия — новый стор (Б-1, правки ревью TL-93, второй раунд):
    // «выдано один раз» — свойство ядра внутри **одного** запущенного
    // приложения (doc `HistoryNotice` в `src/types/generated/history.ts`),
    // а не что-то, что клиент обязан помнить дольше своего собственного
    // стора. Переиспользование того же стора для второго `mount()` больше
    // не годится как симуляция «свежего запуска»: пометка теперь копится
    // на клиенте до явного «Скрыть» (см. следующий тест) и пережила бы
    // такое «переиспользование» настоящим, ожидаемым образом.
    setActivePinia(createPinia())
    invokeMock.mockResolvedValueOnce({ entries: [], notices: [] } satisfies HistoryPage)
    const remounted = await mountScreen()
    expect(remounted.text()).not.toContain('Файл истории был повреждён')
  })

  it('Б-1 (правки ревью TL-93, второй раунд): a notice is not wiped by the next queue://changed before it is dismissed', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [], notices: [{ kind: 'baseRecreated' }] } satisfies HistoryPage)
    const wrapper = await mountScreen()
    expect(wrapper.text()).toContain('Файл истории был повреждён')

    invokeMock.mockResolvedValueOnce({ entries: [], notices: [] } satisfies HistoryPage)
    handlers.get('queue://changed')?.({ payload: undefined })
    await flushPromises()

    expect(wrapper.text()).toContain('Файл истории был повреждён')

    await wrapper.get('.history-screen__banner button').trigger('click')
    expect(wrapper.text()).not.toContain('Файл истории был повреждён')
  })

  it('мелочи (правки ревью TL-93, второй раунд): role="status" sits on the banner text, not on the <li> — list semantics stay intact', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [], notices: [{ kind: 'baseRecreated' }] } satisfies HistoryPage)
    const wrapper = await mountScreen()

    const li = wrapper.get('ul.history-screen__notices > li')
    expect(li.attributes('role')).toBeUndefined()
    expect(li.get('[role="status"]').text()).toContain('Файл истории был повреждён')
  })
})

describe('HistoryScreen — удаление и очистка (С-4)', () => {
  it('deletes a record immediately without any confirmation dialog', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    const wrapper = await mountScreen()
    expect(wrapper.findAll('.history-screen__entry')).toHaveLength(1)

    invokeMock.mockResolvedValueOnce(undefined)
    await wrapper.findAll('button').find((b) => b.text() === 'Удалить')?.trigger('click')
    await flushPromises()

    expect(wrapper.find('[role="alertdialog"]').exists()).toBe(false)
    expect(wrapper.findAll('.history-screen__entry')).toHaveLength(0)
    expect(invokeMock).toHaveBeenLastCalledWith('delete_history_record', { id: '1' })
  })

  it('«Очистить» opens the confirm dialog; Esc/«Отмена» do not call clear_history', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry()], notices: [] } satisfies HistoryPage)
    const wrapper = await mountScreen()

    await wrapper.findAll('button').find((b) => b.text() === 'Очистить')?.trigger('click')
    expect(wrapper.find('[role="alertdialog"]').exists()).toBe(true)

    invokeMock.mockClear()
    await wrapper.get('[role="alertdialog"]').trigger('keydown', { key: 'Escape' })
    await wrapper.vm.$nextTick()

    expect(wrapper.find('[role="alertdialog"]').exists()).toBe(false)
    expect(invokeMock).not.toHaveBeenCalledWith('clear_history')
    expect(wrapper.findAll('.history-screen__entry')).toHaveLength(1)
  })

  it('confirming the dialog calls clear_history and empties the list', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry()], notices: [] } satisfies HistoryPage)
    const wrapper = await mountScreen()

    await wrapper.findAll('button').find((b) => b.text() === 'Очистить')?.trigger('click')
    invokeMock.mockResolvedValueOnce(undefined)
    await wrapper.findAll('button').find((b) => b.text() === 'Очистить всё')?.trigger('click')
    await flushPromises()

    expect(invokeMock).toHaveBeenCalledWith('clear_history')
    expect(wrapper.text()).toContain('История пуста')
  })

  it('confirming the dialog emits requestHeadingFocus (С-7) — «заголовок экрана» is owned by App.vue, this component only asks for it', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry()], notices: [] } satisfies HistoryPage)
    const wrapper = await mountScreen()

    await wrapper.findAll('button').find((b) => b.text() === 'Очистить')?.trigger('click')
    invokeMock.mockResolvedValueOnce(undefined)
    await wrapper.findAll('button').find((b) => b.text() === 'Очистить всё')?.trigger('click')
    await flushPromises()

    expect(wrapper.emitted('requestHeadingFocus')).toHaveLength(1)
  })

  it('a delete failure (writeFailed) keeps the row and shows a dismissible command-error banner', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    const wrapper = await mountScreen()

    invokeMock.mockRejectedValueOnce({ kind: 'writeFailed', message: 'diag' })
    await wrapper.findAll('button').find((b) => b.text() === 'Удалить')?.trigger('click')
    await flushPromises()

    expect(wrapper.findAll('.history-screen__entry')).toHaveLength(1)
    const banner = wrapper.get('[role="alert"].history-screen__command-error')
    expect(banner.text()).toContain('Не удалось сохранить изменение')

    await banner.get('button').trigger('click')
    expect(wrapper.find('.history-screen__command-error').exists()).toBe(false)
  })

  it('С-3 (правки ревью TL-93, второй раунд): a delete failure with unknownRecord removes the row silently, no banner — the goal was already achieved', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    const wrapper = await mountScreen()

    invokeMock.mockRejectedValueOnce({ kind: 'unknownRecord', message: 'diag' })
    await wrapper.findAll('button').find((b) => b.text() === 'Удалить')?.trigger('click')
    await flushPromises()

    expect(wrapper.findAll('.history-screen__entry')).toHaveLength(0)
    expect(wrapper.find('.history-screen__command-error').exists()).toBe(false)
  })

  it('С-4: the command-error banner does not repeat the title inside the explanation', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    const wrapper = await mountScreen()

    invokeMock.mockRejectedValueOnce({ kind: 'writeFailed', message: 'diag' })
    await wrapper.findAll('button').find((b) => b.text() === 'Удалить')?.trigger('click')
    await flushPromises()

    const text = wrapper.get('[role="alert"].history-screen__command-error').text()
    expect(text.match(/Не удалось сохранить изменение/g)).toHaveLength(1)
  })
})

describe('HistoryScreen — С-7 (правки ревью TL-93, второй раунд): фокус после «Удалить»', () => {
  it('moves focus to the next row\'s «Удалить» button when one remains after it', async () => {
    invokeMock.mockResolvedValueOnce({
      entries: [entry({ id: '2', title: 'Второй' }), entry({ id: '1', title: 'Первый' })],
      notices: [],
    } satisfies HistoryPage)
    const wrapper = await mountScreen()

    const deleteButtons = () => wrapper.findAll('.history-screen__delete-button')
    invokeMock.mockResolvedValueOnce(undefined)
    await deleteButtons()[0]!.trigger('click')
    await flushPromises()

    expect(wrapper.findAll('.history-screen__entry')).toHaveLength(1)
    expect(document.activeElement).toBe(deleteButtons()[0]!.element)
  })

  it('falls back to the previous row\'s «Удалить» button when the removed row was last', async () => {
    invokeMock.mockResolvedValueOnce({
      entries: [entry({ id: '2', title: 'Второй' }), entry({ id: '1', title: 'Первый' })],
      notices: [],
    } satisfies HistoryPage)
    const wrapper = await mountScreen()

    const deleteButtons = () => wrapper.findAll('.history-screen__delete-button')
    invokeMock.mockResolvedValueOnce(undefined)
    await deleteButtons()[1]!.trigger('click') // удаляем последнюю строку
    await flushPromises()

    expect(wrapper.findAll('.history-screen__entry')).toHaveLength(1)
    expect(document.activeElement).toBe(deleteButtons()[0]!.element)
  })

  it('emits requestHeadingFocus when the deleted row was the only one left', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    const wrapper = await mountScreen()

    invokeMock.mockResolvedValueOnce(undefined)
    await wrapper.findAll('button').find((b) => b.text() === 'Удалить')?.trigger('click')
    await flushPromises()

    expect(wrapper.emitted('requestHeadingFocus')).toHaveLength(1)
  })

  it('does not move focus at all when the delete failed and the row is still there', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    const wrapper = await mountScreen()

    const deleteButton = wrapper.findAll('.history-screen__delete-button')[0]!
    ;(deleteButton.element as HTMLButtonElement).focus()
    invokeMock.mockRejectedValueOnce({ kind: 'writeFailed', message: 'diag' })
    await deleteButton.trigger('click')
    await flushPromises()

    expect(wrapper.findAll('.history-screen__entry')).toHaveLength(1)
    expect(wrapper.emitted('requestHeadingFocus')).toBeUndefined()
  })
})

describe('HistoryScreen — «Показать в папке» и его исходы (Ф-8)', () => {
  it('success shows no additional feedback beyond the system file manager', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry()], notices: [] } satisfies HistoryPage)
    const wrapper = await mountScreen()

    invokeMock.mockResolvedValueOnce(undefined)
    await wrapper.findAll('button').find((b) => b.text() === 'Показать в папке')?.trigger('click')
    await flushPromises()

    expect(wrapper.find('[role="alert"]').exists()).toBe(false)
  })

  it('the expected fileMissing+folderExists combination shows no extra error text (design table row 2)', async () => {
    invokeMock.mockResolvedValueOnce({
      entries: [entry({ fileStatus: { kind: 'missing', folderExists: true } })],
      notices: [],
    } satisfies HistoryPage)
    const wrapper = await mountScreen()

    invokeMock.mockRejectedValueOnce({ kind: 'fileMissing', message: 'diag' })
    await wrapper.findAll('button').find((b) => b.text() === 'Показать в папке')?.trigger('click')
    await flushPromises()

    expect(wrapper.find('.history-screen__entry-error').exists()).toBe(false)
  })

  it('every other kind gets its own distinct text, exhaustively over the five contract classes', async () => {
    const cases: { kind: string; payload: Record<string, unknown>; expectedFragment: string; triggersRefresh?: boolean }[] = [
      { kind: 'fileMissing', payload: { kind: 'fileMissing' }, expectedFragment: 'Файл не найден' },
      { kind: 'folderMissing', payload: { kind: 'folderMissing' }, expectedFragment: 'Папка не найдена' },
      {
        kind: 'launcherFailed',
        payload: { kind: 'launcherFailed', details: { exitCode: 1 } },
        expectedFragment: 'Не удалось открыть проводник',
      },
      {
        kind: 'unknownRecord',
        payload: { kind: 'unknownRecord' },
        // С-3 (правки ревью TL-93, второй раунд): нейтральный факт, не совет.
        expectedFragment: 'Этой записи больше нет в истории',
        triggersRefresh: true,
      },
      { kind: 'unavailable', payload: { kind: 'unavailable', reason: 'noAccess' }, expectedFragment: 'История недоступна' },
    ]

    for (const testCase of cases) {
      invokeMock.mockReset()
      listenMock.mockClear()
      handlers.clear()
      setActivePinia(createPinia())
      invokeMock.mockResolvedValueOnce({ entries: [entry()], notices: [] } satisfies HistoryPage)
      const wrapper = await mountScreen()

      invokeMock.mockRejectedValueOnce({ ...testCase.payload, message: 'diag' })
      if (testCase.triggersRefresh) {
        // Тот же `id` — строка (и построчный баннер «Этой записи больше
        // нет в истории») переживает автоматический перезапрос первой
        // страницы, который `unknownRecord` запускает сам (С-3); пустой
        // ответ здесь убрал бы саму строку и вместе с ней текст, который
        // эта итерация проверяет — не то, что демонстрирует эта проверка.
        invokeMock.mockResolvedValueOnce({ entries: [entry()], notices: [] } satisfies HistoryPage)
      }
      await wrapper.findAll('button').find((b) => b.text() === 'Показать в папке')?.trigger('click')
      await flushPromises()

      expect(wrapper.text()).toContain(testCase.expectedFragment)
      wrapper.unmount()
    }
  })

  it('С-3: unknownRecord re-requests the first page (does not just advise the user to do it)', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    const wrapper = await mountScreen()

    invokeMock.mockRejectedValueOnce({ kind: 'unknownRecord', message: 'diag' })
    invokeMock.mockResolvedValueOnce({ entries: [], notices: [] } satisfies HistoryPage)
    await wrapper.findAll('button').find((b) => b.text() === 'Показать в папке')?.trigger('click')
    await flushPromises()

    expect(invokeMock).toHaveBeenLastCalledWith('history_page', { cursor: undefined })
  })

  it('С-4: unavailable blocks the whole screen (same paragraph as history_page), not a per-row banner', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry()], notices: [] } satisfies HistoryPage)
    const wrapper = await mountScreen()

    invokeMock.mockRejectedValueOnce({ kind: 'unavailable', reason: 'noAccess', message: 'diag' })
    await wrapper.findAll('button').find((b) => b.text() === 'Показать в папке')?.trigger('click')
    await flushPromises()

    expect(wrapper.find('.history-screen__entry-error').exists()).toBe(false)
    expect(wrapper.find('.history-screen__list').exists()).toBe(false)
    expect(wrapper.text()).toContain('нет доступа на запись в папку данных приложения')
  })

  it('launcherFailed shows a collapsible details block with the exit code', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry()], notices: [] } satisfies HistoryPage)
    const wrapper = await mountScreen()

    invokeMock.mockRejectedValueOnce({ kind: 'launcherFailed', details: { exitCode: 7 }, message: 'diag' })
    await wrapper.findAll('button').find((b) => b.text() === 'Показать в папке')?.trigger('click')
    await flushPromises()

    const details = wrapper.get('details')
    expect(details.text()).toContain('7')
  })

  it('launcherFailed does not repeat the title inside the explanation (С-4)', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry()], notices: [] } satisfies HistoryPage)
    const wrapper = await mountScreen()

    invokeMock.mockRejectedValueOnce({ kind: 'launcherFailed', details: { exitCode: 7 }, message: 'diag' })
    await wrapper.findAll('button').find((b) => b.text() === 'Показать в папке')?.trigger('click')
    await flushPromises()

    const text = wrapper.get('.history-screen__entry-error').text()
    expect(text).toBe('Не удалось открыть проводник: Не удалось запустить файловый менеджер операционной системы.')
  })
})

describe('HistoryScreen — обновление по queue://changed', () => {
  it('re-fetches the first page and shows a newly finished download without dropping the tail loaded via «Показать ещё»', async () => {
    invokeMock.mockResolvedValueOnce({
      entries: [entry({ id: '2' })],
      nextCursor: { finishedAtUnixSecs: 2, id: '2' },
      notices: [],
    } satisfies HistoryPage)
    const wrapper = await mountScreen()

    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    await wrapper.findAll('button').find((b) => b.text() === 'Показать ещё')?.trigger('click')
    await flushPromises()
    expect(wrapper.findAll('.history-screen__entry')).toHaveLength(2)

    invokeMock.mockResolvedValueOnce({
      entries: [entry({ id: '3' }), entry({ id: '2' })],
      nextCursor: { finishedAtUnixSecs: 2, id: '2' },
      notices: [],
    } satisfies HistoryPage)
    handlers.get('queue://changed')?.({ payload: undefined })
    await flushPromises()

    expect(wrapper.findAll('.history-screen__entry')).toHaveLength(3)
  })
})

describe('HistoryScreen — обновление по активации вкладки (Б-2/С-1, правки ревью TL-93, второй раунд)', () => {
  it('refreshes the first page when the `active` prop flips from false to true', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    const wrapper = mount(HistoryScreen, { attachTo: host, props: { active: false } })
    await flushPromises()
    expect(invokeMock).toHaveBeenCalledTimes(1) // монтирование уже запросило первую страницу

    invokeMock.mockResolvedValueOnce({
      entries: [entry({ id: '1', fileStatus: { kind: 'missing', folderExists: false } })],
      notices: [],
    } satisfies HistoryPage)
    await wrapper.setProps({ active: true })
    await flushPromises()

    expect(invokeMock).toHaveBeenCalledTimes(2)
    expect(invokeMock).toHaveBeenLastCalledWith('history_page', { cursor: undefined })
  })

  it('does not request anything when `active` flips from true to false', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    const wrapper = mount(HistoryScreen, { attachTo: host, props: { active: true } })
    await flushPromises()
    invokeMock.mockClear()

    await wrapper.setProps({ active: false })
    await flushPromises()

    expect(invokeMock).not.toHaveBeenCalled()
  })

  function historyPageCallCount(): number {
    return invokeMock.mock.calls.filter(([cmd]) => cmd === 'history_page').length
  }

  it('C1: монтирование на «Главном», затем открыть/уйти/открыть вкладку «История» — по одному запросу на монтирование и на каждое открытие, ни одного на уход', async () => {
    invokeMock.mockResolvedValue({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    const wrapper = mount(HistoryScreen, { attachTo: host, props: { active: false } })
    await flushPromises()
    const atStart = historyPageCallCount()

    await wrapper.setProps({ active: true })
    await flushPromises()
    const afterOpen = historyPageCallCount()

    await wrapper.setProps({ active: false })
    await flushPromises()
    const afterLeave = historyPageCallCount()

    await wrapper.setProps({ active: true })
    await flushPromises()

    expect([atStart, afterOpen, afterLeave, historyPageCallCount()]).toStrictEqual([1, 2, 2, 3])
  })

  it('C2: приложение стартует сразу на вкладке «История» (`active: true` с монтирования) — запрос ровно один, второй от `watch` не задваивается', async () => {
    invokeMock.mockResolvedValue({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    mount(HistoryScreen, { attachTo: host, props: { active: true } })
    await flushPromises()

    expect(historyPageCallCount()).toBe(1)
  })
})

describe('HistoryScreen — выход из недоступности по активации вкладки (U3, правки ревью TL-93, третий раунд)', () => {
  it('unavailable от «Удалить» держит блокировку, пока пользователь остаётся на вкладке; уход и возврат на «Историю» её снимают', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {})
    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    const wrapper = mount(HistoryScreen, { attachTo: host, props: { active: true } })
    await flushPromises()

    invokeMock.mockRejectedValueOnce({ kind: 'unavailable', reason: 'noAccess', message: 'diag' })
    await wrapper.findAll('button').find((b) => b.text() === 'Удалить')?.trigger('click')
    await flushPromises()
    expect(wrapper.find('.history-screen__unavailable').exists()).toBe(true)

    invokeMock.mockResolvedValue({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    await wrapper.setProps({ active: false })
    await wrapper.setProps({ active: true })
    await flushPromises()

    expect(wrapper.find('.history-screen__unavailable').exists()).toBe(false)
    expect(wrapper.findAll('.history-screen__entry')).toHaveLength(1)
  })
})

describe('HistoryScreen — живая зона структурных изменений (дизайн E5, «Доступность»)', () => {
  it('the announcer exists in the DOM from mount, silent before anything structural happens', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    const wrapper = await mountScreen()

    const announcer = wrapper.get('.history-screen__announcer')
    expect(announcer.attributes('aria-live')).toBe('polite')
    expect(announcer.text()).toBe('')
  })

  it('announces a short text naming the entry, not the row content, when a new entry arrives via queue://changed', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    const wrapper = await mountScreen()

    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '2' }), entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    handlers.get('queue://changed')?.({ payload: undefined })
    await flushPromises()

    expect(wrapper.get('.history-screen__announcer').text()).toBe('Добавлена новая запись: «Как приручить дракона» — 1080p')
  })
})
