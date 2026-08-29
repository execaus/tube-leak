import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it } from 'vitest'
import { vi } from 'vitest'
import { defineComponent } from 'vue'

import type { YtDlpUpdateSnapshot } from '@/types/generated/update'

const invokeMock = vi.fn()
const unlistenMock = vi.fn()
type EventHandler = (event: { payload: YtDlpUpdateSnapshot }) => void
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
const { checkYtDlpUpdate, fetchYtDlpUpdateState, useYtDlpUpdate } = await import('./useYtDlpUpdate')

const neverCheckedSnapshot: YtDlpUpdateSnapshot = { busy: false, status: 'neverChecked' }
const checkingSnapshot: YtDlpUpdateSnapshot = { busy: true, status: 'checking' }
const upToDateSnapshot: YtDlpUpdateSnapshot = {
  busy: false,
  status: 'upToDate',
  at: '2026-08-25T12:00:00Z',
}

/**
 * Терминальный исход, эмитированный конвейером по С-12 быстрее, чем
 * успевает разрешиться сам вызов `check_ytdlp_update` (ревью TL-59,
 * «Н-1», окно Б).
 */
const failedSnapshot: YtDlpUpdateSnapshot = {
  busy: false,
  status: 'failed',
  at: '2026-08-25T12:00:05Z',
  failure: { kind: 'networkUnavailable', message: 'connect ETIMEDOUT' },
}

/** Аналог test-utils `withSetup` — composable использует `onMounted`/`onUnmounted`, ему нужен активный инстанс. */
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

describe('fetchYtDlpUpdateState', () => {
  it('calls the ytdlp_update_state Tauri command with no arguments', async () => {
    invokeMock.mockResolvedValueOnce(neverCheckedSnapshot)

    await fetchYtDlpUpdateState()

    expect(invokeMock).toHaveBeenCalledExactlyOnceWith('ytdlp_update_state')
  })
})

describe('checkYtDlpUpdate', () => {
  it('calls the check_ytdlp_update Tauri command with no arguments', async () => {
    invokeMock.mockResolvedValueOnce(checkingSnapshot)

    await checkYtDlpUpdate()

    expect(invokeMock).toHaveBeenCalledExactlyOnceWith('check_ytdlp_update')
  })
})

describe('useYtDlpUpdate', () => {
  it('starts with no snapshot', () => {
    invokeMock.mockReturnValueOnce(new Promise<YtDlpUpdateSnapshot>(() => {}))

    const { result } = withSetup(() => useYtDlpUpdate())

    expect(result.snapshot.value).toBeUndefined()
  })

  it('subscribes to ytdlp://update and awaits it before fetching the initial snapshot (order of calls)', async () => {
    let resolveListen: (fn: typeof unlistenMock) => void = () => {}
    listenMock.mockImplementationOnce((_event, handler) => {
      capturedHandler = handler
      return new Promise<typeof unlistenMock>((resolve) => {
        resolveListen = resolve
      })
    })
    invokeMock.mockResolvedValueOnce(neverCheckedSnapshot)

    withSetup(() => useYtDlpUpdate())
    await flushPromises()

    expect(listenMock).toHaveBeenCalledExactlyOnceWith('ytdlp://update', expect.any(Function))
    expect(invokeMock).not.toHaveBeenCalled()

    resolveListen(unlistenMock)
    await flushPromises()

    expect(invokeMock).toHaveBeenCalledExactlyOnceWith('ytdlp_update_state')
  })

  it('populates the snapshot from the initial ytdlp_update_state response', async () => {
    invokeMock.mockResolvedValueOnce(neverCheckedSnapshot)

    const { result } = withSetup(() => useYtDlpUpdate())
    await flushPromises()

    expect(result.snapshot.value).toStrictEqual(neverCheckedSnapshot)
  })

  it('does not let a stale ytdlp_update_state response overwrite an event that arrived first (Н-1, окно А)', async () => {
    // Ревью TL-59 «Н-1»: конвейер мог прислать `ytdlp://update` раньше,
    // чем разрешился начальный снимок (например, шёл уже до открытия
    // экрана) — без счётчика поколений более старый ответ
    // `ytdlp_update_state` откатил бы уже показанное свежее событие назад.
    let resolveFetch: (value: YtDlpUpdateSnapshot) => void = () => {}
    invokeMock.mockReturnValueOnce(
      new Promise<YtDlpUpdateSnapshot>((resolve) => {
        resolveFetch = resolve
      }),
    )

    const { result } = withSetup(() => useYtDlpUpdate())
    // Подписка (в этом файле резолвится сразу) должна успеть подтвердиться
    // и запустить вызов `ytdlp_update_state`, прежде чем событие придёт.
    // `flushPromises()`, а не фиксированное число `Promise.resolve()`:
    // безопасно здесь, потому что сам ответ `ytdlp_update_state`
    // управляется вручную (`resolveFetch`) и остаётся висеть, пока его не
    // вызвали, — флаш дренирует только уже готовые к разрешению микрозадачи.
    await flushPromises()
    expect(invokeMock).toHaveBeenCalledExactlyOnceWith('ytdlp_update_state')
    expect(capturedHandler).toBeDefined()

    // Событие приходит, пока начальный снимок ещё не разрешился.
    capturedHandler?.({ payload: failedSnapshot })
    expect(result.snapshot.value).toStrictEqual(failedSnapshot)

    // Ответ команды приходит позже, всё ещё несёт устаревшее значение.
    resolveFetch(neverCheckedSnapshot)
    await flushPromises()

    expect(result.snapshot.value).toStrictEqual(failedSnapshot)
  })

  it('updates the snapshot as ytdlp://update events arrive', async () => {
    invokeMock.mockResolvedValueOnce(neverCheckedSnapshot)

    const { result } = withSetup(() => useYtDlpUpdate())
    await flushPromises()

    expect(capturedHandler).toBeDefined()

    capturedHandler?.({ payload: checkingSnapshot })
    expect(result.snapshot.value).toStrictEqual(checkingSnapshot)

    capturedHandler?.({ payload: upToDateSnapshot })
    expect(result.snapshot.value).toStrictEqual(upToDateSnapshot)
  })

  it('checkNow() calls check_ytdlp_update and updates the snapshot from its response', async () => {
    invokeMock.mockResolvedValueOnce(neverCheckedSnapshot)

    const { result } = withSetup(() => useYtDlpUpdate())
    await flushPromises()

    invokeMock.mockResolvedValueOnce(checkingSnapshot)
    await result.checkNow()

    expect(invokeMock).toHaveBeenLastCalledWith('check_ytdlp_update')
    expect(result.snapshot.value).toStrictEqual(checkingSnapshot)
  })

  it('does not let a stale check_ytdlp_update response overwrite a terminal event that arrived first (Н-1, окно Б)', async () => {
    // Ревью TL-59 «Н-1»: конвейер может дойти до терминального исхода и
    // прислать `ytdlp://update` раньше, чем разрешится сам вызов
    // `check_ytdlp_update` — обычно несущий лишь промежуточный `checking`.
    // Без счётчика поколений более старый ответ команды переписал бы
    // честный терминальный текст назад в вечный спиннер (прямая
    // регрессия С-12).
    invokeMock.mockResolvedValueOnce(neverCheckedSnapshot)

    const { result } = withSetup(() => useYtDlpUpdate())
    await flushPromises()

    let resolveCheck: (value: YtDlpUpdateSnapshot) => void = () => {}
    invokeMock.mockReturnValueOnce(
      new Promise<YtDlpUpdateSnapshot>((resolve) => {
        resolveCheck = resolve
      }),
    )
    const pending = result.checkNow()
    await Promise.resolve()

    // Событие приходит раньше ответа команды.
    capturedHandler?.({ payload: failedSnapshot })
    expect(result.snapshot.value).toStrictEqual(failedSnapshot)

    // Ответ команды приходит позже, всё ещё несёт устаревший «checking».
    resolveCheck(checkingSnapshot)
    await pending

    expect(result.snapshot.value).toStrictEqual(failedSnapshot)
  })

  it('swallows a rejection from checkNow() instead of throwing (unreachable command error, e.g. busy)', async () => {
    invokeMock.mockResolvedValueOnce(neverCheckedSnapshot)

    const { result } = withSetup(() => useYtDlpUpdate())
    await flushPromises()

    invokeMock.mockRejectedValueOnce({ kind: 'busy', message: 'already checking' })

    await expect(result.checkNow()).resolves.toBeUndefined()
    // Снимок не портится отказом — остаётся тем, что было до вызова.
    expect(result.snapshot.value).toStrictEqual(neverCheckedSnapshot)
  })

  it('swallows a rejection from the initial snapshot fetch instead of leaving the composable in a broken state', async () => {
    invokeMock.mockRejectedValueOnce(new Error('ytdlp_update_state command unavailable'))

    const { result } = withSetup(() => useYtDlpUpdate())
    await flushPromises()

    expect(result.snapshot.value).toBeUndefined()
  })

  it('unsubscribes from the event when the owning component unmounts', async () => {
    invokeMock.mockResolvedValueOnce(neverCheckedSnapshot)

    const { unmount } = withSetup(() => useYtDlpUpdate())
    await flushPromises()

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
    invokeMock.mockReturnValueOnce(new Promise<YtDlpUpdateSnapshot>(() => {}))

    const { unmount } = withSetup(() => useYtDlpUpdate())
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
