import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import VideoThumbnail from './VideoThumbnail.vue'

describe('VideoThumbnail', () => {
  it('shows the placeholder (not the image) before the image has loaded', () => {
    const wrapper = mount(VideoThumbnail, {
      props: { src: 'https://i.ytimg.com/vi/x/hqdefault.jpg', alt: 'Ролик' },
    })

    expect(wrapper.find('.thumb__placeholder').isVisible()).toBe(true)
    expect(wrapper.find('img').classes()).toContain('thumb__img--hidden')
  })

  it('reveals the image and hides the placeholder once it loads', async () => {
    const wrapper = mount(VideoThumbnail, {
      props: { src: 'https://i.ytimg.com/vi/x/hqdefault.jpg', alt: 'Ролик' },
    })

    await wrapper.find('img').trigger('load')

    expect(wrapper.find('img').classes()).not.toContain('thumb__img--hidden')
    expect(wrapper.find('.thumb__placeholder').isVisible()).toBe(false)
  })

  it('keeps showing the same placeholder on a failed load — no distinct error state, no spinner', async () => {
    const wrapper = mount(VideoThumbnail, {
      props: { src: 'https://i.ytimg.com/vi/broken/hqdefault.jpg', alt: 'Ролик' },
    })

    await wrapper.find('img').trigger('error')

    expect(wrapper.find('.thumb__placeholder').isVisible()).toBe(true)
    expect(wrapper.find('img').classes()).toContain('thumb__img--hidden')
    expect(wrapper.findAll('.thumb__img--spin')).toHaveLength(0)
  })

  it('resets to the placeholder when the src prop changes to a new thumbnail', async () => {
    const wrapper = mount(VideoThumbnail, {
      props: { src: 'https://i.ytimg.com/vi/x/hqdefault.jpg', alt: 'Ролик X' },
    })
    await wrapper.find('img').trigger('load')
    expect(wrapper.find('img').classes()).not.toContain('thumb__img--hidden')

    await wrapper.setProps({ src: 'https://i.ytimg.com/vi/y/hqdefault.jpg', alt: 'Ролик Y' })

    expect(wrapper.find('img').classes()).toContain('thumb__img--hidden')
  })
})
