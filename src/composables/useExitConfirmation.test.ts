import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { defineComponent } from 'vue'

import type { DownloadProgressEvent, DownloadStarted, StartDownloadRequest } from '@/types/generated/download'
import type { QueueSnapshot } from '@/types/generated/queue'

/**
 * Композабл диалога подтверждения выхода (Р-2, эпик E3, TL-46; срез всей
 * очереди — эпик E4, TL-76, С-6, Р-8). Оконное событие подставляется
 * фейковым портом — реальная реализация ждёт разрешение
 * `core:window:allow-destroy` (задача ядра #49, см. `windowExitPort.ts`);
 * здесь проверяется только бизнес-логика: когда показывать диалог и что
 * именно он говорит про очередь, независимо от того, как приходит
 * попытка закрытия.
 *
 * Пять состояний очереди (культура проверки CLAUDE.md: «каждое состояние
 * закрывается отдельным тестом»): нет задач; активная; только ожидающие
 * (в т.ч. пауза на обновление yt-dlp, Р-7 — там тоже нет активной задачи);
 * приостановленная после перезапуска (Р-8); только терминальные.
 */

type Handler = (event: { payload: unknown }) => void
const invokeMock = vi.fn()
const unlistenMock = vi.fn()
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

const { useDownloadTaskStore } = await import('@/stores/downloadTask')
const { useExitConfirmation } = await import('./useExitConfirmation')

function emitProgress(payload: DownloadProgressEvent): void {
  handlers.get('download://progress')?.({ payload })
}

function emitQueueChanged(snapshot: QueueSnapshot): void {
  handlers.get('queue://changed')?.({ payload: snapshot })
}

const EMPTY_SNAPSHOT: QueueSnapshot = { tasks: [], awaitingContinue: false }

/** Роутинг `invoke` по имени команды — `queue_state` по умолчанию пуст, остальное задаётся тестом. */
function routeInvoke(byCommand: Record<string, () => Promise<unknown>>): void {
  const withDefaults: Record<string, () => Promise<unknown>> = {
    queue_state: () => Promise.resolve(EMPTY_SNAPSHOT),
    ...byCommand,
  }
  invokeMock.mockImplementation((command: string) => {
    const handler = withDefaults[command]
    if (!handler) throw new Error(`unexpected invoke: ${command}`)
    return handler()
  })
}

const request: StartDownloadRequest = {
  url: 'https://youtu.be/x',
  title: 'Как приручить дракона',
  streams: { videoFormatId: 'v1080', audioFormatId: 'a' },
  size: { kind: 'known', bytes: 303_038_464 },
  quality: { kind: 'standard', heightPx: 1080 },
}

const started: DownloadStarted = { taskId: 'task-1', phase: 'downloading', plan: 'videoAndAudio' }

/** Фейковый порт — фиксирует, что именно вызвал композабл, без Tauri API. */
function createFakePort() {
  let handler: (() => void) | undefined
  const port = {
    onCloseAttempt: vi.fn((h: () => void) => {
      handler = h
      return vi.fn()
    }),
    finishWindow: vi.fn(async () => {
      port.finishCalls += 1
    }),
    attempt: () => handler?.(),
    finishCalls: 0,
  }
  return port
}

/** Аналог test-utils `withSetup` (см. `useYtDlpPrepare.test.ts`) — композабл использует `onMounted`/`onScopeDispose`. */
function withSetup<T>(composable: () => T): { result: T; unmount: () => void } {
  let result!: T
  const wrapper = mount(
    defineComponent({
      setup() {
        result = composable()
        return () => null
      },
    }),
  )
  return { result, unmount: () => wrapper.unmount() }
}

beforeEach(() => {
  invokeMock.mockReset()
  listenMock.mockClear()
  unlistenMock.mockClear()
  handlers.clear()
  routeInvoke({})
  setActivePinia(createPinia())
})

describe('useExitConfirmation — состояние «нет задач вообще» (С-6)', () => {
  it('попытка закрытия сразу завершает окно, диалог не показывается', () => {
    const port = createFakePort()
    const { result } = withSetup(() => useExitConfirmation(port))

    port.attempt()

    expect(result.visible.value).toBe(false)
    expect(port.finishCalls).toBe(1)
  })
})

describe('useExitConfirmation — состояние «только терминальные задачи» (С-6)', () => {
  it('для терминальной задачи (Done), даже не скрытой: диалог не показывается', async () => {
    const port = createFakePort()
    invokeMock.mockImplementation((command: string) => {
      if (command === 'queue_state') return Promise.resolve(EMPTY_SNAPSHOT)
      if (command === 'start_download') return Promise.resolve(started)
      throw new Error(`unexpected invoke: ${command}`)
    })
    const store = useDownloadTaskStore()
    await store.start(request)
    emitProgress({ taskId: 'task-1', phase: 'done', fileName: 'x.mp4' })

    const { result } = withSetup(() => useExitConfirmation(port))
    port.attempt()

    expect(result.visible.value).toBe(false)
    expect(port.finishCalls).toBe(1)
  })

  it('для терминальной задачи (Cancelled), даже не скрытой: диалог не показывается', async () => {
    const port = createFakePort()
    invokeMock.mockImplementation((command: string) => {
      if (command === 'queue_state') return Promise.resolve(EMPTY_SNAPSHOT)
      if (command === 'start_download') return Promise.resolve(started)
      throw new Error(`unexpected invoke: ${command}`)
    })
    const store = useDownloadTaskStore()
    await store.start(request)
    emitProgress({ taskId: 'task-1', phase: 'cancelled', partialData: 'removed' })

    const { result } = withSetup(() => useExitConfirmation(port))
    port.attempt()

    expect(result.visible.value).toBe(false)
    expect(port.finishCalls).toBe(1)
  })

  it('несколько терминальных задач сразу (Done + Failed), нетерминальных нет: диалог не показывается', async () => {
    const port = createFakePort()
    const store = useDownloadTaskStore()
    await store.initialize()
    emitQueueChanged({
      awaitingContinue: false,
      tasks: [
        { taskId: 'a', title: 'Ролик A', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'done', fileName: 'a.mp4' },
        {
          taskId: 'b',
          title: 'Ролик B',
          quality: { kind: 'audioOnly' },
          plan: 'singleStream',
          phase: 'failed',
          error: { kind: 'connectionLost', message: 'diag', retryable: true, partialData: 'kept' },
        },
      ],
    })

    const { result } = withSetup(() => useExitConfirmation(port))
    port.attempt()

    expect(result.visible.value).toBe(false)
    expect(port.finishCalls).toBe(1)
  })
})

describe('useExitConfirmation — состояние «активная задача» (С-6)', () => {
  it('для активной нетерминальной задачи (Downloading): диалог показывается, окно пока не завершается, названа задача', async () => {
    const port = createFakePort()
    invokeMock.mockImplementation((command: string) => {
      if (command === 'queue_state') return Promise.resolve(EMPTY_SNAPSHOT)
      if (command === 'start_download') return Promise.resolve(started)
      throw new Error(`unexpected invoke: ${command}`)
    })
    const store = useDownloadTaskStore()
    await store.start(request)
    emitProgress({ taskId: 'task-1', phase: 'downloading', state: 'running', percent: 40 })

    const { result } = withSetup(() => useExitConfirmation(port))
    port.attempt()

    expect(result.visible.value).toBe(true)
    expect(port.finishCalls).toBe(0)
    expect(result.activeTask.value).toStrictEqual({
      displayTitle: '«Как приручить дракона» — 1080p',
      progress: { phase: 'downloading', state: 'running', percent: 40 },
    })
    expect(result.waitingCount.value).toBe(0)
  })

  it('активная задача + 2 ожидающих: диалог называет обе величины (критерий приёмки issue #83)', async () => {
    const port = createFakePort()
    const store = useDownloadTaskStore()
    await store.initialize()
    emitQueueChanged({
      awaitingContinue: false,
      tasks: [
        { taskId: 'a', title: 'Ролик A', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'downloading', state: 'running', percent: 40 },
        { taskId: 'b', title: 'Ролик B', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'queued' },
        { taskId: 'c', title: 'Ролик C', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'queued' },
      ],
    })

    const { result } = withSetup(() => useExitConfirmation(port))
    port.attempt()

    expect(result.visible.value).toBe(true)
    expect(result.activeTask.value?.displayTitle).toBe('«Ролик A» — Только аудио')
    expect(result.waitingCount.value).toBe(2)
  })
})

describe('useExitConfirmation — состояние «только ожидающие задачи»', () => {
  it('очередь работает (не приостановлена после перезапуска), но активная фаза отсутствует из-за паузы на обновление yt-dlp (Р-7): диалог показывается без названной задачи', async () => {
    const port = createFakePort()
    const store = useDownloadTaskStore()
    await store.initialize()
    emitQueueChanged({
      awaitingContinue: false,
      pauseReason: 'ytDlpUpdate',
      tasks: [{ taskId: 'w', title: 'Ожидающий ролик', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'queued' }],
    })

    const { result } = withSetup(() => useExitConfirmation(port))
    port.attempt()

    expect(result.visible.value).toBe(true)
    expect(port.finishCalls).toBe(0)
    expect(result.activeTask.value).toBeUndefined()
    expect(result.pauseReason.value).toBe('ytDlpUpdate')
    expect(result.waitingCount.value).toBe(1)
  })
})

describe('useExitConfirmation — состояние «приостановлена после перезапуска, ещё не продолжена» (Р-8)', () => {
  it('awaitingContinue: true и есть нетерминальные задачи — диалог НЕ показывается, окно завершается сразу', async () => {
    const port = createFakePort()
    const store = useDownloadTaskStore()
    await store.initialize()
    emitQueueChanged({
      awaitingContinue: true,
      tasks: [
        { taskId: 'r1', title: 'Восстановленный ролик 1', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'queued' },
        { taskId: 'r2', title: 'Восстановленный ролик 2', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'queued' },
      ],
    })

    const { result } = withSetup(() => useExitConfirmation(port))
    port.attempt()

    expect(result.visible.value).toBe(false)
    expect(port.finishCalls).toBe(1)
  })

  it('после «Продолжить очередь» (awaitingContinue становится false) диалог снова работает как обычно', async () => {
    const port = createFakePort()
    const store = useDownloadTaskStore()
    await store.initialize()
    emitQueueChanged({
      awaitingContinue: true,
      tasks: [{ taskId: 'r1', title: 'Восстановленный ролик', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'queued' }],
    })

    const { result } = withSetup(() => useExitConfirmation(port))

    // Снимок после `resume_queue`: `awaitingContinue: false`, задача теперь реально идёт.
    emitQueueChanged({
      awaitingContinue: false,
      tasks: [{ taskId: 'r1', title: 'Восстановленный ролик', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'downloading', state: 'running', percent: 5 }],
    })
    port.attempt()

    expect(result.visible.value).toBe(true)
    expect(port.finishCalls).toBe(0)
  })
})

describe('useExitConfirmation — «Остаться» против «Всё равно выйти»', () => {
  async function setupActiveTask(port: ReturnType<typeof createFakePort>) {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'queue_state') return Promise.resolve(EMPTY_SNAPSHOT)
      if (command === 'start_download') return Promise.resolve(started)
      throw new Error(`unexpected invoke: ${command}`)
    })
    const store = useDownloadTaskStore()
    await store.start(request)
    emitProgress({ taskId: 'task-1', phase: 'downloading', state: 'running', percent: 40 })
    const setup = withSetup(() => useExitConfirmation(port))
    port.attempt()
    return { store, ...setup }
  }

  it('«Остаться» — Esc-эквивалент: просто закрывает диалог, окно не завершается', async () => {
    const port = createFakePort()
    const { result } = await setupActiveTask(port)
    expect(result.visible.value).toBe(true)

    result.stay()

    expect(result.visible.value).toBe(false)
    expect(port.finishCalls).toBe(0)
  })

  it('«Всё равно выйти» завершает окно и НЕ запускает путь отмены (Cancelled) — cancel_download не вызывается', async () => {
    const port = createFakePort()
    const { result } = await setupActiveTask(port)
    expect(result.visible.value).toBe(true)

    result.exitAnyway()

    expect(result.visible.value).toBe(false)
    expect(port.finishCalls).toBe(1)
    expect(invokeMock).not.toHaveBeenCalledWith('cancel_download', expect.anything())
    // Задача в сторе остаётся как есть — «Всё равно выйти» не трогает её состояние.
    expect(result.activeTask.value?.progress.phase).toBe('downloading')
  })
})

describe('useExitConfirmation — подписка и отписка от оконного события', () => {
  it('подписывается один раз на монтаже и отписывается при размонтировании', () => {
    const port = createFakePort()
    const { unmount } = withSetup(() => useExitConfirmation(port))
    expect(port.onCloseAttempt).toHaveBeenCalledTimes(1)
    const unsubscribe = port.onCloseAttempt.mock.results[0]?.value as (() => void) | undefined
    expect(unsubscribe).toBeDefined()

    unmount()

    expect(unsubscribe).toHaveBeenCalledTimes(1)
  })
})
