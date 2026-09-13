import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { DownloadProgressEvent, DownloadStarted } from '@/types/generated/download'
import type { ProbeResult } from '@/types/generated/probe'
import type { QueueSnapshot } from '@/types/generated/queue'
import type { SidecarCheckReport } from '@/types/generated/sidecar'
import type { YtDlpPrepared } from '@/types/generated/ytdlp'

/**
 * Интеграционные тесты App.vue ↔ секция «Очередь загрузок» (эпик E3 →
 * E4, TL-45 → TL-75): собственный файл, а не довесок к `App.test.ts` —
 * там общий `capturedHandler` типизирован под `ytdlp://prepare` и его
 * нельзя переиспользовать для `download://progress`/`queue://changed`
 * без конфликта типов; здесь `listen` замокан маршрутизацией по имени
 * события.
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

const EMPTY_QUEUE_SNAPSHOT: QueueSnapshot = { tasks: [], awaitingContinue: false }

function emitProgress(payload: DownloadProgressEvent): void {
  handlers.get('download://progress')?.({ payload })
}

function emitQueueChanged(snapshot: QueueSnapshot): void {
  handlers.get('queue://changed')?.({ payload: snapshot })
}

function routeInvoke(handlersByCommand: Record<string, () => Promise<unknown>>) {
  const withDefaults: Record<string, () => Promise<unknown>> = {
    // Снимок очереди (эпик E4, TL-75): `App.vue` запрашивает его в
    // собственном `onMounted`, независимо от готовности yt-dlp/ffmpeg —
    // дефолт «пусто», явно переданный обработчик той же команды имеет
    // приоритет (тот же приём, что `ytdlp_update_state` в `App.test.ts`).
    queue_state: () => Promise.resolve(EMPTY_QUEUE_SNAPSHOT),
    // Первая страница истории (эпик E5, TL-93): `HistoryScreen` рендерится
    // безусловно под вкладкой «История» (`v-show`, К-14) и запрашивает её в
    // своём `onMounted`, который срабатывает сразу при монтаже `App.vue` —
    // этот файл не про историю, поэтому дефолт «пусто».
    history_page: () => Promise.resolve({ entries: [], notices: [] }),
    ...handlersByCommand,
  }
  invokeMock.mockImplementation((command: string) => {
    const handler = withDefaults[command]
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

describe('App — секция «Очередь загрузок» (эпик E3 → E4, TL-45 → TL-75)', () => {
  it('renders no queue section at all before any task exists', async () => {
    const wrapper = await mountReady()
    expect(wrapper.text()).not.toContain('Очередь загрузок')
  })

  it('starts a task on "Скачать", showing the panel with a title built from title+quality, and passes quality in the request (TL-75, эпик E4)', async () => {
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
          quality: { kind: 'audioOnly', heightPx: undefined },
        },
      })
      return Promise.resolve(started)
    })

    await wrapper.findAll('button').find((b) => b.text() === 'Скачать')?.trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('Очередь загрузок')
    expect(wrapper.text()).toContain('«Ролик A» — Только аудио')
  })

  it('starting a second task while the first is active puts it in the queue as waiting, not as a rejected duplicate (Ф-2 E4 — занятый слот больше не отказ)', async () => {
    const wrapper = await mountReady()
    await probeAndSelect(wrapper, 'https://youtu.be/a', resultA)

    invokeMock.mockImplementationOnce(() => Promise.resolve(started))
    await wrapper.findAll('button').find((b) => b.text() === 'Скачать')?.trigger('click')
    await flushPromises()
    expect(wrapper.text()).toContain('«Ролик A» — Только аудио')

    // Новая ссылка поверх идущей загрузки — С-13: разбор работает как
    // обычно, первая задача не трогается. Кнопка «Скачать» больше не
    // заблокирована (TL-74) — постановка второй задачи штатна (Ф-2 E4).
    await probeAndSelect(wrapper, 'https://youtu.be/b', resultB)
    const downloadButton = wrapper.findAll('button').find((b) => b.text() === 'Скачать')
    expect(downloadButton?.attributes('disabled')).toBeUndefined()
    expect(wrapper.text()).not.toContain('Уже идёт другая загрузка')

    invokeMock.mockImplementationOnce(() => Promise.resolve({ taskId: 'task-2', phase: 'queued', plan: 'singleStream' }))
    await downloadButton?.trigger('click')
    await flushPromises()

    // Обе задачи видны: первая — активной панелью, вторая — честно
    // ожидающей строкой (дизайн E4, «Пять состояний»).
    expect(wrapper.text()).toContain('«Ролик A» — Только аудио')
    expect(wrapper.text()).toContain('«Ролик B» — Только аудио')
    expect(wrapper.text()).toContain('В очереди — начнётся после текущей загрузки')
  })

  it('cancel → hide: after the core confirms via queue://changed, the section disappears', async () => {
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

    invokeMock.mockImplementationOnce((command: string, args) => {
      expect(command).toBe('dismiss_queue_task')
      expect(args).toStrictEqual({ taskId: 'task-1' })
      return Promise.resolve(undefined)
    })
    await wrapper.findAll('button').find((b) => b.text() === 'Скрыть')?.trigger('click')
    await flushPromises()

    // «Скрыть» — команда ядра (дизайн E4, «Данные для API», п.5): список
    // ещё не меняется локально до подтверждения снимком.
    expect(wrapper.text()).toContain('Очередь загрузок')

    emitQueueChanged({ tasks: [], awaitingContinue: false })
    await wrapper.vm.$nextTick()

    expect(wrapper.text()).not.toContain('Очередь загрузок')
  })

  it('resumes progress events after a failed phase via retry — the stream is not filtered/unsubscribed (требование п.3, не изменилось в E4)', async () => {
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

describe('App — три задачи подряд, критерий приёмки issue #82 (К-1)', () => {
  it('three links probed and started one after another are all visible: active shows progress, waiting show their place', async () => {
    const wrapper = await mountReady()

    await probeAndSelect(wrapper, 'https://youtu.be/a', resultA)
    invokeMock.mockImplementationOnce(() => Promise.resolve({ taskId: 'task-1', phase: 'downloading', plan: 'singleStream' }))
    await wrapper.findAll('button').find((b) => b.text() === 'Скачать')?.trigger('click')
    await flushPromises()
    emitProgress({ taskId: 'task-1', phase: 'downloading', state: 'running', percent: 30 })
    await wrapper.vm.$nextTick()

    await probeAndSelect(wrapper, 'https://youtu.be/b', resultB)
    invokeMock.mockImplementationOnce(() => Promise.resolve({ taskId: 'task-2', phase: 'queued', plan: 'singleStream' }))
    await wrapper.findAll('button').find((b) => b.text() === 'Скачать')?.trigger('click')
    await flushPromises()

    const resultC: ProbeResult = {
      title: 'Ролик C',
      durationSecs: 40,
      qualities: [{ kind: 'audioOnly', size: { kind: 'unknown' }, streams: { audioFormatId: 'c' } }],
    }
    await probeAndSelect(wrapper, 'https://youtu.be/c', resultC)
    invokeMock.mockImplementationOnce(() => Promise.resolve({ taskId: 'task-3', phase: 'queued', plan: 'singleStream' }))
    await wrapper.findAll('button').find((b) => b.text() === 'Скачать')?.trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('«Ролик A» — Только аудио')
    expect(wrapper.text()).toContain('30 %')
    expect(wrapper.text()).toContain('«Ролик B» — Только аудио')
    expect(wrapper.text()).toContain('В очереди — начнётся после текущей загрузки')
    expect(wrapper.text()).toContain('«Ролик C» — Только аудио')
    expect(wrapper.text()).toContain('и ещё 1 задачи')
  })
})

describe('App — отказ по дублю (Ф-8/Р-5, К-9)', () => {
  it('rejects a repeat submission of the same video+quality with a typed error naming the existing task, and does not add a second task', async () => {
    const wrapper = await mountReady()
    await probeAndSelect(wrapper, 'https://youtu.be/a', resultA)

    invokeMock.mockImplementationOnce(() => Promise.resolve(started))
    await wrapper.findAll('button').find((b) => b.text() === 'Скачать')?.trigger('click')
    await flushPromises()

    // Тот же ролик, то же качество — повторный клик на всё ещё активной карточке.
    invokeMock.mockRejectedValueOnce({
      kind: 'duplicateTask',
      message: 'CORE-DIAGNOSTIC-NOT-SCREEN-TEXT',
      existing: { taskId: 'task-1', title: 'Ролик A', quality: { kind: 'audioOnly' } },
    })
    await wrapper.findAll('button').find((b) => b.text() === 'Скачать')?.trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('Такая задача уже в очереди')
    expect(wrapper.text()).toContain('«Ролик A» — Только аудио уже стоит в очереди')
    expect(wrapper.text()).not.toContain('CORE-DIAGNOSTIC-NOT-SCREEN-TEXT')
    // Вторая задача в списке не появилась.
    expect(wrapper.findAll('.download-panel, .queue-waiting-row')).toHaveLength(1)
  })
})

describe('App — продолжение очереди после перезапуска (Р-3)', () => {
  it('awaitingContinue shows the banner and blocks network calls until "Продолжить очередь" is clicked (мок команды resume_queue)', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(okReport),
      queue_state: () =>
        Promise.resolve({
          awaitingContinue: true,
          tasks: [
            { taskId: 'r1', title: 'Восстановленный ролик', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'queued' },
          ],
        } satisfies QueueSnapshot),
    })
    const wrapper = await mountReady()

    expect(wrapper.text()).toContain('Очередь приостановлена после перезапуска')
    expect(wrapper.text()).toContain('начнётся первой, как только вы продолжите')
    expect(invokeMock).not.toHaveBeenCalledWith('start_download', expect.anything())

    invokeMock.mockImplementationOnce((command: string) => {
      expect(command).toBe('resume_queue')
      return Promise.resolve(undefined)
    })
    await wrapper.findAll('button').find((b) => b.text() === 'Продолжить очередь')?.trigger('click')
    await flushPromises()

    expect(invokeMock).toHaveBeenCalledWith('resume_queue')
  })
})

describe('App — пауза на обновление yt-dlp между задачами (Р-7)', () => {
  it('shows the honest pause line when the queue snapshot reports pauseReason: ytDlpUpdate', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(okReport),
      queue_state: () =>
        Promise.resolve({
          awaitingContinue: false,
          pauseReason: 'ytDlpUpdate',
          tasks: [
            { taskId: 'w', title: 'Ожидающий ролик', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'queued' },
          ],
        } satisfies QueueSnapshot),
    })
    const wrapper = await mountReady()

    expect(wrapper.text()).toContain('Между загрузками устанавливается обновлённый yt-dlp')
  })
})

describe('App — отказ команды виден на экране, не только в консоли (ревью TL-45)', () => {
  it('shows the typed command-error block (with the title from the table, no Retry button, no diagnostic message) when start_download rejects, and never renders a task panel', async () => {
    const wrapper = await mountReady()
    await probeAndSelect(wrapper, 'https://youtu.be/a', resultA)

    invokeMock.mockRejectedValueOnce({ kind: 'invalidUrl', message: 'CORE-DIAGNOSTIC-NOT-SCREEN-TEXT' })
    await wrapper.findAll('button').find((b) => b.text() === 'Скачать')?.trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('Очередь загрузок')
    expect(wrapper.text()).toContain('Ссылка не распознана')
    expect(wrapper.text()).not.toContain('CORE-DIAGNOSTIC-NOT-SCREEN-TEXT')
    expect(wrapper.findAll('button').some((b) => b.text() === 'Повторить')).toBe(false)
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

    expect(wrapper.text()).not.toContain('Очередь загрузок')
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

    expect(wrapper.text()).toContain('Очередь загрузок')
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
    expect(wrapper.text()).not.toContain('Очередь ещё не завершена')
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

    expect(wrapper.text()).toContain('Очередь ещё не завершена')
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
    expect(wrapper.text()).toContain('Очередь ещё не завершена')

    await wrapper.findAll('button').find((b) => b.text() === 'Остаться')?.trigger('click')
    await wrapper.vm.$nextTick()

    expect(wrapper.text()).not.toContain('Очередь ещё не завершена')
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
    expect(wrapper.text()).not.toContain('Очередь ещё не завершена')
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
    expect(wrapper.text()).toContain('Очередь загрузок')

    await attemptWindowClose()
    await flushPromises()

    expect(destroyMock).toHaveBeenCalledTimes(1)
    expect(wrapper.text()).not.toContain('Очередь ещё не завершена')
  })

  it('active task + 2 waiting: the dialog names both quantities and never claims "вставьте ссылку заново" (critерий приёмки issue #83)', async () => {
    const wrapper = await mountReady()
    await probeAndSelect(wrapper, 'https://youtu.be/a', resultA)
    invokeMock.mockImplementationOnce(() => Promise.resolve({ taskId: 'task-1', phase: 'downloading', plan: 'singleStream' }))
    await wrapper.findAll('button').find((b) => b.text() === 'Скачать')?.trigger('click')
    await flushPromises()
    emitProgress({ taskId: 'task-1', phase: 'downloading', state: 'running', percent: 62 })
    await wrapper.vm.$nextTick()

    await probeAndSelect(wrapper, 'https://youtu.be/b', resultB)
    invokeMock.mockImplementationOnce(() => Promise.resolve({ taskId: 'task-2', phase: 'queued', plan: 'singleStream' }))
    await wrapper.findAll('button').find((b) => b.text() === 'Скачать')?.trigger('click')
    await flushPromises()

    const resultC: ProbeResult = {
      title: 'Ролик C',
      durationSecs: 40,
      qualities: [{ kind: 'audioOnly', size: { kind: 'unknown' }, streams: { audioFormatId: 'c' } }],
    }
    await probeAndSelect(wrapper, 'https://youtu.be/c', resultC)
    invokeMock.mockImplementationOnce(() => Promise.resolve({ taskId: 'task-3', phase: 'queued', plan: 'singleStream' }))
    await wrapper.findAll('button').find((b) => b.text() === 'Скачать')?.trigger('click')
    await flushPromises()

    await attemptWindowClose()
    await wrapper.vm.$nextTick()

    expect(wrapper.text()).toContain('Очередь ещё не завершена')
    expect(wrapper.text()).toContain('скачивается (62 %)')
    expect(wrapper.text()).toContain('Ещё в очереди: 2 задачи')
    expect(wrapper.text()).not.toMatch(/вставьте/i)
    expect(destroyMock).not.toHaveBeenCalled()
  })

  it('restored queue after a restart, not yet resumed (Р-8): a close attempt destroys the window immediately, no dialog', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(okReport),
      queue_state: () =>
        Promise.resolve({
          awaitingContinue: true,
          tasks: [
            { taskId: 'r1', title: 'Восстановленный ролик', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'queued' },
          ],
        } satisfies QueueSnapshot),
    })
    const wrapper = await mountReady()
    expect(wrapper.text()).toContain('Очередь приостановлена после перезапуска')

    await attemptWindowClose()
    await flushPromises()

    expect(destroyMock).toHaveBeenCalledTimes(1)
    expect(wrapper.text()).not.toContain('Очередь ещё не завершена')
  })
})
