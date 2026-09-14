import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { onlyVisible } from '@/test-utils/visiblePanel'
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

const okDeno: SidecarCheckResult = {
  name: 'deno',
  path: '/opt/tube-leak/bin/deno',
  status: 'ok',
  version: '2.9.6',
}

const okReport: SidecarCheckReport = { ytDlp: okYtDlp, ffmpeg: okFfmpeg, deno: okDeno }

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
 *
 * `queue_state` (эпик E4, TL-75) — тем же приёмом: `App.vue` вызывает
 * `downloadTaskStore.initialize()` в собственном `onMounted` независимо
 * от экрана подготовки/sidecar (см. doc `App.vue`), и без дефолта здесь
 * каждый тест этого файла (не про очередь) был бы обязан его мокать —
 * вечно висящий промис держит очередь в нейтральном «списка ещё нет».
 *
 * `history_page` (эпик E5, TL-93) — тем же приёмом: `HistoryScreen`
 * рендерится безусловно под вкладкой «История» (`v-show`, К-14) и вызывает
 * `useHistoryStore().initialize()` в своём `onMounted`, который срабатывает
 * сразу при монтаже `App.vue`, а не только когда пользователь переключится
 * на «Историю» — без дефолта здесь этот файл (не про историю) был бы обязан
 * мокать и её.
 *
 * `settings_get` (эпик E5, TL-94) — тем же приёмом: `SettingsScreen`
 * рендерится безусловно под вкладкой «Настройки» (`v-show`, К-14) и вызывает
 * `useSettingsStore().fetchSettings()` в своём `onMounted`, который тоже
 * срабатывает сразу при монтаже `App.vue`.
 */
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

/**
 * Точный набор подписей кнопок на экране (ревью TL-59 «Н-2»): замена
 * трёх `find('button').exists() === false` на отсутствие конкретно
 * «Повторить проверку» была вынужденной (блок «Обновление yt-dlp», TL-59,
 * всегда рисует «Проверить сейчас») — но `some(...) === false` ловит
 * только эту одну подпись и молчит про любую другую постороннюю кнопку.
 * `toStrictEqual` на полном списке возвращает исходную силу: посторонняя
 * кнопка меняет список и ассерт падает.
 *
 * `role="tab"` исключены (TL-92): панель вкладок «Главный/История/
 * Настройки» рендерится безусловно поверх всего экрана (дизайн E5
 * «Навигация») и не относится к тому, что проверяет этот файл, — набор
 * вкладок свой собственный тест (`App.tabs.test.ts`).
 *
 * `onlyVisible` (правки ревью TL-92, Н-6) — панели переключаются `v-show`,
 * не `v-if` (К-14), значит «Главный» не единственная секция в DOM: как
 * только TL-93/TL-94 положат в плейсхолдеры «Истории»/«Настроек» настоящие
 * кнопки, без этого фильтра `buttonLabels` начал бы видеть их тоже —
 * несмотря на то, что панели с ними не видны, пока активна «Главный», ни
 * этому файлу, ни его тестам они не нужны. Фильтр по видимости решает это
 * раз и навсегда, вместо того чтобы «чинить» список фильтрами по классу
 * компонента при каждом новом экране.
 */
function buttonLabels(wrapper: ReturnType<typeof mount>): string[] {
  return onlyVisible(wrapper.findAll('button'))
    .filter((b) => b.attributes('role') !== 'tab')
    .map((b) => b.text())
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
    expect(wrapper.text()).toContain('2.9.6')
    expect(wrapper.text()).not.toContain('Распаковываем')
    expect(wrapper.text()).not.toContain('Готовим yt-dlp')
  })
})

describe('App — first-run preparation (unpacking → warmingUp → ready)', () => {
  it('shows the service screen (checking) before the first event, then progress per stage, then the service screen again', async () => {
    // Композиция «starting = ready» — решение ревью TL-17 (#18,
    // «Композиция тёплого старта»): до первого события экран — та же
    // раскладка, что и после готовности (шапка с версией, все три строки
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

    // До первого события — не пустое окно: версия и все три строки sidecar
    // видны сразу (Ф-9/Н-6), check_sidecar при этом ещё не вызван (см.
    // блок «order of calls»).
    expect(wrapper.text()).toContain('версия 0.1.0')
    expect(wrapper.text().match(/Проверяем…/g)).toHaveLength(3)

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

  it('renders the title immediately, with all three rows Checking before check_sidecar resolves (Н-6)', async () => {
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
    expect(wrapper.text()).toContain('deno')
    expect(wrapper.text().match(/Проверяем…/g)).toHaveLength(3)
    // Не «нет ни одной кнопки вовсе» — блок «Обновление yt-dlp» (TL-59)
    // всегда рисует «Проверить сейчас» (неактивной, пока свой снимок не
    // пришёл, см. doc `routeInvoke` выше); точный список подписей — не
    // «отсутствует конкретно „Повторить проверку“» (doc `buttonLabels`).
    expect(buttonLabels(wrapper)).toStrictEqual(['Проверить сейчас'])

    resolveCheck(okReport)
    await flushPromises()
  })

  it('hides the retry button when all three rows resolve Ok', async () => {
    const wrapper = mount(App)
    await flushPromises()

    expect(wrapper.text()).toContain('2026.08.20')
    expect(wrapper.text()).toContain('7.1')
    expect(wrapper.text()).toContain('2.9.6')
    // См. doc-комментарий у предыдущего теста и `buttonLabels` — точный
    // список, не отсутствие одной конкретной подписи.
    expect(buttonLabels(wrapper)).toStrictEqual(['Проверить сейчас'])
  })

  it('shows the retry button when at least one row is not Ok, for a mixed ok/timeout report', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve({ ytDlp: okYtDlp, ffmpeg: timeoutFfmpeg, deno: okDeno }),
    })

    const wrapper = mount(App)
    await flushPromises()

    expect(wrapper.text()).toContain('2026.08.20')
    expect(wrapper.text()).toContain('не отвечает')

    const retryButton = wrapper.findAll('button').find((b) => b.text().includes('Повторить проверку'))
    expect(retryButton).toBeDefined()
  })

  it('shows the retry button when both yt-dlp and ffmpeg are in error states', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve({ ytDlp: notFoundYtDlp, ffmpeg: timeoutFfmpeg, deno: okDeno }),
    })

    const wrapper = mount(App)
    await flushPromises()

    const retryButton = wrapper.findAll('button').find((b) => b.text().includes('Повторить проверку'))
    expect(retryButton).toBeDefined()
  })

  /**
   * TL-111: doc `showRetry` в App.vue меняет условие с «yt-dlp ИЛИ ffmpeg
   * не ok» на «хотя бы одна из трёх строк не ok» (`SIDECAR_REPORT_KEYS`) —
   * без этого теста мутация, вернувшая проверку только двух старых полей,
   * прошла бы: yt-dlp и ffmpeg здесь оба `ok`, отказавший — только deno.
   */
  it('shows the retry button when only deno is not Ok, even with yt-dlp and ffmpeg both Ok', async () => {
    const launchFailedDeno: SidecarCheckResult = {
      name: 'deno',
      path: '/opt/tube-leak/bin/deno',
      status: 'launchFailed',
      reason: 'unrecognizedOutput',
      stderrTail: 'Deno 1.0\n',
    }
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve({ ytDlp: okYtDlp, ffmpeg: okFfmpeg, deno: launchFailedDeno }),
    })

    const wrapper = mount(App)
    await flushPromises()

    expect(wrapper.text()).toContain('неожиданный ответ')

    // `stderrTail` живёт в свёрнутом по умолчанию блоке «Подробнее»
    // (`SidecarStatusRow`, doc `details`) — раскрываем его тем же приёмом,
    // что `SidecarStatusRow.test.ts`.
    const detailsButton = wrapper.findAll('button').find((b) => b.text().includes('Подробнее'))
    expect(detailsButton).toBeDefined()
    await detailsButton?.trigger('click')
    expect(wrapper.text()).toContain('Deno 1.0')

    const retryButton = wrapper.findAll('button').find((b) => b.text().includes('Повторить проверку'))
    expect(retryButton).toBeDefined()
  })

  it('shows the deno row not found the same way as the other rows', async () => {
    const notFoundDeno: SidecarCheckResult = {
      name: 'deno',
      path: '/opt/tube-leak/bin/deno',
      status: 'notFound',
      osErrorCode: 'ENOENT',
    }
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve({ ytDlp: okYtDlp, ffmpeg: okFfmpeg, deno: notFoundDeno }),
    })

    const wrapper = mount(App)
    await flushPromises()

    expect(wrapper.text()).toContain('не найден')
    expect(wrapper.text()).toContain('Не нашли файл deno по ожидаемому пути')

    const retryButton = wrapper.findAll('button').find((b) => b.text().includes('Повторить проверку'))
    expect(retryButton).toBeDefined()
  })

  it('re-invokes check_sidecar (not prepare_ytdlp again) when the retry button is clicked', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve({ ytDlp: notFoundYtDlp, ffmpeg: okFfmpeg, deno: okDeno }),
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
    // вечно, см. doc `routeInvoke`), queue_state (эпик E4, TL-75 — тоже
    // висит вечно), history_page (эпик E5, TL-93 — тоже висит вечно),
    // settings_get (эпик E5, TL-94 — тоже висит вечно), check_sidecar × 2.
    expect(invokeMock).toHaveBeenCalledTimes(7)
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
      check_sidecar: () => Promise.resolve({ ytDlp: okYtDlp, ffmpeg: timeoutFfmpeg, deno: okDeno }),
    })

    const wrapper = mount(App)
    await flushPromises()

    const input = wrapper.find('input')
    expect(input.attributes('disabled')).toBeUndefined()
  })

  /**
   * TL-111 (#114): deno не блокирует поле ссылки так же, как ffmpeg —
   * разбор без рантайма деградирует (часть форматов пропадает), а не
   * отказывает целиком. Без этого теста мутация, добавившая проверку
   * `report.deno.status` в `ytDlpState`, прошла бы незамеченной.
   */
  it('enables the link field once yt-dlp is ok, even if deno is not', async () => {
    const notFoundDeno: SidecarCheckResult = {
      name: 'deno',
      path: '/opt/tube-leak/bin/deno',
      status: 'notFound',
      osErrorCode: 'ENOENT',
    }
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve({ ytDlp: okYtDlp, ffmpeg: okFfmpeg, deno: notFoundDeno }),
    })

    const wrapper = mount(App)
    await flushPromises()

    const input = wrapper.find('input')
    expect(input.attributes('disabled')).toBeUndefined()
  })

  it('disables the link field with a hint (not repeating the yt-dlp row error text) when yt-dlp is not ok', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve({ ytDlp: notFoundYtDlp, ffmpeg: okFfmpeg, deno: okDeno }),
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
