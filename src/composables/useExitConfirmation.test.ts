import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { defineComponent } from 'vue'

import type { DownloadProgressEvent, DownloadStarted, StartDownloadRequest } from '@/types/generated/download'
/**
 * Композабл диалога подтверждения выхода (Р-2, эпик E3, TL-46). Оконное
 * событие подставляется фейковым портом — реальная реализация ждёт
 * разрешение `core:window:allow-destroy` (задача ядра #49, см.
 * `windowExitPort.ts`); здесь проверяется только бизнес-логика: когда
 * показывать диалог и что делает каждый ответ, независимо от того, как
 * приходит попытка закрытия.
 */

const invokeMock = vi.fn()
const unlistenMock = vi.fn()
type EventHandler = (event: { payload: DownloadProgressEvent }) => void
let capturedHandler: EventHandler | undefined
const listenMock = vi.fn((_event: string, handler: EventHandler) => {
  capturedHandler = handler
  return Promise.resolve(unlistenMock)
})

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: (...args: [string, EventHandler]) => listenMock(...args),
}))

const { useDownloadTaskStore } = await import('@/stores/downloadTask')
const { useExitConfirmation } = await import('./useExitConfirmation')

function emit(payload: DownloadProgressEvent): void {
  capturedHandler?.({ payload })
}

const request: StartDownloadRequest = {
  url: 'https://youtu.be/x',
  title: 'Как приручить дракона',
  streams: { videoFormatId: 'v1080', audioFormatId: 'a' },
  size: { kind: 'known', bytes: 303_038_464 },
  // TL-70/TL-75 (эпик E4): поле обязательно с контракта TL-70 — здесь
  // добавлено чисто механически, чтобы файл компилировался; поведение
  // диалога выхода по срезу очереди — TL-76 (issue #83), не эта задача.
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
  capturedHandler = undefined
  setActivePinia(createPinia())
})

describe('useExitConfirmation — когда показывается диалог (Р-2)', () => {
  it('без задачи вовсе: попытка закрытия сразу завершает окно, диалог не показывается', () => {
    const port = createFakePort()
    const { result } = withSetup(() => useExitConfirmation(port))

    port.attempt()

    expect(result.visible.value).toBe(false)
    expect(port.finishCalls).toBe(1)
  })

  it('для терминальной задачи (Done), даже не скрытой: диалог не показывается', async () => {
    const port = createFakePort()
    invokeMock.mockResolvedValueOnce(started)
    const store = useDownloadTaskStore()
    await store.start(request)
    emit({ taskId: 'task-1', phase: 'done', fileName: 'x.mp4' })

    const { result } = withSetup(() => useExitConfirmation(port))
    port.attempt()

    expect(result.visible.value).toBe(false)
    expect(port.finishCalls).toBe(1)
  })

  it('для терминальной задачи (Cancelled), даже не скрытой: диалог не показывается', async () => {
    const port = createFakePort()
    invokeMock.mockResolvedValueOnce(started)
    const store = useDownloadTaskStore()
    await store.start(request)
    emit({ taskId: 'task-1', phase: 'cancelled', partialData: 'removed' })

    const { result } = withSetup(() => useExitConfirmation(port))
    port.attempt()

    expect(result.visible.value).toBe(false)
    expect(port.finishCalls).toBe(1)
  })

  it('для активной нетерминальной задачи (Downloading): диалог показывается, окно пока не завершается', async () => {
    const port = createFakePort()
    invokeMock.mockResolvedValueOnce(started)
    const store = useDownloadTaskStore()
    await store.start(request)
    emit({ taskId: 'task-1', phase: 'downloading', state: 'running', percent: 40 })

    const { result } = withSetup(() => useExitConfirmation(port))
    port.attempt()

    expect(result.visible.value).toBe(true)
    expect(port.finishCalls).toBe(0)
  })
})

describe('useExitConfirmation — «Остаться» против «Всё равно выйти»', () => {
  async function setupActiveTask(port: ReturnType<typeof createFakePort>) {
    invokeMock.mockResolvedValueOnce(started)
    const store = useDownloadTaskStore()
    await store.start(request)
    emit({ taskId: 'task-1', phase: 'downloading', state: 'running', percent: 40 })
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
    expect(result.progress.value?.phase).toBe('downloading')
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
