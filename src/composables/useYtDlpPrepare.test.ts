import { mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { defineComponent } from 'vue'

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

// Импортируется после мока модулей, чтобы composable получил замоканные `invoke`/`listen`.
const { prepareYtDlp, useYtDlpPrepare } = await import('./useYtDlpPrepare')

const preparedFixture: YtDlpPrepared = {
  version: '2026.08.20',
  path: '/Users/x/Library/Application Support/dev.execaus.tubeleak/ytdlp/yt-dlp',
  prepared: true,
  durationMs: 36_412,
}

const warmupFailedError: YtDlpPrepareError = {
  kind: 'warmupFailed',
  message: 'yt-dlp не ответил за отведённое время прогрева',
}

/** Аналог test-utils `withSetup` — composable использует `onUnmounted`, ему нужен активный инстанс. */
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
})

describe('prepareYtDlp', () => {
  it('calls the prepare_ytdlp Tauri command with no arguments', async () => {
    invokeMock.mockResolvedValueOnce(preparedFixture)

    await prepareYtDlp()

    expect(invokeMock).toHaveBeenCalledExactlyOnceWith('prepare_ytdlp')
  })
})

describe('useYtDlpPrepare', () => {
  it('starts with no stage, zero percent, no result and no error, not pending', () => {
    const { result } = withSetup(() => useYtDlpPrepare())

    expect(result.stage.value).toBeUndefined()
    expect(result.percent.value).toBe(0)
    expect(result.etaSecs.value).toBeUndefined()
    expect(result.result.value).toBeUndefined()
    expect(result.error.value).toBeUndefined()
    expect(result.isPending.value).toBe(false)
  })

  it('awaits the event subscription before invoking prepare_ytdlp (order of calls)', async () => {
    const callOrder: string[] = []
    listenMock.mockImplementationOnce((_event, handler) => {
      callOrder.push('listen')
      capturedHandler = handler
      return Promise.resolve(unlistenMock)
    })
    invokeMock.mockImplementationOnce(() => {
      callOrder.push('invoke')
      return Promise.resolve(preparedFixture)
    })

    const { result } = withSetup(() => useYtDlpPrepare())
    await result.prepare()

    expect(callOrder).toEqual(['listen', 'invoke'])
  })

  it('sets isPending during the call and clears it once prepare_ytdlp resolves', async () => {
    let resolveInvoke: (value: YtDlpPrepared) => void = () => {}
    invokeMock.mockReturnValueOnce(
      new Promise<YtDlpPrepared>((resolve) => {
        resolveInvoke = resolve
      }),
    )

    const { result } = withSetup(() => useYtDlpPrepare())
    const pending = result.prepare()
    await Promise.resolve()
    await Promise.resolve()

    expect(result.isPending.value).toBe(true)

    resolveInvoke(preparedFixture)
    await pending

    expect(result.isPending.value).toBe(false)
    expect(result.result.value).toStrictEqual(preparedFixture)
    expect(result.error.value).toBeUndefined()
  })

  it('updates stage, percent and etaSecs as ytdlp://prepare events arrive', async () => {
    invokeMock.mockReturnValueOnce(new Promise<YtDlpPrepared>(() => {}))

    const { result } = withSetup(() => useYtDlpPrepare())
    void result.prepare()
    await Promise.resolve()
    await Promise.resolve()

    expect(capturedHandler).toBeDefined()

    capturedHandler?.({ payload: { stage: 'unpacking', percent: 4, etaSecs: 1 } })
    expect(result.stage.value).toBe('unpacking')
    expect(result.percent.value).toBe(4)
    expect(result.etaSecs.value).toBe(1)

    capturedHandler?.({ payload: { stage: 'warmingUp', percent: 52, etaSecs: 17 } })
    expect(result.stage.value).toBe('warmingUp')
    expect(result.percent.value).toBe(52)
    expect(result.etaSecs.value).toBe(17)

    // `etaSecs` отсутствует в полезной нагрузке, а не приходит `null` (контракт TL-12).
    capturedHandler?.({ payload: { stage: 'warmingUp', percent: 90 } })
    expect(result.etaSecs.value).toBeUndefined()
  })

  it('captures a typed YtDlpPrepareError on rejection, without touching result', async () => {
    invokeMock.mockRejectedValueOnce(warmupFailedError)

    const { result } = withSetup(() => useYtDlpPrepare())
    await result.prepare()

    expect(result.isPending.value).toBe(false)
    expect(result.error.value).toStrictEqual(warmupFailedError)
    expect(result.result.value).toBeUndefined()
  })

  it('resets stage/percent/etaSecs/error at the start of a retry after a prior failure', async () => {
    invokeMock.mockRejectedValueOnce(warmupFailedError)

    const { result } = withSetup(() => useYtDlpPrepare())
    await result.prepare()
    capturedHandler?.({ payload: { stage: 'warmingUp', percent: 80, etaSecs: 3 } })
    expect(result.stage.value).toBe('warmingUp')

    invokeMock.mockReturnValueOnce(new Promise<YtDlpPrepared>(() => {}))
    const retryPending = result.prepare()
    await Promise.resolve()

    expect(result.error.value).toBeUndefined()
    expect(result.stage.value).toBeUndefined()
    expect(result.percent.value).toBe(0)
    expect(result.etaSecs.value).toBeUndefined()

    void retryPending
  })

  it('reuses the same subscription across retries instead of listening again', async () => {
    invokeMock.mockRejectedValueOnce(warmupFailedError)

    const { result } = withSetup(() => useYtDlpPrepare())
    await result.prepare()

    invokeMock.mockResolvedValueOnce(preparedFixture)
    await result.prepare()

    expect(listenMock).toHaveBeenCalledTimes(1)
  })

  it('unsubscribes from the event when the owning component unmounts', async () => {
    invokeMock.mockResolvedValueOnce(preparedFixture)

    const { result, unmount } = withSetup(() => useYtDlpPrepare())
    await result.prepare()

    expect(unlistenMock).not.toHaveBeenCalled()
    unmount()
    expect(unlistenMock).toHaveBeenCalledTimes(1)
  })
})
