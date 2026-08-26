import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import type { ProbeFailure } from '@/composables/useProbe'

import ProbeErrorBlock from './ProbeErrorBlock.vue'

const DECOY_MESSAGE = 'DECOY-MESSAGE-NOT-FROM-TABLE'

const fixtures: Array<{
  error: ProbeFailure
  expectedTitle: string
  expectRetry: boolean
}> = [
  {
    error: { kind: 'videoUnavailable', message: DECOY_MESSAGE },
    expectedTitle: 'Ролик недоступен',
    expectRetry: true,
  },
  {
    error: { kind: 'signInRequired', message: DECOY_MESSAGE },
    expectedTitle: 'Требуется вход в аккаунт YouTube',
    expectRetry: false,
  },
  {
    error: { kind: 'regionBlocked', message: DECOY_MESSAGE },
    expectedTitle: 'Недоступно в вашем регионе',
    expectRetry: false,
  },
  {
    error: { kind: 'networkUnavailable', message: DECOY_MESSAGE },
    expectedTitle: 'Нет соединения с интернетом',
    expectRetry: true,
  },
  {
    error: { kind: 'playlistUnsupported', message: DECOY_MESSAGE },
    expectedTitle: 'Плейлисты и каналы пока не поддерживаются',
    expectRetry: false,
  },
  {
    error: { kind: 'liveUnsupported', message: DECOY_MESSAGE },
    expectedTitle: 'Прямые трансляции не поддерживаются',
    expectRetry: false,
  },
  {
    error: { kind: 'ytDlpFailure', reason: 'generic', message: DECOY_MESSAGE },
    expectedTitle: 'Не удалось получить данные о ролике',
    expectRetry: true,
  },
  {
    error: { kind: 'ytDlpFailure', reason: 'outdated', message: DECOY_MESSAGE },
    expectedTitle: 'Не удалось получить данные о ролике',
    expectRetry: true,
  },
  {
    error: { kind: 'timeout', timeoutSecs: 30, message: DECOY_MESSAGE },
    expectedTitle: 'Разбор не завершился',
    expectRetry: true,
  },
]

describe('ProbeErrorBlock — заголовок из таблицы, не из message (нормативный тест TL-33)', () => {
  it.each(fixtures)(
    'kind=$error.kind: заголовок рендерится из таблицы дизайна и не совпадает с message',
    ({ error, expectedTitle }) => {
      const wrapper = mount(ProbeErrorBlock, { props: { error } })

      expect(wrapper.find('.probe-error__title').text()).toBe(expectedTitle)
      expect(wrapper.find('.probe-error__title').text()).not.toBe(error.message)
      expect(wrapper.text()).not.toContain(DECOY_MESSAGE)
    },
  )

  it.each(fixtures)('kind=$error.kind: показывает «Повторить» только там, где предписывает таблица', ({ error, expectRetry }) => {
    const wrapper = mount(ProbeErrorBlock, { props: { error } })
    const retryButton = wrapper.findAll('button').find((b) => b.text() === 'Повторить')
    expect(retryButton !== undefined).toBe(expectRetry)
  })

  it('surfaces the decoy message only inside the collapsed "Подробнее" block, not in the main text', async () => {
    const wrapper = mount(ProbeErrorBlock, {
      props: { error: { kind: 'videoUnavailable', message: DECOY_MESSAGE } },
    })

    expect(wrapper.text()).not.toContain(DECOY_MESSAGE)

    const detailsButton = wrapper.findAll('button').find((b) => b.text().includes('Подробнее'))
    await detailsButton?.trigger('click')

    expect(wrapper.text()).toContain(DECOY_MESSAGE)
  })

  it('cannot even be typed with notAUrl — the blocker fix rules it out at compile time, not just by convention', () => {
    // @ts-expect-error notAUrl уходит в состояние `notAUrl` (инлайн под полем,
    // не блок) на уровне useLinkProbe — ProbeFailure структурно его не
    // допускает, а не просто «не должен туда попадать по соглашению».
    const notAllowed: ProbeFailure = { kind: 'notAUrl', message: 'unused' }
    expect(notAllowed).toBeDefined()
  })
})

describe('ProbeErrorBlock — неконтрактный отказ', () => {
  it('falls back to an honest generic title/explanation instead of a blank screen', () => {
    const wrapper = mount(ProbeErrorBlock, {
      props: { error: { message: 'IPC exploded' } },
    })

    expect(wrapper.find('.probe-error__title').text()).toBe('Не удалось получить данные о ролике')
    expect(wrapper.find('.probe-error__explanation').text()).not.toBe('IPC exploded')
    expect(wrapper.findAll('button').some((b) => b.text() === 'Повторить')).toBe(true)
  })
})

describe('ProbeErrorBlock — события', () => {
  it('emits retry when the button is clicked', async () => {
    const wrapper = mount(ProbeErrorBlock, {
      props: { error: { kind: 'networkUnavailable', message: 'offline' } },
    })
    const retryButton = wrapper.findAll('button').find((b) => b.text() === 'Повторить')
    await retryButton?.trigger('click')
    expect(wrapper.emitted('retry')).toHaveLength(1)
  })
})
