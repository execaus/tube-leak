import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { DownloadProgressEvent, DownloadStarted } from '@/types/generated/download'
import type { ProbeResult } from '@/types/generated/probe'
import type { SidecarCheckReport } from '@/types/generated/sidecar'
import type { YtDlpPrepared } from '@/types/generated/ytdlp'

/**
 * Интеграционные тесты App.vue ↔ секция «Текущая загрузка» (эпик E3,
 * TL-45): собственный файл, а не довесок к `App.test.ts` — там общий
 * `capturedHandler` типизирован под `ytdlp://prepare` и его нельзя
 * переиспользовать для `download://progress` без конфликта типов; здесь
 * `listen` замокан маршрутизацией по имени события.
 *
 * Здесь же — диалог подтверждения выхода (Р-2, TL-46) с настоящей оконной
 * привязкой (TL-47/#49): `@tauri-apps/api/window` замокан прямо в этом
 * файле (`getCurrentWindowMock`/`destroyMock`), а не подставным портом —
 * это единственное место, проверяющее, что `windowExitPort.ts` действительно
 * держит подписку на `onCloseRequested` и действительно завершает окно
 * через `destroy()`, а не только то, что композабл верно решает, когда
 * показывать диалог (это уже проверено против фейкового порта в
 * `useExitConfirmation.test.ts`).
 */

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

type CloseAttemptHandler = (event: { preventDefault: () => void }) => void | Promise<void>
let capturedCloseHandler: CloseAttemptHandler | undefined
const closeUnlistenMock = vi.fn()
const onCloseRequestedMock = vi.fn((handler: CloseAttemptHandler) => {
  capturedCloseHandler = handler
  return Promise.resolve(closeUnlistenMock)
})
const destroyMock = vi.fn(() => Promise.resolve())
const getCurrentWindowMock = vi.fn(() => ({
  onCloseRequested: onCloseRequestedMock,
  destroy: destroyMock,
}))

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => getCurrentWindowMock(),
}))

const { default: App } = await import('./App.vue')

/** Симулирует попытку пользователя закрыть окно (крестик, Cmd+Q…). */
async function attemptWindowClose(): Promise<void> {
  await capturedCloseHandler?.({ preventDefault: vi.fn() })
}

const preparedWarm: YtDlpPrepared = {
  version: '2026.08.20',
  path: '/opt/tube-leak/ytdlp/yt-dlp',
  prepared: false,
  durationMs: 120,
}

const okReport: SidecarCheckReport = {
  ytDlp: { name: 'yt-dlp', path: '/opt/tube-leak/bin/yt-dlp', status: 'ok', version: '2026.08.20' },
  ffmpeg: { name: 'ffmpeg', path: '/opt/tube-leak/bin/ffmpeg', status: 'ok', version: '7.1' },
}

const resultA: ProbeResult = {
  title: 'Ролик A',
  durationSecs: 65,
  qualities: [{ kind: 'audioOnly', size: { kind: 'unknown' }, streams: { audioFormatId: 'a' } }],
}

const resultB: ProbeResult = {
  title: 'Ролик B',
  durationSecs: 90,
  qualities: [{ kind: 'audioOnly', size: { kind: 'unknown' }, streams: { audioFormatId: 'b' } }],
}

const started: DownloadStarted = { taskId: 'task-1', phase: 'queued', plan: 'singleStream' }

function emitProgress(payload: DownloadProgressEvent): void {
  handlers.get('download://progress')?.({ payload })
}

function routeInvoke(handlersByCommand: Record<string, () => Promise<unknown>>) {
  invokeMock.mockImplementation((command: string) => {
    const handler = handlersByCommand[command]
    if (!handler) throw new Error(`unexpected invoke: ${command}`)
    return handler()
  })
}

beforeEach(() => {
  vi.useFakeTimers()
  invokeMock.mockReset()
  listenMock.mockClear()
  unlistenMock.mockClear()
  handlers.clear()
  capturedCloseHandler = undefined
  onCloseRequestedMock.mockClear()
  closeUnlistenMock.mockClear()
  destroyMock.mockClear()
  getCurrentWindowMock.mockClear()
  setActivePinia(createPinia())
  routeInvoke({
    prepare_ytdlp: () => Promise.resolve(preparedWarm),
    check_sidecar: () => Promise.resolve(okReport),
  })
})

afterEach(() => {
  vi.useRealTimers()
})

async function mountReady() {
  const wrapper = mount(App)
  await flushPromises()
  return wrapper
}

async function probeAndSelect(wrapper: Awaited<ReturnType<typeof mountReady>>, url: string, result: ProbeResult) {
  invokeMock.mockImplementationOnce((command: string) => {
    if (command === 'probe_url') return Promise.resolve(result)
    throw new Error(`unexpected invoke: ${command}`)
  })
  await wrapper.find('input').setValue(url)
  await vi.advanceTimersByTimeAsync(400)
  await vi.waitFor(() => {
    expect(wrapper.text()).toContain(result.title)
  })
  await wrapper.find('input[type="radio"]').setValue(true)
}

describe('App — секция «Текущая загрузка» (эпик E3, TL-45)', () => {
  it('renders no download section at all before any task exists', async () => {
    const wrapper = await mountReady()
    expect(wrapper.text()).not.toContain('Текущая загрузка')
  })

  it('starts a task on "Скачать", showing the panel with a title snapshot built from the card + quality', async () => {
    const wrapper = await mountReady()
    await probeAndSelect(wrapper, 'https://youtu.be/a', resultA)

    invokeMock.mockImplementationOnce((command: string, args) => {
      expect(command).toBe('start_download')
      expect(args).toStrictEqual({
        request: {
          url: 'https://youtu.be/a',
          title: 'Ролик A',
          streams: { audioFormatId: 'a' },
          size: { kind: 'unknown' },
        },
      })
      return Promise.resolve(started)
    })

    await wrapper.findAll('button').find((b) => b.text() === 'Скачать')?.trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('Текущая загрузка')
    expect(wrapper.text()).toContain('«Ролик A» — Только аудио')
  })

  it('survives replacement of the video card — the panel keeps its own title snapshot (С-13, требование п.6)', async () => {
    const wrapper = await mountReady()
    await probeAndSelect(wrapper, 'https://youtu.be/a', resultA)

    invokeMock.mockImplementationOnce(() => Promise.resolve(started))
    await wrapper.findAll('button').find((b) => b.text() === 'Скачать')?.trigger('click')
    await flushPromises()
    expect(wrapper.text()).toContain('«Ролик A» — Только аудио')

    // Новая ссылка приходит поверх идущей загрузки — С-13: разбор работает
    // как обычно, задача не трогается.
    await probeAndSelect(wrapper, 'https://youtu.be/b', resultB)

    expect(wrapper.text()).toContain('Ролик B')
    expect(wrapper.text()).toContain('«Ролик A» — Только аудио')

    // Кнопка «Скачать» под новой карточкой заблокирована с объясняющей
    // подсказкой — слот занят первой задачей (С-13).
    const downloadButton = wrapper.findAll('button').find((b) => b.text() === 'Скачать')
    expect(downloadButton?.attributes('disabled')).toBeDefined()
    expect(wrapper.text()).toContain('Уже идёт другая загрузка')
  })

  it('cancel → hide fully clears the panel and re-enables the download button', async () => {
    const wrapper = await mountReady()
    await probeAndSelect(wrapper, 'https://youtu.be/a', resultA)

    invokeMock.mockImplementationOnce(() => Promise.resolve(started))
    await wrapper.findAll('button').find((b) => b.text() === 'Скачать')?.trigger('click')
    await flushPromises()

    invokeMock.mockImplementationOnce((command: string, args) => {
      expect(command).toBe('cancel_download')
      expect(args).toStrictEqual({ taskId: 'task-1' })
      return Promise.resolve(undefined)
    })
    await wrapper.findAll('button').find((b) => b.text() === 'Отменить')?.trigger('click')
    await flushPromises()

    emitProgress({ taskId: 'task-1', phase: 'cancelled', partialData: 'removed' })
    await wrapper.vm.$nextTick()
    expect(wrapper.text()).toContain('удалены')

    await wrapper.findAll('button').find((b) => b.text() === 'Скрыть')?.trigger('click')
    await wrapper.vm.$nextTick()

    expect(wrapper.text()).not.toContain('Текущая загрузка')
    const downloadButton = wrapper.findAll('button').find((b) => b.text() === 'Скачать')
    expect(downloadButton?.attributes('disabled')).toBeUndefined()
  })

  it('resumes progress events after a failed phase via retry — the stream is not filtered/unsubscribed (требование п.3)', async () => {
    const wrapper = await mountReady()
    await probeAndSelect(wrapper, 'https://youtu.be/a', resultA)

    invokeMock.mockImplementationOnce(() => Promise.resolve(started))
    await wrapper.findAll('button').find((b) => b.text() === 'Скачать')?.trigger('click')
    await flushPromises()

    emitProgress({
      taskId: 'task-1',
      phase: 'failed',
      error: { kind: 'connectionLost', message: 'diag', retryable: true, partialData: 'kept' },
    })
    await wrapper.vm.$nextTick()
    expect(wrapper.text()).toContain('Соединение потеряно')

    invokeMock.mockImplementationOnce((command: string, args) => {
      expect(command).toBe('retry_download')
      expect(args).toStrictEqual({ taskId: 'task-1' })
      return Promise.resolve(undefined)
    })
    await wrapper.findAll('button').find((b) => b.text() === 'Повторить')?.trigger('click')
    await flushPromises()

    emitProgress({ taskId: 'task-1', phase: 'downloading', state: 'running', percent: 15 })
    await wrapper.vm.$nextTick()

    expect(wrapper.text()).toContain('15 %')
  })
})

describe('App — отказ команды виден на экране, не только в консоли (ревью TL-45)', () => {
  it('shows the typed command-error block (with the title from the table, no Retry button, no diagnostic message) when start_download rejects, and never renders a task panel', async () => {
    const wrapper = await mountReady()
    await probeAndSelect(wrapper, 'https://youtu.be/a', resultA)

    invokeMock.mockRejectedValueOnce({ kind: 'alreadyActive', message: 'CORE-DIAGNOSTIC-NOT-SCREEN-TEXT' })
    await wrapper.findAll('button').find((b) => b.text() === 'Скачать')?.trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('Текущая загрузка')
    expect(wrapper.text()).toContain('Уже идёт другая загрузка')
    expect(wrapper.text()).not.toContain('CORE-DIAGNOSTIC-NOT-SCREEN-TEXT')
    expect(wrapper.findAll('button').some((b) => b.text() === 'Повторить')).toBe(false)
    // Слот не занят — задача не была создана.
    const downloadButton = wrapper.findAll('button').find((b) => b.text() === 'Скачать')
    expect(downloadButton?.attributes('disabled')).toBeUndefined()
  })

  it('clears the command-error banner on "Скрыть", and it does not reappear on its own', async () => {
    const wrapper = await mountReady()
    await probeAndSelect(wrapper, 'https://youtu.be/a', resultA)

    invokeMock.mockRejectedValueOnce({ kind: 'invalidUrl', message: 'diag' })
    await wrapper.findAll('button').find((b) => b.text() === 'Скачать')?.trigger('click')
    await flushPromises()
    expect(wrapper.text()).toContain('Ссылка не распознана')

    await wrapper.findAll('button').find((b) => b.text() === 'Скрыть')?.trigger('click')
    await wrapper.vm.$nextTick()

    expect(wrapper.text()).not.toContain('Текущая загрузка')
    expect(wrapper.text()).not.toContain('Ссылка не распознана')
  })

  it('trims a pasted url with a trailing newline before starting — the achievable silent-failure path from the review is now closed end-to-end', async () => {
    const wrapper = await mountReady()
    invokeMock.mockResolvedValueOnce(resultA)

    await wrapper.find('input').setValue('https://youtu.be/a\n')
    await vi.advanceTimersByTimeAsync(400)
    await vi.waitFor(() => {
      expect(wrapper.text()).toContain('Ролик A')
    })
    await wrapper.find('input[type="radio"]').setValue(true)

    invokeMock.mockImplementationOnce((command: string, args) => {
      expect(command).toBe('start_download')
      expect(args).toMatchObject({ request: { url: 'https://youtu.be/a' } })
      return Promise.resolve(started)
    })
    await wrapper.findAll('button').find((b) => b.text() === 'Скачать')?.trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('Текущая загрузка')
    expect(wrapper.text()).not.toContain('Ссылка не распознана')
  })
})

describe('App — диалог подтверждения выхода, настоящая оконная привязка (Р-2, эпик E3, TL-46/TL-47)', () => {
  it('subscribes to onCloseRequested on mount', async () => {
    await mountReady()
    expect(onCloseRequestedMock).toHaveBeenCalledTimes(1)
  })

  it('no task at all: a close attempt prevents the default and destroys the window immediately, no dialog', async () => {
    const wrapper = await mountReady()

    await attemptWindowClose()
    await flushPromises()

    expect(destroyMock).toHaveBeenCalledTimes(1)
    expect(wrapper.text()).not.toContain('Загрузка ещё не завершена')
  })

  it('active task (Downloading): a close attempt shows the dialog with the panel\'s percent and does not destroy the window yet', async () => {
    const wrapper = await mountReady()
    await probeAndSelect(wrapper, 'https://youtu.be/a', resultA)
    invokeMock.mockImplementationOnce(() => Promise.resolve(started))
    await wrapper.findAll('button').find((b) => b.text() === 'Скачать')?.trigger('click')
    await flushPromises()
    emitProgress({ taskId: 'task-1', phase: 'downloading', state: 'running', percent: 62 })
    await wrapper.vm.$nextTick()

    await attemptWindowClose()
    await wrapper.vm.$nextTick()

    expect(wrapper.text()).toContain('Загрузка ещё не завершена')
    expect(wrapper.text()).toContain('скачивается (62 %)')
    expect(destroyMock).not.toHaveBeenCalled()
  })

  it('"Остаться" dismisses the dialog without destroying the window and without touching the task', async () => {
    const wrapper = await mountReady()
    await probeAndSelect(wrapper, 'https://youtu.be/a', resultA)
    invokeMock.mockImplementationOnce(() => Promise.resolve(started))
    await wrapper.findAll('button').find((b) => b.text() === 'Скачать')?.trigger('click')
    await flushPromises()
    emitProgress({ taskId: 'task-1', phase: 'downloading', state: 'running', percent: 10 })
    await wrapper.vm.$nextTick()

    await attemptWindowClose()
    await wrapper.vm.$nextTick()
    expect(wrapper.text()).toContain('Загрузка ещё не завершена')

    await wrapper.findAll('button').find((b) => b.text() === 'Остаться')?.trigger('click')
    await wrapper.vm.$nextTick()

    expect(wrapper.text()).not.toContain('Загрузка ещё не завершена')
    expect(destroyMock).not.toHaveBeenCalled()
    // Задача не тронута: панель по-прежнему показывает идущую загрузку.
    expect(wrapper.text()).toContain('10 %')
  })

  it('"Всё равно выйти" destroys the window and does not invoke cancel_download (не путь отмены)', async () => {
    const wrapper = await mountReady()
    await probeAndSelect(wrapper, 'https://youtu.be/a', resultA)
    invokeMock.mockImplementationOnce(() => Promise.resolve(started))
    await wrapper.findAll('button').find((b) => b.text() === 'Скачать')?.trigger('click')
    await flushPromises()
    emitProgress({ taskId: 'task-1', phase: 'downloading', state: 'running', percent: 10 })
    await wrapper.vm.$nextTick()

    await attemptWindowClose()
    await wrapper.vm.$nextTick()

    await wrapper.findAll('button').find((b) => b.text() === 'Всё равно выйти')?.trigger('click')
    await flushPromises()

    expect(destroyMock).toHaveBeenCalledTimes(1)
    expect(invokeMock).not.toHaveBeenCalledWith('cancel_download', expect.anything())
    expect(wrapper.text()).not.toContain('Загрузка ещё не завершена')
  })

  it('terminal task (Cancelled), even not hidden yet: a close attempt destroys the window immediately, no dialog', async () => {
    const wrapper = await mountReady()
    await probeAndSelect(wrapper, 'https://youtu.be/a', resultA)
    invokeMock.mockImplementationOnce(() => Promise.resolve(started))
    await wrapper.findAll('button').find((b) => b.text() === 'Скачать')?.trigger('click')
    await flushPromises()
    emitProgress({ taskId: 'task-1', phase: 'cancelled', partialData: 'removed' })
    await wrapper.vm.$nextTick()
    // Панель ещё видна («Скрыть» не нажато) — терминальность решает диалог,
    // а не видимость панели.
    expect(wrapper.text()).toContain('Текущая загрузка')

    await attemptWindowClose()
    await flushPromises()

    expect(destroyMock).toHaveBeenCalledTimes(1)
    expect(wrapper.text()).not.toContain('Загрузка ещё не завершена')
  })
})
