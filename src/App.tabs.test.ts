import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { DownloadProgressEvent, DownloadStarted } from '@/types/generated/download'
import type { ProbeResult } from '@/types/generated/probe'
import type { QueueSnapshot } from '@/types/generated/queue'
import type { SidecarCheckReport } from '@/types/generated/sidecar'
import type { YtDlpPrepared } from '@/types/generated/ytdlp'

/**
 * TL-92 (issue execaus/tube-leak#95, дизайн E5 «Навигация»): панель вкладок
 * «Главный/История/Настройки» и компактная строка состояния очереди.
 * Собственный файл, а не довесок к `App.test.ts` (порядок вызовов E1) или
 * `App.download.test.ts` (очередь E3 → E4) — здесь проверяется именно
 * навигация и то, что переключение вкладки не задевает состояние ни
 * «Главного», ни стора очереди (критерий приёмки эпика К-14).
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

// Диалог выхода (TL-46/TL-47) не тема этого файла — окно замокано так,
// чтобы `onCloseRequested` просто никогда не резолвился (тот же приём,
// что `App.test.ts`), никакой из тестов ниже не трогает закрытие окна.
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

function routeInvoke(handlersByCommand: Record<string, () => Promise<unknown>>) {
  const withDefaults: Record<string, () => Promise<unknown>> = {
    // Снимок очереди — дефолт «пусто», явно переданный обработчик той же
    // команды имеет приоритет (тот же приём, что `App.download.test.ts`).
    queue_state: () => Promise.resolve(EMPTY_QUEUE_SNAPSHOT),
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
    expect(wrapper.text()).toContain('Здесь появится история завершённых загрузок.')

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
    expect(wrapper.text()).toContain('Здесь появится история завершённых загрузок.')

    const heading = tabPanel(wrapper, 'tabpanel-history').get('h2')
    expect(document.activeElement).toBe(heading.element)
  })

  it('clicking «Настройки» switches to its panel/placeholder and moves focus to its heading', async () => {
    const wrapper = await mountReady()

    await tabButton(wrapper, 'Настройки').trigger('click')
    await wrapper.vm.$nextTick()

    expect(tabButton(wrapper, 'Настройки').attributes('aria-selected')).toBe('true')
    expect(tabPanel(wrapper, 'tabpanel-settings').isVisible()).toBe(true)
    expect(wrapper.text()).toContain('Здесь появятся папка назначения, шаблон имени и число попыток.')

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

describe('App — клавиатура вкладок: стрелки, Home/End (TL-92)', () => {
  it('ArrowRight cycles Главный → История → Настройки → Главный', async () => {
    const wrapper = await mountReady()
    const tablist = wrapper.get('[role="tablist"]')

    await tablist.trigger('keydown', { key: 'ArrowRight' })
    await wrapper.vm.$nextTick()
    expect(tabButton(wrapper, 'История').attributes('aria-selected')).toBe('true')

    await tablist.trigger('keydown', { key: 'ArrowRight' })
    await wrapper.vm.$nextTick()
    expect(tabButton(wrapper, 'Настройки').attributes('aria-selected')).toBe('true')

    await tablist.trigger('keydown', { key: 'ArrowRight' })
    await wrapper.vm.$nextTick()
    expect(tabButton(wrapper, 'Главный').attributes('aria-selected')).toBe('true')
  })

  it('ArrowLeft from «Главный» wraps around to «Настройки»', async () => {
    const wrapper = await mountReady()
    const tablist = wrapper.get('[role="tablist"]')

    await tablist.trigger('keydown', { key: 'ArrowLeft' })
    await wrapper.vm.$nextTick()
    expect(tabButton(wrapper, 'Настройки').attributes('aria-selected')).toBe('true')
  })

  it('End jumps to «Настройки», Home jumps back to «Главный»', async () => {
    const wrapper = await mountReady()
    const tablist = wrapper.get('[role="tablist"]')

    await tablist.trigger('keydown', { key: 'End' })
    await wrapper.vm.$nextTick()
    expect(tabButton(wrapper, 'Настройки').attributes('aria-selected')).toBe('true')

    await tablist.trigger('keydown', { key: 'Home' })
    await wrapper.vm.$nextTick()
    expect(tabButton(wrapper, 'Главный').attributes('aria-selected')).toBe('true')
  })

  it('keyboard activation also moves focus into the new panel heading (дизайн: не остаётся на кнопке-вкладке)', async () => {
    const wrapper = await mountReady()
    const tablist = wrapper.get('[role="tablist"]')

    await tablist.trigger('keydown', { key: 'End' })
    await wrapper.vm.$nextTick()

    const heading = tabPanel(wrapper, 'tabpanel-settings').get('h2')
    expect(document.activeElement).toBe(heading.element)
  })

  it('unrelated keys (e.g. Tab) are ignored by the tablist handler', async () => {
    const wrapper = await mountReady()
    const tablist = wrapper.get('[role="tablist"]')

    await tablist.trigger('keydown', { key: 'Tab' })
    await wrapper.vm.$nextTick()
    expect(tabButton(wrapper, 'Главный').attributes('aria-selected')).toBe('true')
  })
})

describe('App — строка состояния очереди под вкладками (TL-92, дизайн «Навигация»)', () => {
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

  it('shows the active-task line (title · phase · percent) on «История», hidden again back on «Главный»', async () => {
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

    expect(wrapper.find('.queue-status-row').exists()).toBe(true)
    expect(wrapper.text()).toContain('«Ролик A» — Только аудио · Скачивание · 40 %')

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

    expect(wrapper.text()).toContain('«Ролик A» — Только аудио · Подготовка')
    expect(wrapper.text()).not.toMatch(/Подготовка\s*·/)
  })

  it('shows the yt-dlp update pause line when pauseReason is ytDlpUpdate, on «Настройки»', async () => {
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

    expect(wrapper.text()).toContain('Между загрузками устанавливается обновлённый yt-dlp')
  })

  it('shows the resumed-after-restart waiting line when awaitingContinue and no active task yet', async () => {
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

    expect(wrapper.text()).toContain('Очередь приостановлена — 2 задачи ждут')
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
})
