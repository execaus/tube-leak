import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import type { DownloadErrorKind, DownloadProgress } from '@/types/generated/download'

import DownloadPanel from './DownloadPanel.vue'

const TITLE = '«Как приручить дракона» — 1080p'

describe('DownloadPanel — степпер фаз (нетерминальные состояния)', () => {
  it('renders all four steps for a two-stream plan, marking the current one', () => {
    const wrapper = mount(DownloadPanel, {
      props: {
        displayTitle: TITLE,
        plan: 'videoAndAudio',
        progress: { phase: 'downloading', state: 'running', percent: 62 },
      },
    })

    const labels = wrapper.findAll('.download-panel__step').map((s) => s.text())
    expect(labels.some((l) => l.includes('Подготовка'))).toBe(true)
    expect(labels.some((l) => l.includes('Скачивание'))).toBe(true)
    expect(labels.some((l) => l.includes('Склейка'))).toBe(true)
    expect(labels.some((l) => l.includes('Готово'))).toBe(true)

    expect(wrapper.find('.download-panel__step--current').text()).toContain('Скачивание')
  })

  it('does not render the "Склейка" step at all for a single-stream plan (С-2/С-3)', () => {
    const wrapper = mount(DownloadPanel, {
      props: {
        displayTitle: TITLE,
        plan: 'singleStream',
        progress: { phase: 'downloading', state: 'running', percent: 40 },
      },
    })

    const labels = wrapper.findAll('.download-panel__step').map((s) => s.text())
    expect(labels.some((l) => l.includes('Склейка'))).toBe(false)
  })

  it('marks preceding steps as done (✓) once past them', () => {
    const wrapper = mount(DownloadPanel, {
      props: {
        displayTitle: TITLE,
        plan: 'videoAndAudio',
        progress: { phase: 'merging' },
      },
    })

    const steps = wrapper.findAll('.download-panel__step')
    const downloadingStep = steps.find((s) => s.text().includes('Скачивание'))
    expect(downloadingStep?.classes()).toContain('download-panel__step--done')
    expect(downloadingStep?.text()).toContain('✓')
  })

  it('does not render a stepper at all for terminal phases', () => {
    const wrapper = mount(DownloadPanel, {
      props: {
        displayTitle: TITLE,
        plan: 'videoAndAudio',
        progress: { phase: 'done', fileName: 'video.mp4' },
      },
    })

    expect(wrapper.find('.download-panel__steps').exists()).toBe(false)
  })
})

describe('DownloadPanel — доступность степпера: текущий шаг не только цветом (ревью TL-45)', () => {
  it('marks exactly the current step with aria-current="step", not the others', () => {
    const wrapper = mount(DownloadPanel, {
      props: {
        displayTitle: TITLE,
        plan: 'videoAndAudio',
        progress: { phase: 'downloading', state: 'running', percent: 10 },
      },
    })

    const steps = wrapper.findAll('.download-panel__step')
    const withAriaCurrent = steps.filter((s) => s.attributes('aria-current') === 'step')
    expect(withAriaCurrent).toHaveLength(1)
    expect(withAriaCurrent[0]?.text()).toContain('Скачивание')

    const others = steps.filter((s) => !s.text().includes('Скачивание'))
    for (const other of others) {
      expect(other.attributes('aria-current')).toBeUndefined()
    }
  })
})

describe('DownloadPanel — одна живая зона за раз, процент не в ней (ревью TL-45, «Заметки»)', () => {
  it('has no aria-live on its root — the panel is not one big nested live region', () => {
    const wrapper = mount(DownloadPanel, {
      props: {
        displayTitle: TITLE,
        plan: 'videoAndAudio',
        progress: { phase: 'downloading', state: 'running', percent: 10 },
      },
    })
    expect(wrapper.attributes('aria-live')).toBeUndefined()
  })

  it('exposes exactly one aria-live element while running, and the percent line sits outside of it', () => {
    const wrapper = mount(DownloadPanel, {
      props: {
        displayTitle: TITLE,
        plan: 'videoAndAudio',
        progress: {
          phase: 'downloading',
          state: 'running',
          percent: 62,
          speedBytesPerSec: 1024,
        },
        softStallSeconds: 7,
      },
    })

    const liveRegions = wrapper.findAll('[aria-live]')
    expect(liveRegions).toHaveLength(1)
    expect(liveRegions[0]?.find('.download-panel__percent').exists()).toBe(false)
    expect(wrapper.find('.download-panel__percent').exists()).toBe(true)
  })

  it('exposes no aria-live element (and no nesting) on a terminal panel — role="status" alone carries the announcement', () => {
    const wrapper = mount(DownloadPanel, {
      props: {
        displayTitle: TITLE,
        plan: 'videoAndAudio',
        progress: { phase: 'done', fileName: 'x.mp4' },
      },
    })

    expect(wrapper.findAll('[aria-live]')).toHaveLength(0)
    expect(wrapper.findAll('[role="status"]')).toHaveLength(1)
  })
})

describe('DownloadPanel — заголовок (С-13, требование п.6)', () => {
  it('renders exactly the displayTitle snapshot, regardless of progress content', () => {
    const wrapper = mount(DownloadPanel, {
      props: {
        displayTitle: TITLE,
        plan: 'videoAndAudio',
        progress: { phase: 'queued' },
      },
    })

    expect(wrapper.find('.download-panel__title').text()).toBe(TITLE)
  })
})

describe('DownloadPanel — Подготовка (queued/fetching)', () => {
  it('shows a neutral preparing message with a Cancel button, and no percent', () => {
    for (const phase of ['queued', 'fetching'] as const) {
      const wrapper = mount(DownloadPanel, {
        props: { displayTitle: TITLE, plan: 'videoAndAudio', progress: { phase } },
      })
      expect(wrapper.text()).toContain('Готовим загрузку')
      expect(wrapper.findAll('button').some((b) => b.text() === 'Отменить')).toBe(true)
      expect(wrapper.find('.download-panel__percent').exists()).toBe(false)
    }
  })
})

describe('DownloadPanel — Скачивание (running)', () => {
  it('renders percent, progressbar attributes, speed, eta and stream label (two streams)', () => {
    const wrapper = mount(DownloadPanel, {
      props: {
        displayTitle: TITLE,
        plan: 'videoAndAudio',
        progress: {
          phase: 'downloading',
          state: 'running',
          stream: 'video',
          percent: 62,
          speedBytesPerSec: 4.2 * 1024 * 1024,
          etaSecs: 100,
        },
      },
    })

    const bar = wrapper.find('[role="progressbar"]')
    expect(bar.attributes('aria-valuenow')).toBe('62')
    expect(bar.attributes('aria-valuemax')).toBe('100')
    expect(wrapper.find('.download-panel__percent').text()).toBe('62 %')
    expect(wrapper.text()).toContain('Скачиваем видео')
    expect(wrapper.text()).toContain('4.2 МБ/с')
    expect(wrapper.text()).toContain('осталось ≈ 1 мин 40 с')
  })

  it('uses a plain "Скачиваем…" label (no stream word) for a single-stream plan', () => {
    const wrapper = mount(DownloadPanel, {
      props: {
        displayTitle: TITLE,
        plan: 'singleStream',
        progress: { phase: 'downloading', state: 'running', percent: 10 },
      },
    })
    expect(wrapper.text()).toContain('Скачиваем')
    expect(wrapper.text()).not.toContain('Скачиваем видео')
    expect(wrapper.text()).not.toContain('Скачиваем звук')
  })

  it('omits percent, speed, eta and the bar itself entirely when absent — never renders a dash, zero, or a bar frozen at 0%', () => {
    const wrapper = mount(DownloadPanel, {
      props: {
        displayTitle: TITLE,
        plan: 'videoAndAudio',
        progress: { phase: 'downloading', state: 'running' },
      },
    })
    expect(wrapper.find('.download-panel__percent').exists()).toBe(false)
    expect(wrapper.text()).not.toMatch(/МБ\/с|КБ\/с|осталось/)
    // Без известного процента полосы нет вовсе (ревью TL-45) — нулевая
    // ширина выглядела бы как «почти ничего не скачано», хотя данных
    // попросту ещё нет.
    expect(wrapper.find('[role="progressbar"]').exists()).toBe(false)
  })

  it('does not show the attempt number on the first attempt, but shows it from the second on', () => {
    const first = mount(DownloadPanel, {
      props: {
        displayTitle: TITLE,
        plan: 'videoAndAudio',
        progress: {
          phase: 'downloading',
          state: 'running',
          percent: 10,
          attempt: { number: 1, total: 6 },
        },
      },
    })
    expect(first.text()).not.toContain('попытка')

    const second = mount(DownloadPanel, {
      props: {
        displayTitle: TITLE,
        plan: 'videoAndAudio',
        progress: {
          phase: 'downloading',
          state: 'running',
          percent: 10,
          attempt: { number: 2, total: 6 },
        },
      },
    })
    expect(second.text()).toContain('попытка 2 из 6')
  })

  it('emits cancel when the Cancel button is clicked', async () => {
    const wrapper = mount(DownloadPanel, {
      props: {
        displayTitle: TITLE,
        plan: 'videoAndAudio',
        progress: { phase: 'downloading', state: 'running', percent: 10 },
      },
    })
    await wrapper.find('button').trigger('click')
    expect(wrapper.emitted('cancel')).toHaveLength(1)
  })
})

describe('DownloadPanel — мягкий индикатор зависания (косметика фронтенда, требование п.4)', () => {
  it('replaces speed and eta with an honest placeholder, and shows the stall line, without changing the percent', () => {
    const wrapper = mount(DownloadPanel, {
      props: {
        displayTitle: TITLE,
        plan: 'videoAndAudio',
        progress: {
          phase: 'downloading',
          state: 'running',
          percent: 62,
          speedBytesPerSec: 4.2 * 1024 * 1024,
          etaSecs: 100,
        },
        softStallSeconds: 7,
      },
    })

    expect(wrapper.find('.download-panel__percent').text()).toBe('62 %')
    expect(wrapper.text()).toContain('медленно или не отвечает')
    expect(wrapper.text()).not.toContain('4.2 МБ/с')
    expect(wrapper.text()).not.toContain('осталось ≈')
    expect(wrapper.text()).toContain('Нет новых данных уже 7 с')
  })
})

describe('DownloadPanel — пауза перед повтором (waitingRetry, С-6)', () => {
  it('freezes the percent with "(сохранено)", shows the attempt and countdown, no speed/eta, aria-busy without aria-valuenow', () => {
    const wrapper = mount(DownloadPanel, {
      props: {
        displayTitle: TITLE,
        plan: 'videoAndAudio',
        progress: {
          phase: 'downloading',
          state: 'waitingRetry',
          percent: 62,
          attempt: { number: 2, total: 6 },
          delaySecs: 10,
          remainingSecs: 8,
        },
      },
    })

    expect(wrapper.find('.download-panel__percent').text()).toBe('62 % (сохранено)')
    expect(wrapper.text()).toContain('попытки (2 из 6)')
    expect(wrapper.text()).toContain('через 8 с')
    expect(wrapper.text()).not.toMatch(/МБ\/с|осталось/)

    const bar = wrapper.find('[role="progressbar"]')
    expect(bar.attributes('aria-busy')).toBe('true')
    expect(bar.attributes('aria-valuenow')).toBeUndefined()
  })

  it('still offers Cancel during the retry pause (Ф-4: cancel is available at every stage)', () => {
    const wrapper = mount(DownloadPanel, {
      props: {
        displayTitle: TITLE,
        plan: 'videoAndAudio',
        progress: {
          phase: 'downloading',
          state: 'waitingRetry',
          attempt: { number: 2, total: 6 },
          delaySecs: 10,
          remainingSecs: 8,
        },
      },
    })
    expect(wrapper.findAll('button').some((b) => b.text() === 'Отменить')).toBe(true)
  })
})

describe('DownloadPanel — Склейка (merging)', () => {
  it('shows a neutral spinner, no percent, and no progress bar at all — design draws no bar for merging (ревью TL-45)', () => {
    const wrapper = mount(DownloadPanel, {
      props: { displayTitle: TITLE, plan: 'videoAndAudio', progress: { phase: 'merging' } },
    })
    expect(wrapper.text()).toContain('Склеиваем видео и звук')
    expect(wrapper.find('.download-panel__percent').exists()).toBe(false)
    // Раньше здесь рендерилась полностью залитая полоса (блочный div без
    // ширины = 100% родителя) — читалась как «готово», хотя remux ещё
    // идёт. Дизайн для этой фазы полосы не рисует вовсе.
    expect(wrapper.find('.download-panel__bar').exists()).toBe(false)
    expect(wrapper.find('[role="progressbar"]').exists()).toBe(false)
    expect(wrapper.find('[aria-busy="true"]').exists()).toBe(true)
  })
})

describe('DownloadPanel — Готово', () => {
  it('shows the final file name and a Hide button, with role="status", no Cancel/Retry', () => {
    const wrapper = mount(DownloadPanel, {
      props: {
        displayTitle: TITLE,
        plan: 'videoAndAudio',
        progress: { phase: 'done', fileName: 'Как приручить дракона.mp4' },
      },
    })

    expect(wrapper.find('[role="status"]').exists()).toBe(true)
    expect(wrapper.text()).toContain('Как приручить дракона.mp4')
    expect(wrapper.text()).toContain('папке «Загрузки»')
    expect(wrapper.findAll('button').map((b) => b.text())).toStrictEqual(['Скрыть'])
  })

  it('emits hide when the Hide button is clicked', async () => {
    const wrapper = mount(DownloadPanel, {
      props: {
        displayTitle: TITLE,
        plan: 'videoAndAudio',
        progress: { phase: 'done', fileName: 'x.mp4' },
      },
    })
    await wrapper.find('button').trigger('click')
    expect(wrapper.emitted('hide')).toHaveLength(1)
  })
})

describe('DownloadPanel — Ошибки: 9 классов (Ф-10, требование п.1 и п.2)', () => {
  const cases: { kind: DownloadErrorKind; expectTitle: string }[] = [
    { kind: 'connectionLost', expectTitle: 'Соединение потеряно' },
    { kind: 'diskFull', expectTitle: 'Не хватает места на диске' },
    { kind: 'staleFormat', expectTitle: 'Данные о ролике устарели' },
    { kind: 'mergeFailed', expectTitle: 'Не удалось склеить видео и звук' },
    { kind: 'destinationUnavailable', expectTitle: 'Папка «Загрузки» недоступна' },
    { kind: 'videoUnavailable', expectTitle: 'Ролик недоступен' },
    { kind: 'signInRequired', expectTitle: 'Требуется вход в аккаунт YouTube' },
    { kind: 'regionBlocked', expectTitle: 'Недоступно в вашем регионе' },
    { kind: 'ytDlpFailure', expectTitle: 'Не удалось скачать ролик' },
  ]

  it.each(cases)('renders the design-table title for $kind, not the diagnostic message', ({ kind, expectTitle }) => {
    const progress: DownloadProgress = {
      phase: 'failed',
      error: {
        kind,
        message: 'THIS-IS-CORE-DIAGNOSTIC-NOT-SCREEN-TEXT',
        retryable: true,
        partialData: 'kept',
      },
    }
    const wrapper = mount(DownloadPanel, {
      props: { displayTitle: TITLE, plan: 'videoAndAudio', progress },
    })

    expect(wrapper.text()).toContain(expectTitle)
    expect(wrapper.text()).not.toContain('THIS-IS-CORE-DIAGNOSTIC-NOT-SCREEN-TEXT')
  })

  it('shows the outdated yt-dlp sub-reason distinctly from the generic ytDlpFailure explanation (CLAUDE.md invariant)', () => {
    const genericWrapper = mount(DownloadPanel, {
      props: {
        displayTitle: TITLE,
        plan: 'videoAndAudio',
        progress: {
          phase: 'failed',
          error: { kind: 'ytDlpFailure', message: 'diag', retryable: true, partialData: 'kept' },
        },
      },
    })
    const outdatedWrapper = mount(DownloadPanel, {
      props: {
        displayTitle: TITLE,
        plan: 'videoAndAudio',
        progress: {
          phase: 'failed',
          error: {
            kind: 'ytDlpFailure',
            message: 'diag',
            retryable: true,
            partialData: 'kept',
            reason: 'outdated',
          },
        },
      },
    })

    expect(outdatedWrapper.text()).toContain('устарел')
    expect(outdatedWrapper.text()).not.toBe(genericWrapper.text())
  })

  it('shows the Retry button only when error.retryable is true — driven by the contract field, not a local copy of the table', () => {
    const retryable = mount(DownloadPanel, {
      props: {
        displayTitle: TITLE,
        plan: 'videoAndAudio',
        progress: {
          phase: 'failed',
          error: { kind: 'connectionLost', message: 'diag', retryable: true, partialData: 'kept' },
        },
      },
    })
    expect(retryable.findAll('button').some((b) => b.text() === 'Повторить')).toBe(true)

    const notRetryable = mount(DownloadPanel, {
      props: {
        displayTitle: TITLE,
        plan: 'videoAndAudio',
        progress: {
          phase: 'failed',
          error: { kind: 'signInRequired', message: 'diag', retryable: false, partialData: 'removed' },
        },
      },
    })
    expect(notRetryable.findAll('button').some((b) => b.text() === 'Повторить')).toBe(false)
  })

  it('shows the partial-data note driven by the partialData field, not derived from kind', () => {
    const kept = mount(DownloadPanel, {
      props: {
        displayTitle: TITLE,
        plan: 'videoAndAudio',
        progress: {
          phase: 'failed',
          error: { kind: 'connectionLost', message: 'diag', retryable: true, partialData: 'kept' },
        },
      },
    })
    expect(kept.text()).toContain('осталось на диске')

    const removed = mount(DownloadPanel, {
      props: {
        displayTitle: TITLE,
        plan: 'videoAndAudio',
        progress: {
          phase: 'failed',
          error: { kind: 'staleFormat', message: 'diag', retryable: false, partialData: 'removed' },
        },
      },
    })
    expect(removed.text()).toContain('удалены')
  })

  it('shows collapsible details only when details are present, and never leaks message as the title/explanation', () => {
    const withDetails = mount(DownloadPanel, {
      props: {
        displayTitle: TITLE,
        plan: 'videoAndAudio',
        progress: {
          phase: 'failed',
          error: {
            kind: 'mergeFailed',
            message: 'core message',
            retryable: true,
            partialData: 'kept',
            details: { stderrTail: 'ffmpeg: unknown codec', exitCode: 1 },
          },
        },
      },
    })
    expect(withDetails.find('details').exists()).toBe(true)
    expect(withDetails.text()).toContain('ffmpeg: unknown codec')

    const withoutDetails = mount(DownloadPanel, {
      props: {
        displayTitle: TITLE,
        plan: 'videoAndAudio',
        progress: {
          phase: 'failed',
          error: { kind: 'mergeFailed', message: 'core message', retryable: true, partialData: 'kept' },
        },
      },
    })
    expect(withoutDetails.find('details').exists()).toBe(false)
  })

  it('emits retry and hide from the failed terminal panel', async () => {
    const wrapper = mount(DownloadPanel, {
      props: {
        displayTitle: TITLE,
        plan: 'videoAndAudio',
        progress: {
          phase: 'failed',
          error: { kind: 'connectionLost', message: 'diag', retryable: true, partialData: 'kept' },
        },
      },
    })
    const retryButton = wrapper.findAll('button').find((b) => b.text() === 'Повторить')
    await retryButton?.trigger('click')
    expect(wrapper.emitted('retry')).toHaveLength(1)

    const hideButton = wrapper.findAll('button').find((b) => b.text() === 'Скрыть')
    await hideButton?.trigger('click')
    expect(wrapper.emitted('hide')).toHaveLength(1)
  })
})

describe('DownloadPanel — Отменено (Ф-4: подчистка всегда полная)', () => {
  it('shows a different message for cancel-in-Queued (nothingCreated) vs cancel-after-start (removed)', () => {
    const beforeStart = mount(DownloadPanel, {
      props: {
        displayTitle: TITLE,
        plan: 'videoAndAudio',
        progress: { phase: 'cancelled', partialData: 'nothingCreated' },
      },
    })
    expect(beforeStart.text()).toContain('отменена до начала скачивания')

    const afterStart = mount(DownloadPanel, {
      props: {
        displayTitle: TITLE,
        plan: 'videoAndAudio',
        progress: { phase: 'cancelled', partialData: 'removed' },
      },
    })
    expect(afterStart.text()).toContain('удалены')
  })

  it('offers only Hide, no Retry, on a cancelled panel', () => {
    const wrapper = mount(DownloadPanel, {
      props: {
        displayTitle: TITLE,
        plan: 'videoAndAudio',
        progress: { phase: 'cancelled', partialData: 'removed' },
      },
    })
    expect(wrapper.findAll('button').map((b) => b.text())).toStrictEqual(['Скрыть'])
  })
})
