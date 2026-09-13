import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import type { QueueTask } from '@/types/generated/queue'

import QueueSection from './QueueSection.vue'

const active: QueueTask = {
  taskId: 'a',
  title: 'Как приручить дракона',
  quality: { kind: 'standard', heightPx: 1080 },
  plan: 'videoAndAudio',
  phase: 'downloading',
  state: 'running',
  percent: 62,
}

const waiting1: QueueTask = {
  taskId: 'b',
  title: 'Урок кулинарии',
  quality: { kind: 'audioOnly' },
  plan: 'singleStream',
  phase: 'queued',
}

const waiting2: QueueTask = {
  taskId: 'c',
  title: 'Летний влог',
  quality: { kind: 'standard', heightPx: 720 },
  plan: 'videoAndAudio',
  phase: 'queued',
}

const done: QueueTask = {
  taskId: 'd',
  title: 'Прошлый ролик',
  quality: { kind: 'audioOnly' },
  plan: 'singleStream',
  phase: 'done',
  fileName: 'Прошлый ролик.mp3',
  folderDisplay: { kind: 'systemDownloads' },
}

describe('QueueSection — пустое состояние', () => {
  it('renders nothing when there are no tasks and no command error (дизайн, «Где живёт список» — макет не резервирует место)', () => {
    const wrapper = mount(QueueSection, { props: { tasks: [], awaitingContinue: false } })
    expect(wrapper.find('.queue-section').exists()).toBe(false)
  })

  it('renders when there is a command error even with an empty task list (ревью TL-45, достижимый путь к молчаливому отказу)', () => {
    const wrapper = mount(QueueSection, {
      props: { tasks: [], awaitingContinue: false, commandError: { kind: 'invalidUrl', message: 'diag' } },
    })
    expect(wrapper.find('.queue-section').exists()).toBe(true)
    expect(wrapper.text()).toContain('Ссылка не распознана')
  })
})

describe('QueueSection — K-1: три задачи подряд (issue #82, критерий приёмки)', () => {
  const wrapper = mount(QueueSection, {
    props: { tasks: [active, waiting1, waiting2], awaitingContinue: false },
  })

  it('renders the active task through DownloadPanel, with progress and the accent border', () => {
    const panel = wrapper.find('.download-panel')
    expect(panel.exists()).toBe(true)
    expect(panel.classes()).toContain('queue-section__task--active')
    expect(wrapper.text()).toContain('«Как приручить дракона» — 1080p')
    expect(wrapper.text()).toContain('62 %')
  })

  it('renders the first waiting task with zero tasks ahead of it — the active task itself is not counted (дизайн: «не считая активную»)', () => {
    const rows = wrapper.findAll('.queue-waiting-row')
    expect(rows[0]?.text()).toContain('«Урок кулинарии» — Только аудио')
    expect(rows[0]?.text()).toContain('В очереди — начнётся после текущей загрузки')
    expect(rows[0]?.text()).not.toContain('и ещё')
  })

  it('renders the second waiting task with "и ещё 1 задачи" — counts only the one queued task ahead of it, not the active one (exact count from list order, not a contract field)', () => {
    const rows = wrapper.findAll('.queue-waiting-row')
    expect(rows[1]?.text()).toContain('«Летний влог» — 720p')
    expect(rows[1]?.text()).toContain('и ещё 1 задачи')
  })
})

describe('QueueSection — терминальные задачи и «Скрыть завершённые»', () => {
  it('does not show "Скрыть завершённые" when there is no terminal task', () => {
    const wrapper = mount(QueueSection, { props: { tasks: [active, waiting1], awaitingContinue: false } })
    expect(wrapper.findAll('button').some((b) => b.text() === 'Скрыть завершённые')).toBe(false)
  })

  it('shows "Скрыть завершённые" once at least one terminal task is present, and emits hide-all-terminal on click', async () => {
    const wrapper = mount(QueueSection, { props: { tasks: [active, done], awaitingContinue: false } })
    const button = wrapper.findAll('button').find((b) => b.text() === 'Скрыть завершённые')
    expect(button).toBeDefined()

    await button?.trigger('click')
    expect(wrapper.emitted('hide-all-terminal')).toHaveLength(1)
  })

  it('renders the terminal task through DownloadPanel without the accent border (only the active task gets it)', () => {
    const wrapper = mount(QueueSection, { props: { tasks: [done], awaitingContinue: false } })
    const panel = wrapper.find('.download-panel')
    expect(panel.exists()).toBe(true)
    expect(panel.classes()).not.toContain('queue-section__task--active')
  })

  it('keeps a terminal task in its original list position (дизайн, «Порядок в списке») — not moved to the end', () => {
    const wrapper = mount(QueueSection, { props: { tasks: [done, waiting1], awaitingContinue: false } })
    const rows = wrapper.findAll('.queue-section__task')
    expect(rows[0]?.classes()).toContain('download-panel')
    expect(rows[1]?.classes()).toContain('queue-waiting-row')
  })
})

describe('QueueSection — баннер продолжения после перезапуска (Р-3)', () => {
  it('shows the banner and its "Продолжить очередь" button when awaitingContinue, and emits resume on click', async () => {
    const wrapper = mount(QueueSection, { props: { tasks: [waiting1, waiting2], awaitingContinue: true } })

    expect(wrapper.text()).toContain('Очередь приостановлена после перезапуска')
    expect(wrapper.text()).toContain('2 задачи ждут')

    const button = wrapper.findAll('button').find((b) => b.text() === 'Продолжить очередь')
    await button?.trigger('click')
    expect(wrapper.emitted('resume')).toHaveLength(1)
  })

  it('does not render the banner when awaitingContinue is false', () => {
    const wrapper = mount(QueueSection, { props: { tasks: [waiting1], awaitingContinue: false } })
    expect(wrapper.text()).not.toContain('Очередь приостановлена')
  })

  it('propagates awaitingContinue into the waiting rows\' wording ("как только вы продолжите")', () => {
    const wrapper = mount(QueueSection, { props: { tasks: [waiting1], awaitingContinue: true } })
    expect(wrapper.text()).toContain('начнётся первой, как только вы продолжите')
  })
})

describe('QueueSection — пауза на обновление yt-dlp между задачами (Р-7/С-8)', () => {
  it('shows the pause line when pauseReason is ytDlpUpdate, own text (not YtDlpUpdateBlock\'s)', () => {
    const wrapper = mount(QueueSection, {
      props: { tasks: [waiting1], awaitingContinue: false, pauseReason: 'ytDlpUpdate' },
    })
    expect(wrapper.text()).toContain('Между загрузками устанавливается обновлённый yt-dlp')
  })

  it('does not show the pause line when there is no pauseReason', () => {
    const wrapper = mount(QueueSection, { props: { tasks: [waiting1], awaitingContinue: false } })
    expect(wrapper.text()).not.toContain('Между загрузками устанавливается')
  })
})

describe('QueueSection — действия построчно (cancel/retry/hide несут taskId)', () => {
  it('cancel on the active task panel emits cancel with its taskId', async () => {
    const wrapper = mount(QueueSection, { props: { tasks: [active], awaitingContinue: false } })
    await wrapper.findAll('button').find((b) => b.text() === 'Отменить')?.trigger('click')
    expect(wrapper.emitted('cancel')).toStrictEqual([['a']])
  })

  it('cancel on a waiting row emits cancel with that row\'s taskId, not the active one', async () => {
    const wrapper = mount(QueueSection, { props: { tasks: [active, waiting1], awaitingContinue: false } })
    const cancelButtons = wrapper.findAll('button').filter((b) => b.text() === 'Отменить')
    expect(cancelButtons).toHaveLength(2)
    await cancelButtons[1]?.trigger('click')
    expect(wrapper.emitted('cancel')).toStrictEqual([['b']])
  })

  it('retry on a failed task panel emits retry with its taskId', async () => {
    const failed: QueueTask = {
      taskId: 'f',
      title: 'Ролик с ошибкой',
      quality: { kind: 'audioOnly' },
      plan: 'singleStream',
      phase: 'failed',
      error: { kind: 'connectionLost', message: 'diag', retryable: true, partialData: 'kept' },
    }
    const wrapper = mount(QueueSection, { props: { tasks: [failed], awaitingContinue: false } })
    await wrapper.findAll('button').find((b) => b.text() === 'Повторить')?.trigger('click')
    expect(wrapper.emitted('retry')).toStrictEqual([['f']])
  })

  it('hide on a terminal task panel emits hide with its taskId', async () => {
    const wrapper = mount(QueueSection, { props: { tasks: [done], awaitingContinue: false } })
    await wrapper.findAll('button').find((b) => b.text() === 'Скрыть')?.trigger('click')
    expect(wrapper.emitted('hide')).toStrictEqual([['d']])
  })

  it('dismiss-command-error is emitted from the command error block\'s "Скрыть"', async () => {
    const wrapper = mount(QueueSection, {
      props: { tasks: [], awaitingContinue: false, commandError: { kind: 'invalidUrl', message: 'diag' } },
    })
    await wrapper.findAll('button').find((b) => b.text() === 'Скрыть')?.trigger('click')
    expect(wrapper.emitted('dismiss-command-error')).toHaveLength(1)
  })
})
