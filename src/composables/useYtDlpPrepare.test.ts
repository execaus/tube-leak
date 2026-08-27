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

  it('does not invoke prepare_ytdlp until the event subscription actually settles (order of calls)', async () => {
    // Ревью TL-17 (#18, «Обязательно»): предыдущая версия этого теста
    // разрешала `listen()` синхронно, поэтому фиксировала лишь порядок
    // *синхронных* вызовов — гонка, ради которой была построена вся
    // конструкция, тестом не сторожилась. Здесь подписка отложена по-настоящему.
    let resolveListen: (fn: typeof unlistenMock) => void = () => {}
    listenMock.mockImplementationOnce((_event, handler) => {
      capturedHandler = handler
      return new Promise<typeof unlistenMock>((resolve) => {
        resolveListen = resolve
      })
    })
    invokeMock.mockResolvedValueOnce(preparedFixture)

    const { result } = withSetup(() => useYtDlpPrepare())
    const pending = result.prepare()
    await Promise.resolve()
    await Promise.resolve()

    expect(listenMock).toHaveBeenCalledTimes(1)
    expect(invokeMock).not.toHaveBeenCalled()

    resolveListen(unlistenMock)
    await pending

    expect(invokeMock).toHaveBeenCalledExactlyOnceWith('prepare_ytdlp')
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

  it('ignores terminal stages (ready/failed) from events — final state comes from the command promise, not from events', async () => {
    // Ревью TL-17 (#18, «Стоит поправить», «мигание в конце ожидания»):
    // ядро эмитит `ready`/`failed` непосредственно перед разрешением
    // промиса, и порядок «доставка события» vs «ответ IPC» не гарантирован.
    // Если бы `stage` буквально копировал событие, `ready`, пришедший чуть
    // раньше промиса, на мгновение убрал бы экран подготовки из-под
    // прогресса — видимый откат назад в последний момент 40-секундного
    // ожидания.
    invokeMock.mockReturnValueOnce(new Promise<YtDlpPrepared>(() => {}))

    const { result } = withSetup(() => useYtDlpPrepare())
    void result.prepare()
    await Promise.resolve()
    await Promise.resolve()

    capturedHandler?.({ payload: { stage: 'warmingUp', percent: 92, etaSecs: 2 } })
    expect(result.stage.value).toBe('warmingUp')

    capturedHandler?.({ payload: { stage: 'ready', percent: 100, version: '2026.08.20' } })
    expect(result.stage.value).toBe('warmingUp')
    expect(result.percent.value).toBe(100)
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

  it('unsubscribes even when unmount happens before listen() has resolved (unsubscribe race)', async () => {
    // Ревью TL-17 (#18, «Стоит поправить»): `unlisten` присваивается только
    // после разрешения `listen()`; unmount, случившийся раньше, не должен
    // оставить слушателя висеть навсегда.
    let resolveListen: (fn: typeof unlistenMock) => void = () => {}
    listenMock.mockImplementationOnce((_event, handler) => {
      capturedHandler = handler
      return new Promise<typeof unlistenMock>((resolve) => {
        resolveListen = resolve
      })
    })
    invokeMock.mockReturnValueOnce(new Promise<YtDlpPrepared>(() => {}))

    const { result, unmount } = withSetup(() => useYtDlpPrepare())
    void result.prepare()
    await Promise.resolve()

    unmount()
    expect(unlistenMock).not.toHaveBeenCalled()

    resolveListen(unlistenMock)
    // Цепочка `listen().then(...).catch(...)` внутри `ensureListening`, а
    // затем ещё один `.then()` в `onUnmounted` — несколько хопов
    // микрозадач, а не один.
    for (let i = 0; i < 5; i += 1) {
      await Promise.resolve()
    }

    expect(unlistenMock).toHaveBeenCalledTimes(1)
  })

  it('does not get stuck loading forever if the event subscription itself is rejected (blocker, TL-17 #18)', async () => {
    // Раньше `await ensureListening()` стоял вне `try`: отказ `listen()`
    // (тот же IPC-вызов `plugin:event|listen`, тоже может не пройти) не
    // ловился, `finally` не выполнялся, `isPending` навсегда оставался
    // `true`, `error` не заполнялся — экран подготовки не поднимался и не
    // опускался, вечное «Запускаем…», `check_sidecar` не вызывался никогда.
    const listenFailure = new Error('plugin:event|listen failed')
    listenMock.mockRejectedValueOnce(listenFailure)

    const { result } = withSetup(() => useYtDlpPrepare())
    await result.prepare()

    expect(result.isPending.value).toBe(false)
    expect(result.error.value).toStrictEqual({ message: listenFailure.message })
    expect(invokeMock).not.toHaveBeenCalled()
  })

  it('lets a retry re-subscribe after the subscription itself failed on the previous attempt', async () => {
    listenMock.mockRejectedValueOnce(new Error('plugin:event|listen failed'))

    const { result } = withSetup(() => useYtDlpPrepare())
    await result.prepare()
    expect(result.error.value).toBeDefined()

    listenMock.mockImplementationOnce((_event, handler) => {
      capturedHandler = handler
      return Promise.resolve(unlistenMock)
    })
    invokeMock.mockResolvedValueOnce(preparedFixture)

    await result.prepare()

    expect(listenMock).toHaveBeenCalledTimes(2)
    expect(result.error.value).toBeUndefined()
    expect(result.result.value).toStrictEqual(preparedFixture)
  })

  it('recognizes notEnoughSpace (TL-18/TL-50) as a typed contractual error, not a fallback', async () => {
    // До TL-50 KNOWN_ERROR_KINDS не знал про notEnoughSpace: isYtDlpPrepareError
    // возвращал false, и toPrepareFailure терял и kind, и Rust-сообщение с
    // цифрами needed/available, заменяя их общей заглушкой.
    const notEnoughSpaceError: YtDlpPrepareError = {
      kind: 'notEnoughSpace',
      message: 'не хватает места для распаковки yt-dlp: нужно ещё 130 МиБ, свободно 12 МиБ (/data/ytdlp)',
    }
    invokeMock.mockRejectedValueOnce(notEnoughSpaceError)

    const { result } = withSetup(() => useYtDlpPrepare())
    await result.prepare()

    expect(result.error.value).toStrictEqual(notEnoughSpaceError)
  })

  it('falls back to a message-only failure for a non-contractual rejection (no kind), instead of an empty explanation', async () => {
    invokeMock.mockRejectedValueOnce('yt-dlp panicked')

    const { result } = withSetup(() => useYtDlpPrepare())
    await result.prepare()

    expect(result.error.value).toStrictEqual({ message: 'yt-dlp panicked' })
  })

  it('falls back to a generic message when the rejection has no usable text at all', async () => {
    invokeMock.mockRejectedValueOnce({ some: 'unexpected shape' })

    const { result } = withSetup(() => useYtDlpPrepare())
    await result.prepare()

    expect(result.error.value).toStrictEqual({
      message: 'Подготовка yt-dlp не удалась по нераспознанной причине.',
    })
  })
})
