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
    sizeBytes: 224_395_264, // ≈ 214 МБ
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

    expect(invokeMock).toHaveBeenLastCalledWith('history_page', { cursor: { finishedAtUnixSecs: 1, id: '1' }, limit: 30 })
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

  it('does not show a notice again after remounting with a fresh mock response that carries none', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [], notices: [{ kind: 'baseRecreated' }] } satisfies HistoryPage)
    const wrapper = await mountScreen()
    expect(wrapper.text()).toContain('Файл истории был повреждён')
    wrapper.unmount()

    invokeMock.mockResolvedValueOnce({ entries: [], notices: [] } satisfies HistoryPage)
    const remounted = await mountScreen()
    expect(remounted.text()).not.toContain('Файл истории был повреждён')
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

  it('a delete failure keeps the row and shows a dismissible command-error banner', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    const wrapper = await mountScreen()

    invokeMock.mockRejectedValueOnce({ kind: 'unknownRecord', message: 'diag' })
    await wrapper.findAll('button').find((b) => b.text() === 'Удалить')?.trigger('click')
    await flushPromises()

    expect(wrapper.findAll('.history-screen__entry')).toHaveLength(1)
    const banner = wrapper.get('[role="alert"].history-screen__command-error')
    expect(banner.text()).toContain('Запись не найдена')

    await banner.get('button').trigger('click')
    expect(wrapper.find('.history-screen__command-error').exists()).toBe(false)
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
    const cases: { kind: string; payload: Record<string, unknown>; expectedFragment: string }[] = [
      { kind: 'fileMissing', payload: { kind: 'fileMissing' }, expectedFragment: 'Файл не найден' },
      { kind: 'folderMissing', payload: { kind: 'folderMissing' }, expectedFragment: 'Папка не найдена' },
      {
        kind: 'launcherFailed',
        payload: { kind: 'launcherFailed', details: { exitCode: 1 } },
        expectedFragment: 'Не удалось открыть проводник',
      },
      { kind: 'unknownRecord', payload: { kind: 'unknownRecord' }, expectedFragment: 'Запись не найдена' },
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
      await wrapper.findAll('button').find((b) => b.text() === 'Показать в папке')?.trigger('click')
      await flushPromises()

      expect(wrapper.text()).toContain(testCase.expectedFragment)
      wrapper.unmount()
    }
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

describe('HistoryScreen — живая зона структурных изменений (дизайн E5, «Доступность»)', () => {
  it('the announcer exists in the DOM from mount, silent before anything structural happens', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    const wrapper = await mountScreen()

    const announcer = wrapper.get('.history-screen__announcer')
    expect(announcer.attributes('aria-live')).toBe('polite')
    expect(announcer.text()).toBe('')
  })

  it('announces a short text, not the row content, when a new entry arrives via queue://changed', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    const wrapper = await mountScreen()

    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '2' }), entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    handlers.get('queue://changed')?.({ payload: undefined })
    await flushPromises()

    expect(wrapper.get('.history-screen__announcer').text()).toBe('Добавлена новая запись')
  })
})
