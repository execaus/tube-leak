import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import type { ProbeResult } from '@/types/probe'

import VideoCard from './VideoCard.vue'

const full: ProbeResult = {
  title: 'Заголовок ролика — с длинным & «странным» текстом',
  durationSecs: 3725,
  channel: 'Канал автора',
  thumbnailUrl: 'https://i.ytimg.com/vi/x/hqdefault.jpg',
  qualities: [{ kind: 'audioOnly', size: { kind: 'known', bytes: 14 * 1024 ** 2 }, streams: { audioFormatId: 'a' } }],
}

describe('VideoCard', () => {
  it('renders the title character-for-character (no truncation), duration, channel, and a thumbnail', () => {
    const wrapper = mount(VideoCard, { props: { result: full } })

    expect(wrapper.find('.video-card__title').text()).toBe(full.title)
    expect(wrapper.find('.video-card__duration').text()).toBe('1:02:05')
    expect(wrapper.find('.video-card__channel').text()).toBe('Канал автора')
    expect(wrapper.find('.thumb').exists()).toBe(true)
  })

  it('does not render the channel line when the channel is absent (card stays useful without it)', () => {
    const noChannel: ProbeResult = { ...full, channel: undefined }
    const wrapper = mount(VideoCard, { props: { result: noChannel } })

    expect(wrapper.find('.video-card__channel').exists()).toBe(false)
  })

  it('does not render the thumbnail area when thumbnailUrl is absent', () => {
    const noThumb: ProbeResult = { ...full, thumbnailUrl: undefined }
    const wrapper = mount(VideoCard, { props: { result: noThumb } })

    expect(wrapper.find('.thumb').exists()).toBe(false)
  })

  it('renders the quality ladder', () => {
    const wrapper = mount(VideoCard, { props: { result: full } })
    expect(wrapper.find('[role="radiogroup"]').exists()).toBe(true)
  })
})
