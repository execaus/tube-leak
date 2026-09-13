import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import ClearHistoryConfirmDialog from './ClearHistoryConfirmDialog.vue'

function mountDialog(props: { knownRecordCount?: number } = {}, attachTo?: HTMLElement) {
  return mount(ClearHistoryConfirmDialog, { props, attachTo })
}

describe('ClearHistoryConfirmDialog (С-4, дизайн E5, «Очистить — подтверждение»)', () => {
  it('is an alertdialog with the exact heading text', () => {
    const wrapper = mountDialog()
    const dialog = wrapper.get('[role="alertdialog"]')
    expect(dialog.attributes('aria-modal')).toBe('true')
    expect(wrapper.text()).toContain('Очистить всю историю?')
  })

  it('names the exact known count in the body when the loaded list is known to be complete', () => {
    const wrapper = mountDialog({ knownRecordCount: 42 })
    expect(wrapper.text()).toContain('Будут удалены все 42 записи истории.')
    expect(wrapper.text()).toContain('Файлы на диске не тронет ничего')
  })

  it('omits the number when the loaded list is not known to be complete', () => {
    const wrapper = mountDialog({ knownRecordCount: undefined })
    expect(wrapper.text()).toContain('Будут удалены все записи истории.')
  })

  it('both buttons meet the 40x40 tap-target class, "Отмена" first, "Очистить всё" second', () => {
    const wrapper = mountDialog()
    const buttons = wrapper.findAll('button')
    expect(buttons).toHaveLength(2)
    expect(buttons[0]?.text()).toBe('Отмена')
    expect(buttons[0]?.classes()).toContain('tap-target')
    expect(buttons[1]?.text()).toBe('Очистить всё')
    expect(buttons[1]?.classes()).toContain('tap-target')
  })

  it('emits "cancel" when "Отмена" is clicked, and "confirm" when "Очистить всё" is clicked', async () => {
    const wrapper = mountDialog()
    await wrapper.get('button:nth-of-type(1)').trigger('click')
    expect(wrapper.emitted('cancel')).toHaveLength(1)
    expect(wrapper.emitted('confirm')).toBeUndefined()

    await wrapper.get('button:nth-of-type(2)').trigger('click')
    expect(wrapper.emitted('confirm')).toHaveLength(1)
  })

  it('focuses "Отмена" by default (safe default per design)', async () => {
    const host = document.createElement('div')
    document.body.appendChild(host)
    const wrapper = mountDialog({}, host)
    await wrapper.vm.$nextTick()
    await wrapper.vm.$nextTick()
    expect(document.activeElement?.textContent?.trim()).toBe('Отмена')
    wrapper.unmount()
    host.remove()
  })

  it('Escape is equivalent to "Отмена" — emits "cancel", not "confirm"', async () => {
    const wrapper = mountDialog()
    await wrapper.get('[role="alertdialog"]').trigger('keydown', { key: 'Escape' })
    expect(wrapper.emitted('cancel')).toHaveLength(1)
    expect(wrapper.emitted('confirm')).toBeUndefined()
  })

  it('focus trap: Tab from the last button wraps to the first, Shift+Tab from the first wraps to the last', async () => {
    const host = document.createElement('div')
    document.body.appendChild(host)
    const wrapper = mountDialog({}, host)
    await wrapper.vm.$nextTick()
    await wrapper.vm.$nextTick()

    const cancel = wrapper.get('button:nth-of-type(1)').element as HTMLButtonElement
    const confirm = wrapper.get('button:nth-of-type(2)').element as HTMLButtonElement

    cancel.focus()
    await wrapper.get('[role="alertdialog"]').trigger('keydown', { key: 'Tab', shiftKey: true })
    expect(document.activeElement).toBe(confirm)

    confirm.focus()
    await wrapper.get('[role="alertdialog"]').trigger('keydown', { key: 'Tab' })
    expect(document.activeElement).toBe(cancel)

    wrapper.unmount()
    host.remove()
  })

  it('restores focus to the previously focused element on unmount', async () => {
    const host = document.createElement('div')
    document.body.appendChild(host)
    const trigger = document.createElement('button')
    trigger.textContent = 'Очистить'
    host.appendChild(trigger)
    trigger.focus()
    expect(document.activeElement).toBe(trigger)

    const wrapper = mountDialog({}, host)
    await wrapper.vm.$nextTick()
    await wrapper.vm.$nextTick()
    expect(document.activeElement).not.toBe(trigger)

    wrapper.unmount()
    expect(document.activeElement).toBe(trigger)
    host.remove()
  })
})
