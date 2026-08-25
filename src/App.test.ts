import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { SidecarCheckReport, SidecarCheckResult } from '@/types/sidecar'
import type { YtDlpPrepareError, YtDlpPrepareEvent, YtDlpPrepared } from '@/types/ytdlp'

const invokeMock = vi.fn()
const unlistenMock = vi.fn()
type EventHandler = (event: { payload: YtDlpPrepareEvent }) => void
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

// Импортируется после мока `invoke`/`listen` (тот же приём, что и в
// useSidecarCheck.test.ts), т.к. App.vue использует composables как есть.
const { default: App } = await import('./App.vue')

const okYtDlp: SidecarCheckResult = {
  name: 'yt-dlp',
  path: '/opt/tube-leak/bin/yt-dlp',
  status: 'ok',
  version: '2026.08.20',
}

const okFfmpeg: SidecarCheckResult = {
  name: 'ffmpeg',
  path: '/opt/tube-leak/bin/ffmpeg',
  status: 'ok',
  version: '7.1',
}

const timeoutFfmpeg: SidecarCheckResult = {
  name: 'ffmpeg',
  path: '/opt/tube-leak/bin/ffmpeg',
  status: 'timeout',
  timeoutMs: 5000,
}

const notFoundYtDlp: SidecarCheckResult = {
  name: 'yt-dlp',
  path: '/opt/tube-leak/bin/yt-dlp',
  status: 'notFound',
  osErrorCode: 'ENOENT',
}

const okReport: SidecarCheckReport = { ytDlp: okYtDlp, ffmpeg: okFfmpeg }

const preparedWarm: YtDlpPrepared = {
  version: '2026.08.20',
  path: '/opt/tube-leak/ytdlp/yt-dlp',
  prepared: false,
  durationMs: 120,
}

const preparedCold: YtDlpPrepared = {
  version: '2026.08.20',
  path: '/opt/tube-leak/ytdlp/yt-dlp',
  prepared: true,
  durationMs: 36_500,
}

const warmupFailedError: YtDlpPrepareError = {
  kind: 'warmupFailed',
  message: 'yt-dlp не ответил за отведённое время прогрева',
}

/** Роутер `invoke` по имени команды — так же ведёт себя настоящий Tauri IPC. */
function routeInvoke(handlers: Record<string, () => Promise<unknown>>) {
  invokeMock.mockImplementation((command: string) => {
    const handler = handlers[command]
    if (!handler) throw new Error(`unexpected invoke: ${command}`)
    return handler()
  })
}

beforeEach(() => {
  invokeMock.mockReset()
  listenMock.mockClear()
  unlistenMock.mockClear()
  capturedHandler = undefined
})

describe('App — order of calls (TL-17, #18)', () => {
  it('never invokes check_sidecar before prepare_ytdlp resolves, even while the prepare screen is up', async () => {
    let resolvePrepare: (value: YtDlpPrepared) => void = () => {}
    routeInvoke({
      prepare_ytdlp: () =>
        new Promise<YtDlpPrepared>((resolve) => {
          resolvePrepare = resolve
        }),
      check_sidecar: () => Promise.resolve(okReport),
    })

    mount(App)
    await flushPromises()

    expect(invokeMock).toHaveBeenCalledWith('prepare_ytdlp')
    expect(invokeMock).not.toHaveBeenCalledWith('check_sidecar')

    capturedHandler?.({ payload: { stage: 'warmingUp', percent: 50 } })
    await flushPromises()
    expect(invokeMock).not.toHaveBeenCalledWith('check_sidecar')

    resolvePrepare(preparedCold)
    await flushPromises()

    expect(invokeMock).toHaveBeenCalledWith('check_sidecar')
  })

  it('subscribes to ytdlp://prepare before invoking prepare_ytdlp', async () => {
    const order: string[] = []
    listenMock.mockImplementationOnce((_event, handler) => {
      order.push('listen')
      capturedHandler = handler
      return Promise.resolve(unlistenMock)
    })
    routeInvoke({
      prepare_ytdlp: () => {
        order.push('prepare_ytdlp')
        return Promise.resolve(preparedWarm)
      },
      check_sidecar: () => {
        order.push('check_sidecar')
        return Promise.resolve(okReport)
      },
    })

    mount(App)
    await flushPromises()

    expect(order).toEqual(['listen', 'prepare_ytdlp', 'check_sidecar'])
  })
})

describe('App — warm start (no prepare events)', () => {
  it('skips the prepare screen entirely and shows the service screen right away', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(okReport),
    })

    const wrapper = mount(App)
    await flushPromises()

    expect(wrapper.text()).toContain('версия 0.1.0')
    expect(wrapper.text()).toContain('2026.08.20')
    expect(wrapper.text()).toContain('7.1')
    expect(wrapper.text()).not.toContain('Распаковываем')
    expect(wrapper.text()).not.toContain('Готовим yt-dlp')
  })
})

describe('App — first-run preparation (unpacking → warmingUp → ready)', () => {
  it('shows a non-empty state before the first event, then progress per stage, then the service screen', async () => {
    let resolvePrepare: (value: YtDlpPrepared) => void = () => {}
    routeInvoke({
      prepare_ytdlp: () =>
        new Promise<YtDlpPrepared>((resolve) => {
          resolvePrepare = resolve
        }),
      check_sidecar: () => Promise.resolve(okReport),
    })

    const wrapper = mount(App)
    await flushPromises()

    // До первого события — не пустое окно и не «зависшая» надпись.
    expect(wrapper.text().trim().length).toBeGreaterThan(0)
    expect(wrapper.text()).toContain('Запускаем…')

    capturedHandler?.({ payload: { stage: 'unpacking', percent: 4, etaSecs: 1 } })
    await wrapper.vm.$nextTick()
    expect(wrapper.text()).toContain('Распаковываем yt-dlp')
    expect(wrapper.text()).toContain('4%')

    capturedHandler?.({ payload: { stage: 'warmingUp', percent: 60, etaSecs: 14 } })
    await wrapper.vm.$nextTick()
    expect(wrapper.text()).toContain('Готовим yt-dlp к первому запуску')
    expect(wrapper.text()).toContain('60%')
    expect(wrapper.text()).toContain('осталось ~14 с')

    resolvePrepare(preparedCold)
    await flushPromises()

    expect(wrapper.text()).not.toContain('Распаковываем')
    expect(wrapper.text()).not.toContain('Готовим yt-dlp')
    expect(wrapper.text()).toContain('версия 0.1.0')
    expect(wrapper.text()).toContain('2026.08.20')
  })
})

describe('App — preparation failure', () => {
  it('shows the typed error explanation instead of an endless loading state, and does not check sidecars', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.reject(warmupFailedError),
      check_sidecar: () => Promise.resolve(okReport),
    })

    const wrapper = mount(App)
    await flushPromises()

    expect(wrapper.text()).toContain('Не удалось подготовить yt-dlp')
    expect(wrapper.text()).toContain('yt-dlp распаковался, но не запускается')
    expect(invokeMock).not.toHaveBeenCalledWith('check_sidecar')
  })

  it('retries the whole sequence (prepare then check) when the retry button is clicked', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.reject(warmupFailedError),
      check_sidecar: () => Promise.resolve(okReport),
    })

    const wrapper = mount(App)
    await flushPromises()

    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(okReport),
    })

    const retryButton = wrapper.findAll('button').find((b) => b.text().includes('Повторить'))
    expect(retryButton).toBeDefined()
    await retryButton?.trigger('click')
    await flushPromises()

    expect(wrapper.text()).not.toContain('Не удалось подготовить yt-dlp')
    expect(wrapper.text()).toContain('версия 0.1.0')
    expect(wrapper.text()).toContain('2026.08.20')
  })
})

describe('App — service screen (unchanged behaviour from TL-8)', () => {
  beforeEach(() => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(okReport),
    })
  })

  it('renders the title immediately, with both rows Checking before check_sidecar resolves (Н-6)', async () => {
    let resolveCheck: (value: SidecarCheckReport) => void = () => {}
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () =>
        new Promise<SidecarCheckReport>((resolve) => {
          resolveCheck = resolve
        }),
    })

    const wrapper = mount(App)
    await flushPromises()

    expect(wrapper.text()).toContain('tube-leak')
    expect(wrapper.text()).toContain('yt-dlp')
    expect(wrapper.text()).toContain('ffmpeg')
    expect(wrapper.text().match(/Проверяем…/g)).toHaveLength(2)
    expect(wrapper.find('button').exists()).toBe(false)

    resolveCheck(okReport)
    await flushPromises()
  })

  it('hides the retry button when both rows resolve Ok', async () => {
    const wrapper = mount(App)
    await flushPromises()

    expect(wrapper.text()).toContain('2026.08.20')
    expect(wrapper.text()).toContain('7.1')
    expect(wrapper.find('button').exists()).toBe(false)
  })

  it('shows the retry button when at least one row is not Ok, for a mixed ok/timeout report', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve({ ytDlp: okYtDlp, ffmpeg: timeoutFfmpeg }),
    })

    const wrapper = mount(App)
    await flushPromises()

    expect(wrapper.text()).toContain('2026.08.20')
    expect(wrapper.text()).toContain('не отвечает')

    const retryButton = wrapper.findAll('button').find((b) => b.text().includes('Повторить проверку'))
    expect(retryButton).toBeDefined()
  })

  it('shows the retry button when both rows are in error states', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve({ ytDlp: notFoundYtDlp, ffmpeg: timeoutFfmpeg }),
    })

    const wrapper = mount(App)
    await flushPromises()

    const retryButton = wrapper.findAll('button').find((b) => b.text().includes('Повторить проверку'))
    expect(retryButton).toBeDefined()
  })

  it('re-invokes check_sidecar (not prepare_ytdlp again) when the retry button is clicked', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve({ ytDlp: notFoundYtDlp, ffmpeg: okFfmpeg }),
    })

    const wrapper = mount(App)
    await flushPromises()

    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(okReport),
    })

    const retryButton = wrapper.findAll('button').find((b) => b.text().includes('Повторить проверку'))
    await retryButton?.trigger('click')
    await flushPromises()

    expect(invokeMock).toHaveBeenCalledTimes(3) // prepare_ytdlp, check_sidecar, check_sidecar
    expect(invokeMock.mock.calls.filter(([cmd]) => cmd === 'prepare_ytdlp')).toHaveLength(1)
    expect(wrapper.find('button').exists()).toBe(false)
  })
})
