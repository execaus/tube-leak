import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { DownloadProgressEvent, DownloadStarted, StartDownloadRequest } from '@/types/download'

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

const { useDownloadTaskStore } = await import('./downloadTask')

const request: StartDownloadRequest = {
  url: 'https://youtu.be/x',
  title: 'Как приручить дракона',
  streams: { videoFormatId: 'v1080', audioFormatId: 'a' },
}

const started: DownloadStarted = {
  taskId: 'task-1',
  phase: 'queued',
  plan: 'videoAndAudio',
}

function emit(payload: DownloadProgressEvent): void {
  capturedHandler?.({ payload })
}

beforeEach(() => {
  vi.useFakeTimers()
  invokeMock.mockReset()
  listenMock.mockClear()
  unlistenMock.mockClear()
  capturedHandler = undefined
  setActivePinia(createPinia())
})

afterEach(() => {
  vi.useRealTimers()
})

describe('useDownloadTaskStore — start', () => {
  it('invokes start_download and captures the task with the display title snapshot', async () => {
    invokeMock.mockResolvedValueOnce(started)
    const store = useDownloadTaskStore()

    await store.start(request, '«Как приручить дракона» — 1080p')

    expect(invokeMock).toHaveBeenCalledWith('start_download', { request })
    expect(store.task).toStrictEqual({
      taskId: 'task-1',
      plan: 'videoAndAudio',
      displayTitle: '«Как приручить дракона» — 1080p',
    })
    expect(store.progress).toStrictEqual({ phase: 'queued' })
    expect(store.isActive).toBe(true)
  })

  it('subscribes to download://progress before invoking the command', async () => {
    invokeMock.mockResolvedValueOnce(started)
    const store = useDownloadTaskStore()

    await store.start(request, 'title')

    expect(listenMock).toHaveBeenCalledWith('download://progress', expect.any(Function))
  })

  it('does not throw and logs when the core rejects the start (e.g. alreadyActive)', async () => {
    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    invokeMock.mockRejectedValueOnce({ kind: 'alreadyActive', message: 'slot busy' })
    const store = useDownloadTaskStore()

    await expect(store.start(request, 'title')).resolves.toBeUndefined()

    expect(store.task).toBeUndefined()
    expect(consoleErrorSpy).toHaveBeenCalled()
    consoleErrorSpy.mockRestore()
  })
})

describe('useDownloadTaskStore — события прогресса, ключуемые taskId', () => {
  it('updates progress only for events matching the current taskId', async () => {
    invokeMock.mockResolvedValueOnce(started)
    const store = useDownloadTaskStore()
    await store.start(request, 'title')

    emit({ taskId: 'other-task', phase: 'downloading', state: 'running', percent: 5 })
    expect(store.progress).toStrictEqual({ phase: 'queued' })

    emit({ taskId: 'task-1', phase: 'downloading', state: 'running', percent: 5 })
    expect(store.progress).toStrictEqual({ phase: 'downloading', state: 'running', percent: 5 })
  })

  it('keeps receiving events after a failed phase — retry resumes the same stream (TL-45 п.3)', async () => {
    invokeMock.mockResolvedValueOnce(started)
    const store = useDownloadTaskStore()
    await store.start(request, 'title')

    emit({
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
    await store.retry()
    expect(invokeMock).toHaveBeenCalledWith('retry_download', { taskId: 'task-1' })

    // Поток не отписан и не отфильтрован по факту отказа.
    emit({ taskId: 'task-1', phase: 'downloading', state: 'running', percent: 10 })
    expect(store.progress).toStrictEqual({ phase: 'downloading', state: 'running', percent: 10 })
  })
})

describe('useDownloadTaskStore — isActive и слот', () => {
  it('is not active once the task reaches a terminal phase', async () => {
    invokeMock.mockResolvedValueOnce(started)
    const store = useDownloadTaskStore()
    await store.start(request, 'title')
    expect(store.isActive).toBe(true)

    emit({ taskId: 'task-1', phase: 'done', fileName: 'video.mp4' })
    expect(store.isActive).toBe(false)
  })

  it('is not active when no task exists', () => {
    const store = useDownloadTaskStore()
    expect(store.isActive).toBe(false)
  })
})

describe('useDownloadTaskStore — cancel/hide', () => {
  it('invokes cancel_download with the current taskId', async () => {
    invokeMock.mockResolvedValueOnce(started)
    const store = useDownloadTaskStore()
    await store.start(request, 'title')

    invokeMock.mockResolvedValueOnce(undefined)
    await store.cancel()

    expect(invokeMock).toHaveBeenCalledWith('cancel_download', { taskId: 'task-1' })
  })

  it('hide clears the task only in a terminal phase', async () => {
    invokeMock.mockResolvedValueOnce(started)
    const store = useDownloadTaskStore()
    await store.start(request, 'title')

    store.hide()
    expect(store.task).not.toBeUndefined()

    emit({ taskId: 'task-1', phase: 'cancelled', partialData: 'removed' })
    store.hide()
    expect(store.task).toBeUndefined()
    expect(store.progress).toBeUndefined()
  })
})

describe('useDownloadTaskStore — мягкий индикатор зависания (5с, косметика фронтенда)', () => {
  it('sets softStallSeconds after 5s without a new running event, and clears it on the next event', async () => {
    invokeMock.mockResolvedValueOnce(started)
    const store = useDownloadTaskStore()
    await store.start(request, 'title')

    emit({ taskId: 'task-1', phase: 'downloading', state: 'running', percent: 40, speedBytesPerSec: 1024 })
    expect(store.softStallSeconds).toBeUndefined()

    await vi.advanceTimersByTimeAsync(5_000)
    expect(store.softStallSeconds).toBeGreaterThanOrEqual(5)

    emit({ taskId: 'task-1', phase: 'downloading', state: 'running', percent: 41, speedBytesPerSec: 1024 })
    expect(store.softStallSeconds).toBeUndefined()
  })

  it('does not show the stall indicator during waitingRetry (its own honest state, not a stalled Downloading)', async () => {
    invokeMock.mockResolvedValueOnce(started)
    const store = useDownloadTaskStore()
    await store.start(request, 'title')

    emit({ taskId: 'task-1', phase: 'downloading', state: 'running', percent: 40 })
    emit({
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
