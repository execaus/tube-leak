import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'

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

const { useHistoryStore } = await import('./history')

function entry(overrides: Partial<HistoryEntry> = {}): HistoryEntry {
  return {
    id: '1',
    videoId: 'dQw4w9WgXcQ',
    url: 'https://youtu.be/dQw4w9WgXcQ',
    title: 'Ролик',
    quality: { kind: 'standard', heightPx: 1080 },
    fileName: 'Ролик.mp4',
    folderDisplay: { kind: 'systemDownloads' },
    sizeBytes: 224_395_264,
    finishedAtUnixSecs: 1_756_130_400,
    fileStatus: { kind: 'present' },
    ...overrides,
  }
}

/** Строит `n` записей с id/`finishedAtUnixSecs` от `from` вниз до `to` включительно (для сценариев с большим числом записей, R3/R4). */
function entriesRange(from: number, to: number): HistoryEntry[] {
  const out: HistoryEntry[] = []
  for (let i = from; i >= to; i--) out.push(entry({ id: String(i), finishedAtUnixSecs: i }))
  return out
}

function emitQueueChanged(): void {
  handlers.get('queue://changed')?.({ payload: undefined })
}

beforeEach(() => {
  invokeMock.mockReset()
  listenMock.mockClear()
  unlistenMock.mockClear()
  handlers.clear()
  setActivePinia(createPinia())
})

describe('useHistoryStore.initialize', () => {
  it('requests the first page without a cursor and stores entries/cursor/notices', async () => {
    const page: HistoryPage = {
      entries: [entry({ id: '2' }), entry({ id: '1' })],
      nextCursor: { finishedAtUnixSecs: 1_000, id: '1' },
      notices: [{ kind: 'baseRecreated' }],
    }
    invokeMock.mockResolvedValueOnce(page)
    const store = useHistoryStore()

    await store.initialize()

    // С-2 (правки ревью TL-93, второй раунд): у `history_page` только
    // `cursor` — размер страницы решает константа ядра, аргумента `limit`
    // на стороне клиента нет.
    expect(invokeMock).toHaveBeenCalledWith('history_page', { cursor: undefined })
    expect(store.entries.map((e) => e.id)).toStrictEqual(['2', '1'])
    expect(store.nextCursor).toStrictEqual({ finishedAtUnixSecs: 1_000, id: '1' })
    expect(store.notices).toStrictEqual([{ kind: 'baseRecreated' }])
    expect(store.loaded).toBe(true)
    expect(store.availability).toBeUndefined()
  })

  it('sets availability on a typed HistoryUnavailableError and marks the store loaded', async () => {
    invokeMock.mockRejectedValueOnce({ reason: 'noAccess', message: 'diag' })
    const store = useHistoryStore()

    await store.initialize()

    expect(store.availability).toStrictEqual({ reason: 'noAccess', message: 'diag' })
    expect(store.entries).toStrictEqual([])
    expect(store.loaded).toBe(true)
  })

  it('subscribes to queue://changed exactly once even across repeated initialize calls', async () => {
    invokeMock.mockResolvedValue({ entries: [], notices: [] } satisfies HistoryPage)
    const store = useHistoryStore()

    await store.initialize()
    await store.initialize()

    expect(listenMock.mock.calls.filter(([name]) => name === 'queue://changed')).toHaveLength(1)
  })
})

describe('useHistoryStore — С-6: отказ history_page, который не типизированная HistoryUnavailableError', () => {
  it('a non-typed rejection of the very first page is flagged distinctly from a real empty history (mutation guard for R10 — the ipcFailure branch, not "loaded+empty")', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {})
    invokeMock.mockRejectedValueOnce(new Error('ipc broken'))
    const store = useHistoryStore()

    await store.initialize()

    expect(store.loaded).toBe(true)
    expect(store.availability).toBeUndefined()
    expect(store.ipcFailure).toBe(true)
    expect(store.entries).toStrictEqual([])
  })

  it('a subsequent successful refresh clears the ipcFailure flag', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {})
    invokeMock.mockRejectedValueOnce(new Error('ipc broken'))
    const store = useHistoryStore()
    await store.initialize()
    expect(store.ipcFailure).toBe(true)

    invokeMock.mockResolvedValueOnce({ entries: [], notices: [] } satisfies HistoryPage)
    emitQueueChanged()
    await vi.waitFor(() => expect(store.ipcFailure).toBe(false))
  })
})

describe('useHistoryStore — loadMore (Ф-4, курсор как есть)', () => {
  it('passes the exact cursor object received from the previous page, not a rebuilt one', async () => {
    const cursor = { finishedAtUnixSecs: 500, id: '7' }
    invokeMock.mockResolvedValueOnce({
      entries: [entry({ id: '9' })],
      nextCursor: cursor,
      notices: [],
    } satisfies HistoryPage)
    const store = useHistoryStore()
    await store.initialize()

    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '3' })], notices: [] } satisfies HistoryPage)
    await store.loadMore()

    // С-8 (правки ревью TL-93, второй раунд): честная формулировка того,
    // что здесь на самом деле проверяется. «Пересборка курсора из его же
    // двух полей» (`{ finishedAtUnixSecs: cursor.value.finishedAtUnixSecs,
    // id: cursor.value.id }`) дала бы структурно то же самое значение и
    // этой проверкой не ловится — глубокое равенство `toHaveBeenLastCalledWith`
    // не отличает «взято как есть» от «собрано заново из тех же двух
    // полей значения». Ловится другой, настоящий баг: если бы курсор
    // строился из данных **записи** (`entry({id:'9'}).finishedAtUnixSecs`,
    // `.id`), а не из `nextCursor`, ответ команды, тестовые данные нарочно
    // расходятся (курсор — `{500, '7'}`, последняя запись — `{9,
    // 1_756_130_400}`), и вызов не совпал бы с ожидаемым.
    expect(invokeMock).toHaveBeenLastCalledWith('history_page', { cursor })
    expect(store.entries.map((e) => e.id)).toStrictEqual(['9', '3'])
    expect(store.nextCursor).toBeUndefined()
  })

  it('does nothing when there is no next cursor', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry()], notices: [] } satisfies HistoryPage)
    const store = useHistoryStore()
    await store.initialize()
    invokeMock.mockClear()

    await store.loadMore()

    expect(invokeMock).not.toHaveBeenCalled()
  })
})

describe('useHistoryStore — сверка первой страницы со списком (Б-2/С-1, правки ревью TL-93, второй раунд)', () => {
  it('R1 (исправлено): fileStatus у уже показанной записи обновляется свежим ответом, а не остаётся устаревшим', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    const store = useHistoryStore()
    await store.initialize()
    expect(store.entries[0]!.fileStatus).toStrictEqual({ kind: 'present' })

    invokeMock.mockResolvedValueOnce({
      entries: [entry({ id: '1', fileStatus: { kind: 'missing', folderExists: false } })],
      notices: [],
    } satisfies HistoryPage)
    emitQueueChanged()

    await vi.waitFor(() => expect(store.entries[0]!.fileStatus).toStrictEqual({ kind: 'missing', folderExists: false }))
  })

  it('R2 (исправлено): пустая свежая страница (например, после пересоздания базы) сбрасывает список, а не оставляет устаревшие записи на экране', async () => {
    invokeMock.mockResolvedValueOnce({
      entries: [entry({ id: '3' }), entry({ id: '2' }), entry({ id: '1' })],
      notices: [],
    } satisfies HistoryPage)
    const store = useHistoryStore()
    await store.initialize()

    invokeMock.mockResolvedValueOnce({ entries: [], notices: [{ kind: 'baseRecreated' }] } satisfies HistoryPage)
    emitQueueChanged()

    await vi.waitFor(() => expect(store.notices).toHaveLength(1))
    expect(store.entries).toStrictEqual([])
    expect(store.nextCursor).toBeUndefined()
  })

  it('R3 (исправлено): больше страницы новых записей между обновлениями — список сброшен к свежей странице целиком, курсор свежий', async () => {
    invokeMock.mockResolvedValueOnce({
      entries: entriesRange(100, 71),
      nextCursor: { finishedAtUnixSecs: 71, id: '71' },
      notices: [],
    } satisfies HistoryPage)
    const store = useHistoryStore()
    await store.initialize()
    expect(store.entries).toHaveLength(30)

    invokeMock.mockResolvedValueOnce({
      entries: entriesRange(140, 111),
      nextCursor: { finishedAtUnixSecs: 111, id: '111' },
      notices: [],
    } satisfies HistoryPage)
    emitQueueChanged()

    await vi.waitFor(() => expect(store.entries.map((e) => e.id)[0]).toBe('140'))
    expect(store.entries).toHaveLength(30)
    expect(store.nextCursor).toStrictEqual({ finishedAtUnixSecs: 111, id: '111' })
    // «Дыра» 110..101 никогда не будет загружена — цена сброса, названная
    // в doc-комментарии класса `useHistoryStore` («Первая страница»).
    expect(store.entries.map((e) => e.id)).not.toContain('105')
  })

  it('R4 (исправлено): после «Очистить» полная первая страница снова даёт «Показать ещё» (курсор не потерян)', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    const store = useHistoryStore()
    await store.initialize()

    invokeMock.mockResolvedValueOnce(undefined)
    await store.clearHistory()
    expect(store.entries).toStrictEqual([])

    invokeMock.mockResolvedValueOnce({
      entries: entriesRange(131, 102),
      nextCursor: { finishedAtUnixSecs: 102, id: '102' },
      notices: [],
    } satisfies HistoryPage)
    emitQueueChanged()

    await vi.waitFor(() => expect(store.entries).toHaveLength(30))
    expect(store.nextCursor).toStrictEqual({ finishedAtUnixSecs: 102, id: '102' })
  })

  it('prepends a genuinely new record above the tail loaded via «Показать ещё», replacing only the matched prefix and leaving the pagination cursor pointing past that tail', async () => {
    invokeMock.mockResolvedValueOnce({
      entries: [entry({ id: '3' })],
      nextCursor: { finishedAtUnixSecs: 300, id: '3' },
      notices: [],
    } satisfies HistoryPage)
    const store = useHistoryStore()
    await store.initialize()

    invokeMock.mockResolvedValueOnce({
      entries: [entry({ id: '2' })],
      nextCursor: { finishedAtUnixSecs: 200, id: '2' },
      notices: [],
    } satisfies HistoryPage)
    await store.loadMore()
    expect(store.entries.map((e) => e.id)).toStrictEqual(['3', '2'])

    invokeMock.mockResolvedValueOnce({
      entries: [entry({ id: '4' }), entry({ id: '3' })],
      nextCursor: { finishedAtUnixSecs: 300, id: '3' },
      notices: [],
    } satisfies HistoryPage)
    emitQueueChanged()
    await vi.waitFor(() => expect(store.entries.map((e) => e.id)).toStrictEqual(['4', '3', '2']))

    // Курсор продолжения не переписан из ответа обновления — «Показать ещё»
    // должно уйти за уже показанный хвост («2»), а не повторить то, что уже видно.
    expect(store.nextCursor).toStrictEqual({ finishedAtUnixSecs: 200, id: '2' })
  })

  it('does not overwrite the tail when queue://changed fires but nothing genuinely new arrived on the first page', async () => {
    invokeMock.mockResolvedValueOnce({
      entries: [entry({ id: '3' })],
      nextCursor: { finishedAtUnixSecs: 300, id: '3' },
      notices: [],
    } satisfies HistoryPage)
    const store = useHistoryStore()
    await store.initialize()

    invokeMock.mockResolvedValueOnce({
      entries: [entry({ id: '2' })],
      nextCursor: { finishedAtUnixSecs: 200, id: '2' },
      notices: [],
    } satisfies HistoryPage)
    await store.loadMore()

    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '3' })], nextCursor: { finishedAtUnixSecs: 300, id: '3' }, notices: [] } satisfies HistoryPage)
    emitQueueChanged()
    await vi.waitFor(() => expect(invokeMock).toHaveBeenCalledTimes(3))

    expect(store.entries.map((e) => e.id)).toStrictEqual(['3', '2'])
  })
})

describe('useHistoryStore — гонка двух refreshFirst (правки ревью TL-93, второй раунд)', () => {
  it('applies only the response of the most recently started refreshFirst, even if it resolves before an older, still in-flight one', async () => {
    const store = useHistoryStore()

    let resolveOlder: ((page: HistoryPage) => void) | undefined
    invokeMock.mockImplementationOnce(() => new Promise((resolve) => (resolveOlder = resolve)))
    const olderCall = store.refreshFirst()

    let resolveNewer: ((page: HistoryPage) => void) | undefined
    invokeMock.mockImplementationOnce(() => new Promise((resolve) => (resolveNewer = resolve)))
    const newerCall = store.refreshFirst()

    // Более новый запрос отвечает первым по часам стенда...
    resolveNewer!({ entries: [entry({ id: 'newer' })], notices: [] } satisfies HistoryPage)
    await newerCall
    expect(store.entries.map((e) => e.id)).toStrictEqual(['newer'])

    // ...а более старый — только теперь. Его ответ обязан быть отброшен
    // целиком: применить его значило бы откатить экран к устаревшим данным.
    resolveOlder!({ entries: [entry({ id: 'older' })], notices: [] } satisfies HistoryPage)
    await olderCall

    expect(store.entries.map((e) => e.id)).toStrictEqual(['newer'])
  })

  it('discards a stale rejection the same way it discards a stale resolve', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {})
    const store = useHistoryStore()

    let rejectOlder: ((err: unknown) => void) | undefined
    invokeMock.mockImplementationOnce(() => new Promise((_resolve, reject) => (rejectOlder = reject)))
    const olderCall = store.refreshFirst()

    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: 'newer' })], notices: [] } satisfies HistoryPage)
    await store.refreshFirst()
    expect(store.entries.map((e) => e.id)).toStrictEqual(['newer'])

    rejectOlder!(new Error('stale ipc failure'))
    await olderCall.catch(() => undefined)

    // Устаревший отказ не должен ни включить `ipcFailure`, ни стереть уже
    // применённые новые данные.
    expect(store.ipcFailure).toBe(false)
    expect(store.entries.map((e) => e.id)).toStrictEqual(['newer'])
  })

  it('one invoke("history_page") call per queue://changed event, no extra calls from the race guard itself', async () => {
    invokeMock.mockResolvedValue({ entries: [], notices: [] } satisfies HistoryPage)
    const store = useHistoryStore()
    await store.initialize()
    invokeMock.mockClear()

    for (let i = 0; i < 5; i++) emitQueueChanged()
    await vi.waitFor(() => expect(invokeMock).toHaveBeenCalledTimes(5))
    expect(invokeMock.mock.calls.every(([cmd]) => cmd === 'history_page')).toBe(true)
  })
})

describe('useHistoryStore — живая зона (дизайн E5, «Доступность»)', () => {
  it('stays silent on the very first load (mounting is not a live structural change)', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry()], notices: [] } satisfies HistoryPage)
    const store = useHistoryStore()

    await store.initialize()

    expect(store.liveAnnouncement).toBe('')
  })

  it('announces once, naming the entry, when a genuinely new record arrives via queue://changed after the first load', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    const store = useHistoryStore()
    await store.initialize()
    expect(store.liveAnnouncement).toBe('')

    invokeMock.mockResolvedValueOnce({
      entries: [entry({ id: '2', title: 'Новый ролик' }), entry({ id: '1' })],
      notices: [],
    } satisfies HistoryPage)
    emitQueueChanged()
    await vi.waitFor(() => expect(store.entries).toHaveLength(2))

    // Мелочи правок ревью TL-93 (второй раунд): текст меняется с
    // названием — та же строка `"Добавлена новая запись"` дважды подряд
    // не была бы замечена скринридером (aria-live озвучивает изменение
    // текста, не факт присваивания одного и того же значения).
    expect(store.liveAnnouncement).toContain('Новый ролик')
  })

  it('stays silent when queue://changed fires but nothing genuinely new arrived', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    const store = useHistoryStore()
    await store.initialize()

    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    emitQueueChanged()
    await vi.waitFor(() => expect(invokeMock).toHaveBeenCalledTimes(2))

    expect(store.liveAnnouncement).toBe('')
  })

  it('does not announce for a page loaded via the user-initiated «Показать ещё» either', async () => {
    invokeMock.mockResolvedValueOnce({
      entries: [entry({ id: '1' })],
      nextCursor: { finishedAtUnixSecs: 1, id: '1' },
      notices: [],
    } satisfies HistoryPage)
    const store = useHistoryStore()
    await store.initialize()

    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '2' })], notices: [] } satisfies HistoryPage)
    await store.loadMore()

    expect(store.liveAnnouncement).toBe('')
  })
})

describe('useHistoryStore — пометки (Ф-3/С-10, Б-1 правки ревью TL-93, второй раунд)', () => {
  it('shows both notices at once when both arrive together (mutation: taking only the first must fail this)', async () => {
    invokeMock.mockResolvedValueOnce({
      entries: [],
      notices: [{ kind: 'baseRecreated' }, { kind: 'lastWriteFailed', cause: 'diskFull' }],
    } satisfies HistoryPage)
    const store = useHistoryStore()

    await store.initialize()

    expect(store.notices).toHaveLength(2)
  })

  it('dismissNotice removes only the matching kind locally', async () => {
    invokeMock.mockResolvedValueOnce({
      entries: [],
      notices: [{ kind: 'baseRecreated' }, { kind: 'lastWriteFailed', cause: 'diskFull' }],
    } satisfies HistoryPage)
    const store = useHistoryStore()
    await store.initialize()

    store.dismissNotice('baseRecreated')

    expect(store.notices).toStrictEqual([{ kind: 'lastWriteFailed', cause: 'diskFull' }])
  })

  it('a cursor page (loadMore) never overwrites already-shown notices, even if the response carried some', async () => {
    invokeMock.mockResolvedValueOnce({
      entries: [entry({ id: '1' })],
      nextCursor: { finishedAtUnixSecs: 1, id: '1' },
      notices: [{ kind: 'baseRecreated' }],
    } satisfies HistoryPage)
    const store = useHistoryStore()
    await store.initialize()
    expect(store.notices).toStrictEqual([{ kind: 'baseRecreated' }])

    invokeMock.mockResolvedValueOnce({ entries: [], notices: [] } satisfies HistoryPage)
    await store.loadMore()

    expect(store.notices).toStrictEqual([{ kind: 'baseRecreated' }])
  })

  it('R5 (исправлено): пометка не гаснет сама на следующем пустом queue://changed — уходит только через dismissNotice', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [], notices: [] } satisfies HistoryPage)
    const store = useHistoryStore()
    await store.initialize()

    invokeMock.mockResolvedValueOnce({
      entries: [entry({ id: '1' })],
      notices: [{ kind: 'lastWriteFailed', cause: 'diskFull' }],
    } satisfies HistoryPage)
    emitQueueChanged() // Done зафиксирован, пометка приходит
    await vi.waitFor(() => expect(store.notices).toHaveLength(1))

    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    emitQueueChanged() // следующее событие очереди — пустой ответ без пометок
    await vi.waitFor(() => expect(invokeMock).toHaveBeenCalledTimes(3))

    expect(store.notices).toStrictEqual([{ kind: 'lastWriteFailed', cause: 'diskFull' }])

    store.dismissNotice('lastWriteFailed')
    expect(store.notices).toStrictEqual([])
  })
})

describe('useHistoryStore.deleteRecord', () => {
  it('removes the row immediately on success, no confirmation involved at this layer', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '1' }), entry({ id: '2' })], notices: [] } satisfies HistoryPage)
    const store = useHistoryStore()
    await store.initialize()

    invokeMock.mockResolvedValueOnce(undefined)
    await store.deleteRecord('1')

    expect(invokeMock).toHaveBeenLastCalledWith('delete_history_record', { id: '1' })
    expect(store.entries.map((e) => e.id)).toStrictEqual(['2'])
  })

  it('С-3 (правки ревью TL-93, второй раунд): unknownRecord removes the row silently — no commandError, the goal (row gone) is already achieved', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    const store = useHistoryStore()
    await store.initialize()

    invokeMock.mockRejectedValueOnce({ kind: 'unknownRecord', message: 'diag' })
    await store.deleteRecord('1')

    expect(store.entries).toStrictEqual([])
    expect(store.commandError).toBeUndefined()
  })

  it('records a typed commandError on writeFailed and keeps the row', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    const store = useHistoryStore()
    await store.initialize()

    invokeMock.mockRejectedValueOnce({ kind: 'writeFailed', message: 'diag' })
    await store.deleteRecord('1')

    expect(store.entries.map((e) => e.id)).toStrictEqual(['1'])
    expect(store.commandError).toStrictEqual({ kind: 'writeFailed', message: 'diag' })
  })

  it('мелочи (правки ревью TL-93, второй раунд): unavailable blocks the whole screen, the same way history_page does — no commandError banner', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    const store = useHistoryStore()
    await store.initialize()

    invokeMock.mockRejectedValueOnce({ kind: 'unavailable', reason: 'noAccess', message: 'diag' })
    await store.deleteRecord('1')

    expect(store.availability).toStrictEqual({ reason: 'noAccess', message: 'diag' })
    expect(store.commandError).toBeUndefined()
    expect(store.entries.map((e) => e.id)).toStrictEqual(['1'])
  })
})

describe('useHistoryStore.clearHistory', () => {
  it('empties entries and the pagination cursor on success', async () => {
    invokeMock.mockResolvedValueOnce({
      entries: [entry({ id: '1' })],
      nextCursor: { finishedAtUnixSecs: 1, id: '1' },
      notices: [],
    } satisfies HistoryPage)
    const store = useHistoryStore()
    await store.initialize()

    invokeMock.mockResolvedValueOnce(undefined)
    await store.clearHistory()

    expect(invokeMock).toHaveBeenLastCalledWith('clear_history')
    expect(store.entries).toStrictEqual([])
    expect(store.nextCursor).toBeUndefined()
  })

  it('records a typed commandError and keeps entries on failure', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    const store = useHistoryStore()
    await store.initialize()

    invokeMock.mockRejectedValueOnce({ kind: 'writeFailed', message: 'diag' })
    await store.clearHistory()

    expect(store.entries).toHaveLength(1)
    expect(store.commandError).toStrictEqual({ kind: 'writeFailed', message: 'diag' })
  })

  it('мелочи (правки ревью TL-93, второй раунд): unavailable blocks the whole screen instead of a commandError banner', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    const store = useHistoryStore()
    await store.initialize()

    invokeMock.mockRejectedValueOnce({ kind: 'unavailable', reason: 'migrationFailed', message: 'diag' })
    await store.clearHistory()

    expect(store.availability).toStrictEqual({ reason: 'migrationFailed', message: 'diag' })
    expect(store.commandError).toBeUndefined()
    expect(store.entries).toHaveLength(1)
  })
})

describe('useHistoryStore.showInFolder', () => {
  it('clears any prior row error on success', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    const store = useHistoryStore()
    await store.initialize()

    invokeMock.mockResolvedValueOnce(undefined)
    await store.showInFolder(store.entries[0]!)

    expect(invokeMock).toHaveBeenLastCalledWith('show_in_folder', { id: '1' })
    expect(store.showInFolderErrors['1']).toBeUndefined()
  })

  it('does not record a row error for the expected fileMissing+folderExists combination (design table row 2)', async () => {
    const missingEntry = entry({ id: '1', fileStatus: { kind: 'missing', folderExists: true } })
    invokeMock.mockResolvedValueOnce({ entries: [missingEntry], notices: [] } satisfies HistoryPage)
    const store = useHistoryStore()
    await store.initialize()

    invokeMock.mockRejectedValueOnce({ kind: 'fileMissing', message: 'diag' })
    await store.showInFolder(store.entries[0]!)

    expect(store.showInFolderErrors['1']).toBeUndefined()
  })

  it('records a row error for fileMissing on a row that looked present (race)', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    const store = useHistoryStore()
    await store.initialize()

    invokeMock.mockRejectedValueOnce({ kind: 'fileMissing', message: 'diag' })
    await store.showInFolder(store.entries[0]!)

    expect(store.showInFolderErrors['1']).toStrictEqual({ kind: 'fileMissing', message: 'diag' })
  })

  it('records a row error for launcherFailed regardless of file status', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    const store = useHistoryStore()
    await store.initialize()

    invokeMock.mockRejectedValueOnce({ kind: 'launcherFailed', details: { exitCode: 1 }, message: 'diag' })
    await store.showInFolder(store.entries[0]!)

    expect(store.showInFolderErrors['1']).toStrictEqual({ kind: 'launcherFailed', details: { exitCode: 1 }, message: 'diag' })
  })

  it('С-3 (правки ревью TL-93, второй раунд): unknownRecord marks the row goneFromHistory and re-requests the first page, instead of advising to "refresh history"', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    const store = useHistoryStore()
    await store.initialize()

    invokeMock.mockRejectedValueOnce({ kind: 'unknownRecord', message: 'diag' })
    invokeMock.mockResolvedValueOnce({ entries: [], notices: [] } satisfies HistoryPage)
    await store.showInFolder(store.entries[0]!)
    await vi.waitFor(() => expect(invokeMock).toHaveBeenCalledTimes(3))

    expect(store.showInFolderErrors['1']).toStrictEqual({ kind: 'goneFromHistory' })
    expect(invokeMock).toHaveBeenLastCalledWith('history_page', { cursor: undefined })
  })

  it('мелочи (правки ревью TL-93, второй раунд): unavailable blocks the whole screen instead of a per-row error', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    const store = useHistoryStore()
    await store.initialize()

    invokeMock.mockRejectedValueOnce({ kind: 'unavailable', reason: 'noAccess', message: 'diag' })
    await store.showInFolder(store.entries[0]!)

    expect(store.availability).toStrictEqual({ reason: 'noAccess', message: 'diag' })
    expect(store.showInFolderErrors['1']).toBeUndefined()
  })
})
