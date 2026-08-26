import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { DownloadProgressEvent, DownloadStarted } from '@/types/download'
import type { ProbeResult } from '@/types/probe'
import type { SidecarCheckReport } from '@/types/sidecar'
import type { YtDlpPrepared } from '@/types/ytdlp'

/**
 * Интеграционные тесты App.vue ↔ секция «Текущая загрузка» (эпик E3,
 * TL-45): собственный файл, а не довесок к `App.test.ts` — там общий
 * `capturedHandler` типизирован под `ytdlp://prepare` и его нельзя
 * переиспользовать для `download://progress` без конфликта типов; здесь
 * `listen` замокан маршрутизацией по имени события.
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

const { default: App } = await import('./App.vue')

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
        request: { url: 'https://youtu.be/a', title: 'Ролик A', streams: { audioFormatId: 'a' } },
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
