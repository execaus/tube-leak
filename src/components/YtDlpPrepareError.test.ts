import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import type { PrepareFailure } from '@/composables/useYtDlpPrepare'
import type { YtDlpPrepareError, YtDlpPrepareErrorKind } from '@/types/ytdlp'

import YtDlpPrepareErrorComponent from './YtDlpPrepareError.vue'

const warmupFailed: YtDlpPrepareError = {
  kind: 'warmupFailed',
  message: 'yt-dlp не ответил за отведённое время прогрева',
}

const dataDirUnavailable: YtDlpPrepareError = {
  kind: 'dataDirUnavailable',
  message: 'app_data_dir() failed: no home directory',
}

/**
 * Все шесть контрактных `kind` — по одному фрагменту текста, специфичному
 * именно для этой причины. `it.each` ловит копипасту (два kind с одним и
 * тем же объяснением) и пустую строку — то, что ревью TL-17 (#18, «Стоит
 * поправить») просило явно проверить, а не только рассуждением.
 */
const explanationFragmentByKind: Record<YtDlpPrepareErrorKind, string> = {
  dataDirUnavailable: 'рабочий каталог приложения',
  archiveMissing: 'отсутствует часть с yt-dlp',
  archiveCorrupted: 'повреждён',
  notEnoughSpace: 'не хватает места',
  unpackFailed: 'распаковать yt-dlp',
  layoutUnexpected: 'не нашёлся ожидаемый исполняемый файл',
  warmupFailed: 'распаковался, но не запускается',
}

const notEnoughSpace: YtDlpPrepareError = {
  kind: 'notEnoughSpace',
  message: 'не хватает места для распаковки yt-dlp: нужно ещё 130 МиБ, свободно 12 МиБ (/data/ytdlp)',
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

  it.each(Object.entries(explanationFragmentByKind) as Array<[YtDlpPrepareErrorKind, string]>)(
    'renders a non-empty, distinct explanation for kind=%s',
    (kind, fragment) => {
      const wrapper = mount(YtDlpPrepareErrorComponent, {
        props: { error: { kind, message: 'diagnostic detail' } },
      })

      expect(wrapper.text()).toContain(fragment)
    },
  )

  it('renders a distinct explanation for a different error kind (regression fixture)', () => {
    const wrapper = mount(YtDlpPrepareErrorComponent, { props: { error: dataDirUnavailable } })

    expect(wrapper.text()).toContain('рабочий каталог приложения')
  })

  describe('notEnoughSpace (TL-50, TL-18 mirror) — regression', () => {
    it('recognizes notEnoughSpace as a known kind and keeps the Rust message intact', () => {
      // До TL-50 `notEnoughSpace` не входил в белый список KNOWN_ERROR_KINDS
      // (useYtDlpPrepare.ts): composable подменял весь объект заглушкой
      // «Подготовка yt-dlp не удалась по нераспознанной причине» ещё до
      // того, как этот компонент получал `error` — числа needed/available
      // из Rust-сообщения не доезжали даже до «Подробнее».
      const wrapper = mount(YtDlpPrepareErrorComponent, { props: { error: notEnoughSpace } })

      expect(wrapper.text()).not.toContain('Не удалось разобрать причину отказа')
      expect(wrapper.text()).toContain('не хватает места')
    })

    it('advises freeing space and retrying, distinct from the irreversible unpackFailed advice', () => {
      const wrapper = mount(YtDlpPrepareErrorComponent, { props: { error: notEnoughSpace } })

      expect(wrapper.text()).toContain('освободите место')
      expect(wrapper.text()).not.toContain('Переустановите tube-leak')
    })

    it('exposes the Rust-formatted МиБ figures via the details block', async () => {
      const wrapper = mount(YtDlpPrepareErrorComponent, { props: { error: notEnoughSpace } })

      const detailsButton = wrapper.findAll('button').find((b) => b.text().includes('Подробнее'))
      await detailsButton?.trigger('click')

      expect(wrapper.text()).toContain('нужно ещё 130 МиБ, свободно 12 МиБ')
    })
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

  describe('non-contractual failure (no kind) — ревью TL-17, #18, «Обязательно»', () => {
    it('renders a generic fallback explanation instead of an empty one', () => {
      const nonContractual: PrepareFailure = { message: 'yt-dlp panicked: index out of bounds' }
      const wrapper = mount(YtDlpPrepareErrorComponent, { props: { error: nonContractual } })

      expect(wrapper.text()).toContain('Не удалось подготовить yt-dlp')
      expect(wrapper.text()).toContain('Не удалось разобрать причину отказа')
      expect(wrapper.text()).not.toContain('undefined')
    })

    it('still shows the raw message and a non-crashing kind label inside details', async () => {
      const nonContractual: PrepareFailure = { message: 'yt-dlp panicked: index out of bounds' }
      const wrapper = mount(YtDlpPrepareErrorComponent, { props: { error: nonContractual } })

      const detailsButton = wrapper.findAll('button').find((b) => b.text().includes('Подробнее'))
      await detailsButton?.trigger('click')

      expect(wrapper.text()).toContain('yt-dlp panicked: index out of bounds')
      expect(wrapper.text()).not.toContain('undefined')
    })

    it('still offers a working retry button', async () => {
      const nonContractual: PrepareFailure = { message: 'no useful message here either' }
      const wrapper = mount(YtDlpPrepareErrorComponent, { props: { error: nonContractual } })

      const retryButton = wrapper.findAll('button').find((b) => b.text().includes('Повторить'))
      await retryButton?.trigger('click')

      expect(wrapper.emitted('retry')).toHaveLength(1)
    })
  })
})
