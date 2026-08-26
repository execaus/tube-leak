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

  it('does not invoke prepare_ytdlp until the ytdlp://prepare subscription actually settles', async () => {
    // Ревью TL-17 (#18, «Обязательно»): версия этого теста, разрешавшая
    // `listen()` синхронно, фиксировала лишь порядок синхронных вызовов —
    // гонку между «подписка подтверждена» и «команда вызвана» она не
    // сторожила. Здесь подписка отложена по-настоящему.
    let resolveListen: (fn: typeof unlistenMock) => void = () => {}
    listenMock.mockImplementationOnce((_event, handler) => {
      capturedHandler = handler
      return new Promise<typeof unlistenMock>((resolve) => {
        resolveListen = resolve
      })
    })
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(okReport),
    })

    mount(App)
    await flushPromises()

    expect(listenMock).toHaveBeenCalledTimes(1)
    expect(invokeMock).not.toHaveBeenCalledWith('prepare_ytdlp')
    expect(invokeMock).not.toHaveBeenCalledWith('check_sidecar')

    resolveListen(unlistenMock)
    await flushPromises()

    expect(invokeMock).toHaveBeenCalledWith('prepare_ytdlp')
    expect(invokeMock).toHaveBeenCalledWith('check_sidecar')
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
  it('shows the service screen (checking) before the first event, then progress per stage, then the service screen again', async () => {
    // Композиция «starting = ready» — решение ревью TL-17 (#18,
    // «Композиция тёплого старта»): до первого события экран — та же
    // раскладка, что и после готовности (шапка с версией, обе строки
    // sidecar «Проверяем…»), а не отдельная надпись-заглушка.
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

    // До первого события — не пустое окно: версия и обе строки sidecar
    // видны сразу (Ф-9/Н-6), check_sidecar при этом ещё не вызван (см.
    // блок «order of calls»).
    expect(wrapper.text()).toContain('версия 0.1.0')
    expect(wrapper.text().match(/Проверяем…/g)).toHaveLength(2)

    capturedHandler?.({ payload: { stage: 'unpacking', percent: 4, etaSecs: 1 } })
    await wrapper.vm.$nextTick()
    expect(wrapper.text()).toContain('Распаковываем yt-dlp')
    expect(wrapper.text()).toContain('4%')
    // Версия остаётся видимой даже во время экрана подготовки (ревью TL-17,
    // #18, «Версия приложения — всегда в шапке»).
    expect(wrapper.text()).toContain('версия 0.1.0')

    capturedHandler?.({ payload: { stage: 'warmingUp', percent: 60, etaSecs: 14 } })
    await wrapper.vm.$nextTick()
    expect(wrapper.text()).toContain('Готовим yt-dlp к первому запуску')
    expect(wrapper.text()).toContain('60%')
    expect(wrapper.text()).toContain('осталось ~14 с')

    // Событие `ready`, пришедшее чуть раньше разрешения промиса, не должно
    // ронять экран обратно в служебный раньше времени (ревью TL-17, #18,
    // «мигание в конце ожидания»).
    capturedHandler?.({ payload: { stage: 'ready', percent: 100, version: '2026.08.20' } })
    await wrapper.vm.$nextTick()
    expect(wrapper.text()).toContain('Готовим yt-dlp к первому запуску')
    // Версия по-прежнему видна — и на экране подготовки её не прячут
    // (ревью TL-17, #18), и заодно не мигает служебным экраном раньше
    // времени из-за события `ready`, пришедшего раньше промиса.
    expect(wrapper.text()).toContain('версия 0.1.0')

    resolvePrepare(preparedCold)
    await flushPromises()

    expect(wrapper.text()).not.toContain('Распаковываем')
    expect(wrapper.text()).not.toContain('Готовим yt-dlp')
    expect(wrapper.text()).toContain('версия 0.1.0')
    expect(wrapper.text()).toContain('2026.08.20')
  })

  it('shows the prepare screen for the "OS forgot the signature cache" scenario, which starts at warmingUp with no unpacking', async () => {
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

    capturedHandler?.({ payload: { stage: 'warmingUp', percent: 30, etaSecs: 25 } })
    await wrapper.vm.$nextTick()

    expect(wrapper.text()).toContain('Готовим yt-dlp к первому запуску')
    expect(wrapper.text()).not.toContain('Распаковываем')

    resolvePrepare(preparedCold)
    await flushPromises()
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

  it('stays on the error screen (with a working retry) when the retry attempt fails again', async () => {
    // Ревью TL-17 (#18, «Тесты, которых нет»): раньше проверялось только
    // рассуждением, что второй провал не ломает и не подвешивает экран.
    routeInvoke({
      prepare_ytdlp: () => Promise.reject(warmupFailedError),
      check_sidecar: () => Promise.resolve(okReport),
    })

    const wrapper = mount(App)
    await flushPromises()
    expect(wrapper.text()).toContain('Не удалось подготовить yt-dlp')

    const dataDirError: YtDlpPrepareError = {
      kind: 'dataDirUnavailable',
      message: 'app_data_dir() failed: read-only volume',
    }
    routeInvoke({
      prepare_ytdlp: () => Promise.reject(dataDirError),
      check_sidecar: () => Promise.resolve(okReport),
    })

    const firstRetryButton = wrapper.findAll('button').find((b) => b.text().includes('Повторить'))
    await firstRetryButton?.trigger('click')
    await flushPromises()

    // Другой отказ — другое объяснение, экран ошибки никуда не делся.
    expect(wrapper.text()).toContain('Не удалось подготовить yt-dlp')
    expect(wrapper.text()).toContain('рабочий каталог приложения')
    expect(invokeMock).not.toHaveBeenCalledWith('check_sidecar')

    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(okReport),
    })

    const secondRetryButton = wrapper.findAll('button').find((b) => b.text().includes('Повторить'))
    await secondRetryButton?.trigger('click')
    await flushPromises()

    expect(wrapper.text()).not.toContain('Не удалось подготовить yt-dlp')
    expect(wrapper.text()).toContain('2026.08.20')
    expect(invokeMock).toHaveBeenCalledWith('check_sidecar')
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

describe('App — link probe section gating by yt-dlp status only (эпик E2, TL-33)', () => {
  it('disables the link field with a "checking" placeholder before check_sidecar resolves', async () => {
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

    const input = wrapper.find('input')
    expect(input.attributes('disabled')).toBeDefined()
    expect(input.attributes('placeholder')).toBe('Проверяем yt-dlp…')

    resolveCheck(okReport)
    await flushPromises()
  })

  it('enables the link field once yt-dlp is ok, even if ffmpeg is not', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve({ ytDlp: okYtDlp, ffmpeg: timeoutFfmpeg }),
    })

    const wrapper = mount(App)
    await flushPromises()

    const input = wrapper.find('input')
    expect(input.attributes('disabled')).toBeUndefined()
  })

  it('disables the link field with a hint (not repeating the yt-dlp row error text) when yt-dlp is not ok', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve({ ytDlp: notFoundYtDlp, ffmpeg: okFfmpeg }),
    })

    const wrapper = mount(App)
    await flushPromises()

    const input = wrapper.find('input')
    expect(input.attributes('disabled')).toBeDefined()
    expect(input.attributes('placeholder')).toBe(
      'Разбор ссылок недоступен, пока не решена проблема с yt-dlp выше',
    )
  })
})
