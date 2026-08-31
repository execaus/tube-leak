import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import QueueWaitingRow from './QueueWaitingRow.vue'

describe('QueueWaitingRow', () => {
  it('renders the title and no-tasks-ahead status (дизайн, пример «Урок кулинарии»)', () => {
    const wrapper = mount(QueueWaitingRow, {
      props: { displayTitle: '«Урок кулинарии» — 480p', aheadCount: 0, awaitingContinue: false },
    })

    expect(wrapper.text()).toContain('«Урок кулинарии» — 480p')
    expect(wrapper.text()).toContain('В очереди — начнётся после текущей загрузки')
    expect(wrapper.text()).not.toContain('и ещё')
  })

  it('renders the "и ещё N задачи" tail for tasks ahead (дизайн, пример «Летний влог»)', () => {
    const wrapper = mount(QueueWaitingRow, {
      props: { displayTitle: '«Летний влог» — 720p', aheadCount: 1, awaitingContinue: false },
    })

    expect(wrapper.text()).toContain('и ещё 1 задачи')
  })

  it('switches to the "как только вы продолжите" wording while awaitingContinue', () => {
    const wrapper = mount(QueueWaitingRow, {
      props: { displayTitle: '«Как приручить дракона» — 1080p', aheadCount: 0, awaitingContinue: true },
    })

    expect(wrapper.text()).toContain('начнётся первой, как только вы продолжите')
    expect(wrapper.text()).not.toContain('после текущей загрузки')
  })

  it('has an aria-live="polite" status region (структурные события реже прогресса — дизайн, «Доступность»)', () => {
    const wrapper = mount(QueueWaitingRow, {
      props: { displayTitle: 'x', aheadCount: 0, awaitingContinue: false },
    })

    expect(wrapper.find('[aria-live="polite"]').exists()).toBe(true)
  })

  it('emits cancel when the button is clicked, and only cancel', async () => {
    const wrapper = mount(QueueWaitingRow, {
      props: { displayTitle: 'x', aheadCount: 0, awaitingContinue: false },
    })

    await wrapper.find('button').trigger('click')

    expect(wrapper.emitted('cancel')).toHaveLength(1)
  })

  it('has a 40x40 tap target for the cancel button (accessibility invariant, E1–E3)', () => {
    const wrapper = mount(QueueWaitingRow, {
      props: { displayTitle: 'x', aheadCount: 0, awaitingContinue: false },
    })

    expect(wrapper.find('button').classes()).toContain('tap-target')
  })
})
