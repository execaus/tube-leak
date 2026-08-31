import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import type { ProbeResult } from '@/types/generated/probe'

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

describe('VideoCard — кнопка «Скачать» (эпик E3, дизайн «Кнопка Скачать»)', () => {
  function downloadButton(wrapper: ReturnType<typeof mount>) {
    return wrapper.findAll('button').find((b) => b.text() === 'Скачать')
  }

  it('is disabled with no hint when nothing is selected yet', () => {
    const wrapper = mount(VideoCard, { props: { result: full } })
    expect(downloadButton(wrapper)?.attributes('disabled')).toBeDefined()
    expect(wrapper.find('.video-card__download-hint').exists()).toBe(false)
  })

  it('becomes enabled once a quality row is selected, with no hint about any other active download (TL-74/TL-75, Ф-2 E4 — постановка при занятом слоте больше не отказ)', async () => {
    const wrapper = mount(VideoCard, { props: { result: full } })
    await wrapper.find('input[type="radio"]').setValue(true)

    expect(downloadButton(wrapper)?.attributes('disabled')).toBeUndefined()
    expect(wrapper.find('.video-card__download-hint').exists()).toBe(false)
    expect(wrapper.text()).not.toContain('Уже идёт другая загрузка')
  })

  it('has no aria-describedby — there is no hint left to point to (TL-74)', () => {
    const wrapper = mount(VideoCard, { props: { result: full } })
    expect(downloadButton(wrapper)?.attributes('aria-describedby')).toBeUndefined()
  })

  it('emits download with the title, the streams of the selected item, and its structured quality (kind/heightPx, TL-75, эпик E4)', async () => {
    const wrapper = mount(VideoCard, { props: { result: full } })
    await wrapper.find('input[type="radio"]').setValue(true)
    await downloadButton(wrapper)?.trigger('click')

    expect(wrapper.emitted('download')).toStrictEqual([
      [
        {
          title: full.title,
          streams: { audioFormatId: 'a' },
          size: { kind: 'known', bytes: 14 * 1024 ** 2 },
          quality: { kind: 'audioOnly', heightPx: undefined },
        },
      ],
    ])
  })

  it('emits the exact heightPx of a standard step (TL-75) — needed to rebuild the queue title after a restart', async () => {
    const withStandard: ProbeResult = {
      ...full,
      qualities: [{ kind: 'standard', heightPx: 1080, size: { kind: 'unknown' }, streams: { videoFormatId: 'v' } }],
    }
    const wrapper = mount(VideoCard, { props: { result: withStandard } })
    await wrapper.find('input[type="radio"]').setValue(true)
    await downloadButton(wrapper)?.trigger('click')

    expect(wrapper.emitted('download')?.[0]?.[0]).toMatchObject({
      quality: { kind: 'standard', heightPx: 1080 },
    })
  })
})
