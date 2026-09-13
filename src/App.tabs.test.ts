import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { DownloadProgressEvent, DownloadStarted } from '@/types/generated/download'
import type { ProbeResult } from '@/types/generated/probe'
import type { QueueSnapshot } from '@/types/generated/queue'
import type { Settings, SettingsView } from '@/types/generated/settings'
import type { SidecarCheckReport } from '@/types/generated/sidecar'
import type { YtDlpPrepared } from '@/types/generated/ytdlp'

/** Настройки по умолчанию (Ф-10) — для тестов, которым содержимое экрана «Настройки» безразлично. */
const DEFAULT_SETTINGS: Settings = { destinationFolder: { kind: 'system' }, nameTemplate: '{title}', maxAttempts: 8 }
const DEFAULT_SETTINGS_VIEW: SettingsView = {
  settings: DEFAULT_SETTINGS,
  defaults: DEFAULT_SETTINGS,
  resetFields: [],
  wholeFileReset: false,
  destinationFolderExists: true,
}

/**
 * TL-92 (issue execaus/tube-leak#95, дизайн E5 «Навигация»): панель вкладок
 * «Главный/История/Настройки» и компактная строка состояния очереди.
 * Собственный файл, а не довесок к `App.test.ts` (порядок вызовов E1) или
 * `App.download.test.ts` (очередь E3 → E4) — здесь проверяется именно
 * навигация и то, что переключение вкладки не задевает состояние ни
 * «Главного», ни стора очереди (критерий приёмки эпика К-14).
 *
 * # Второй раунд (правки ревью)
 *
 * Б-1 — клавиатура переведена на «автоматическую активацию» (см.
 * `focusTab`/`pressOnFocused` ниже и doc `activateTabFromKeyboard` в
 * `App.vue`): первая версия тестов слала `keydown` прямо на `tablist`, а
 * не на реально сфокусированный элемент, и не увидела, что второе нажатие
 * стрелки подряд било мимо в настоящем браузере. Б-2 — проверки строки
 * статуса прицельно читают `.queue-status-row`, не `wrapper.text()`
 * целиком (см. doc блока «строка состояния очереди» ниже — скрытая секция
 * «Главного» рисует те же слова). С-1/С-2/Н-1/Н-3/Н-4 — отдельные блоки в
 * конце файла.
 */

const invokeMock = vi.fn()
const unlistenMock = vi.fn()
type Handler = (event: { payload: unknown }) => void
const handlers = new Map<string, Handler>()
const listenMock = vi.fn((eventName: string, handler: Handler) => {
  handlers.set(eventName, handler)
  return Promise.resolve(unlistenMock)
})

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: (...args: [string, Handler]) => listenMock(...args),
}))

/**
 * Диалог выхода (TL-46/TL-47) не главная тема этого файла (полная матрица
 * — `App.download.test.ts`), но правки ревью TL-92 (С-1) требуют доказать,
 * что он работает и с неглавной вкладки — для этого окно замокано по-
 * настоящему (тот же приём, что `App.download.test.ts`), а не подвешенным
 * навечно промисом: обработчик закрытия и `destroy` перехватываются.
 */
type CloseHandler = (event: { preventDefault: () => void }) => void | Promise<void>
let capturedCloseHandler: CloseHandler | undefined
const destroyMock = vi.fn(() => Promise.resolve())
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    onCloseRequested: (handler: CloseHandler) => {
      capturedCloseHandler = handler
      return Promise.resolve(() => {})
    },
    destroy: destroyMock,
  }),
}))

const { default: App } = await import('./App.vue')

const preparedWarm: YtDlpPrepared = {
  version: '2026.08.20',
  path: '/opt/tube-leak/ytdlp/yt-dlp',
  prepared: false,
  durationMs: 120,
}

const okReport: SidecarCheckReport = {
  ytDlp: { name: 'yt-dlp', path: '/opt/tube-leak/bin/yt-dlp', status: 'ok', version: '2026.08.20' },
  ffmpeg: { name: 'ffmpeg', path: '/opt/tube-leak/bin/ffmpeg', status: 'ok', version: '7.1' },
}

const resultA: ProbeResult = {
  title: 'Ролик A',
  durationSecs: 65,
  qualities: [{ kind: 'audioOnly', size: { kind: 'unknown' }, streams: { audioFormatId: 'a' } }],
}

const started: DownloadStarted = { taskId: 'task-1', phase: 'downloading', plan: 'singleStream' }

const EMPTY_QUEUE_SNAPSHOT: QueueSnapshot = { tasks: [], awaitingContinue: false }

function emitProgress(payload: DownloadProgressEvent): void {
  handlers.get('download://progress')?.({ payload })
}

function emitQueueChanged(snapshot: QueueSnapshot): void {
  handlers.get('queue://changed')?.({ payload: snapshot })
}

/** Симулирует попытку пользователя закрыть окно (крестик, Cmd+Q…). */
async function attemptWindowClose(): Promise<void> {
  await capturedCloseHandler?.({ preventDefault: vi.fn() })
}

function routeInvoke(handlersByCommand: Record<string, () => Promise<unknown>>) {
  const withDefaults: Record<string, () => Promise<unknown>> = {
    // Снимок очереди — дефолт «пусто», явно переданный обработчик той же
    // команды имеет приоритет (тот же приём, что `App.download.test.ts`).
    queue_state: () => Promise.resolve(EMPTY_QUEUE_SNAPSHOT),
    // Первая страница истории (эпик E5, TL-93): `HistoryScreen` рендерится
    // безусловно под вкладкой «История» (`v-show`, К-14) и запрашивает её
    // сразу при монтаже `App.vue`, независимо от активной вкладки — дефолт
    // «пусто», явно переданный обработчик той же команды имеет приоритет.
    history_page: () => Promise.resolve({ entries: [], notices: [] }),
    // Настройки (эпик E5, TL-94): `SettingsScreen` рендерится безусловно
    // под вкладкой «Настройки» (`v-show`, К-14) и запрашивает их сразу при
    // монтаже `App.vue`, независимо от активной вкладки — тот же приём,
    // что и у `history_page` выше.
    settings_get: () => Promise.resolve(DEFAULT_SETTINGS_VIEW),
    ...handlersByCommand,
  }
  invokeMock.mockImplementation((command: string) => {
    const handler = withDefaults[command]
    if (!handler) throw new Error(`unexpected invoke: ${command}`)
    return handler()
  })
}

let host: HTMLElement

beforeEach(() => {
  vi.useFakeTimers()
  invokeMock.mockReset()
  listenMock.mockClear()
  unlistenMock.mockClear()
  handlers.clear()
  capturedCloseHandler = undefined
  destroyMock.mockClear()
  setActivePinia(createPinia())
  routeInvoke({
    prepare_ytdlp: () => Promise.resolve(preparedWarm),
    check_sidecar: () => Promise.resolve(okReport),
  })
  // Монтаж в реальный DOM (не detached) — фокус программный
  // (`element.focus()`) виден через `document.activeElement` только когда
  // элемент вставлен в документ (тот же приём, что `ExitConfirmDialog.test.ts`).
  host = document.createElement('div')
  document.body.appendChild(host)
})

afterEach(() => {
  vi.useRealTimers()
  host.remove()
})

async function mountReady() {
  const wrapper = mount(App, { attachTo: host })
  await flushPromises()
  return wrapper
}

/** Печатает ссылку, ждёт карточку и выбирает единственный пункт лестницы — не нажимает «Скачать». */
async function probeAndSelect(wrapper: Awaited<ReturnType<typeof mountReady>>, url: string, result: ProbeResult) {
  invokeMock.mockImplementationOnce((command: string) => {
    if (command === 'probe_url') return Promise.resolve(result)
    throw new Error(`unexpected invoke: ${command}`)
  })
  await wrapper.find('input').setValue(url)
  await vi.advanceTimersByTimeAsync(400)
  await vi.waitFor(() => {
    expect(wrapper.text()).toContain(result.title)
  })
  await wrapper.find('input[type="radio"]').setValue(true)
}

function tabButton(wrapper: Awaited<ReturnType<typeof mountReady>>, label: string) {
  const button = wrapper.findAll('[role="tab"]').find((b) => b.text() === label)
  if (!button) throw new Error(`tab not found: ${label}`)
  return button
}

function tabPanel(wrapper: Awaited<ReturnType<typeof mountReady>>, id: string) {
  const panel = wrapper.find(`#${id}`)
  if (!panel.exists()) throw new Error(`panel not found: ${id}`)
  return panel
}

/** Кладёт настоящий DOM-фокус на кнопку-вкладку по её `id` (`#tab-<id>`). */
function focusTab(wrapper: Awaited<ReturnType<typeof mountReady>>, tabId: string): HTMLElement {
  const el = wrapper.get(`#tab-${tabId}`).element as HTMLElement
  el.focus()
  return el
}

/**
 * Отправляет `keydown` на элемент, реально сфокусированный сейчас
 * (`document.activeElement`), а не на контейнер `tablist` напрямую (Б-1,
 * правки ревью TL-92, второй раунд, doc-комментарий этого файла ниже) —
 * `bubbles: true`, потому что обработчик висит на `.tabs`, и событию
 * нужно всплыть от кнопки до него, как в настоящем браузере.
 */
async function pressOnFocused(key: string): Promise<void> {
  const target = document.activeElement as HTMLElement
  target.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }))
  await flushPromises()
}

describe('App — панель вкладок (TL-92, дизайн E5 «Навигация»)', () => {
  it('renders three tabs, «Главный» selected by default, main content unchanged from E1', async () => {
    const wrapper = await mountReady()

    const tabs = wrapper.findAll('[role="tab"]')
    expect(tabs.map((t) => t.text())).toStrictEqual(['Главный', 'История', 'Настройки'])
    expect(tabButton(wrapper, 'Главный').attributes('aria-selected')).toBe('true')
    expect(tabButton(wrapper, 'История').attributes('aria-selected')).toBe('false')
    expect(tabButton(wrapper, 'Настройки').attributes('aria-selected')).toBe('false')

    expect(tabPanel(wrapper, 'tabpanel-main').isVisible()).toBe(true)
    expect(tabPanel(wrapper, 'tabpanel-history').isVisible()).toBe(false)
    expect(tabPanel(wrapper, 'tabpanel-settings').isVisible()).toBe(false)

    // «Главный» показывает ровно то, что показывал бы без вкладок (дизайн,
    // пункт 1): версия и обе строки sidecar видны сразу, как в E1.
    expect(wrapper.text()).toContain('версия 0.1.0')
    expect(wrapper.text()).toContain('2026.08.20')
    expect(wrapper.text()).toContain('7.1')
  })

  it('tabs render unconditionally even while the yt-dlp prepare screen is up, and «История»/«Настройки» work without waiting for sidecar', async () => {
    let resolvePrepare: (value: YtDlpPrepared) => void = () => {}
    routeInvoke({
      prepare_ytdlp: () =>
        new Promise<YtDlpPrepared>((resolve) => {
          resolvePrepare = resolve
        }),
      check_sidecar: () => Promise.resolve(okReport),
    })

    const wrapper = await mountReady()

    // Всё ещё идёт подготовка — «Главный» показывает служебный экран
    // (Ф-9/Н-6), а вкладки уже на месте и переключаются.
    expect(wrapper.findAll('[role="tab"]')).toHaveLength(3)

    await tabButton(wrapper, 'История').trigger('click')
    await wrapper.vm.$nextTick()

    expect(tabPanel(wrapper, 'tabpanel-history').isVisible()).toBe(true)
    expect(wrapper.text()).toContain('История пуста. Здесь появятся ролики после первой завершённой загрузки.')

    resolvePrepare(preparedWarm)
    await flushPromises()
    // Готовность sidecar не переключает вкладку сама по себе.
    expect(tabPanel(wrapper, 'tabpanel-history').isVisible()).toBe(true)
  })

  it('clicking «История» switches aria-selected/visibility and moves focus to its heading', async () => {
    const wrapper = await mountReady()

    await tabButton(wrapper, 'История').trigger('click')
    await wrapper.vm.$nextTick()

    expect(tabButton(wrapper, 'История').attributes('aria-selected')).toBe('true')
    expect(tabButton(wrapper, 'Главный').attributes('aria-selected')).toBe('false')
    expect(tabPanel(wrapper, 'tabpanel-history').isVisible()).toBe(true)
    expect(tabPanel(wrapper, 'tabpanel-main').isVisible()).toBe(false)
    expect(wrapper.text()).toContain('История пуста. Здесь появятся ролики после первой завершённой загрузки.')

    const heading = tabPanel(wrapper, 'tabpanel-history').get('h2')
    expect(document.activeElement).toBe(heading.element)
  })

  it('clicking «Настройки» switches to its panel and moves focus to its heading', async () => {
    const wrapper = await mountReady()

    await tabButton(wrapper, 'Настройки').trigger('click')
    await wrapper.vm.$nextTick()
    await flushPromises()

    expect(tabButton(wrapper, 'Настройки').attributes('aria-selected')).toBe('true')
    expect(tabPanel(wrapper, 'tabpanel-settings').isVisible()).toBe(true)
    // TL-94: содержимое экрана настроек — `SettingsScreen`, не плейсхолдер TL-92.
    expect(wrapper.text()).toContain('Папка назначения')
    expect(wrapper.text()).toContain('Шаблон имени файла')
    expect(wrapper.text()).toContain('Число попыток')

    const heading = tabPanel(wrapper, 'tabpanel-settings').get('h2')
    expect(document.activeElement).toBe(heading.element)
  })

  it('"На главный" in the status row returns to the main tab and moves focus to the existing h1', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(okReport),
      queue_state: () =>
        Promise.resolve({
          tasks: [
            {
              taskId: 't1',
              title: 'Ролик A',
              quality: { kind: 'audioOnly' },
              plan: 'singleStream',
              phase: 'downloading',
              state: 'running',
              percent: 40,
            },
          ],
          awaitingContinue: false,
        } satisfies QueueSnapshot),
    })
    const wrapper = await mountReady()
    await tabButton(wrapper, 'История').trigger('click')
    await wrapper.vm.$nextTick()

    const backButton = wrapper.findAll('button').find((b) => b.text() === 'На главный')
    expect(backButton).toBeDefined()
    await backButton?.trigger('click')
    await wrapper.vm.$nextTick()

    expect(tabButton(wrapper, 'Главный').attributes('aria-selected')).toBe('true')
    expect(tabPanel(wrapper, 'tabpanel-main').isVisible()).toBe(true)
    expect(document.activeElement).toBe(wrapper.get('h1').element)
  })
})

/**
 * Клавиатура `tablist`: стрелки, Home/End (правки ревью TL-92, второй
 * раунд, Б-1).
 *
 * Первая версия этих тестов слала `keydown` прямо на `[role="tablist"]`
 * (`tablist.trigger('keydown', …)`), а не на элемент, реально
 * сфокусированный в этот момент, — и была зелёной даже с багом, который
 * она была обязана поймать: обработчик реагировал на первое нажатие, но
 * `selectTab` переводил фокус на заголовок панели (вне `tablist`), и
 * второе нажатие подряд било мимо в настоящем браузере (`keydown` с
 * заголовка до контейнера не всплывает). Искусственный вызов `.trigger()`
 * на самом контейнере этого не видел — событие и так рождалось на нём.
 * Ниже — `focusTab`/`pressOnFocused`: фокус выставляется по-настоящему,
 * событие уходит с `document.activeElement` и должно дойти до `tablist`
 * всплытием, как в реальном взаимодействии.
 */
describe('App — клавиатура вкладок: стрелки, Home/End (TL-92, правки ревью, Б-1)', () => {
  it('three ArrowRight presses in a row cycle Главный → История → Настройки → Главный, each delivered to the newly focused tab (обязательный тест Б-1)', async () => {
    const wrapper = await mountReady()
    focusTab(wrapper, 'main')

    await pressOnFocused('ArrowRight')
    expect(tabButton(wrapper, 'История').attributes('aria-selected')).toBe('true')
    expect(document.activeElement).toBe(wrapper.get('#tab-history').element)

    await pressOnFocused('ArrowRight')
    expect(tabButton(wrapper, 'Настройки').attributes('aria-selected')).toBe('true')
    expect(document.activeElement).toBe(wrapper.get('#tab-settings').element)

    await pressOnFocused('ArrowRight')
    expect(tabButton(wrapper, 'Главный').attributes('aria-selected')).toBe('true')
    expect(document.activeElement).toBe(wrapper.get('#tab-main').element)
  })

  it('ArrowLeft from «Главный» wraps around to «Настройки» and moves focus there', async () => {
    const wrapper = await mountReady()
    focusTab(wrapper, 'main')

    await pressOnFocused('ArrowLeft')
    expect(tabButton(wrapper, 'Настройки').attributes('aria-selected')).toBe('true')
    expect(document.activeElement).toBe(wrapper.get('#tab-settings').element)
  })

  it('End jumps to «Настройки», Home jumps back to «Главный», focus follows both times', async () => {
    const wrapper = await mountReady()
    focusTab(wrapper, 'main')

    await pressOnFocused('End')
    expect(tabButton(wrapper, 'Настройки').attributes('aria-selected')).toBe('true')
    expect(document.activeElement).toBe(wrapper.get('#tab-settings').element)

    await pressOnFocused('Home')
    expect(tabButton(wrapper, 'Главный').attributes('aria-selected')).toBe('true')
    expect(document.activeElement).toBe(wrapper.get('#tab-main').element)
  })

  it('keyboard activation keeps focus on the newly selected tab button, not the panel heading (дизайн «Фокус при переключении экрана»: заголовок — только после клика/«На главный»)', async () => {
    const wrapper = await mountReady()
    focusTab(wrapper, 'main')

    await pressOnFocused('End')

    const heading = tabPanel(wrapper, 'tabpanel-settings').get('h2').element
    expect(document.activeElement).not.toBe(heading)
    expect(document.activeElement).toBe(wrapper.get('#tab-settings').element)
  })

  it('unrelated keys (e.g. Tab) are ignored by the tablist handler', async () => {
    const wrapper = await mountReady()
    focusTab(wrapper, 'main')

    await pressOnFocused('Tab')
    expect(tabButton(wrapper, 'Главный').attributes('aria-selected')).toBe('true')
  })

  it('ArrowUp/ArrowDown are left alone — tablist is horizontal, browser scrolling must not be blocked (Н-4)', async () => {
    const wrapper = await mountReady()
    focusTab(wrapper, 'main')

    const target = document.activeElement as HTMLElement
    const down = new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, cancelable: true })
    target.dispatchEvent(down)
    await flushPromises()
    expect(down.defaultPrevented).toBe(false)
    expect(tabButton(wrapper, 'Главный').attributes('aria-selected')).toBe('true')

    const up = new KeyboardEvent('keydown', { key: 'ArrowUp', bubbles: true, cancelable: true })
    target.dispatchEvent(up)
    await flushPromises()
    expect(up.defaultPrevented).toBe(false)
    expect(tabButton(wrapper, 'Главный').attributes('aria-selected')).toBe('true')
  })
})

describe('App — клик по вкладке: фокус (TL-92, правки ревью, Н-4)', () => {
  it('clicking the already-active tab does not move focus anywhere', async () => {
    const wrapper = await mountReady()
    const mainButtonEl = focusTab(wrapper, 'main')
    expect(document.activeElement).toBe(mainButtonEl)

    await tabButton(wrapper, 'Главный').trigger('click')
    await wrapper.vm.$nextTick()

    // Мутация (doc-комментарий `selectTab` в `App.vue`): убрать защиту
    // «тот же таб — не трогать фокус» — и фокус уедет на `<h1>`, хотя
    // никакого настоящего переключения не произошло.
    expect(document.activeElement).toBe(mainButtonEl)
    expect(tabButton(wrapper, 'Главный').attributes('aria-selected')).toBe('true')
  })
})

/**
 * Строка состояния очереди под вкладками (TL-92, дизайн «Навигация»).
 *
 * Правки ревью (Б-2): каждая проверка текста читает `.queue-status-row`
 * прицельно, а не `wrapper.text()` целиком. `v-show` держит «Главный» в
 * DOM всегда, и его секция «Очередь загрузок» рисует ровно те же слова
 * («Между загрузками устанавливается обновлённый yt-dlp», сама
 * `QueueSection.vue`) независимо от активной вкладки, как только в списке
 * есть хоть одна задача, — `wrapper.text()` находил бы совпадение даже
 * без единой строчки кода компактной строки статуса. Тест на пауле
 * обновления ниже — ровно тот случай: он попадал в эту ловушку буквально
 * (задача есть, `pauseReason: 'ytDlpUpdate'` рисует тот же текст в
 * скрытой секции «Главного»).
 */
describe('App — строка состояния очереди под вкладками (TL-92, дизайн «Навигация», правки ревью Б-2/С-3)', () => {
  it('never renders on «Главный», even with an active task — the full queue section already plays that role there', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(okReport),
      queue_state: () =>
        Promise.resolve({
          tasks: [
            {
              taskId: 't1',
              title: 'Ролик A',
              quality: { kind: 'audioOnly' },
              plan: 'singleStream',
              phase: 'downloading',
              state: 'running',
              percent: 40,
            },
          ],
          awaitingContinue: false,
        } satisfies QueueSnapshot),
    })
    const wrapper = await mountReady()
    expect(wrapper.find('.queue-status-row').exists()).toBe(false)
  })

  it('shows the active-task line (bullet · title · phase · percent) on «История», hidden again back on «Главный»', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(okReport),
      queue_state: () =>
        Promise.resolve({
          tasks: [
            {
              taskId: 't1',
              title: 'Ролик A',
              quality: { kind: 'audioOnly' },
              plan: 'singleStream',
              phase: 'downloading',
              state: 'running',
              percent: 40,
            },
          ],
          awaitingContinue: false,
        } satisfies QueueSnapshot),
    })
    const wrapper = await mountReady()

    await tabButton(wrapper, 'История').trigger('click')
    await wrapper.vm.$nextTick()

    const row = wrapper.get('.queue-status-row')
    expect(row.text()).toContain('«Ролик A» — Только аудио · Скачивание · 40 %')

    // «●» перед названием активной задачи (С-3, макет дизайна
    // «Навигация») — декоративный, `aria-hidden`, не входит в живую зону
    // (см. описание ниже).
    const bullet = row.find('[aria-hidden="true"]')
    expect(bullet.exists()).toBe(true)
    expect(bullet.text()).toBe('●')

    await tabButton(wrapper, 'Главный').trigger('click')
    await wrapper.vm.$nextTick()
    expect(wrapper.find('.queue-status-row').exists()).toBe(false)
  })

  it('shows no percent for «fetching» (степпер «Подготовка»)', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(okReport),
      queue_state: () =>
        Promise.resolve({
          tasks: [
            { taskId: 't1', title: 'Ролик A', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'fetching' },
          ],
          awaitingContinue: false,
        } satisfies QueueSnapshot),
    })
    const wrapper = await mountReady()
    await tabButton(wrapper, 'Настройки').trigger('click')
    await wrapper.vm.$nextTick()

    const row = wrapper.get('.queue-status-row')
    expect(row.text()).toContain('«Ролик A» — Только аудио · Подготовка')
    expect(row.text()).not.toMatch(/Подготовка\s*·/)
  })

  it('shows the yt-dlp update pause line when pauseReason is ytDlpUpdate, on «Настройки» — exact design copy, no bullet, no tail (С-3)', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(okReport),
      queue_state: () =>
        Promise.resolve({
          tasks: [
            { taskId: 't1', title: 'Ролик A', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'queued' },
          ],
          awaitingContinue: false,
          pauseReason: 'ytDlpUpdate',
        } satisfies QueueSnapshot),
    })
    const wrapper = await mountReady()
    await tabButton(wrapper, 'Настройки').trigger('click')
    await wrapper.vm.$nextTick()

    // Прицельно `.queue-status-row`, не весь `wrapper.text()` (Б-2,
    // doc-комментарий блока выше): скрытая секция «Главного» рисует тот
    // же текст через `QueueSection.vue` для того же снимка очереди, и
    // `wrapper.text()` не отличил бы одно от другого.
    const row = wrapper.get('.queue-status-row')
    expect(row.text()).toContain('Между загрузками устанавливается обновлённый yt-dlp')
    // Хвост «— обычно занимает меньше минуты» — только у полной секции
    // очереди/диалога выхода, не у компактной строки статуса (С-3, своя
    // константа `STATUS_ROW_YT_DLP_UPDATE_PAUSE_TEXT`, а не
    // `YT_DLP_UPDATE_PAUSE_TEXT`).
    expect(row.text()).not.toContain('обычно занимает меньше минуты')
    // Пауза — не активная задача: без декоративного «●».
    expect(row.find('[aria-hidden="true"]').exists()).toBe(false)
  })

  it('shows the resumed-after-restart waiting line when awaitingContinue and no active task yet, without a bullet', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(okReport),
      queue_state: () =>
        Promise.resolve({
          tasks: [
            { taskId: 't1', title: 'Ролик A', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'queued' },
            { taskId: 't2', title: 'Ролик B', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'queued' },
          ],
          awaitingContinue: true,
        } satisfies QueueSnapshot),
    })
    const wrapper = await mountReady()
    await tabButton(wrapper, 'История').trigger('click')
    await wrapper.vm.$nextTick()

    const row = wrapper.get('.queue-status-row')
    expect(row.text()).toContain('Очередь приостановлена — 2 задачи ждут')
    expect(row.find('[aria-hidden="true"]').exists()).toBe(false)
  })

  it('shows nothing at all when the queue is empty and inactive, on any non-main tab', async () => {
    const wrapper = await mountReady()
    await tabButton(wrapper, 'История').trigger('click')
    await wrapper.vm.$nextTick()
    expect(wrapper.find('.queue-status-row').exists()).toBe(false)

    await tabButton(wrapper, 'Настройки').trigger('click')
    await wrapper.vm.$nextTick()
    expect(wrapper.find('.queue-status-row').exists()).toBe(false)
  })

  it('mutation guard: removing the ytDlpUpdate pause branch from queueStatusText would only be caught scoped to .queue-status-row, not by wrapper.text() (Б-2 doc)', async () => {
    // Не мутирует исходник — фиксирует утверждение отдельно от предыдущего
    // теста: полный `wrapper.text()` содержит фразу паузы, даже когда
    // строки статуса вовсе нет на экране (задача есть, но пользователь на
    // «Главном» — секция очереди видна напрямую, не через строку статуса).
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(okReport),
      queue_state: () =>
        Promise.resolve({
          tasks: [
            { taskId: 't1', title: 'Ролик A', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'queued' },
          ],
          awaitingContinue: false,
          pauseReason: 'ytDlpUpdate',
        } satisfies QueueSnapshot),
    })
    const wrapper = await mountReady()
    // На «Главном» строки статуса нет вовсе — она не для этой вкладки.
    expect(wrapper.find('.queue-status-row').exists()).toBe(false)
    // …и тем не менее тот же текст уже виден на экране — через полную
    // секцию очереди, а не через компактную строку.
    expect(wrapper.text()).toContain('Между загрузками устанавливается обновлённый yt-dlp')
  })
})

describe('App — живая зона строки статуса (TL-92, правки ревью, Н-3)', () => {
  it('the announcer exists in the DOM from mount, before there is anything to announce, and is not itself the visible row', async () => {
    const wrapper = await mountReady()
    const announcer = wrapper.get('.queue-status-announcer')
    expect(announcer.attributes('aria-live')).toBe('polite')
    expect(announcer.text()).toBe('')
    expect(announcer.classes()).not.toContain('queue-status-row')
  })

  it('the announcer mirrors the same text as the visible row once the queue has something to say, without the decorative bullet', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(okReport),
      queue_state: () =>
        Promise.resolve({
          tasks: [
            {
              taskId: 't1',
              title: 'Ролик A',
              quality: { kind: 'audioOnly' },
              plan: 'singleStream',
              phase: 'downloading',
              state: 'running',
              percent: 40,
            },
          ],
          awaitingContinue: false,
        } satisfies QueueSnapshot),
    })
    const wrapper = await mountReady()
    await tabButton(wrapper, 'История').trigger('click')
    await wrapper.vm.$nextTick()

    expect(wrapper.get('.queue-status-announcer').text()).toBe('«Ролик A» — Только аудио · Скачивание · 40 %')
  })

  it('stays silent on «Главный» even as the percent changes, instead of announcing every step (Б-3, третий раунд, регрессия к ревью TL-45)', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(okReport),
      queue_state: () =>
        Promise.resolve({
          tasks: [
            {
              taskId: 't1',
              title: 'Ролик A',
              quality: { kind: 'audioOnly' },
              plan: 'singleStream',
              phase: 'downloading',
              state: 'running',
              percent: 40,
            },
          ],
          awaitingContinue: false,
        } satisfies QueueSnapshot),
    })
    const wrapper = await mountReady()
    expect(tabButton(wrapper, 'Главный').attributes('aria-selected')).toBe('true')
    expect(wrapper.get('.queue-status-announcer').text()).toBe('')

    emitQueueChanged({
      tasks: [
        {
          taskId: 't1',
          title: 'Ролик A',
          quality: { kind: 'audioOnly' },
          plan: 'singleStream',
          phase: 'downloading',
          state: 'running',
          percent: 41,
        },
      ],
      awaitingContinue: false,
    })
    await wrapper.vm.$nextTick()

    expect(wrapper.get('.queue-status-announcer').text()).toBe('')
  })

  it('starts announcing the status text once the user switches away from «Главный» to «История»', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(okReport),
      queue_state: () =>
        Promise.resolve({
          tasks: [
            {
              taskId: 't1',
              title: 'Ролик A',
              quality: { kind: 'audioOnly' },
              plan: 'singleStream',
              phase: 'downloading',
              state: 'running',
              percent: 40,
            },
          ],
          awaitingContinue: false,
        } satisfies QueueSnapshot),
    })
    const wrapper = await mountReady()
    expect(wrapper.get('.queue-status-announcer').text()).toBe('')

    await tabButton(wrapper, 'История').trigger('click')
    await wrapper.vm.$nextTick()

    expect(wrapper.get('.queue-status-announcer').text()).toContain('«Ролик A» — Только аудио · Скачивание · 40 %')
  })

  it('moves focus to the current panel heading when the status row disappears while «На главный» was focused, instead of dropping it to <body> (Н-3)', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(okReport),
      queue_state: () =>
        Promise.resolve({
          tasks: [
            {
              taskId: 't1',
              title: 'Ролик A',
              quality: { kind: 'audioOnly' },
              plan: 'singleStream',
              phase: 'downloading',
              state: 'running',
              percent: 40,
            },
          ],
          awaitingContinue: false,
        } satisfies QueueSnapshot),
    })
    const wrapper = await mountReady()
    await tabButton(wrapper, 'История').trigger('click')
    await wrapper.vm.$nextTick()

    const backButton = wrapper.findAll('button').find((b) => b.text() === 'На главный')
    expect(backButton).toBeDefined()
    ;(backButton!.element as HTMLElement).focus()
    expect(document.activeElement).toBe(backButton!.element)

    // Очередь опустела (задача скрыта/снята) — строка статуса пропадает
    // вместе с кнопкой, на которой стоял фокус.
    emitQueueChanged({ tasks: [], awaitingContinue: false })
    await wrapper.vm.$nextTick()
    await flushPromises()

    expect(wrapper.find('.queue-status-row').exists()).toBe(false)
    const heading = tabPanel(wrapper, 'tabpanel-history').get('h2').element
    expect(document.activeElement).toBe(heading)
  })

  it('does not touch focus when the status row disappears while focus was elsewhere (e.g. the История heading itself)', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(okReport),
      queue_state: () =>
        Promise.resolve({
          tasks: [
            { taskId: 't1', title: 'Ролик A', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'fetching' },
          ],
          awaitingContinue: false,
        } satisfies QueueSnapshot),
    })
    const wrapper = await mountReady()
    await tabButton(wrapper, 'История').trigger('click')
    await wrapper.vm.$nextTick()

    const heading = tabPanel(wrapper, 'tabpanel-history').get('h2').element
    expect(document.activeElement).toBe(heading)

    emitQueueChanged({ tasks: [], awaitingContinue: false })
    await wrapper.vm.$nextTick()
    await flushPromises()

    expect(wrapper.find('.queue-status-row').exists()).toBe(false)
    expect(document.activeElement).toBe(heading)
  })
})

/**
 * TL-98 (issue #105) — живая зона исходов на «Истории»/«Настройках»:
 * терминальный исход (Done/Failed) задачи, которая была активной, и
 * отказ команды постановки (`start_download`). Отдельная зона от
 * `.queue-status-announcer` (Н-3/Б-3, TL-92) — та зеркалит только
 * *текущий* текст строки статуса и не годится для разового события,
 * которое обязано пережить следующую же смену этого текста.
 *
 * Запись в зону синхронна с приходом события (`watch(outcomeAnnouncement,
 * …)` в `App.vue` пишет текст и меняет `:key` в одном вызове), поэтому
 * каждой проверке хватает одного `await wrapper.vm.$nextTick()`. Тест
 * переозвучки (ниже) прицельно сравнивает DOM-узел зоны до/после второго
 * одинакового по тексту исхода — без пересоздания узла мутация «убрать
 * `:key`» осталась бы незамеченной: итоговый текст совпадает и без него.
 */
describe('App — живая зона исходов (TL-98, issue 105)', () => {
  const oneActiveTaskSnapshot: QueueSnapshot = {
    tasks: [
      {
        taskId: 't1',
        title: 'Ролик A',
        quality: { kind: 'audioOnly' },
        plan: 'singleStream',
        phase: 'downloading',
        state: 'running',
        percent: 90,
      },
    ],
    awaitingContinue: false,
  }

  it('announces a Failed outcome for the task that was active, on «История»', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(okReport),
      queue_state: () => Promise.resolve(oneActiveTaskSnapshot),
    })
    const wrapper = await mountReady()
    await tabButton(wrapper, 'История').trigger('click')
    await wrapper.vm.$nextTick()

    emitQueueChanged({
      tasks: [
        {
          taskId: 't1',
          title: 'Ролик A',
          quality: { kind: 'audioOnly' },
          plan: 'singleStream',
          phase: 'failed',
          error: { kind: 'connectionLost', message: 'diag', retryable: true, partialData: 'kept' },
        },
      ],
      awaitingContinue: false,
    })
    await wrapper.vm.$nextTick()

    expect(wrapper.get('.queue-outcome-announcer').text()).toBe('«Ролик A» — Только аудио — не удалось')
  })

  it('announces a Done outcome for the task that was active, on «Настройки»', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(okReport),
      queue_state: () => Promise.resolve(oneActiveTaskSnapshot),
    })
    const wrapper = await mountReady()
    await tabButton(wrapper, 'Настройки').trigger('click')
    await wrapper.vm.$nextTick()

    emitQueueChanged({
      tasks: [
        {
          taskId: 't1',
          title: 'Ролик A',
          quality: { kind: 'audioOnly' },
          plan: 'singleStream',
          phase: 'done',
          fileName: 'a.mp3',
          folderDisplay: { kind: 'systemDownloads' },
        },
      ],
      awaitingContinue: false,
    })
    await wrapper.vm.$nextTick()

    expect(wrapper.get('.queue-outcome-announcer').text()).toBe('«Ролик A» — Только аудио — готово')
  })

  it('announces a start_download (posting command) rejection that resolves only after the user already left «Главный»', async () => {
    const wrapper = await mountReady()
    await probeAndSelect(wrapper, 'https://youtu.be/a', resultA)

    let rejectStart: (err: unknown) => void = () => {}
    invokeMock.mockImplementationOnce(
      () =>
        new Promise((_resolve, reject) => {
          rejectStart = reject
        }),
    )
    await wrapper.findAll('button').find((b) => b.text() === 'Скачать')?.trigger('click')
    await wrapper.vm.$nextTick()

    // Ушли с «Главного» до того, как команда постановки разрешилась —
    // баннер `DownloadCommandErrorBlock` (role="alert") появится в
    // скрытой секции и сам по себе останется неозвученным.
    await tabButton(wrapper, 'История').trigger('click')
    await wrapper.vm.$nextTick()

    rejectStart({ kind: 'invalidUrl', message: 'CORE-DIAGNOSTIC-NOT-SCREEN-TEXT' })
    await flushPromises()
    await wrapper.vm.$nextTick()

    expect(wrapper.get('.queue-outcome-announcer').text()).toBe('Ссылка не распознана')
  })

  it('never populates the outcome zone while on «Главный» — DownloadPanel/role="status" already announces it there live (mutation guard)', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(okReport),
      queue_state: () => Promise.resolve(oneActiveTaskSnapshot),
    })
    const wrapper = await mountReady()
    expect(tabButton(wrapper, 'Главный').attributes('aria-selected')).toBe('true')

    emitQueueChanged({
      tasks: [
        {
          taskId: 't1',
          title: 'Ролик A',
          quality: { kind: 'audioOnly' },
          plan: 'singleStream',
          phase: 'failed',
          error: { kind: 'connectionLost', message: 'diag', retryable: true, partialData: 'kept' },
        },
      ],
      awaitingContinue: false,
    })
    await wrapper.vm.$nextTick()

    expect(wrapper.get('.queue-outcome-announcer').text()).toBe('')

    // Возврат на «Историю» не показывает задним числом исход, случившийся,
    // пока пользователь был на «Главном» (doc `outcomeAnnouncementText`,
    // п. 1 — иначе это было бы дублем уже увиденного на «Главном»).
    await tabButton(wrapper, 'История').trigger('click')
    await wrapper.vm.$nextTick()
    expect(wrapper.get('.queue-outcome-announcer').text()).toBe('')
  })

  it('keeps the first task’s outcome text through a second queue://changed that starts the next task in the same tick (commit, then pump — mutation guard)', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(okReport),
      queue_state: () =>
        Promise.resolve({
          tasks: [
            { ...oneActiveTaskSnapshot.tasks[0]! },
            { taskId: 't2', title: 'Ролик B', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'queued' },
          ],
          awaitingContinue: false,
        } satisfies QueueSnapshot),
    })
    const wrapper = await mountReady()
    await tabButton(wrapper, 'История').trigger('click')
    await wrapper.vm.$nextTick()

    // commit — t1 завершилась.
    emitQueueChanged({
      tasks: [
        { taskId: 't1', title: 'Ролик A', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'done', fileName: 'a.mp3', folderDisplay: { kind: 'systemDownloads' } },
        { taskId: 't2', title: 'Ролик B', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'queued' },
      ],
      awaitingContinue: false,
    })
    // pump — t2 стартовала, тем же тиком, без ожидания между событиями.
    emitQueueChanged({
      tasks: [
        { taskId: 't1', title: 'Ролик A', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'done', fileName: 'a.mp3', folderDisplay: { kind: 'systemDownloads' } },
        { taskId: 't2', title: 'Ролик B', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'fetching' },
      ],
      awaitingContinue: false,
    })

    await wrapper.vm.$nextTick()

    expect(wrapper.get('.queue-outcome-announcer').text()).toBe('«Ролик A» — Только аудио — готово')
  })

  it('re-announces an identical outcome text for two different tasks finishing one after another — the DOM node itself is recreated, not just its text (mutation guard for the `:key` remount)', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(okReport),
      queue_state: () =>
        Promise.resolve({
          tasks: [
            { taskId: 't1', title: 'Ролик', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'downloading', state: 'running' },
            { taskId: 't2', title: 'Ролик', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'queued' },
          ],
          awaitingContinue: false,
        } satisfies QueueSnapshot),
    })
    const wrapper = await mountReady()
    await tabButton(wrapper, 'История').trigger('click')
    await wrapper.vm.$nextTick()

    emitQueueChanged({
      tasks: [
        { taskId: 't1', title: 'Ролик', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'done', fileName: 'a.mp3', folderDisplay: { kind: 'systemDownloads' } },
        { taskId: 't2', title: 'Ролик', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'downloading', state: 'running' },
      ],
      awaitingContinue: false,
    })
    await wrapper.vm.$nextTick()
    expect(wrapper.get('.queue-outcome-announcer').text()).toBe('«Ролик» — Только аудио — готово')
    const firstAnnouncerNode = wrapper.get('.queue-outcome-announcer').element

    // Второй ролик с тем же названием тоже завершается — тот же итоговый
    // текст, но это новое событие и оно обязано прозвучать снова.
    emitQueueChanged({
      tasks: [
        { taskId: 't1', title: 'Ролик', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'done', fileName: 'a.mp3', folderDisplay: { kind: 'systemDownloads' } },
        { taskId: 't2', title: 'Ролик', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'done', fileName: 'b.mp3', folderDisplay: { kind: 'systemDownloads' } },
      ],
      awaitingContinue: false,
    })
    await wrapper.vm.$nextTick()

    expect(wrapper.get('.queue-outcome-announcer').text()).toBe('«Ролик» — Только аудио — готово')
    // Мутация «убрать `:key`» не тронула бы этот текст (тот же и без
    // ключа), но узел остался бы прежним — для дерева доступности это не
    // новое сообщение, а неозвученная правка уже известного текста.
    expect(wrapper.get('.queue-outcome-announcer').element).not.toBe(firstAnnouncerNode)
  })

  it('does not double-announce when both download://progress and the following queue://changed commit report the same Done transition', async () => {
    // Активная задача через `start()` (клик «Скачать»), не через снимок
    // очереди: только так подписан слушатель `download://progress`
    // (doc {@link ensureProgressListening} в `downloadTask.ts`) — без него
    // `emitProgress` ниже был бы холостым, и тест доказывал бы не то, что
    // заявлен.
    const wrapper = await mountReady()
    await probeAndSelect(wrapper, 'https://youtu.be/a', resultA)
    invokeMock.mockImplementationOnce(() => Promise.resolve(started))
    await wrapper.findAll('button').find((b) => b.text() === 'Скачать')?.trigger('click')
    await flushPromises()

    await tabButton(wrapper, 'История').trigger('click')
    await wrapper.vm.$nextTick()

    emitProgress({ taskId: 'task-1', phase: 'done', fileName: 'a.mp3', folderDisplay: { kind: 'systemDownloads' } })
    await wrapper.vm.$nextTick()
    expect(wrapper.get('.queue-outcome-announcer').text()).toBe('«Ролик A» — Только аудио — готово')
    const announcerNode = wrapper.get('.queue-outcome-announcer').element

    emitQueueChanged({
      tasks: [
        { taskId: 'task-1', title: 'Ролик A', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'done', fileName: 'a.mp3', folderDisplay: { kind: 'systemDownloads' } },
      ],
      awaitingContinue: false,
    })
    await wrapper.vm.$nextTick()
    // Если бы снимок породил второе объявление того же исхода, узел был
    // бы пересоздан (`:key`, doc выше, «Переозвучка») — он не пересоздан,
    // потому что второго события не было: тот же самый DOM-узел.
    expect(wrapper.get('.queue-outcome-announcer').element).toBe(announcerNode)
    expect(wrapper.get('.queue-outcome-announcer').text()).toBe('«Ролик A» — Только аудио — готово')
  })
})

describe('App — К-14: переключение вкладок не теряет состояние (TL-92)', () => {
  it('typed link + probed card on «Главный» survive a trip to «История» and back — ProbeSection is not unmounted', async () => {
    const wrapper = await mountReady()
    await probeAndSelect(wrapper, 'https://youtu.be/a', resultA)
    expect((wrapper.find('input').element as HTMLInputElement).value).toBe('https://youtu.be/a')
    expect(wrapper.text()).toContain('Ролик A')

    await tabButton(wrapper, 'История').trigger('click')
    await wrapper.vm.$nextTick()
    await tabButton(wrapper, 'Главный').trigger('click')
    await wrapper.vm.$nextTick()

    // Мутация, доказывающая тест (doc-комментарий брифа TL-92): замена
    // `v-show` на `v-if` у секции «Главного» пересоздала бы `ProbeSection`
    // и, с ней, `useLinkProbe()` с нуля — поле и карточка обнулились бы, и
    // это утверждение стало бы ложным.
    expect((wrapper.find('input').element as HTMLInputElement).value).toBe('https://youtu.be/a')
    expect(wrapper.text()).toContain('Ролик A')
  })

  it('progress events keep reaching the queue store while away from «Главный», render correctly on return, and queue_state is not re-fetched (listeners survive the trip)', async () => {
    const wrapper = await mountReady()
    await probeAndSelect(wrapper, 'https://youtu.be/a', resultA)
    invokeMock.mockImplementationOnce(() => Promise.resolve(started))
    await wrapper.findAll('button').find((b) => b.text() === 'Скачать')?.trigger('click')
    await flushPromises()
    emitProgress({ taskId: 'task-1', phase: 'downloading', state: 'running', percent: 10 })
    await wrapper.vm.$nextTick()
    expect(wrapper.text()).toContain('10 %')

    const queueStateCallsBefore = invokeMock.mock.calls.filter(([cmd]) => cmd === 'queue_state').length

    await tabButton(wrapper, 'История').trigger('click')
    await wrapper.vm.$nextTick()

    // Прогресс продолжает приходить, пока пользователь на «Истории» — стор
    // и подписка на `download://progress` живут на верхнем уровне
    // `App.vue`, а не внутри переключаемой секции (doc `activeTab` в
    // `App.vue`).
    emitProgress({ taskId: 'task-1', phase: 'downloading', state: 'running', percent: 55 })
    await wrapper.vm.$nextTick()
    expect(wrapper.text()).toContain('«Ролик A» — Только аудио · Скачивание · 55 %')

    await tabButton(wrapper, 'Главный').trigger('click')
    await wrapper.vm.$nextTick()

    // Возврат на «Главный» показывает актуальный (55 %), а не замороженный
    // на 10 % снимок — панель не была ни разрушена, ни отстала от событий.
    expect(wrapper.text()).toContain('55 %')
    expect(wrapper.text()).not.toContain('10 %')

    // `queue_state` не запрашивается повторно: подписка ни разу не
    // порвалась и не пересоздавалась при переключении вкладок.
    const queueStateCallsAfter = invokeMock.mock.calls.filter(([cmd]) => cmd === 'queue_state').length
    expect(queueStateCallsAfter).toBe(queueStateCallsBefore)
  })

  it('a page loaded via «Показать ещё» on «История» (TL-93) survives a trip to «Главный» and back — HistoryScreen is not unmounted, and returning to «История» does refresh the first page (правки ревью TL-93, второй раунд, п. 7)', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(okReport),
      history_page: () =>
        Promise.resolve({
          entries: [{ id: '1', videoId: 'a', url: 'u', title: 'A', quality: { kind: 'audioOnly' }, fileName: 'a.mp3', folderDisplay: { kind: 'systemDownloads' }, sizeBytes: 1, finishedAtUnixSecs: 1, fileStatus: { kind: 'present' } }],
          nextCursor: { finishedAtUnixSecs: 1, id: '1' },
          notices: [],
        }),
    })
    const wrapper = await mountReady()
    await tabButton(wrapper, 'История').trigger('click')
    await wrapper.vm.$nextTick()

    invokeMock.mockImplementationOnce((command: string) => {
      if (command === 'history_page') {
        return Promise.resolve({
          entries: [{ id: '2', videoId: 'b', url: 'u', title: 'B', quality: { kind: 'audioOnly' }, fileName: 'b.mp3', folderDisplay: { kind: 'systemDownloads' }, sizeBytes: 1, finishedAtUnixSecs: 0, fileStatus: { kind: 'present' } }],
          notices: [],
        })
      }
      throw new Error(`unexpected invoke: ${command}`)
    })
    await wrapper.findAll('button').find((b) => b.text() === 'Показать ещё')?.trigger('click')
    await flushPromises()
    expect(wrapper.findAll('.history-screen__entry')).toHaveLength(2)

    const historyPageCallsBefore = invokeMock.mock.calls.filter(([cmd]) => cmd === 'history_page').length

    await tabButton(wrapper, 'Главный').trigger('click')
    await wrapper.vm.$nextTick()
    await tabButton(wrapper, 'История').trigger('click')
    await flushPromises()

    // Тот же курсорный «хвост» (id «2», подгруженный «Показать ещё») всё
    // ещё виден — `HistoryScreen` не был размонтирован (`v-show`, К-14), и
    // алгоритм сверки первой страницы (Б-2/С-1, doc-класс `useHistoryStore`)
    // заменяет только префикс списка до последнего id свежей страницы
    // («1»), не трогая хвост за ним.
    expect(wrapper.findAll('.history-screen__entry')).toHaveLength(2)

    // Инверсия исходного теста (правки ревью TL-93, второй раунд, п. 7):
    // возврат на «Историю» **обязан** перезапросить первую страницу —
    // решение ведущего подключить обновление первой страницы к активации
    // вкладки, не только к монтированию и `queue://changed`. Мутация
    // «watch по `props.active` убран» вернула бы это число к
    // `historyPageCallsBefore` без изменений.
    const historyPageCallsAfter = invokeMock.mock.calls.filter(([cmd]) => cmd === 'history_page').length
    expect(historyPageCallsAfter).toBe(historyPageCallsBefore + 1)
  })
})

/**
 * С-1 (правки ревью TL-92, второй раунд) — диалог подтверждения выхода
 * (Р-2, TL-46/TL-47) подписан на верхнем уровне `App.vue`, независимо от
 * активной вкладки; полная матрица его поведения — `App.download.test.ts`.
 * Здесь — ровно тот сценарий ревью (взят из R2 черновика ревьюера),
 * которого раньше не было в ветке: попытка выйти **с «Истории»**, а не
 * только с «Главного».
 */
describe('App — диалог подтверждения выхода с неглавной вкладки (TL-92, правки ревью, С-1)', () => {
  it('shows the dialog while on «История», keeps destroy from firing, and «Остаться» returns focus to the История heading', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(okReport),
      queue_state: () =>
        Promise.resolve({
          tasks: [
            {
              taskId: 't1',
              title: 'Ролик A',
              quality: { kind: 'audioOnly' },
              plan: 'singleStream',
              phase: 'downloading',
              state: 'running',
              percent: 40,
            },
          ],
          awaitingContinue: false,
        } satisfies QueueSnapshot),
    })
    const wrapper = await mountReady()
    await tabButton(wrapper, 'История').trigger('click')
    await wrapper.vm.$nextTick()
    const heading = tabPanel(wrapper, 'tabpanel-history').get('h2').element

    await attemptWindowClose()
    await wrapper.vm.$nextTick()

    expect(wrapper.text()).toContain('Очередь ещё не завершена')
    expect(destroyMock).not.toHaveBeenCalled()

    await wrapper.findAll('button').find((b) => b.text() === 'Остаться')?.trigger('click')
    await wrapper.vm.$nextTick()

    expect(wrapper.text()).not.toContain('Очередь ещё не завершена')
    expect(destroyMock).not.toHaveBeenCalled()
    expect(document.activeElement).toBe(heading)
    expect(tabButton(wrapper, 'История').attributes('aria-selected')).toBe('true')
  })
})

/**
 * С-2 (правки ревью TL-92, второй раунд) — атрибуты ARIA паттерна tabs,
 * помимо тех, что уже покрыты косвенно (`aria-selected` в тестах клика/
 * клавиатуры выше). Каждый тест снимает ровно один атрибут — мутация
 * «удалить атрибут» роняет соответствующий тест и никакой другой.
 */
describe('App — атрибуты ARIA панели вкладок (TL-92, правки ревью, С-2)', () => {
  it('roving tabindex: the active tab is 0, the other two are -1, and it moves with the selection', async () => {
    const wrapper = await mountReady()
    expect(tabButton(wrapper, 'Главный').attributes('tabindex')).toBe('0')
    expect(tabButton(wrapper, 'История').attributes('tabindex')).toBe('-1')
    expect(tabButton(wrapper, 'Настройки').attributes('tabindex')).toBe('-1')

    await tabButton(wrapper, 'История').trigger('click')
    await wrapper.vm.$nextTick()

    expect(tabButton(wrapper, 'Главный').attributes('tabindex')).toBe('-1')
    expect(tabButton(wrapper, 'История').attributes('tabindex')).toBe('0')
    expect(tabButton(wrapper, 'Настройки').attributes('tabindex')).toBe('-1')
  })

  it('aria-selected is "true" for exactly the active tab and "false" for the other two', async () => {
    const wrapper = await mountReady()
    await tabButton(wrapper, 'Настройки').trigger('click')
    await wrapper.vm.$nextTick()

    expect(tabButton(wrapper, 'Главный').attributes('aria-selected')).toBe('false')
    expect(tabButton(wrapper, 'История').attributes('aria-selected')).toBe('false')
    expect(tabButton(wrapper, 'Настройки').attributes('aria-selected')).toBe('true')
  })

  it('aria-controls on each tab is exactly the id of its own panel', async () => {
    const wrapper = await mountReady()
    expect(tabButton(wrapper, 'Главный').attributes('aria-controls')).toBe('tabpanel-main')
    expect(tabButton(wrapper, 'История').attributes('aria-controls')).toBe('tabpanel-history')
    expect(tabButton(wrapper, 'Настройки').attributes('aria-controls')).toBe('tabpanel-settings')

    for (const tabId of ['main', 'history', 'settings']) {
      const controls = wrapper.get(`#tab-${tabId}`).attributes('aria-controls')
      expect(wrapper.find(`#${controls}`).exists()).toBe(true)
    }
  })

  it('every panel has aria-labelledby pointing back at its own tab id', async () => {
    const wrapper = await mountReady()
    expect(tabPanel(wrapper, 'tabpanel-main').attributes('aria-labelledby')).toBe('tab-main')
    expect(tabPanel(wrapper, 'tabpanel-history').attributes('aria-labelledby')).toBe('tab-history')
    expect(tabPanel(wrapper, 'tabpanel-settings').attributes('aria-labelledby')).toBe('tab-settings')
  })
})

/**
 * Н-1 (правки ревью TL-92, второй, затем третий раунд) — под панелью
 * вкладок должна быть одна линия, не две. Раньше нижняя граница `.tabs`
 * шла вместе с безусловным `<hr class="screen__divider">` сразу следом —
 * визуально две черты почти вплотную.
 *
 * Проверка структурная (соседство узлов через обход DOM), не через
 * вычисленные CSS-стили: `<style scoped>` компонента не гарантированно
 * применяется к дереву в jsdom так же, как в браузере, а соседство тегов
 * — факт разметки независимо от того, применились ли стили.
 *
 * Третий раунд: `querySelector('.tabs + hr')` проверяет только
 * непосредственного соседа `.tabs`, а с TL-92 (правки ревью, третий
 * раунд, Н-3/Б-3) сразу за `.tabs` всегда стоит постоянный `<p
 * class="queue-status-announcer">` живой зоны — `<hr>`, вставленный сразу
 * после этого `<p>` (а не после `.tabs` напрямую), для прежней проверки
 * невидим. {@link dividersBetweenTabsAndFirstPanel} вместо этого обходит
 * все соседние узлы от `.tabs` до первой секции `[role="tabpanel"]`
 * (первая из трёх всегда «Главный» — они все время в DOM, `v-show`, не
 * `v-if`, К-14) и считает `<hr>` среди них — не важно, к какому именно
 * промежуточному узлу он приклеен.
 */
function dividersBetweenTabsAndFirstPanel(wrapper: Awaited<ReturnType<typeof mountReady>>): number {
  const tabsEl = wrapper.get('.tabs').element
  let node = tabsEl.nextElementSibling
  let count = 0
  while (node && node.getAttribute('role') !== 'tabpanel') {
    if (node.tagName === 'HR') count++
    node = node.nextElementSibling
  }
  return count
}

describe('App — одна линия под вкладками, не две (TL-92, правки ревью, Н-1)', () => {
  it('no <hr> appears anywhere between .tabs and the first panel when the status row is hidden — the single line is the .tabs border itself', async () => {
    const wrapper = await mountReady()
    expect(dividersBetweenTabsAndFirstPanel(wrapper)).toBe(0)
  })

  it('exactly one divider appears between .tabs and the first panel when the status row is shown', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(okReport),
      queue_state: () =>
        Promise.resolve({
          tasks: [
            { taskId: 't1', title: 'Ролик A', quality: { kind: 'audioOnly' }, plan: 'singleStream', phase: 'fetching' },
          ],
          awaitingContinue: false,
        } satisfies QueueSnapshot),
    })
    const wrapper = await mountReady()
    await tabButton(wrapper, 'История').trigger('click')
    await wrapper.vm.$nextTick()

    expect(dividersBetweenTabsAndFirstPanel(wrapper)).toBe(1)
    expect(wrapper.find('.queue-status-row + hr').exists()).toBe(true)
  })
})

/**
 * С-7 (правки ревью TL-93, второй раунд): «если записей нет — на заголовок
 * экрана» / «после подтверждённой «Очистить» — на заголовок экрана».
 * `HistoryScreen.vue` не владеет узлом `<h2>История</h2>` (он в `App.vue`,
 * К-14) и эмитит `request-heading-focus`, а не двигает фокус сам —
 * поэтому конечная проверка (реальный `document.activeElement`) идёт
 * здесь, на смонтированном целиком `App`, а не в изолированных тестах
 * `HistoryScreen.test.ts` (там нет самого узла, на который нужно навести
 * фокус, — см. doc-комментарий этого пропа/эмита в `HistoryScreen.vue`).
 */
describe('App — фокус после «Удалить»/«Очистить» на экране истории уходит на заголовок панели (TL-93, правки ревью, второй раунд, С-7)', () => {
  const oneEntry = {
    id: '1',
    videoId: 'a',
    url: 'u',
    title: 'A',
    quality: { kind: 'audioOnly' as const },
    fileName: 'a.mp3',
    folderDisplay: { kind: 'systemDownloads' as const },
    sizeBytes: 1,
    finishedAtUnixSecs: 1,
    fileStatus: { kind: 'present' as const },
  }

  it('deleting the only remaining row moves focus to the История heading', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(okReport),
      history_page: () => Promise.resolve({ entries: [oneEntry], notices: [] }),
    })
    const wrapper = await mountReady()
    await tabButton(wrapper, 'История').trigger('click')
    await flushPromises()

    invokeMock.mockResolvedValueOnce(undefined)
    await wrapper.findAll('button').find((b) => b.text() === 'Удалить')?.trigger('click')
    await flushPromises()

    expect(wrapper.findAll('.history-screen__entry')).toHaveLength(0)
    expect(document.activeElement).toBe(tabPanel(wrapper, 'tabpanel-history').get('h2').element)
  })

  it('confirming «Очистить всё» moves focus to the История heading, not to the (now gone) «Очистить» button', async () => {
    routeInvoke({
      prepare_ytdlp: () => Promise.resolve(preparedWarm),
      check_sidecar: () => Promise.resolve(okReport),
      history_page: () => Promise.resolve({ entries: [oneEntry], notices: [] }),
    })
    const wrapper = await mountReady()
    await tabButton(wrapper, 'История').trigger('click')
    await flushPromises()

    await wrapper.findAll('button').find((b) => b.text() === 'Очистить')?.trigger('click')
    invokeMock.mockResolvedValueOnce(undefined)
    await wrapper.findAll('button').find((b) => b.text() === 'Очистить всё')?.trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('История пуста')
    expect(document.activeElement).toBe(tabPanel(wrapper, 'tabpanel-history').get('h2').element)
  })
})
