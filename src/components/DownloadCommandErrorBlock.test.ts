import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import type { DownloadCommandError } from '@/types/generated/download'

import DownloadCommandErrorBlock from './DownloadCommandErrorBlock.vue'

const DUPLICATE_EXISTING = {
  taskId: 'task-existing',
  title: 'Летний влог',
  quality: { kind: 'standard' as const, heightPx: 720 },
}

const SEVEN_CLASSES: DownloadCommandError[] = [
  { kind: 'unknownTask', message: 'diag' },
  { kind: 'notFailed', message: 'diag' },
  { kind: 'notRetryable', message: 'diag' },
  { kind: 'noStreamsSelected', message: 'diag' },
  { kind: 'invalidUrl', message: 'diag' },
  { kind: 'duplicateTask', message: 'diag', existing: DUPLICATE_EXISTING },
  { kind: 'taskNotFinished', message: 'diag' },
]

describe('DownloadCommandErrorBlock', () => {
  it('renders the title/explanation for a known class, never the diagnostic message, and role="alert"', () => {
    const wrapper = mount(DownloadCommandErrorBlock, {
      props: { error: { kind: 'invalidUrl', message: 'THIS-IS-CORE-DIAGNOSTIC' } },
    })

    expect(wrapper.attributes('role')).toBe('alert')
    expect(wrapper.text()).toContain('Ссылка не распознана')
    expect(wrapper.text()).not.toContain('THIS-IS-CORE-DIAGNOSTIC')
  })

  it('never renders a Retry button — none of the seven classes are solved by retrying the same call (TL-70/TL-75)', () => {
    for (const error of SEVEN_CLASSES) {
      const wrapper = mount(DownloadCommandErrorBlock, { props: { error } })
      expect(wrapper.findAll('button').map((b) => b.text())).toStrictEqual(['Скрыть'])
    }
  })

  it('names the exact existing task (title + quality) for duplicateTask, and does not leak the diagnostic message (Р-5)', () => {
    const wrapper = mount(DownloadCommandErrorBlock, {
      props: { error: { kind: 'duplicateTask', message: 'CORE-DIAGNOSTIC', existing: DUPLICATE_EXISTING } },
    })

    expect(wrapper.text()).toContain('«Летний влог» — 720p')
    expect(wrapper.text()).not.toContain('CORE-DIAGNOSTIC')
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
      props: { error: { kind: 'taskNotFinished', message: 'diag' } },
    })
    await wrapper.find('button').trigger('click')
    expect(wrapper.emitted('hide')).toHaveLength(1)
  })
})
