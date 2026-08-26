import { mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { defineComponent } from 'vue'

import type { ProbeError, ProbeResult } from '@/types/probe'

const invokeMock = vi.fn()

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}))

// Импортируется после мока `invoke` (тот же приём, что useSidecarCheck.test.ts).
const { cancelProbe, probeUrl, useLinkProbe } = await import('./useProbe')

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

const resultA: ProbeResult = {
  title: 'Ролик A',
  durationSecs: 120,
  qualities: [{ kind: 'audioOnly', size: { kind: 'unknown' }, streams: { audioFormatId: 'a' } }],
}

const resultB: ProbeResult = {
  title: 'Ролик B',
  durationSecs: 90,
  qualities: [{ kind: 'audioOnly', size: { kind: 'unknown' }, streams: { audioFormatId: 'b' } }],
}

const notAUrlError: ProbeError = { kind: 'notAUrl', message: 'not used by UI' }

beforeEach(() => {
  vi.useFakeTimers()
  invokeMock.mockReset()
})

afterEach(() => {
  vi.useRealTimers()
})

describe('probeUrl / cancelProbe', () => {
  it('calls probe_url with the url argument', async () => {
    invokeMock.mockResolvedValueOnce(resultA)
    await probeUrl('https://youtu.be/x')
    expect(invokeMock).toHaveBeenCalledExactlyOnceWith('probe_url', { url: 'https://youtu.be/x' })
  })

  it('calls cancel_probe with no arguments', async () => {
    invokeMock.mockResolvedValueOnce(undefined)
    await cancelProbe()
    expect(invokeMock).toHaveBeenCalledExactlyOnceWith('cancel_probe')
  })
})

describe('useLinkProbe — instant check (no debounce) vs. debounced probe start', () => {
  it('starts empty and touches invoke for nothing', () => {
    const { result } = withSetup(() => useLinkProbe())
    expect(result.state.value).toStrictEqual({ kind: 'empty' })
    expect(invokeMock).not.toHaveBeenCalled()
  })

  it('treats a whitespace-only field as empty, not as notAUrl', async () => {
    const { result } = withSetup(() => useLinkProbe())
    result.url.value = '   '
    await vi.waitFor(() => {})
    expect(result.state.value).toStrictEqual({ kind: 'empty' })
    expect(invokeMock).not.toHaveBeenCalled()
  })

  it('shows notAUrl immediately (no debounce, no process launch) for non-url input, including a leading-dash string', async () => {
    const { result } = withSetup(() => useLinkProbe())

    result.url.value = 'просто текст'
    await Promise.resolve()
    expect(result.state.value).toStrictEqual({ kind: 'notAUrl' })
    expect(invokeMock).not.toHaveBeenCalled()

    result.url.value = '-о--'
    await Promise.resolve()
    expect(result.state.value).toStrictEqual({ kind: 'notAUrl' })
    expect(invokeMock).not.toHaveBeenCalled()

    // Даже если подождать полный период дебаунса, процесс не запускается —
    // ветка «не ссылка» вообще не планирует вызов probe_url.
    await vi.advanceTimersByTimeAsync(1000)
    expect(invokeMock).not.toHaveBeenCalled()
  })

  it('does not call probe_url before ~400ms of input silence for url-like content, even though the card area already shows "loading"', async () => {
    // Дизайн: «карточная область немедленно показывает» решение по полю —
    // старая карточка/ошибка убирается сразу, не дожидаясь тишины ввода.
    // Debounce задерживает только сам вызов probe_url (не спамить yt-dlp
    // на каждое нажатие), а не переход экрана в «получаем данные…».
    const { result } = withSetup(() => useLinkProbe())
    invokeMock.mockReturnValue(new Promise(() => {}))

    result.url.value = 'https://www.youtube.com/watch?v=x'
    await Promise.resolve()

    expect(result.state.value).toStrictEqual({ kind: 'loading', slow: false })
    expect(invokeMock).not.toHaveBeenCalled()

    await vi.advanceTimersByTimeAsync(399)
    expect(invokeMock).not.toHaveBeenCalled()

    await vi.advanceTimersByTimeAsync(1)
    expect(invokeMock).toHaveBeenCalledExactlyOnceWith('probe_url', {
      url: 'https://www.youtube.com/watch?v=x',
    })
    expect(result.state.value).toStrictEqual({ kind: 'loading', slow: false })
  })

  it('replaces a stale card/error with "loading" the instant the field becomes url-like again, without waiting for the debounce', async () => {
    const err: ProbeError = { kind: 'networkUnavailable', message: 'offline' }
    const { result } = withSetup(() => useLinkProbe())
    invokeMock.mockRejectedValueOnce(err)

    result.url.value = 'https://youtu.be/a'
    await vi.advanceTimersByTimeAsync(400)
    await vi.waitFor(() => {
      expect(result.state.value.kind).toBe('error')
    })

    invokeMock.mockReturnValue(new Promise(() => {}))
    result.url.value = 'https://youtu.be/b'
    await Promise.resolve()

    // Мгновенно — до истечения debounce и до вызова probe_url.
    expect(result.state.value).toStrictEqual({ kind: 'loading', slow: false })
    expect(invokeMock).toHaveBeenCalledTimes(1)
  })

  it('does not restart the debounce timer on every keystroke while the url keeps changing', async () => {
    const { result } = withSetup(() => useLinkProbe())
    invokeMock.mockReturnValue(new Promise(() => {}))

    result.url.value = 'https://www.youtube.com/watch?v=x'
    await Promise.resolve()
    await vi.advanceTimersByTimeAsync(300)
    result.url.value = 'https://www.youtube.com/watch?v=xy'
    await Promise.resolve()
    await vi.advanceTimersByTimeAsync(300)
    // 600ms прошло суммарно, но с последнего изменения — только 300мс.
    expect(invokeMock).not.toHaveBeenCalled()

    await vi.advanceTimersByTimeAsync(100)
    expect(invokeMock).toHaveBeenCalledExactlyOnceWith('probe_url', {
      url: 'https://www.youtube.com/watch?v=xy',
    })
  })

  it('shows the secondary "slow" hint after ~6s of the probe still running', async () => {
    const { result } = withSetup(() => useLinkProbe())
    invokeMock.mockReturnValue(new Promise(() => {}))

    result.url.value = 'https://www.youtube.com/watch?v=x'
    await vi.advanceTimersByTimeAsync(400)
    expect(result.state.value).toStrictEqual({ kind: 'loading', slow: false })

    await vi.advanceTimersByTimeAsync(5999)
    expect(result.state.value).toStrictEqual({ kind: 'loading', slow: false })

    await vi.advanceTimersByTimeAsync(1)
    expect(result.state.value).toStrictEqual({ kind: 'loading', slow: true })
  })
})

describe('useLinkProbe — success and error resolution', () => {
  it('resolves into a success state carrying the probe result', async () => {
    const { result } = withSetup(() => useLinkProbe())
    invokeMock.mockResolvedValueOnce(resultA)

    result.url.value = 'https://youtu.be/a'
    await vi.advanceTimersByTimeAsync(400)
    await vi.waitFor(() => {
      expect(result.state.value.kind).toBe('success')
    })
    expect(result.state.value).toStrictEqual({ kind: 'success', result: resultA })
  })

  it('resolves a contractual rejection into an error state carrying the typed ProbeError as-is', async () => {
    const { result } = withSetup(() => useLinkProbe())
    const err: ProbeError = { kind: 'videoUnavailable', message: 'core message' }
    invokeMock.mockRejectedValueOnce(err)

    result.url.value = 'https://youtu.be/gone'
    await vi.advanceTimersByTimeAsync(400)
    await vi.waitFor(() => {
      expect(result.state.value.kind).toBe('error')
    })
    expect(result.state.value).toStrictEqual({ kind: 'error', error: err })
  })

  it('turns a non-contractual rejection (plain Error) into a message-only failure, not a crash', async () => {
    const { result } = withSetup(() => useLinkProbe())
    invokeMock.mockRejectedValueOnce(new Error('IPC exploded'))

    result.url.value = 'https://youtu.be/oops'
    await vi.advanceTimersByTimeAsync(400)
    await vi.waitFor(() => {
      expect(result.state.value.kind).toBe('error')
    })
    expect(result.state.value).toStrictEqual({ kind: 'error', error: { message: 'IPC exploded' } })
  })

  it('reduces a notAUrl rejection from the core to the same notAUrl state as the instant front-end check, not an error block (blocker fix)', async () => {
    // Путь реальный, не теоретический (ревью TL-33): фронтовая проверка —
    // не полная валидация (Ф-2 отдаёт её Rust), значит ядро может честно
    // вернуть notAUrl даже для содержимого, которое фронт счёл похожим на
    // ссылку и на котором уже начался разбор.
    const { result } = withSetup(() => useLinkProbe())
    const err: ProbeError = { kind: 'notAUrl', message: 'core: not a supported url shape' }
    invokeMock.mockRejectedValueOnce(err)

    result.url.value = 'https://y.y'
    await vi.advanceTimersByTimeAsync(400)
    await vi.waitFor(() => {
      expect(result.state.value.kind).toBe('notAUrl')
    })
    expect(result.state.value).toStrictEqual({ kind: 'notAUrl' })
  })
})

describe('useLinkProbe — сторож по поколениям (К-4, TL-27 review)', () => {
  it('discards a resolve for a probe superseded before it settled — the second link wins even if the first finishes later', async () => {
    const { result } = withSetup(() => useLinkProbe())

    let resolveA: (value: ProbeResult) => void = () => {}
    invokeMock.mockImplementationOnce(
      () =>
        new Promise<ProbeResult>((resolve) => {
          resolveA = resolve
        }),
    )

    result.url.value = 'https://youtu.be/a'
    await vi.advanceTimersByTimeAsync(400)
    expect(invokeMock).toHaveBeenCalledTimes(1)
    expect(result.state.value).toStrictEqual({ kind: 'loading', slow: false })

    // Вторая ссылка — до готовности первой.
    invokeMock.mockResolvedValueOnce(resultB)
    result.url.value = 'https://youtu.be/b'
    await vi.advanceTimersByTimeAsync(400)
    expect(invokeMock).toHaveBeenCalledTimes(2)

    await vi.waitFor(() => {
      expect(result.state.value).toStrictEqual({ kind: 'success', result: resultB })
    })

    // Ядро «честно» отвечает на первую ссылку уже после того, как экран
    // показывает вторую — устаревшее поколение должно быть отброшено
    // целиком, экран обязан остаться про B.
    resolveA(resultA)
    await Promise.resolve()
    await Promise.resolve()
    expect(result.state.value).toStrictEqual({ kind: 'success', result: resultB })
  })

  it('discards a reject for a superseded probe the same way it discards a resolve', async () => {
    const { result } = withSetup(() => useLinkProbe())

    let rejectA: (err: unknown) => void = () => {}
    invokeMock.mockImplementationOnce(
      () =>
        new Promise((_resolve, reject) => {
          rejectA = reject
        }),
    )

    result.url.value = 'https://youtu.be/a'
    await vi.advanceTimersByTimeAsync(400)

    invokeMock.mockResolvedValueOnce(resultB)
    result.url.value = 'https://youtu.be/b'
    await vi.advanceTimersByTimeAsync(400)

    await vi.waitFor(() => {
      expect(result.state.value).toStrictEqual({ kind: 'success', result: resultB })
    })

    rejectA(notAUrlError)
    await Promise.resolve()
    await Promise.resolve()
    // Отклонённое устаревшее поколение не должно затереть успех B ошибкой.
    expect(result.state.value).toStrictEqual({ kind: 'success', result: resultB })
  })

  it('keys the guard on a monotonic counter, not on the URL: the same link probed twice in a row still runs twice', async () => {
    const { result } = withSetup(() => useLinkProbe())
    invokeMock.mockResolvedValueOnce(resultA)

    result.url.value = 'https://youtu.be/same'
    await vi.advanceTimersByTimeAsync(400)
    await vi.waitFor(() => {
      expect(result.state.value.kind).toBe('success')
    })
    expect(invokeMock).toHaveBeenCalledTimes(1)

    result.url.value = ''
    await Promise.resolve()
    expect(result.state.value).toStrictEqual({ kind: 'empty' })

    invokeMock.mockResolvedValueOnce(resultA)
    result.url.value = 'https://youtu.be/same'
    await vi.advanceTimersByTimeAsync(400)
    await vi.waitFor(() => {
      expect(invokeMock).toHaveBeenCalledTimes(2)
    })
    expect(result.state.value).toStrictEqual({ kind: 'success', result: resultA })
  })

  it('does not overwrite state from a discarded stale generation even when it resolves after several supersessions', async () => {
    const { result } = withSetup(() => useLinkProbe())

    const resolvers: Array<(value: ProbeResult) => void> = []
    invokeMock.mockImplementation(
      () =>
        new Promise<ProbeResult>((resolve) => {
          resolvers.push(resolve)
        }),
    )

    result.url.value = 'https://youtu.be/1'
    await vi.advanceTimersByTimeAsync(400)
    result.url.value = 'https://youtu.be/2'
    await vi.advanceTimersByTimeAsync(400)
    result.url.value = 'https://youtu.be/3'
    await vi.advanceTimersByTimeAsync(400)

    expect(resolvers).toHaveLength(3)

    // Разрешаем в обратном порядке — только третье (текущее) поколение
    // должно повлиять на состояние.
    resolvers[0]?.({ ...resultA, title: 'первая' })
    await Promise.resolve()
    expect(result.state.value.kind).toBe('loading')

    resolvers[1]?.({ ...resultA, title: 'вторая' })
    await Promise.resolve()
    expect(result.state.value.kind).toBe('loading')

    resolvers[2]?.({ ...resultA, title: 'третья' })
    await vi.waitFor(() => {
      expect(result.state.value.kind).toBe('success')
    })
    expect(result.state.value).toStrictEqual({
      kind: 'success',
      result: { ...resultA, title: 'третья' },
    })
  })
})

describe('useLinkProbe — единое правило отмены и «Экономия вызовов»', () => {
  it('calls cancel_probe when clearing the field while a probe is in flight', async () => {
    const { result } = withSetup(() => useLinkProbe())
    invokeMock.mockImplementation(() => new Promise(() => {}))

    result.url.value = 'https://youtu.be/a'
    await vi.advanceTimersByTimeAsync(400)
    expect(invokeMock).toHaveBeenCalledTimes(1)

    invokeMock.mockResolvedValueOnce(undefined)
    result.url.value = ''
    await Promise.resolve()

    expect(result.state.value).toStrictEqual({ kind: 'empty' })
    expect(invokeMock).toHaveBeenCalledWith('cancel_probe')
  })

  it('calls cancel_probe when the field becomes non-url-like while a probe is in flight', async () => {
    const { result } = withSetup(() => useLinkProbe())
    invokeMock.mockImplementation(() => new Promise(() => {}))

    result.url.value = 'https://youtu.be/a'
    await vi.advanceTimersByTimeAsync(400)

    invokeMock.mockResolvedValueOnce(undefined)
    result.url.value = 'больше не ссылка'
    await Promise.resolve()

    expect(result.state.value).toStrictEqual({ kind: 'notAUrl' })
    expect(invokeMock).toHaveBeenCalledWith('cancel_probe')
  })

  it('does NOT call cancel_probe when replacing one url-like value with another (core self-cancels via probe_url, Ф-8)', async () => {
    const { result } = withSetup(() => useLinkProbe())
    invokeMock.mockImplementation(() => new Promise(() => {}))

    result.url.value = 'https://youtu.be/a'
    await vi.advanceTimersByTimeAsync(400)
    result.url.value = 'https://youtu.be/b'
    await vi.advanceTimersByTimeAsync(400)

    expect(invokeMock.mock.calls.some(([cmd]) => cmd === 'cancel_probe')).toBe(false)
  })

  it('does not call cancel_probe when clearing an already-empty/idle field (no probe was running)', async () => {
    const { result } = withSetup(() => useLinkProbe())

    result.url.value = 'просто текст'
    await Promise.resolve()
    result.url.value = ''
    await Promise.resolve()

    expect(invokeMock).not.toHaveBeenCalled()
  })
})

describe('useLinkProbe — retry', () => {
  it('re-probes the current field value immediately, without waiting for the debounce', async () => {
    const { result } = withSetup(() => useLinkProbe())
    const err: ProbeError = { kind: 'networkUnavailable', message: 'offline' }
    invokeMock.mockRejectedValueOnce(err)

    result.url.value = 'https://youtu.be/a'
    await vi.advanceTimersByTimeAsync(400)
    await vi.waitFor(() => {
      expect(result.state.value.kind).toBe('error')
    })

    invokeMock.mockResolvedValueOnce(resultA)
    result.retry()
    // Немедленно после retry() — уже в состоянии «получаем данные», без 400мс ожидания.
    expect(result.state.value).toStrictEqual({ kind: 'loading', slow: false })
    expect(invokeMock).toHaveBeenCalledTimes(2)

    await vi.waitFor(() => {
      expect(result.state.value).toStrictEqual({ kind: 'success', result: resultA })
    })
  })

  it('bumps the generation on retry() — a stale response from before the retry click is discarded', async () => {
    const { result } = withSetup(() => useLinkProbe())

    let resolveFirst: (value: ProbeResult) => void = () => {}
    invokeMock.mockImplementationOnce(
      () =>
        new Promise<ProbeResult>((resolve) => {
          resolveFirst = resolve
        }),
    )

    result.url.value = 'https://youtu.be/a'
    await vi.advanceTimersByTimeAsync(400)
    expect(invokeMock).toHaveBeenCalledTimes(1)

    invokeMock.mockResolvedValueOnce(resultB)
    result.retry()
    expect(invokeMock).toHaveBeenCalledTimes(2)
    await vi.waitFor(() => {
      expect(result.state.value).toStrictEqual({ kind: 'success', result: resultB })
    })

    // Ответ на самый первый (уже вытесненный retry'ем) вызов приходит
    // позже — он обязан быть отброшен тем же сторожем по поколениям.
    resolveFirst(resultA)
    await Promise.resolve()
    await Promise.resolve()
    expect(result.state.value).toStrictEqual({ kind: 'success', result: resultB })
  })
})

describe('useLinkProbe — очистка при размонтировании', () => {
  it('does not fire the debounced probe after the component unmounts', async () => {
    const { result, unmount } = withSetup(() => useLinkProbe())

    result.url.value = 'https://youtu.be/a'
    unmount()

    await vi.advanceTimersByTimeAsync(1000)
    expect(invokeMock).not.toHaveBeenCalled()
  })

  it('does not fire the "slow" hint timer after the component unmounts', async () => {
    const { result, unmount } = withSetup(() => useLinkProbe())
    invokeMock.mockReturnValue(new Promise(() => {}))

    result.url.value = 'https://youtu.be/a'
    await vi.advanceTimersByTimeAsync(400)
    expect(result.state.value).toStrictEqual({ kind: 'loading', slow: false })

    unmount()
    // Размонтирование само по себе зовёт cancel_probe (см. следующий тест)
    // — единственный вызов invoke сверх исходного probe_url. Значимая
    // проверка здесь — что после unmount больше НИЧЕГО не прибавляется:
    // ни отложенный debounce, ни 6-секундный таймер "slow" не должны были
    // пережить очистку и вызвать что-то ещё.
    const callsRightAfterUnmount = invokeMock.mock.calls.length
    await vi.advanceTimersByTimeAsync(10_000)
    expect(invokeMock.mock.calls.length).toBe(callsRightAfterUnmount)
  })

  it('calls cancel_probe on unmount when a probe is in flight (Ф-8, "отмена обязательна на каждом этапе")', async () => {
    const { result, unmount } = withSetup(() => useLinkProbe())
    invokeMock.mockImplementation(() => new Promise(() => {}))

    result.url.value = 'https://youtu.be/a'
    await vi.advanceTimersByTimeAsync(400)
    expect(invokeMock).toHaveBeenCalledTimes(1)

    invokeMock.mockResolvedValueOnce(undefined)
    unmount()
    await Promise.resolve()

    expect(invokeMock).toHaveBeenCalledWith('cancel_probe')
  })
})
