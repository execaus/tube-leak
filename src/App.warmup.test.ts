import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { SidecarCheckReport, SidecarCheckResult } from '@/types/generated/sidecar'
import type { YtDlpPrepared, YtDlpWarmupEvent, YtDlpWarmupOutcome } from '@/types/generated/ytdlp'

/**
 * Интеграционный уровень TL-118 (долг #22): в отличие от
 * `useYtDlpWarmupRecheck.test.ts` (composable в изоляции, все три исхода,
 * защита от повторного вызова, отписка), этот файл доказывает, что
 * подписка действительно подключена к настоящему `useSidecarCheck()`
 * внутри `App.vue` — свежий отчёт `check_sidecar` реально доезжает до
 * поля ссылки (гейт TL-33), а не только до внутреннего состояния
 * composable.
 */
const invokeMock = vi.fn()
const unlistenMock = vi.fn()
type WarmupHandler = (event: { payload: YtDlpWarmupEvent }) => void
let capturedWarmupHandler: WarmupHandler | undefined

function defaultListenImpl(
  event: string,
  handler: (event: unknown) => void,
): Promise<typeof unlistenMock> {
  if (event === 'ytdlp://warmup') {
    capturedWarmupHandler = handler as WarmupHandler
  }
  return Promise.resolve(unlistenMock)
}

const listenMock = vi.fn(defaultListenImpl)

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: (...args: [string, (event: unknown) => void]) => listenMock(...args),
}))

// `useExitConfirmation` (TL-46) — тем же приёмом, что `App.test.ts`: подписка
// на настоящее оконное событие просто не резолвится, этот файл не про неё.
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    onCloseRequested: () => new Promise<() => void>(() => {}),
    destroy: () => Promise.resolve(),
  }),
}))

const { default: App } = await import('./App.vue')

const preparedWarm: YtDlpPrepared = {
  version: '2026.08.20',
  path: '/opt/tube-leak/ytdlp/yt-dlp',
  prepared: false,
  durationMs: 120,
}

const timeoutYtDlp: SidecarCheckResult = {
  name: 'yt-dlp',
  path: '/opt/tube-leak/bin/yt-dlp',
  status: 'timeout',
  timeoutMs: 5000,
}

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

const okDeno: SidecarCheckResult = {
  name: 'deno',
  path: '/opt/tube-leak/bin/deno',
  status: 'ok',
  version: '2.9.6',
}

const coldReport: SidecarCheckReport = { ytDlp: timeoutYtDlp, ffmpeg: okFfmpeg, deno: okDeno }
const warmReport: SidecarCheckReport = { ytDlp: okYtDlp, ffmpeg: okFfmpeg, deno: okDeno }

/** Тот же роутер `invoke`, что `App.test.ts` — см. doc там для полного списка дефолтов. */
function routeInvoke(handlers: Record<string, () => Promise<unknown>>) {
  const withDefaults: Record<string, () => Promise<unknown>> = {
    ytdlp_update_state: () => new Promise<unknown>(() => {}),
    queue_state: () => new Promise<unknown>(() => {}),
    history_page: () => new Promise<unknown>(() => {}),
    settings_get: () => new Promise<unknown>(() => {}),
    ...handlers,
  }
  invokeMock.mockImplementation((command: string) => {
    const handler = withDefaults[command]
    if (!handler) throw new Error(`unexpected invoke: ${command}`)
    return handler()
  })
}

beforeEach(() => {
  invokeMock.mockReset()
  listenMock.mockReset()
  listenMock.mockImplementation(defaultListenImpl)
  unlistenMock.mockClear()
  capturedWarmupHandler = undefined
  setActivePinia(createPinia())
})

describe('App — ytdlp://warmup rechecks the service screen (TL-118, долг #22)', () => {
  it('subscribes to ytdlp://warmup', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(coldReport),
    })

    mount(App)
    await flushPromises()

    expect(listenMock).toHaveBeenCalledWith('ytdlp://warmup', expect.any(Function))
    expect(capturedWarmupHandler).toBeDefined()
  })

  it('a warmed event triggers exactly one additional check_sidecar call and unlocks the link field', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(coldReport),
    })

    const wrapper = mount(App)
    await flushPromises()

    expect(wrapper.find('input').attributes('disabled')).toBeDefined()
    expect(invokeMock.mock.calls.filter(([cmd]) => cmd === 'check_sidecar')).toHaveLength(1)

    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(warmReport),
    })

    capturedWarmupHandler?.({ payload: { outcome: 'warmed' } })
    await flushPromises()

    expect(invokeMock.mock.calls.filter(([cmd]) => cmd === 'check_sidecar')).toHaveLength(2)
    expect(wrapper.find('input').attributes('disabled')).toBeUndefined()
  })

  it('a warmup event that arrives while a check is already in flight does not trigger a second check_sidecar call', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(coldReport),
    })

    const wrapper = mount(App)
    await flushPromises()

    let resolveRetryCheck: (value: SidecarCheckReport) => void = () => {}
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () =>
        new Promise<SidecarCheckReport>((resolve) => {
          resolveRetryCheck = resolve
        }),
    })

    const retryButton = wrapper.findAll('button').find((b) => b.text().includes('Повторить проверку'))
    expect(retryButton).toBeDefined()
    await retryButton?.trigger('click')
    // Клик уже начал вторую проверку (isLoading === true), её собственный
    // ответ ещё не пришёл.
    expect(invokeMock.mock.calls.filter(([cmd]) => cmd === 'check_sidecar')).toHaveLength(2)

    capturedWarmupHandler?.({ payload: { outcome: 'timedOut' } })
    await flushPromises()

    // Событие не добавило третьего вызова поверх уже идущего.
    expect(invokeMock.mock.calls.filter(([cmd]) => cmd === 'check_sidecar')).toHaveLength(2)

    resolveRetryCheck(warmReport)
    await flushPromises()
  })

  it.each<YtDlpWarmupOutcome>(['timedOut', 'failed'])(
    'a %s event also triggers a recheck',
    async (outcome) => {
      routeInvoke({
        prepare_ytdlp: () => Promise.resolve(preparedWarm),
        check_sidecar: () => Promise.resolve(coldReport),
      })

      const wrapper = mount(App)
      await flushPromises()

      routeInvoke({
        prepare_ytdlp: () => Promise.resolve(preparedWarm),
        check_sidecar: () => Promise.resolve(coldReport),
      })

      capturedWarmupHandler?.({ payload: { outcome } })
      await flushPromises()

      expect(invokeMock.mock.calls.filter(([cmd]) => cmd === 'check_sidecar')).toHaveLength(2)
      // Оба неудачных исхода оставляют строку yt-dlp с той же кнопкой повтора.
      const retryButton = wrapper.findAll('button').find((b) => b.text().includes('Повторить проверку'))
      expect(retryButton).toBeDefined()
    },
  )

  it('unsubscribes from ytdlp://warmup when the app unmounts', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(warmReport),
    })

    const wrapper = mount(App)
    await flushPromises()

    expect(unlistenMock).not.toHaveBeenCalled()
    wrapper.unmount()
    expect(unlistenMock).toHaveBeenCalled()
  })
})
