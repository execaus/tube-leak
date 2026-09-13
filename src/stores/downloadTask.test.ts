import { createPinia, setActivePinia, type Pinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { DownloadProgressEvent, DownloadStarted, StartDownloadRequest } from '@/types/generated/download'
import type { QueueSnapshot } from '@/types/generated/queue'

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

const { useDownloadTaskStore } = await import('./downloadTask')

const request: StartDownloadRequest = {
  url: 'https://youtu.be/x',
  title: 'Как приручить дракона',
  streams: { videoFormatId: 'v1080', audioFormatId: 'a' },
  size: { kind: 'known', bytes: 303_038_464 },
  quality: { kind: 'standard', heightPx: 1080 },
}

const started: DownloadStarted = {
  taskId: 'task-1',
  phase: 'queued',
  plan: 'videoAndAudio',
}

function emitProgress(payload: DownloadProgressEvent): void {
  handlers.get('download://progress')?.({ payload })
}

function emitQueueChanged(snapshot: QueueSnapshot): void {
  handlers.get('queue://changed')?.({ payload: snapshot })
}

beforeEach(() => {
  vi.useFakeTimers()
  invokeMock.mockReset()
  listenMock.mockClear()
  unlistenMock.mockClear()
  handlers.clear()
  setActivePinia(createPinia())
})

afterEach(() => {
  vi.useRealTimers()
})

describe('useDownloadTaskStore — start (постановка в хвост очереди, Ф-2 E4)', () => {
  it('invokes start_download and optimistically appends the task, building the display title from title+quality (TL-75)', async () => {
    invokeMock.mockResolvedValueOnce(started)
    const store = useDownloadTaskStore()

    await store.start(request)

    expect(invokeMock).toHaveBeenCalledWith('start_download', { request })
    expect(store.tasks).toStrictEqual([
      { taskId: 'task-1', title: 'Как приручить дракона', quality: request.quality, plan: 'videoAndAudio', phase: 'queued' },
    ])
    expect(store.task).toStrictEqual({
      taskId: 'task-1',
      plan: 'videoAndAudio',
      displayTitle: '«Как приручить дракона» — 1080p',
    })
    expect(store.progress).toStrictEqual({ phase: 'queued' })
    expect(store.isActive).toBe(true) // тот же критерий, что в E3: задача есть и не терминальна.
  })

  it('subscribes to download://progress before invoking the command', async () => {
    invokeMock.mockResolvedValueOnce(started)
    const store = useDownloadTaskStore()

    await store.start(request)

    expect(listenMock).toHaveBeenCalledWith('download://progress', expect.any(Function))
  })

  it('does not throw and logs when the core rejects the start (e.g. duplicateTask)', async () => {
    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    invokeMock.mockRejectedValueOnce({
      kind: 'duplicateTask',
      message: 'diag',
      existing: { taskId: 'task-0', title: 'Летний влог', quality: { kind: 'standard', heightPx: 720 } },
    })
    const store = useDownloadTaskStore()

    await expect(store.start(request)).resolves.toBeUndefined()

    expect(store.tasks).toStrictEqual([])
    expect(consoleErrorSpy).toHaveBeenCalled()
    consoleErrorSpy.mockRestore()
  })

  it('builds the initial phase from the response, not a hardcoded constant (ревью TL-45) — a second task can start in "fetching" directly', async () => {
    // Контракт гарантирует `queued` для первой задачи пустой очереди
    // сегодня, но doc `DownloadStarted.phase` прямо требует рисовать по
    // присланному полю (задел под очередь E4, теперь штатный путь) —
    // проверяем это буквально, подставив фазу `fetching` в ответ.
    invokeMock.mockResolvedValueOnce({ taskId: 'task-9', phase: 'fetching', plan: 'singleStream' })
    const store = useDownloadTaskStore()

    await store.start(request)

    expect(store.progress).toStrictEqual({ phase: 'fetching' })
    expect(store.isActive).toBe(true)
  })

  it('ignores a second concurrent start() call while the first is still in flight (double-click window)', async () => {
    let resolveFirst: (value: DownloadStarted) => void = () => {}
    invokeMock.mockImplementationOnce(
      () =>
        new Promise<DownloadStarted>((resolve) => {
          resolveFirst = resolve
        }),
    )

    const store = useDownloadTaskStore()
    const firstCall = store.start(request)
    const secondCall = store.start(request)

    // Дать первому вызову дойти до `invoke()` (несколько микротасков внутри
    // `ensureProgressListening()`), прежде чем разрешать его, — иначе
    // `resolveFirst` мог бы вызваться раньше, чем `mockImplementationOnce`
    // успел его переприсвоить, и промис никогда бы не разрешился.
    await vi.waitFor(() => {
      expect(invokeMock).toHaveBeenCalledTimes(1)
    })
    resolveFirst(started)
    await Promise.all([firstCall, secondCall])

    expect(invokeMock).toHaveBeenCalledTimes(1)
    expect(store.tasks).toHaveLength(1)
  })

  it('appends a second task after the first — FIFO order (Р-4), not sorted or replaced', async () => {
    invokeMock.mockResolvedValueOnce(started)
    const store = useDownloadTaskStore()
    await store.start(request)

    invokeMock.mockResolvedValueOnce({ taskId: 'task-2', phase: 'queued', plan: 'singleStream' })
    await store.start({ ...request, title: 'Ролик Б', quality: { kind: 'audioOnly' } })

    expect(store.tasks.map((t) => t.taskId)).toStrictEqual(['task-1', 'task-2'])
  })

  it('surfaces a rejected start as a typed commandError, visible on screen (not just logged), and clears it on the next successful start', async () => {
    invokeMock.mockRejectedValueOnce({ kind: 'invalidUrl', message: 'core diagnostic' })
    const store = useDownloadTaskStore()

    await store.start(request)
    expect(store.commandError).toStrictEqual({ kind: 'invalidUrl', message: 'core diagnostic' })

    invokeMock.mockResolvedValueOnce(started)
    await store.start(request)
    expect(store.commandError).toBeUndefined()
  })

  it('falls back to a message-only failure for a non-contractual rejection (unrecognized shape)', async () => {
    invokeMock.mockRejectedValueOnce(new Error('boom'))
    const store = useDownloadTaskStore()

    await store.start(request)

    expect(store.commandError).toStrictEqual({ message: 'boom' })
  })

  it('subscribes to download://progress only once across repeated starts on the same store instance (no leaked/duplicated listeners)', async () => {
    invokeMock.mockResolvedValueOnce(started)
    const store = useDownloadTaskStore()
    await store.start(request)

    invokeMock.mockResolvedValueOnce({ taskId: 'task-2', phase: 'queued', plan: 'singleStream' })
    await store.start(request)

    const progressListenCalls = listenMock.mock.calls.filter(([name]) => name === 'download://progress')
    expect(progressListenCalls).toHaveLength(1)
  })
})

describe('useDownloadTaskStore — cancel/retry/hide/resume: отказ команды тоже не глушится', () => {
  it('invokes cancel_download with the given taskId', async () => {
    invokeMock.mockResolvedValueOnce(started)
    const store = useDownloadTaskStore()
    await store.start(request)

    invokeMock.mockResolvedValueOnce(undefined)
    await store.cancel('task-1')

    expect(invokeMock).toHaveBeenCalledWith('cancel_download', { taskId: 'task-1' })
  })

  it('surfaces a rejected cancel as commandError', async () => {
    invokeMock.mockRejectedValueOnce({ kind: 'unknownTask', message: 'diag' })
    const store = useDownloadTaskStore()

    await store.cancel('task-1')

    expect(store.commandError).toStrictEqual({ kind: 'unknownTask', message: 'diag' })
  })

  it('invokes retry_download with the given taskId', async () => {
    invokeMock.mockResolvedValueOnce(undefined)
    const store = useDownloadTaskStore()

    await store.retry('task-1')

    expect(invokeMock).toHaveBeenCalledWith('retry_download', { taskId: 'task-1' })
  })

  it('surfaces a rejected retry as commandError', async () => {
    invokeMock.mockRejectedValueOnce({ kind: 'notFailed', message: 'diag' })
    const store = useDownloadTaskStore()

    await store.retry('task-1')

    expect(store.commandError).toStrictEqual({ kind: 'notFailed', message: 'diag' })
  })

  it('invokes dismiss_queue_task for hide(), and surfaces a rejected hide (taskNotFinished) as commandError', async () => {
    invokeMock.mockRejectedValueOnce({ kind: 'taskNotFinished', message: 'diag' })
    const store = useDownloadTaskStore()

    await store.hide('task-1')

    expect(invokeMock).toHaveBeenCalledWith('dismiss_queue_task', { taskId: 'task-1' })
    expect(store.commandError).toStrictEqual({ kind: 'taskNotFinished', message: 'diag' })
  })

  it('hideAllTerminal calls dismiss_queue_task once per terminal task, and leaves non-terminal tasks alone', async () => {
    const store = useDownloadTaskStore()
    invokeMock.mockResolvedValueOnce({ tasks: [], awaitingContinue: false })
    await store.initialize()
    emitQueueChanged({
      awaitingContinue: false,
      tasks: [
        { taskId: 'a', title: 'A', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'done', fileName: 'a.mp3', folderDisplay: { kind: 'systemDownloads' } },
        { taskId: 'b', title: 'B', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'queued' },
        { taskId: 'c', title: 'C', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'cancelled', partialData: 'removed' },
      ],
    })

    invokeMock.mockResolvedValue(undefined)
    await store.hideAllTerminal()

    expect(invokeMock).toHaveBeenCalledWith('dismiss_queue_task', { taskId: 'a' })
    expect(invokeMock).toHaveBeenCalledWith('dismiss_queue_task', { taskId: 'c' })
    expect(invokeMock).not.toHaveBeenCalledWith('dismiss_queue_task', { taskId: 'b' })
  })

  it('invokes resume_queue with no arguments, and surfaces a rejection as commandError', async () => {
    invokeMock.mockRejectedValueOnce({ kind: 'unknownTask', message: 'diag' })
    const store = useDownloadTaskStore()

    await store.resume()

    expect(invokeMock).toHaveBeenCalledWith('resume_queue')
    expect(store.commandError).toStrictEqual({ kind: 'unknownTask', message: 'diag' })
  })

  it('dismissCommandError clears the banner', async () => {
    invokeMock.mockRejectedValueOnce({ kind: 'invalidUrl', message: 'diag' })
    const store = useDownloadTaskStore()
    await store.start(request)
    expect(store.commandError).not.toBeUndefined()

    store.dismissCommandError()
    expect(store.commandError).toBeUndefined()
  })
})

describe('useDownloadTaskStore — подписка не течёт: снимается при dispose стора', () => {
  it('calls unlisten once the store is disposed ($dispose — Pinia\'s public teardown, tears down the store\'s effect scope and its onScopeDispose hooks)', async () => {
    // Собственный, изолированный Pinia-инстанс — предыдущий из общего
    // beforeEach() здесь не годится: тест проверяет именно уничтожение
    // конкретного стора, а не просто создаёт очередной.
    const pinia: Pinia = createPinia()
    setActivePinia(pinia)

    invokeMock.mockResolvedValueOnce(started)
    const store = useDownloadTaskStore()
    await store.start(request)
    invokeMock.mockResolvedValueOnce({ tasks: [], awaitingContinue: false })
    await store.initialize()

    expect(unlistenMock).not.toHaveBeenCalled()
    store.$dispose()
    // Обе подписки (прогресс и очередь) должны быть сняты.
    expect(unlistenMock).toHaveBeenCalledTimes(2)
  })
})

describe('useDownloadTaskStore — события прогресса, ключуемые taskId (Ф-6: только по активной задаче)', () => {
  it('updates progress only for events matching an existing taskId', async () => {
    invokeMock.mockResolvedValueOnce(started)
    const store = useDownloadTaskStore()
    await store.start(request)

    emitProgress({ taskId: 'other-task', phase: 'downloading', state: 'running', percent: 5 })
    expect(store.progress).toStrictEqual({ phase: 'queued' })

    emitProgress({ taskId: 'task-1', phase: 'downloading', state: 'running', percent: 5 })
    expect(store.progress).toStrictEqual({ phase: 'downloading', state: 'running', percent: 5 })
  })

  it('keeps receiving events after a failed phase — retry resumes the same stream (TL-45 п.3, unchanged by E4)', async () => {
    invokeMock.mockResolvedValueOnce(started)
    const store = useDownloadTaskStore()
    await store.start(request)

    emitProgress({
      taskId: 'task-1',
      phase: 'failed',
      error: {
        kind: 'connectionLost',
        message: 'diagnostic',
        retryable: true,
        partialData: 'kept',
      },
    })
    expect(store.progress?.phase).toBe('failed')

    invokeMock.mockResolvedValueOnce(undefined)
    await store.retry('task-1')
    expect(invokeMock).toHaveBeenCalledWith('retry_download', { taskId: 'task-1' })

    // Поток не отписан и не отфильтрован по факту отказа.
    emitProgress({ taskId: 'task-1', phase: 'downloading', state: 'running', percent: 10 })
    expect(store.progress).toStrictEqual({ phase: 'downloading', state: 'running', percent: 10 })
  })
})

describe('useDownloadTaskStore — task/progress/isActive: обратная совместимость на первой задаче списка', () => {
  it('is active right after start(), even while the sole task is still "queued" (тот же критерий, что был в E3 до появления списка)', async () => {
    invokeMock.mockResolvedValueOnce(started)
    const store = useDownloadTaskStore()
    await store.start(request)

    expect(store.isActive).toBe(true)
    expect(store.progress).toStrictEqual({ phase: 'queued' })
  })

  it('is not active once the task reaches a terminal phase', async () => {
    invokeMock.mockResolvedValueOnce({ taskId: 'task-1', phase: 'downloading', plan: 'videoAndAudio' })
    const store = useDownloadTaskStore()
    await store.start(request)
    expect(store.isActive).toBe(true)

    emitProgress({ taskId: 'task-1', phase: 'done', fileName: 'video.mp4', folderDisplay: { kind: 'systemDownloads' } })
    expect(store.isActive).toBe(false)
    // `progress`/`task` продолжают показывать терминальную задачу — тот
    // же приём, что и в E3 (панель рисует Done/Failed/Cancelled тем же
    // `progress`); терминальность решает только `isActive`.
    expect(store.progress).toStrictEqual({ phase: 'done', fileName: 'video.mp4', folderDisplay: { kind: 'systemDownloads' } })
  })

  it('is not active when no task exists', () => {
    const store = useDownloadTaskStore()
    expect(store.isActive).toBe(false)
  })

  it('known limitation (documented for TL-76): reflects the first task in list order, not necessarily the one actually running — a terminal task earlier in the list still wins', async () => {
    const store = useDownloadTaskStore()
    invokeMock.mockResolvedValueOnce({ tasks: [], awaitingContinue: false })
    await store.initialize()
    emitQueueChanged({
      awaitingContinue: false,
      tasks: [
        { taskId: 'a', title: 'A', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'done', fileName: 'a.mp3', folderDisplay: { kind: 'systemDownloads' } },
        { taskId: 'b', title: 'B', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'downloading', state: 'running' },
      ],
    })

    // Обратная совместимость (doc `useDownloadTaskStore`, «Обратная
    // совместимость») намеренно не «умная»: `useExitConfirmation.ts`
    // сегодня рассчитан на однозадачный случай, и полноценный срез всей
    // очереди для диалога выхода — TL-76 (issue #83), не эта задача.
    expect(store.task?.taskId).toBe('a')
    expect(store.isActive).toBe(false)
  })
})

describe('useDownloadTaskStore — снимок очереди (queue_state/queue://changed, С-9)', () => {
  it('initialize() subscribes to queue://changed before requesting queue_state, and applies the fetched snapshot', async () => {
    invokeMock.mockResolvedValueOnce({
      tasks: [{ taskId: 'r', title: 'Restored', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'queued' }],
      awaitingContinue: true,
    } satisfies QueueSnapshot)
    const store = useDownloadTaskStore()

    await store.initialize()

    expect(listenMock).toHaveBeenCalledWith('queue://changed', expect.any(Function))
    expect(invokeMock).toHaveBeenCalledWith('queue_state')
    expect(store.tasks).toHaveLength(1)
    expect(store.awaitingContinue).toBe(true)
  })

  it('applies a later queue://changed snapshot in full — including pauseReason', async () => {
    const store = useDownloadTaskStore()
    invokeMock.mockResolvedValueOnce({ tasks: [], awaitingContinue: false })
    await store.initialize()

    emitQueueChanged({
      tasks: [],
      awaitingContinue: false,
      pauseReason: 'ytDlpUpdate',
    })

    expect(store.pauseReason).toBe('ytDlpUpdate')
    expect(store.tasks).toStrictEqual([])
  })

  it('does not throw when queue_state rejects — leaves the list empty and logs', async () => {
    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    invokeMock.mockRejectedValueOnce(new Error('ipc down'))
    const store = useDownloadTaskStore()

    await expect(store.initialize()).resolves.toBeUndefined()

    expect(store.tasks).toStrictEqual([])
    expect(consoleErrorSpy).toHaveBeenCalled()
    consoleErrorSpy.mockRestore()
  })
})

describe('useDownloadTaskStore — мягкий индикатор зависания (5с, косметика фронтенда)', () => {
  it('sets softStallSeconds after 5s without a new running event, and clears it on the next event', async () => {
    invokeMock.mockResolvedValueOnce(started)
    const store = useDownloadTaskStore()
    await store.start(request)

    emitProgress({ taskId: 'task-1', phase: 'downloading', state: 'running', percent: 40, speedBytesPerSec: 1024 })
    expect(store.softStallSeconds).toBeUndefined()

    await vi.advanceTimersByTimeAsync(5_000)
    expect(store.softStallSeconds).toBeGreaterThanOrEqual(5)

    emitProgress({ taskId: 'task-1', phase: 'downloading', state: 'running', percent: 41, speedBytesPerSec: 1024 })
    expect(store.softStallSeconds).toBeUndefined()
  })

  it('does not show the stall indicator during waitingRetry (its own honest state, not a stalled Downloading)', async () => {
    invokeMock.mockResolvedValueOnce(started)
    const store = useDownloadTaskStore()
    await store.start(request)

    emitProgress({ taskId: 'task-1', phase: 'downloading', state: 'running', percent: 40 })
    emitProgress({
      taskId: 'task-1',
      phase: 'downloading',
      state: 'waitingRetry',
      percent: 40,
      attempt: { number: 2, total: 6 },
      delaySecs: 10,
      remainingSecs: 8,
    })

    await vi.advanceTimersByTimeAsync(6_000)
    expect(store.softStallSeconds).toBeUndefined()
  })
})

/**
 * Issue 86: `firstTask`/`tasks.value[0]` — позиция, не смысл. Планировщик
 * ядра (`pump`, `src-tauri/src/queue/scheduler.rs`) держит терминальную
 * задачу на её месте до явного «Скрыть» и берёт в работу первую
 * **нетерминальную**; это делает состав «терминальная в голове, рабочая
 * дальше» воспроизводимым тривиально — скачал, не скрыл, начал
 * следующую. Таймер зависания обязан следить за активной задачей по
 * смыслу, а не за позицией `[0]`.
 */
describe('useDownloadTaskStore — таймер зависания смотрит на активную задачу, не на позицию (issue 86)', () => {
  it('a terminal, not-yet-hidden task at the head of the list does not blind the timer to the second task, which is actually downloading', async () => {
    const store = useDownloadTaskStore()
    invokeMock.mockResolvedValueOnce({ tasks: [], awaitingContinue: false } satisfies QueueSnapshot)
    await store.initialize()

    // Воспроизведение состава из issue 86: терминальная задача в голове
    // (не скрыта), рабочая — дальше.
    emitQueueChanged({
      awaitingContinue: false,
      tasks: [
        { taskId: 'a', title: 'A', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'done', fileName: 'a.mp3', folderDisplay: { kind: 'systemDownloads' } },
        { taskId: 'b', title: 'B', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'downloading', state: 'running', percent: 10 },
      ],
    })

    await vi.advanceTimersByTimeAsync(5_000)

    expect(store.softStallSeconds).toBeGreaterThanOrEqual(5)
  })

  it('a terminal task at the head with no active task behind it — no stall message ever appears', async () => {
    const store = useDownloadTaskStore()
    invokeMock.mockResolvedValueOnce({ tasks: [], awaitingContinue: false } satisfies QueueSnapshot)
    await store.initialize()

    emitQueueChanged({
      awaitingContinue: false,
      tasks: [
        { taskId: 'a', title: 'A', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'done', fileName: 'a.mp3', folderDisplay: { kind: 'systemDownloads' } },
      ],
    })

    await vi.advanceTimersByTimeAsync(10_000)

    expect(store.softStallSeconds).toBeUndefined()
  })

  it('the active task changes — the timer restarts for the new one, the old stall message is not carried over', async () => {
    const store = useDownloadTaskStore()
    invokeMock.mockResolvedValueOnce({ tasks: [], awaitingContinue: false } satisfies QueueSnapshot)
    await store.initialize()

    emitQueueChanged({
      awaitingContinue: false,
      tasks: [
        { taskId: 'a', title: 'A', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'downloading', state: 'running', percent: 10 },
      ],
    })

    await vi.advanceTimersByTimeAsync(5_000)
    expect(store.softStallSeconds).toBeGreaterThanOrEqual(5)

    // 'a' завершилась, слот занял 'b' — тем же снимком, без единого
    // download://progress по новой задаче.
    emitQueueChanged({
      awaitingContinue: false,
      tasks: [
        { taskId: 'a', title: 'A', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'done', fileName: 'a.mp3', folderDisplay: { kind: 'systemDownloads' } },
        { taskId: 'b', title: 'B', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'downloading', state: 'running', percent: 0 },
      ],
    })

    // Сообщение про 'a' не переезжает на 'b' по факту смены активной задачи.
    expect(store.softStallSeconds).toBeUndefined()

    // И не появляется раньше своего порога для новой задачи — отсчёт
    // действительно начался заново, а не унаследовал старый `lastEventAt`.
    await vi.advanceTimersByTimeAsync(4_000)
    expect(store.softStallSeconds).toBeUndefined()

    await vi.advanceTimersByTimeAsync(1_000)
    expect(store.softStallSeconds).toBeGreaterThanOrEqual(5)
  })
})

/**
 * TL-98 (issue 105), правки ревью, второй раунд, С-2 — уровень стора, не
 * `App.vue`: `outcomeTextForTransition`/`findSnapshotOutcomeText` не
 * зависят ни от одной вкладки и заслуживают собственных тестов, а не
 * только косвенной проверки через смонтированное дерево `App.vue`.
 */
describe('useDownloadTaskStore — живая зона исходов outcomeAnnouncement (TL-98, issue 105, правки ревью С-2)', () => {
  it('R1: reverse order — queue://changed reports Done first, then download://progress reports the same Done — one announcement, not two', async () => {
    // `initialize()` перед `start()` — подписка на `queue://changed`
    // нужна именно этому тесту (`emitQueueChanged` иначе холостой, doc
    // `ensureQueueListening`); обычный порядок вызовов `App.vue` тот же.
    invokeMock.mockResolvedValueOnce({ tasks: [], awaitingContinue: false } satisfies QueueSnapshot)
    const store = useDownloadTaskStore()
    await store.initialize()

    invokeMock.mockResolvedValueOnce(started)
    await store.start(request)

    emitQueueChanged({
      tasks: [
        { taskId: 'task-1', title: request.title, quality: request.quality, plan: started.plan, phase: 'done', fileName: 'a.mp4', folderDisplay: { kind: 'systemDownloads' } },
      ],
      awaitingContinue: false,
    })
    expect(store.outcomeAnnouncement?.text).toBe(`«${request.title}» — готово`)
    const firstId = store.outcomeAnnouncement?.id

    // Ядро шлёт `download://progress` о том же переходе следующим — стор
    // уже видит фазу терминальной по снимку выше, второго объявления нет.
    emitProgress({ taskId: 'task-1', phase: 'done', fileName: 'a.mp4', folderDisplay: { kind: 'systemDownloads' } })

    expect(store.outcomeAnnouncement?.id).toBe(firstId)
  })

  it('R2: restart (К-13) — queue_state already carries a done/failed task with no prior phase to compare against — silent', async () => {
    invokeMock.mockResolvedValueOnce({
      tasks: [
        { taskId: 't1', title: 'A', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'done', fileName: 'a.mp3', folderDisplay: { kind: 'systemDownloads' } },
        {
          taskId: 't2',
          title: 'B',
          quality: { kind: 'audioOnly' },
          plan: 'singleStream',
          phase: 'failed',
          error: { kind: 'connectionLost', message: 'diag', retryable: true, partialData: 'kept' },
        },
      ],
      awaitingContinue: false,
    } satisfies QueueSnapshot)
    const store = useDownloadTaskStore()

    await store.initialize()

    expect(store.outcomeAnnouncement).toBeUndefined()
  })

  it('R3: a task disappears from the snapshot (hidden via dismiss_queue_task) — no comparison, no announcement', async () => {
    invokeMock.mockResolvedValueOnce({
      tasks: [{ taskId: 't1', title: 'A', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'downloading', state: 'running' }],
      awaitingContinue: false,
    } satisfies QueueSnapshot)
    const store = useDownloadTaskStore()
    await store.initialize()

    emitQueueChanged({ tasks: [], awaitingContinue: false })

    expect(store.outcomeAnnouncement).toBeUndefined()
  })

  it('R4: Failed → Retry (queued, fetching) → Failed again — the second outcome announces too, not just the first', async () => {
    invokeMock.mockResolvedValueOnce({
      tasks: [{ taskId: 't1', title: 'A', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'downloading', state: 'running' }],
      awaitingContinue: false,
    } satisfies QueueSnapshot)
    const store = useDownloadTaskStore()
    await store.initialize()

    const failedTask = {
      taskId: 't1',
      title: 'A',
      quality: { kind: 'audioOnly' as const },
      plan: 'singleStream' as const,
      phase: 'failed' as const,
      error: { kind: 'connectionLost' as const, message: 'diag', retryable: true, partialData: 'kept' as const },
    }
    emitQueueChanged({ tasks: [failedTask], awaitingContinue: false })
    const firstId = store.outcomeAnnouncement?.id
    expect(store.outcomeAnnouncement?.text).toContain('не удалось')

    emitQueueChanged({
      tasks: [{ taskId: 't1', title: 'A', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'queued' }],
      awaitingContinue: false,
    })
    emitQueueChanged({
      tasks: [{ taskId: 't1', title: 'A', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'fetching' }],
      awaitingContinue: false,
    })
    emitQueueChanged({ tasks: [failedTask], awaitingContinue: false })

    expect(store.outcomeAnnouncement?.id).not.toBe(firstId)
    expect(store.outcomeAnnouncement?.text).toContain('не удалось')
  })

  it('С-3: a Cancelled transition announces the same way as Done/Failed — core cancellation is async and can resolve after the user already navigated away', async () => {
    invokeMock.mockResolvedValueOnce({
      tasks: [{ taskId: 't1', title: 'A', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'downloading', state: 'running' }],
      awaitingContinue: false,
    } satisfies QueueSnapshot)
    const store = useDownloadTaskStore()
    await store.initialize()

    emitQueueChanged({
      tasks: [{ taskId: 't1', title: 'A', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'cancelled', partialData: 'removed' }],
      awaitingContinue: false,
    })

    expect(store.outcomeAnnouncement?.text).toBe('«A» — отменено')
  })
})
