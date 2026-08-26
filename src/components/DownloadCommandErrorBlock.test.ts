import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import DownloadCommandErrorBlock from './DownloadCommandErrorBlock.vue'

describe('DownloadCommandErrorBlock', () => {
  it('renders the title/explanation for a known class, never the diagnostic message, and role="alert"', () => {
    const wrapper = mount(DownloadCommandErrorBlock, {
      props: { error: { kind: 'invalidUrl', message: 'THIS-IS-CORE-DIAGNOSTIC' } },
    })

    expect(wrapper.attributes('role')).toBe('alert')
    expect(wrapper.text()).toContain('Ссылка не распознана')
    expect(wrapper.text()).not.toContain('THIS-IS-CORE-DIAGNOSTIC')
  })

  it('never renders a Retry button — none of the six classes are solved by retrying the same call', () => {
    for (const kind of [
      'alreadyActive',
      'unknownTask',
      'notFailed',
      'notRetryable',
      'noStreamsSelected',
      'invalidUrl',
    ] as const) {
      const wrapper = mount(DownloadCommandErrorBlock, {
        props: { error: { kind, message: 'diag' } },
      })
      expect(wrapper.findAll('button').map((b) => b.text())).toStrictEqual(['Скрыть'])
    }
  })

  it('falls back to a generic text for a non-contractual failure (unrecognized shape)', () => {
    const wrapper = mount(DownloadCommandErrorBlock, {
      props: { error: { message: 'some unexpected rejection' } },
    })
    expect(wrapper.text()).toContain('Не удалось выполнить команду')
    expect(wrapper.text()).not.toContain('some unexpected rejection')
  })

  it('emits hide when the Hide button is clicked', async () => {
    const wrapper = mount(DownloadCommandErrorBlock, {
      props: { error: { kind: 'alreadyActive', message: 'diag' } },
    })
    await wrapper.find('button').trigger('click')
    expect(wrapper.emitted('hide')).toHaveLength(1)
  })
})
