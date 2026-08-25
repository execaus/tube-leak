import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import type { YtDlpPrepareError } from '@/types/ytdlp'

import YtDlpPrepareErrorComponent from './YtDlpPrepareError.vue'

const warmupFailed: YtDlpPrepareError = {
  kind: 'warmupFailed',
  message: 'yt-dlp не ответил за отведённое время прогрева',
}

const dataDirUnavailable: YtDlpPrepareError = {
  kind: 'dataDirUnavailable',
  message: 'app_data_dir() failed: no home directory',
}

describe('YtDlpPrepareError', () => {
  it('renders a human explanation for the given error kind, collapsed details, and a retry button', () => {
    const wrapper = mount(YtDlpPrepareErrorComponent, { props: { error: warmupFailed } })

    expect(wrapper.text()).toContain('Не удалось подготовить yt-dlp')
    expect(wrapper.text()).toContain('yt-dlp распаковался, но не запускается')
    expect(wrapper.find('dl').exists()).toBe(false)

    const retryButton = wrapper.findAll('button').find((b) => b.text().includes('Повторить'))
    expect(retryButton).toBeDefined()
  })

  it('renders a distinct explanation for a different error kind', () => {
    const wrapper = mount(YtDlpPrepareErrorComponent, { props: { error: dataDirUnavailable } })

    expect(wrapper.text()).toContain('рабочий каталог приложения')
  })

  it('reveals kind and message inside the details block on click', async () => {
    const wrapper = mount(YtDlpPrepareErrorComponent, { props: { error: warmupFailed } })

    const detailsButton = wrapper.findAll('button').find((b) => b.text().includes('Подробнее'))
    expect(detailsButton).toBeDefined()

    await detailsButton?.trigger('click')

    expect(wrapper.find('dl').exists()).toBe(true)
    expect(wrapper.text()).toContain('warmupFailed')
    expect(wrapper.text()).toContain(warmupFailed.message)
  })

  it('emits retry when the retry button is clicked', async () => {
    const wrapper = mount(YtDlpPrepareErrorComponent, { props: { error: warmupFailed } })

    const retryButton = wrapper.findAll('button').find((b) => b.text().includes('Повторить'))
    await retryButton?.trigger('click')

    expect(wrapper.emitted('retry')).toHaveLength(1)
  })
})
