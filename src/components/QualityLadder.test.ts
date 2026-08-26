import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import { defineComponent, h } from 'vue'

import type { QualityItem } from '@/types/probe'

import QualityLadder from './QualityLadder.vue'

const fullLadder: QualityItem[] = [
  { kind: 'standard', heightPx: 2160, size: { kind: 'known', bytes: 1.8 * 1024 ** 3 }, streams: { videoFormatId: 'v2160', audioFormatId: 'a' } },
  { kind: 'standard', heightPx: 1440, size: { kind: 'known', bytes: 980 * 1024 ** 2 }, streams: { videoFormatId: 'v1440', audioFormatId: 'a' } },
  { kind: 'standard', heightPx: 1080, size: { kind: 'known', bytes: 512 * 1024 ** 2 }, streams: { videoFormatId: 'v1080', audioFormatId: 'a' } },
  { kind: 'standard', heightPx: 720, size: { kind: 'unknown' }, streams: { videoFormatId: 'v720', audioFormatId: 'a' } },
  { kind: 'audioOnly', size: { kind: 'known', bytes: 14 * 1024 ** 2 }, streams: { audioFormatId: 'a' } },
]

describe('QualityLadder', () => {
  it('renders as an accessible radiogroup with a real radio input per row', () => {
    const wrapper = mount(QualityLadder, { props: { items: fullLadder } })

    expect(wrapper.attributes('role')).toBe('radiogroup')
    const inputs = wrapper.findAll('input[type="radio"]')
    expect(inputs).toHaveLength(5)
  })

  it('renders rows in the given order (no client-side sorting), with labels and sizes', () => {
    const wrapper = mount(QualityLadder, { props: { items: fullLadder } })
    const rows = wrapper.findAll('.ladder__row')

    expect(rows.map((r) => r.find('.ladder__label').text())).toStrictEqual([
      '2160p',
      '1440p',
      '1080p',
      '720p',
      'Только аудио',
    ])
    expect(rows.map((r) => r.find('.ladder__size').text())).toStrictEqual([
      '≈ 1.8 ГБ',
      '≈ 980 МБ',
      '≈ 512 МБ',
      'размер неизвестен',
      '≈ 14 МБ',
    ])
  })

  it('has nothing selected by default (Р-1: no pre-selected best quality)', () => {
    const wrapper = mount(QualityLadder, { props: { items: fullLadder } })
    const inputs = wrapper.findAll('input[type="radio"]')
    expect(inputs.some((i) => (i.element as HTMLInputElement).checked)).toBe(false)
  })

  it('marks a row selected on click, and only one row at a time', async () => {
    const wrapper = mount(QualityLadder, { props: { items: fullLadder } })
    const inputs = wrapper.findAll('input[type="radio"]')

    await inputs[2]?.setValue(true)
    expect((inputs[2]?.element as HTMLInputElement).checked).toBe(true)

    await inputs[0]?.setValue(true)
    expect((inputs[0]?.element as HTMLInputElement).checked).toBe(true)
    expect((inputs[2]?.element as HTMLInputElement).checked).toBe(false)
  })

  it('resets the selection when the item list is replaced (new probe or cleared field)', async () => {
    const wrapper = mount(QualityLadder, { props: { items: fullLadder } })
    const inputs = wrapper.findAll('input[type="radio"]')
    await inputs[1]?.setValue(true)
    expect((inputs[1]?.element as HTMLInputElement).checked).toBe(true)

    const otherLadder: QualityItem[] = [
      { kind: 'maxAvailable', heightPx: 480, size: { kind: 'known', bytes: 96 * 1024 ** 2 }, streams: { videoFormatId: 'v480' } },
      { kind: 'audioOnly', size: { kind: 'known', bytes: 6 * 1024 ** 2 }, streams: { audioFormatId: 'a' } },
    ]
    await wrapper.setProps({ items: otherLadder })

    const newInputs = wrapper.findAll('input[type="radio"]')
    expect(newInputs.some((i) => (i.element as HTMLInputElement).checked)).toBe(false)
    expect(wrapper.find('.ladder__label').text()).toBe('Максимальное доступное (480p)')
  })

  it('emits update:selected with the chosen item on click (эпик E3, TL-45)', async () => {
    const wrapper = mount(QualityLadder, { props: { items: fullLadder } })
    const inputs = wrapper.findAll('input[type="radio"]')

    await inputs[2]?.setValue(true)

    const emitted = wrapper.emitted('update:selected')
    expect(emitted).toHaveLength(1)
    expect(emitted?.[0]).toStrictEqual([fullLadder[2]])
  })

  it('emits update:selected with undefined when the item list is replaced (selection reset)', async () => {
    const wrapper = mount(QualityLadder, { props: { items: fullLadder } })
    const inputs = wrapper.findAll('input[type="radio"]')
    await inputs[1]?.setValue(true)

    const otherLadder: QualityItem[] = [
      { kind: 'audioOnly', size: { kind: 'known', bytes: 6 * 1024 ** 2 }, streams: { audioFormatId: 'a' } },
    ]
    await wrapper.setProps({ items: otherLadder })

    const emitted = wrapper.emitted('update:selected')
    expect(emitted?.at(-1)).toStrictEqual([undefined])
  })

  it('scopes the radio group name per instance — two ladders on the same screen do not merge into one group (E4 concern)', () => {
    // Обе лестницы обязаны жить в одном дереве приложения — `useId()`
    // уникален в рамках инстанса Vue-приложения, а не глобально, поэтому
    // два независимых mount() тут дали бы одинаковый id и не проверяли бы
    // ничего.
    const TwoLadders = defineComponent({
      render: () =>
        h('div', [
          h(QualityLadder, { items: fullLadder, class: 'first' }),
          h(QualityLadder, { items: fullLadder, class: 'second' }),
        ]),
    })

    const wrapper = mount(TwoLadders)
    const firstName = wrapper.find('.first input[type="radio"]').attributes('name')
    const secondName = wrapper.find('.second input[type="radio"]').attributes('name')

    expect(firstName).toBeTruthy()
    expect(secondName).toBeTruthy()
    expect(firstName).not.toBe(secondName)
  })
})
