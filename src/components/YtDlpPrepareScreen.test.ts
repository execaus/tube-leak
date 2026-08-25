import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import YtDlpPrepareScreen from './YtDlpPrepareScreen.vue'

describe('YtDlpPrepareScreen', () => {
  it('renders the unpacking label and progress without an ETA when absent', () => {
    const wrapper = mount(YtDlpPrepareScreen, {
      props: { stage: 'unpacking', percent: 4 },
    })

    expect(wrapper.text()).toContain('Распаковываем yt-dlp')
    expect(wrapper.text()).toContain('4%')
    expect(wrapper.text()).not.toContain('осталось')

    const bar = wrapper.find('[role="progressbar"]')
    expect(bar.attributes('aria-valuenow')).toBe('4')
  })

  it('renders the warmingUp label with an ETA in seconds when under a minute', () => {
    const wrapper = mount(YtDlpPrepareScreen, {
      props: { stage: 'warmingUp', percent: 52, etaSecs: 17 },
    })

    expect(wrapper.text()).toContain('Готовим yt-dlp к первому запуску')
    expect(wrapper.text()).toContain('52%')
    expect(wrapper.text()).toContain('осталось ~17 с')
  })

  it('formats an ETA of a minute or more as minutes and seconds', () => {
    const wrapper = mount(YtDlpPrepareScreen, {
      props: { stage: 'warmingUp', percent: 10, etaSecs: 90 },
    })

    expect(wrapper.text()).toContain('осталось ~1 мин 30 с')
  })

  it('omits seconds when the ETA is an exact number of minutes', () => {
    const wrapper = mount(YtDlpPrepareScreen, {
      props: { stage: 'warmingUp', percent: 10, etaSecs: 120 },
    })

    expect(wrapper.text()).toContain('осталось ~2 мин')
    expect(wrapper.text()).not.toContain('0 с')
  })

  it('explains that this happens only once and the next launch will be fast', () => {
    const wrapper = mount(YtDlpPrepareScreen, {
      props: { stage: 'unpacking', percent: 0 },
    })

    expect(wrapper.text()).toContain('один раз')
  })
})
