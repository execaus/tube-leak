import { mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { defineComponent, ref, type Ref } from 'vue'

import type { YtDlpWarmupEvent, YtDlpWarmupOutcome } from '@/types/generated/ytdlp'

const unlistenMock = vi.fn()
type EventHandler = (event: { payload: YtDlpWarmupEvent }) => void
let capturedHandler: EventHandler | undefined
const listenMock = vi.fn((_event: string, handler: EventHandler) => {
  capturedHandler = handler
  return Promise.resolve(unlistenMock)
})

vi.mock('@tauri-apps/api/event', () => ({
  listen: (...args: [string, EventHandler]) => listenMock(...args),
}))

// Импортируется после мока `listen`, тем же приёмом, что и соседние composable-тесты.
const { useYtDlpWarmupRecheck } = await import('./useYtDlpWarmupRecheck')

/** Аналог test-utils `withSetup` — composable использует `onMounted`/`onUnmounted`. */
function withSetup(isLoading: Ref<boolean>, check: () => Promise<void>) {
  const wrapper = mount(
    defineComponent({
      setup() {
        useYtDlpWarmupRecheck({ isLoading, check })
        return () => null
      },
    }),
  )
  return { unmount: () => wrapper.unmount() }
}

beforeEach(() => {
  listenMock.mockClear()
  unlistenMock.mockClear()
  capturedHandler = undefined
})

describe('useYtDlpWarmupRecheck', () => {
  it('subscribes to ytdlp://warmup on mount', async () => {
    withSetup(ref(false), vi.fn().mockResolvedValue(undefined))
    await Promise.resolve()

    expect(listenMock).toHaveBeenCalledExactlyOnceWith('ytdlp://warmup', expect.any(Function))
  })

  it('rechecks exactly once when a warmed event arrives while idle', async () => {
    const check = vi.fn().mockResolvedValue(undefined)
    withSetup(ref(false), check)
    await Promise.resolve()

    const event: YtDlpWarmupEvent = { outcome: 'warmed' }
    capturedHandler?.({ payload: event })

    expect(check).toHaveBeenCalledOnce()
  })

  it('does not recheck when a check is already in flight (isLoading)', async () => {
    const check = vi.fn().mockResolvedValue(undefined)
    withSetup(ref(true), check)
    await Promise.resolve()

    capturedHandler?.({ payload: { outcome: 'warmed' } })

    expect(check).not.toHaveBeenCalled()
  })

  it.each<YtDlpWarmupOutcome>(['warmed', 'timedOut', 'failed'])(
    'rechecks on the %s outcome',
    async (outcome) => {
      const check = vi.fn().mockResolvedValue(undefined)
      withSetup(ref(false), check)
      await Promise.resolve()

      capturedHandler?.({ payload: { outcome } })

      expect(check).toHaveBeenCalledOnce()
    },
  )

  it('unsubscribes from the event when the owning component unmounts', async () => {
    const { unmount } = withSetup(ref(false), vi.fn().mockResolvedValue(undefined))
    await Promise.resolve()
    await Promise.resolve()

    expect(unlistenMock).not.toHaveBeenCalled()
    unmount()
    expect(unlistenMock).toHaveBeenCalledTimes(1)
  })

  it('unsubscribes even when unmount happens before listen() has resolved (unsubscribe race)', async () => {
    let resolveListen: (fn: typeof unlistenMock) => void = () => {}
    listenMock.mockImplementationOnce((_event, handler) => {
      capturedHandler = handler
      return new Promise<typeof unlistenMock>((resolve) => {
        resolveListen = resolve
      })
    })

    const { unmount } = withSetup(ref(false), vi.fn().mockResolvedValue(undefined))
    await Promise.resolve()

    unmount()
    expect(unlistenMock).not.toHaveBeenCalled()

    resolveListen(unlistenMock)
    for (let i = 0; i < 5; i += 1) {
      await Promise.resolve()
    }

    expect(unlistenMock).toHaveBeenCalledTimes(1)
  })
})
