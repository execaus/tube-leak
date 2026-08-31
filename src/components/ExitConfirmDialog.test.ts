import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import type { DownloadProgress } from '@/types/generated/download'
import ExitConfirmDialog from './ExitConfirmDialog.vue'

function mountDialog(
  props: { activeTask?: { displayTitle: string; progress: DownloadProgress }; pauseReason?: 'ytDlpUpdate'; waitingCount?: number },
  attachTo?: HTMLElement,
) {
  return mount(ExitConfirmDialog, {
    props: { waitingCount: 0, ...props },
    attachTo,
  })
}

function activeTask(progress: DownloadProgress, displayTitle = '«Как приручить дракона» — 1080p') {
  return { displayTitle, progress }
}

describe('ExitConfirmDialog — диалог выхода для очереди (дизайн E4, TL-76)', () => {
  it('is an alertdialog with the heading and body from getExitDialogText', () => {
    const wrapper = mountDialog({ activeTask: activeTask({ phase: 'downloading', state: 'running', percent: 62 }) })
    const dialog = wrapper.get('[role="alertdialog"]')
    expect(dialog.attributes('aria-modal')).toBe('true')
    expect(wrapper.text()).toContain('Очередь ещё не завершена')
    expect(wrapper.text()).toContain('скачивается (62 %)')
  })

  it('renders the no-percent text for Queued/Fetching', () => {
    const wrapper = mountDialog({ activeTask: activeTask({ phase: 'fetching' }) })
    expect(wrapper.text()).toContain('загрузка ещё готовится')
  })

  it('renders the waiting count when greater than zero', () => {
    const wrapper = mountDialog({
      activeTask: activeTask({ phase: 'downloading', state: 'running', percent: 62 }),
      waitingCount: 2,
    })
    expect(wrapper.text()).toContain('Ещё в очереди: 2 задачи')
  })

  it('does not render a waiting-count sentence when it is zero', () => {
    const wrapper = mountDialog({ activeTask: activeTask({ phase: 'queued' }), waitingCount: 0 })
    expect(wrapper.text()).not.toContain('Ещё в очереди')
  })

  it('renders the yt-dlp update pause sentence instead of a percent, without a named task', () => {
    const wrapper = mountDialog({ pauseReason: 'ytDlpUpdate', waitingCount: 1 })
    expect(wrapper.text()).toContain('Между загрузками устанавливается обновлённый yt-dlp.')
    expect(wrapper.text()).toContain('Ещё в очереди: 1 задача')
  })

  it('never claims the user must paste the link again — that became false with the queue snapshot', () => {
    const wrapper = mountDialog({ activeTask: activeTask({ phase: 'queued' }), waitingCount: 3 })
    expect(wrapper.text()).not.toMatch(/вставьте/i)
    expect(wrapper.text()).not.toMatch(/ссылку/i)
  })

  it('both buttons meet the 40x40 tap-target class and "Остаться" comes first', () => {
    const wrapper = mountDialog({ activeTask: activeTask({ phase: 'queued' }) })
    const buttons = wrapper.findAll('button')
    expect(buttons).toHaveLength(2)
    expect(buttons[0]?.text()).toBe('Остаться')
    expect(buttons[0]?.classes()).toContain('tap-target')
    expect(buttons[1]?.text()).toBe('Всё равно выйти')
    expect(buttons[1]?.classes()).toContain('tap-target')
  })

  it('emits "stay" when "Остаться" is clicked, and "exitAnyway" when "Всё равно выйти" is clicked', async () => {
    const wrapper = mountDialog({ activeTask: activeTask({ phase: 'queued' }) })
    await wrapper.get('button:nth-of-type(1)').trigger('click')
    expect(wrapper.emitted('stay')).toHaveLength(1)
    expect(wrapper.emitted('exitAnyway')).toBeUndefined()

    await wrapper.get('button:nth-of-type(2)').trigger('click')
    expect(wrapper.emitted('exitAnyway')).toHaveLength(1)
  })

  it('focuses "Остаться" by default (safe default per design)', async () => {
    const host = document.createElement('div')
    document.body.appendChild(host)
    const wrapper = mountDialog({ activeTask: activeTask({ phase: 'queued' }) }, host)
    await wrapper.vm.$nextTick()
    await wrapper.vm.$nextTick()
    expect(document.activeElement?.textContent?.trim()).toBe('Остаться')
    wrapper.unmount()
    host.remove()
  })

  it('Escape is equivalent to "Остаться" — emits "stay", not "exitAnyway"', async () => {
    const wrapper = mountDialog({ activeTask: activeTask({ phase: 'queued' }) })
    await wrapper.get('[role="alertdialog"]').trigger('keydown', { key: 'Escape' })
    expect(wrapper.emitted('stay')).toHaveLength(1)
    expect(wrapper.emitted('exitAnyway')).toBeUndefined()
  })

  it('focus trap: Tab from the last button wraps to the first, Shift+Tab from the first wraps to the last', async () => {
    const host = document.createElement('div')
    document.body.appendChild(host)
    const wrapper = mountDialog({ activeTask: activeTask({ phase: 'queued' }) }, host)
    await wrapper.vm.$nextTick()
    await wrapper.vm.$nextTick()

    const stay = wrapper.get('button:nth-of-type(1)').element as HTMLButtonElement
    const exit = wrapper.get('button:nth-of-type(2)').element as HTMLButtonElement

    stay.focus()
    await wrapper.get('[role="alertdialog"]').trigger('keydown', { key: 'Tab', shiftKey: true })
    expect(document.activeElement).toBe(exit)

    exit.focus()
    await wrapper.get('[role="alertdialog"]').trigger('keydown', { key: 'Tab' })
    expect(document.activeElement).toBe(stay)

    wrapper.unmount()
    host.remove()
  })

  it('restores focus to the previously focused element on unmount', async () => {
    const host = document.createElement('div')
    document.body.appendChild(host)
    const trigger = document.createElement('button')
    trigger.textContent = 'Закрыть окно'
    host.appendChild(trigger)
    trigger.focus()
    expect(document.activeElement).toBe(trigger)

    const wrapper = mountDialog({ activeTask: activeTask({ phase: 'queued' }) }, host)
    await wrapper.vm.$nextTick()
    await wrapper.vm.$nextTick()
    expect(document.activeElement).not.toBe(trigger)

    wrapper.unmount()
    expect(document.activeElement).toBe(trigger)
    host.remove()
  })
})
