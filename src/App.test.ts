import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { SidecarCheckReport, SidecarCheckResult } from '@/types/generated/sidecar'
import type { YtDlpPrepareError, YtDlpPrepareEvent, YtDlpPrepared } from '@/types/generated/ytdlp'

const invokeMock = vi.fn()
const unlistenMock = vi.fn()
type EventHandler = (event: { payload: YtDlpPrepareEvent }) => void
let capturedHandler: EventHandler | undefined

/**
 * `App.vue` подписывается на два независимых канала событий: `ytdlp://prepare`
 * (TL-17, этот файл) и `ytdlp://update` (TL-59, блок «Обновление yt-dlp» —
 * не тема этого файла, см. `useYtDlpUpdate.test.ts`/`YtDlpUpdateBlock.test.ts`).
 * Роутинг по имени события обязателен: `useYtDlpUpdate`'s `onMounted`
 * регистрируется раньше, чем `App.vue` вызывает `prepare()` из своего
 * собственного `onMounted` (порядок регистрации hooks в setup), поэтому
 * первый вызов `listen()` при монтаже — за каналом обновления, не за
 * `ytdlp://prepare`; общий безусловный `capturedHandler = handler` без
 * различения событий подставил бы сюда не тот обработчик.
 */
function defaultListenImpl(
  event: string,
  handler: (event: unknown) => void,
): Promise<typeof unlistenMock> {
  if (event === 'ytdlp://prepare') {
    capturedHandler = handler as EventHandler
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

// `useExitConfirmation` (TL-46) вызывается на верхнем уровне `App.vue` и
// подписывается на настоящее оконное событие через `windowExitPort.ts` —
// без мока `getCurrentWindow().onCloseRequested()` бросит на монтаже
// (в jsdom нет `window.__TAURI_INTERNALS__`). Этот файл не про диалог
// выхода (см. `App.download.test.ts`), поэтому подписка здесь просто
// не резолвится — она не нужна ни одному тесту в этом файле.
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    onCloseRequested: () => new Promise<() => void>(() => {}),
    destroy: () => Promise.resolve(),
  }),
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

/**
 * Роутер `invoke` по имени команды — так же ведёт себя настоящий Tauri IPC.
 *
 * `ytdlp_update_state` (TL-59) замешана сюда дефолтом, который никогда не
 * разрешается: блок «Обновление yt-dlp» — не тема этого файла (см. doc
 * `listenMock` выше), а вечно висящий промис держит его в нейтральном
 * состоянии «Загружаем статус обновления…», не мешая ни одному из
 * существующих здесь ассертов. Явно переданный в `handlers` обработчик
 * той же команды имеет приоритет — по тому же принципу, что и остальные.
 */
function routeInvoke(handlers: Record<string, () => Promise<unknown>>) {
  const withDefaults: Record<string, () => Promise<unknown>> = {
    ytdlp_update_state: () => new Promise<unknown>(() => {}),
    ...handlers,
  }
  invokeMock.mockImplementation((command: string) => {
    const handler = withDefaults[command]
    if (!handler) throw new Error(`unexpected invoke: ${command}`)
    return handler()
  })
}

/**
 * Точный набор подписей кнопок на экране (ревью TL-59 «Н-2»): замена
 * трёх `find('button').exists() === false` на отсутствие конкретно
 * «Повторить проверку» была вынужденной (блок «Обновление yt-dlp», TL-59,
 * всегда рисует «Проверить сейчас») — но `some(...) === false` ловит
 * только эту одну подпись и молчит про любую другую постороннюю кнопку.
 * `toStrictEqual` на полном списке возвращает исходную силу: посторонняя
 * кнопка меняет список и ассерт падает.
 */
function buttonLabels(wrapper: ReturnType<typeof mount>): string[] {
  return wrapper.findAll('button').map((b) => b.text())
}

beforeEach(() => {
  invokeMock.mockReset()
  listenMock.mockReset()
  listenMock.mockImplementation(defaultListenImpl)
  unlistenMock.mockClear()
  capturedHandler = undefined
  // App.vue использует `useDownloadTaskStore` (эпик E3, TL-45) — стору
  // нужен активный Pinia-инстанс, иначе `useStore()` падает ещё на mount.
  setActivePinia(createPinia())
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
    // Только канал `ytdlp://prepare` отложен здесь: `ytdlp://update`
    // (TL-59, не тема этого теста) продолжает разрешаться сразу же через
    // `defaultListenImpl`, иначе `mockImplementationOnce` перехватил бы
    // первый вызов `listen()` при монтаже — а это вызов за каналом
    // обновления (см. doc `defaultListenImpl` выше), не за prepare.
    let resolveListen: (fn: typeof unlistenMock) => void = () => {}
    listenMock.mockImplementation((event: string, handler: (event: unknown) => void) => {
      if (event !== 'ytdlp://prepare') {
        return defaultListenImpl(event, handler)
      }
      capturedHandler = handler as EventHandler
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

    expect(listenMock.mock.calls.filter(([event]) => event === 'ytdlp://prepare')).toHaveLength(1)
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
    // Не «нет ни одной кнопки вовсе» — блок «Обновление yt-dlp» (TL-59)
    // всегда рисует «Проверить сейчас» (неактивной, пока свой снимок не
    // пришёл, см. doc `routeInvoke` выше); точный список подписей — не
    // «отсутствует конкретно „Повторить проверку“» (doc `buttonLabels`).
    expect(buttonLabels(wrapper)).toStrictEqual(['Проверить сейчас'])

    resolveCheck(okReport)
    await flushPromises()
  })

  it('hides the retry button when both rows resolve Ok', async () => {
    const wrapper = mount(App)
    await flushPromises()

    expect(wrapper.text()).toContain('2026.08.20')
    expect(wrapper.text()).toContain('7.1')
    // См. doc-комментарий у предыдущего теста и `buttonLabels` — точный
    // список, не отсутствие одной конкретной подписи.
    expect(buttonLabels(wrapper)).toStrictEqual(['Проверить сейчас'])
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

    // prepare_ytdlp, ytdlp_update_state (TL-59, блок обновления — висит
    // вечно, см. doc `routeInvoke`), check_sidecar × 2.
    expect(invokeMock).toHaveBeenCalledTimes(4)
    expect(invokeMock.mock.calls.filter(([cmd]) => cmd === 'prepare_ytdlp')).toHaveLength(1)
    expect(buttonLabels(wrapper)).toStrictEqual(['Проверить сейчас'])
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
