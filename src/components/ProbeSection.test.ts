import { mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { ProbeError, ProbeResult } from '@/types/probe'

const invokeMock = vi.fn()

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}))

// Импортируется после мока `invoke` (тот же приём, что в App.test.ts).
const { default: ProbeSection } = await import('./ProbeSection.vue')

const resultA: ProbeResult = {
  title: 'Ролик A',
  durationSecs: 65,
  qualities: [{ kind: 'audioOnly', size: { kind: 'unknown' }, streams: { audioFormatId: 'a' } }],
}

const resultB: ProbeResult = {
  title: 'Ролик B',
  durationSecs: 90,
  qualities: [{ kind: 'audioOnly', size: { kind: 'unknown' }, streams: { audioFormatId: 'b' } }],
}

beforeEach(() => {
  vi.useFakeTimers()
  invokeMock.mockReset()
})

afterEach(() => {
  vi.useRealTimers()
})

describe('ProbeSection — гейт по статусу yt-dlp (дизайн E2, «Где живёт поле ссылки»)', () => {
  it('disables the field with a "checking" placeholder while yt-dlp is still being checked', () => {
    const wrapper = mount(ProbeSection, { props: { ytDlpState: 'checking' } })
    const input = wrapper.find('input')
    expect(input.attributes('disabled')).toBeDefined()
    expect(input.attributes('placeholder')).toBe('Проверяем yt-dlp…')
  })

  it('disables the field with a hint (not repeating the sidecar error text) when yt-dlp is not ok', () => {
    const wrapper = mount(ProbeSection, { props: { ytDlpState: 'blocked' } })
    const input = wrapper.find('input')
    expect(input.attributes('disabled')).toBeDefined()
    expect(input.attributes('placeholder')).toBe(
      'Разбор ссылок недоступен, пока не решена проблема с yt-dlp выше',
    )
  })

  it('enables the field once yt-dlp is ok', () => {
    const wrapper = mount(ProbeSection, { props: { ytDlpState: 'ready' } })
    const input = wrapper.find('input')
    expect(input.attributes('disabled')).toBeUndefined()
    expect(input.attributes('placeholder')).toBe('Вставьте ссылку на ролик YouTube')
  })
})

describe('ProbeSection — состояние 0, пусто', () => {
  it('shows the example hint and the permanent responsibility disclaimer (Р-3), nothing else', () => {
    const wrapper = mount(ProbeSection, { props: { ytDlpState: 'ready' } })
    expect(wrapper.text()).toContain('например, https://www.youtube.com/watch?v=…')
    expect(wrapper.text()).toContain('Скачивайте только то, на что у вас есть право')
    expect(wrapper.find('[role="alert"]').exists()).toBe(false)
    expect(wrapper.find('[role="radiogroup"]').exists()).toBe(false)
  })
})

describe('ProbeSection — «не ссылка» инлайн, не блок (С-4)', () => {
  it('shows the inline aria-live message under the field, with no block, no retry, no process launch', async () => {
    const wrapper = mount(ProbeSection, { props: { ytDlpState: 'ready' } })
    await wrapper.find('input').setValue('просто текст')

    const inline = wrapper.find('.probe-section__inline-error')
    expect(inline.attributes('aria-live')).toBe('polite')
    expect(inline.text()).toContain('не похоже на ссылку')
    expect(wrapper.find('[role="alert"]').exists()).toBe(false)
    expect(wrapper.findAll('button').find((b) => b.text() === 'Повторить')).toBeUndefined()
    expect(invokeMock).not.toHaveBeenCalled()
  })

  it('clears the inline message once the field looks like a url again', async () => {
    const wrapper = mount(ProbeSection, { props: { ytDlpState: 'ready' } })
    await wrapper.find('input').setValue('просто текст')
    expect(wrapper.find('.probe-section__inline-error').text()).not.toBe('')

    invokeMock.mockReturnValue(new Promise(() => {}))
    await wrapper.find('input').setValue('https://youtu.be/x')
    expect(wrapper.find('.probe-section__inline-error').text()).toBe('')
  })

  it('renders a notAUrl rejection from the core the same way as the instant check — inline, no block, no retry (blocker fix)', async () => {
    // Путь реальный: фронтовая проверка — не полная валидация (Ф-2), можно
    // замереть на 400мс сразу после "https://" и получить честный notAUrl
    // от ядра уже после того, как разбор запустился.
    const wrapper = mount(ProbeSection, { props: { ytDlpState: 'ready' } })
    const err: ProbeError = { kind: 'notAUrl', message: 'core: rejected shape' }
    invokeMock.mockRejectedValueOnce(err)

    await wrapper.find('input').setValue('https://y.y')
    await vi.advanceTimersByTimeAsync(400)
    await vi.waitFor(() => {
      expect(wrapper.find('.probe-section__inline-error').text()).not.toBe('')
    })

    expect(wrapper.find('[role="alert"]').exists()).toBe(false)
    expect(wrapper.findAll('button').find((b) => b.text() === 'Повторить')).toBeUndefined()
    expect(wrapper.find('.probe-section__inline-error').text()).toContain('не похоже на ссылку')
  })
})

describe('ProbeSection — «получаем данные…» и медленная подсказка', () => {
  it('shows the loading state ~400ms after input settles, then a slow hint after ~6s', async () => {
    const wrapper = mount(ProbeSection, { props: { ytDlpState: 'ready' } })
    invokeMock.mockReturnValue(new Promise(() => {}))

    await wrapper.find('input').setValue('https://youtu.be/x')
    await vi.advanceTimersByTimeAsync(400)

    expect(wrapper.text()).toContain('Получаем данные о ролике…')
    expect(wrapper.text()).not.toContain('это иногда занимает больше времени')

    await vi.advanceTimersByTimeAsync(6000)
    expect(wrapper.text()).toContain('это иногда занимает больше времени')
  })
})

describe('ProbeSection — карточка и её замена (С-2, С-3)', () => {
  it('renders the card on success', async () => {
    const wrapper = mount(ProbeSection, { props: { ytDlpState: 'ready' } })
    invokeMock.mockResolvedValueOnce(resultA)

    await wrapper.find('input').setValue('https://youtu.be/a')
    await vi.advanceTimersByTimeAsync(400)
    await vi.waitFor(() => {
      expect(wrapper.text()).toContain('Ролик A')
    })
    expect(wrapper.find('[role="radiogroup"]').exists()).toBe(true)
  })

  it('removes the stale card immediately when a second link is inserted before the first probe settles (К-4)', async () => {
    const wrapper = mount(ProbeSection, { props: { ytDlpState: 'ready' } })

    let resolveA: (value: ProbeResult) => void = () => {}
    invokeMock.mockImplementationOnce(
      () =>
        new Promise<ProbeResult>((resolve) => {
          resolveA = resolve
        }),
    )

    await wrapper.find('input').setValue('https://youtu.be/a')
    await vi.advanceTimersByTimeAsync(400)

    invokeMock.mockResolvedValueOnce(resultB)
    await wrapper.find('input').setValue('https://youtu.be/b')
    await vi.advanceTimersByTimeAsync(400)

    await vi.waitFor(() => {
      expect(wrapper.text()).toContain('Ролик B')
    })
    expect(wrapper.text()).not.toContain('Ролик A')

    // Ролик A «честно» приходит из ядра позже — экран обязан остаться про B.
    resolveA(resultA)
    await Promise.resolve()
    await Promise.resolve()
    expect(wrapper.text()).toContain('Ролик B')
    expect(wrapper.text()).not.toContain('Ролик A')
  })
})

describe('ProbeSection — ошибки (8 классов из 9, кроме notAUrl)', () => {
  it('renders an error block with role="alert" and wires the retry click back into the composable', async () => {
    const wrapper = mount(ProbeSection, { props: { ytDlpState: 'ready' } })
    const err: ProbeError = { kind: 'networkUnavailable', message: 'core diagnostic, unused' }
    invokeMock.mockRejectedValueOnce(err)

    await wrapper.find('input').setValue('https://youtu.be/x')
    await vi.advanceTimersByTimeAsync(400)
    await vi.waitFor(() => {
      expect(wrapper.find('[role="alert"]').exists()).toBe(true)
    })
    expect(wrapper.text()).toContain('Нет соединения с интернетом')
    expect(wrapper.text()).not.toContain('core diagnostic, unused')

    invokeMock.mockResolvedValueOnce(resultA)
    const retryButton = wrapper.findAll('button').find((b) => b.text() === 'Повторить')
    await retryButton?.trigger('click')

    await vi.waitFor(() => {
      expect(wrapper.text()).toContain('Ролик A')
    })
  })
})

describe('ProbeSection — ретрансляция запроса скачивания (эпик E3, TL-45)', () => {
  it('relays VideoCard\'s download event upward, adding the current url', async () => {
    const wrapper = mount(ProbeSection, { props: { ytDlpState: 'ready' } })
    invokeMock.mockResolvedValueOnce(resultA)

    await wrapper.find('input').setValue('https://youtu.be/a')
    await vi.advanceTimersByTimeAsync(400)
    await vi.waitFor(() => {
      expect(wrapper.text()).toContain('Ролик A')
    })

    await wrapper.find('input[type="radio"]').setValue(true)
    await wrapper.findAll('button').find((b) => b.text() === 'Скачать')?.trigger('click')

    expect(wrapper.emitted('download')).toStrictEqual([
      [
        {
          url: 'https://youtu.be/a',
          title: 'Ролик A',
          streams: { audioFormatId: 'a' },
          size: { kind: 'unknown' },
          qualityLabel: 'Только аудио',
        },
      ],
    ])
  })

  it('trims the url before emitting the download request — a trailing newline from a paste must not silently break start_download (ревью TL-45)', async () => {
    const wrapper = mount(ProbeSection, { props: { ytDlpState: 'ready' } })
    invokeMock.mockResolvedValueOnce(resultA)

    // Разбор уже работает по обрезанной строке (useLinkProbe.evaluate
    // делает value.trim()) — карточка строится как обычно, но `url.value`
    // хранит сырой ввод с завершающим переносом строки.
    await wrapper.find('input').setValue('https://youtu.be/a\n')
    await vi.advanceTimersByTimeAsync(400)
    await vi.waitFor(() => {
      expect(wrapper.text()).toContain('Ролик A')
    })

    await wrapper.find('input[type="radio"]').setValue(true)
    await wrapper.findAll('button').find((b) => b.text() === 'Скачать')?.trigger('click')

    const emitted = wrapper.emitted('download')
    expect(emitted?.[0]?.[0]).toMatchObject({ url: 'https://youtu.be/a' })
  })

  it('forwards downloadBlocked to VideoCard so its download button carries the С-13 hint', async () => {
    const wrapper = mount(ProbeSection, { props: { ytDlpState: 'ready', downloadBlocked: true } })
    invokeMock.mockResolvedValueOnce(resultA)

    await wrapper.find('input').setValue('https://youtu.be/a')
    await vi.advanceTimersByTimeAsync(400)
    await vi.waitFor(() => {
      expect(wrapper.text()).toContain('Ролик A')
    })

    expect(wrapper.text()).toContain('Уже идёт другая загрузка')
  })
})
