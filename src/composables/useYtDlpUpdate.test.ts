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
const { checkYtDlpUpdate, fetchYtDlpUpdateState, rollBackYtDlp, useYtDlpUpdate } = await import('./useYtDlpUpdate')

const neverCheckedSnapshot: YtDlpUpdateSnapshot = { busy: false, status: 'neverChecked' }
const checkingSnapshot: YtDlpUpdateSnapshot = { busy: true, status: 'checking' }
const upToDateSnapshot: YtDlpUpdateSnapshot = {
  busy: false,
  status: 'upToDate',
  at: '2026-08-25T12:00:00Z',
}

/** Строка 14 таблицы — откат принят, ждёт паузы между задачами (Ф-7, Р-3). */
const rollbackWaitingSnapshot: YtDlpUpdateSnapshot = {
  busy: true,
  status: 'rollbackWaiting',
  version: '2026.07.11',
  rollbackTarget: '2026.08.20',
}

/** Строка 13 — откат уже применён немедленно (нет активной загрузки, Ф-8/Р-3). */
const rolledBackSnapshot: YtDlpUpdateSnapshot = {
  busy: false,
  status: 'rolledBack',
  at: '2026-08-25T12:00:05Z',
  active: '2026.07.11',
  abandoned: '2026.08.20',
  rollbackTarget: '2026.08.20',
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

describe('rollBackYtDlp', () => {
  it('calls the roll_back_ytdlp Tauri command with no arguments (TL-60, Р-3: цель ровно одна, не параметр)', async () => {
    invokeMock.mockResolvedValueOnce(rollbackWaitingSnapshot)

    await rollBackYtDlp()

    expect(invokeMock).toHaveBeenCalledExactlyOnceWith('roll_back_ytdlp')
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

  /*
   * TL-60 (Р-3): `rollback()` — тот же контракт, что `checkNow()`, только
   * за `roll_back_ytdlp`, и он им и является технически (`callCommandAndApply`
   * общий, doc composable выше). Проверяется отдельно, а не по аналогии —
   * дублирующий код неизбежно расходится с оригиналом на следующей правке,
   * и тест обязан ловить это здесь, а не полагаться на симметрию с checkNow.
   */
  it('rollback() calls roll_back_ytdlp and updates the snapshot from its response', async () => {
    invokeMock.mockResolvedValueOnce(upToDateSnapshot)

    const { result } = withSetup(() => useYtDlpUpdate())
    await flushPromises()

    invokeMock.mockResolvedValueOnce(rollbackWaitingSnapshot)
    await result.rollback()

    expect(invokeMock).toHaveBeenLastCalledWith('roll_back_ytdlp')
    expect(result.snapshot.value).toStrictEqual(rollbackWaitingSnapshot)
  })

  /*
   * Критерий приёмки TL-60: «во время активной загрузки клик «Вернуться»
   * переводит блок в состояние 14 (ожидание границы), не в 13 (уже
   * применено) немедленно». `rollback()` не решает это сам — он лишь
   * применяет то, что вернула команда (doc `rollback` в composable);
   * здесь доказывается, что при ответе `rollbackWaiting` composable
   * действительно оседает на строке 14, а не на что-то другое.
   * Симметричный случай (нет активной загрузки, ответ — `rolledBack`,
   * строка 13, применяется немедленно) проверяется тестом выше по
   * тому же коду — если бы composable решал это сам, для двух исходов
   * потребовались бы разные пути, а не один и тот же вызов.
   */
  it('rollback() reflects rollbackWaiting (row 14), not an assumed immediate rolledBack (row 13), when the command says so', async () => {
    invokeMock.mockResolvedValueOnce(upToDateSnapshot)

    const { result } = withSetup(() => useYtDlpUpdate())
    await flushPromises()

    invokeMock.mockResolvedValueOnce(rollbackWaitingSnapshot)
    await result.rollback()

    expect(result.snapshot.value?.status).toBe('rollbackWaiting')
    expect(result.snapshot.value?.status).not.toBe('rolledBack')
  })

  it('rollback() reflects an immediate rolledBack (row 13) when the command applies it right away', async () => {
    invokeMock.mockResolvedValueOnce(upToDateSnapshot)

    const { result } = withSetup(() => useYtDlpUpdate())
    await flushPromises()

    invokeMock.mockResolvedValueOnce(rolledBackSnapshot)
    await result.rollback()

    expect(result.snapshot.value).toStrictEqual(rolledBackSnapshot)
  })

  it('does not let a stale roll_back_ytdlp response overwrite a terminal event that arrived first (Н-1, окно Б, зеркало для отката)', async () => {
    invokeMock.mockResolvedValueOnce(upToDateSnapshot)

    const { result } = withSetup(() => useYtDlpUpdate())
    await flushPromises()

    let resolveRollback: (value: YtDlpUpdateSnapshot) => void = () => {}
    invokeMock.mockReturnValueOnce(
      new Promise<YtDlpUpdateSnapshot>((resolve) => {
        resolveRollback = resolve
      }),
    )
    const pending = result.rollback()
    await Promise.resolve()

    // Событие приходит раньше ответа команды.
    capturedHandler?.({ payload: rolledBackSnapshot })
    expect(result.snapshot.value).toStrictEqual(rolledBackSnapshot)

    // Ответ команды приходит позже, всё ещё несёт устаревший `rollbackWaiting`.
    resolveRollback(rollbackWaitingSnapshot)
    await pending

    expect(result.snapshot.value).toStrictEqual(rolledBackSnapshot)
  })

  it('swallows a rejection from rollback() instead of throwing (unreachable command error, e.g. nothingToRollBackTo)', async () => {
    invokeMock.mockResolvedValueOnce(upToDateSnapshot)

    const { result } = withSetup(() => useYtDlpUpdate())
    await flushPromises()

    invokeMock.mockRejectedValueOnce({ kind: 'nothingToRollBackTo', message: 'no known-good install' })

    await expect(result.rollback()).resolves.toBeUndefined()
    expect(result.snapshot.value).toStrictEqual(upToDateSnapshot)
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

  /*
   * TL-120 (issue #127): регрессия TL-66 — откат без активной загрузки
   * отвечает только после переключения (до 24 с на холодном дереве), а
   * `snapshot.busy` до этого момента ещё несёт старое значение. `pending`
   * — отдельный от `snapshot` сигнал «ответ ещё не пришёл», который
   * вызывающая сторона обязана учитывать наравне со `snapshot.busy` (doc
   * `pending` в `UseYtDlpUpdateReturn`).
   */
  describe('pending (TL-120, issue #127)', () => {
    it('starts false', () => {
      invokeMock.mockReturnValueOnce(new Promise<YtDlpUpdateSnapshot>(() => {}))

      const { result } = withSetup(() => useYtDlpUpdate())

      expect(result.pending.value).toBe(false)
    })

    it('is true while rollback() awaits roll_back_ytdlp, even though snapshot.busy is still the old (false) value', async () => {
      invokeMock.mockResolvedValueOnce(upToDateSnapshot)

      const { result } = withSetup(() => useYtDlpUpdate())
      await flushPromises()
      expect(result.snapshot.value?.busy).toBe(false)

      let resolveRollback: (value: YtDlpUpdateSnapshot) => void = () => {}
      invokeMock.mockReturnValueOnce(
        new Promise<YtDlpUpdateSnapshot>((resolve) => {
          resolveRollback = resolve
        }),
      )
      const pendingCall = result.rollback()
      await Promise.resolve()

      // Ответ ещё не пришёл — snapshot.busy всё ещё унаследован от
      // старого снимка (false), но pending уже держит кнопки неактивными.
      expect(result.pending.value).toBe(true)
      expect(result.snapshot.value?.busy).toBe(false)

      resolveRollback(rolledBackSnapshot)
      await pendingCall

      expect(result.pending.value).toBe(false)
      expect(result.snapshot.value).toStrictEqual(rolledBackSnapshot)
    })

    it('is true while checkNow() awaits check_ytdlp_update', async () => {
      invokeMock.mockResolvedValueOnce(neverCheckedSnapshot)

      const { result } = withSetup(() => useYtDlpUpdate())
      await flushPromises()

      let resolveCheck: (value: YtDlpUpdateSnapshot) => void = () => {}
      invokeMock.mockReturnValueOnce(
        new Promise<YtDlpUpdateSnapshot>((resolve) => {
          resolveCheck = resolve
        }),
      )
      const pendingCall = result.checkNow()
      await Promise.resolve()

      expect(result.pending.value).toBe(true)

      resolveCheck(checkingSnapshot)
      await pendingCall

      expect(result.pending.value).toBe(false)
    })

    it('is cleared when rollback() rejects (command refused as busy/nothingToRollBackTo)', async () => {
      invokeMock.mockResolvedValueOnce(upToDateSnapshot)

      const { result } = withSetup(() => useYtDlpUpdate())
      await flushPromises()

      let rejectRollback: (err: unknown) => void = () => {}
      invokeMock.mockReturnValueOnce(
        new Promise<YtDlpUpdateSnapshot>((_resolve, reject) => {
          rejectRollback = reject
        }),
      )
      const pendingCall = result.rollback()
      await Promise.resolve()

      expect(result.pending.value).toBe(true)

      rejectRollback({ kind: 'busy', message: 'already rolling back' })
      await pendingCall

      expect(result.pending.value).toBe(false)
    })

    it('reflects the response case where the command answers immediately with the terminal snapshot (rolledBack, row 13)', async () => {
      invokeMock.mockResolvedValueOnce(upToDateSnapshot)

      const { result } = withSetup(() => useYtDlpUpdate())
      await flushPromises()

      invokeMock.mockResolvedValueOnce(rolledBackSnapshot)
      await result.rollback()

      expect(result.pending.value).toBe(false)
      expect(result.snapshot.value).toStrictEqual(rolledBackSnapshot)
    })

    it('reflects the response case where the command answers with rollbackWaiting (row 14, unchanged behaviour)', async () => {
      invokeMock.mockResolvedValueOnce(upToDateSnapshot)

      const { result } = withSetup(() => useYtDlpUpdate())
      await flushPromises()

      invokeMock.mockResolvedValueOnce(rollbackWaitingSnapshot)
      await result.rollback()

      expect(result.pending.value).toBe(false)
      expect(result.snapshot.value).toStrictEqual(rollbackWaitingSnapshot)
    })
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
