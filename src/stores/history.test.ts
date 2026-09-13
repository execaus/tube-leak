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

    expect(invokeMock).toHaveBeenCalledWith('history_page', { cursor: undefined, limit: 30 })
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

describe('useHistoryStore — loadMore (Ф-4, курсор как есть)', () => {
  it('passes the exact cursor object received from the previous page, not a rebuilt one (mutation: reconstructing it must fail this assertion)', async () => {
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

    expect(invokeMock).toHaveBeenLastCalledWith('history_page', { cursor, limit: 30 })
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

describe('useHistoryStore — refresh on queue://changed does not drop pages already loaded via «Показать ещё»', () => {
  it('prepends only genuinely new entries to the front, leaving the tail (loaded by loadMore) untouched, and keeps the pagination cursor pointing past that tail', async () => {
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

    // Новая завершённая загрузка появилась выше уже показанного — курсорless
    // ответ содержит и новую запись, и уже видимую «3» (сортировка по
    // (finishedAt, id) не поменялась).
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
})

describe('useHistoryStore — пометки (Ф-3/С-10)', () => {
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

  it('keeps the row and records a typed commandError on unknownRecord', async () => {
    invokeMock.mockResolvedValueOnce({ entries: [entry({ id: '1' })], notices: [] } satisfies HistoryPage)
    const store = useHistoryStore()
    await store.initialize()

    invokeMock.mockRejectedValueOnce({ kind: 'unknownRecord', message: 'diag' })
    await store.deleteRecord('1')

    expect(store.entries.map((e) => e.id)).toStrictEqual(['1'])
    expect(store.commandError).toStrictEqual({ kind: 'unknownRecord', message: 'diag' })
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
})
