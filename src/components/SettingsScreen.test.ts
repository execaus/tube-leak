import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { Settings, SettingsView } from '@/types/generated/settings'

const invokeMock = vi.fn()
const pickDestinationFolderMock = vi.fn()

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}))

vi.mock('@/composables/usePickFolder', () => ({
  pickDestinationFolder: (...args: unknown[]) => pickDestinationFolderMock(...args),
}))

const { default: SettingsScreen } = await import('./SettingsScreen.vue')

function settings(overrides: Partial<Settings> = {}): Settings {
  return {
    destinationFolder: { kind: 'system' },
    nameTemplate: '{title}',
    maxAttempts: 8,
    ...overrides,
  }
}

function view(overrides: Partial<SettingsView> = {}): SettingsView {
  return {
    settings: settings(),
    defaults: settings(),
    resetFields: [],
    wholeFileReset: false,
    destinationFolderExists: true,
    ...overrides,
  }
}

let host: HTMLElement

beforeEach(() => {
  vi.useFakeTimers()
  invokeMock.mockReset()
  pickDestinationFolderMock.mockReset()
  setActivePinia(createPinia())
  host = document.createElement('div')
  document.body.appendChild(host)
})

afterEach(() => {
  host.remove()
  vi.useRealTimers()
})

async function mountScreen(initialView: SettingsView = view()) {
  invokeMock.mockResolvedValueOnce(initialView)
  const wrapper = mount(SettingsScreen, { attachTo: host })
  await flushPromises()
  return wrapper
}

describe('loading / unavailable states', () => {
  it('shows a loading text before settings_get resolves', () => {
    invokeMock.mockReturnValueOnce(new Promise(() => {}))
    const wrapper = mount(SettingsScreen, { attachTo: host })
    expect(wrapper.text()).toContain('Загружаем настройки')
  })

  it('shows a neutral unavailable text on a rejected settings_get, not an empty screen', async () => {
    invokeMock.mockRejectedValueOnce(new Error('boom'))
    const wrapper = mount(SettingsScreen, { attachTo: host })
    await flushPromises()
    expect(wrapper.text()).toContain('Настройки сейчас недоступны')
    expect(wrapper.find('#settings-template-input').exists()).toBe(false)
  })
})

describe('re-fetch on tab activation (by analogy with HistoryScreen)', () => {
  it('re-requests settings_get when the "active" prop flips from false to true', async () => {
    invokeMock.mockResolvedValueOnce(view())
    const wrapper = mount(SettingsScreen, { props: { active: false }, attachTo: host })
    await flushPromises()
    expect(invokeMock.mock.calls.filter((c) => c[0] === 'settings_get')).toHaveLength(1)

    invokeMock.mockResolvedValueOnce(view())
    await wrapper.setProps({ active: true })
    await flushPromises()
    expect(invokeMock.mock.calls.filter((c) => c[0] === 'settings_get')).toHaveLength(2)
  })
})

describe('destination folder', () => {
  it('shows the system Downloads folder in quotes and no reset button', async () => {
    const wrapper = await mountScreen()
    expect(wrapper.text()).toContain('«Загрузки»')
    expect(wrapper.findAll('button').find((b) => b.text() === 'Сбросить к «Загрузки»')).toBeUndefined()
  })

  it('picking a folder calls settings_set with { kind: "custom", path } right away', async () => {
    const wrapper = await mountScreen()
    pickDestinationFolderMock.mockResolvedValueOnce('/Users/execaus/Movies/YouTube')
    invokeMock.mockResolvedValueOnce(
      view({ settings: settings({ destinationFolder: { kind: 'custom', path: '/Users/execaus/Movies/YouTube' } }) }),
    )

    await wrapper.find('button').trigger('click') // «Выбрать папку…» — первая кнопка экрана
    await flushPromises()

    expect(invokeMock).toHaveBeenCalledWith('settings_set', {
      destinationFolder: { kind: 'custom', path: '/Users/execaus/Movies/YouTube' },
    })
    expect(wrapper.text()).toContain('/Users/execaus/Movies/YouTube')
  })

  it('cancelling the dialog (null) does not call settings_set', async () => {
    const wrapper = await mountScreen()
    pickDestinationFolderMock.mockResolvedValueOnce(null)

    await wrapper.find('button').trigger('click')
    await flushPromises()

    expect(invokeMock.mock.calls.some((c) => c[0] === 'settings_set')).toBe(false)
  })

  it('reset is instant — no dialog call — and visible only for a custom folder', async () => {
    const wrapper = await mountScreen(view({ settings: settings({ destinationFolder: { kind: 'custom', path: '/x' } }) }))
    const resetButton = wrapper.findAll('button').find((b) => b.text() === 'Сбросить к «Загрузки»')
    expect(resetButton).toBeDefined()

    invokeMock.mockResolvedValueOnce(view())
    await resetButton!.trigger('click')
    await flushPromises()

    expect(pickDestinationFolderMock).not.toHaveBeenCalled()
    expect(invokeMock).toHaveBeenCalledWith('settings_set', { destinationFolder: { kind: 'system' } })
  })

  it('shows the "folder is missing" warning from destinationFolderExists', async () => {
    const wrapper = await mountScreen(
      view({ settings: settings({ destinationFolder: { kind: 'custom', path: '/gone' } }), destinationFolderExists: false }),
    )
    expect(wrapper.text()).toContain('Папки сейчас нет')
  })

  it('warns about a path longer than 200 characters without blocking anything', async () => {
    const longPath = `/Volumes/${'a'.repeat(200)}`
    const wrapper = await mountScreen(view({ settings: settings({ destinationFolder: { kind: 'custom', path: longPath } }) }))
    expect(wrapper.text()).toContain('превысить')
  })

  it('shows the notADirectory reason under the path on failure', async () => {
    const wrapper = await mountScreen()
    pickDestinationFolderMock.mockResolvedValueOnce('/gone')
    invokeMock.mockRejectedValueOnce({ kind: 'notADirectory', problem: 'notFound', message: 'diag' })

    await wrapper.find('button').trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('Эта папка недоступна')
  })
})

describe('name template', () => {
  it('seeds the input from the saved value and disables Save until the value changes', async () => {
    const wrapper = await mountScreen(view({ settings: settings({ nameTemplate: '{id} — {title}' }) }))
    const input = wrapper.find<HTMLInputElement>('#settings-template-input')
    expect(input.element.value).toBe('{id} — {title}')
    const saveButton = wrapper.findAll('button').find((b) => b.text() === 'Сохранить')
    expect(saveButton!.attributes('disabled')).toBeDefined()
  })

  it('disables Save for a client-invalid draft (unknown variable) even though it differs from the saved value', async () => {
    const wrapper = await mountScreen()
    await wrapper.find('#settings-template-input').setValue('{channel}')
    await vi.advanceTimersByTimeAsync(400)
    const saveButton = wrapper.findAll('button').filter((b) => b.text() === 'Сохранить')[0]
    expect(saveButton!.attributes('disabled')).toBeDefined()
  })

  it('enables Save for a valid, changed draft and sends nameTemplate on click', async () => {
    const wrapper = await mountScreen()
    invokeMock.mockResolvedValueOnce({ result: 'dQw4w9WgXcQ' })
    await wrapper.find('#settings-template-input').setValue('{id}')
    await vi.advanceTimersByTimeAsync(400) // предпросмотр

    const saveButton = wrapper.findAll('button').filter((b) => b.text() === 'Сохранить')[0]
    expect(saveButton!.attributes('disabled')).toBeUndefined()

    invokeMock.mockResolvedValueOnce(view({ settings: settings({ nameTemplate: '{id}' }) }))
    await saveButton!.trigger('click')
    await flushPromises()

    expect(invokeMock).toHaveBeenCalledWith('settings_set', { nameTemplate: '{id}' })
  })

  it('debounces the live preview — a burst of edits results in exactly one preview_name_template call, for the final value', async () => {
    const wrapper = await mountScreen()
    invokeMock.mockResolvedValue({ result: 'preview' })
    const previewCallsBefore = invokeMock.mock.calls.filter((c) => c[0] === 'preview_name_template').length

    const input = wrapper.find('#settings-template-input')
    await input.setValue('{t')
    await vi.advanceTimersByTimeAsync(200)
    await input.setValue('{ti')
    await vi.advanceTimersByTimeAsync(200)
    await input.setValue('{title}')
    await vi.advanceTimersByTimeAsync(400)

    const previewCalls = invokeMock.mock.calls.filter((c) => c[0] === 'preview_name_template')
    expect(previewCalls.length - previewCallsBefore).toBe(1)
    expect(previewCalls.at(-1)).toStrictEqual(['preview_name_template', { template: '{title}' }])
  })

  it('shows the exact server invalidTemplate message when saving is rejected', async () => {
    const wrapper = await mountScreen()
    invokeMock.mockResolvedValueOnce({ result: 'x' }) // предпросмотр после setValue
    await wrapper.find('#settings-template-input').setValue('{id}')
    await vi.advanceTimersByTimeAsync(400)

    invokeMock.mockRejectedValueOnce({
      kind: 'invalidTemplate',
      problem: { kind: 'noVariables' },
      message: 'diag',
    })
    const saveButton = wrapper.findAll('button').filter((b) => b.text() === 'Сохранить')[0]
    await saveButton!.trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('нет ни одной переменной')
  })

  it('shows "Пример недоступен." on a writeFailed preview response (TL-91 stub) — not "Не удалось сохранить"', async () => {
    const wrapper = await mountScreen()
    invokeMock.mockRejectedValueOnce({ kind: 'writeFailed', message: 'not implemented yet' })

    await wrapper.find('#settings-template-input').setValue('{id}')
    await vi.advanceTimersByTimeAsync(400)
    await flushPromises()

    expect(wrapper.text()).toContain('Пример недоступен')
    expect(wrapper.text()).not.toContain('Не удалось сохранить')
  })
})

describe('max attempts', () => {
  it('does not enable Save for an empty, non-numeric, or fractional value', async () => {
    const wrapper = await mountScreen()
    const input = wrapper.find('#settings-attempts-input')
    const saveButton = () => wrapper.findAll('button').filter((b) => b.text() === 'Сохранить')[1]!

    await input.setValue('')
    expect(saveButton().attributes('disabled')).toBeDefined()

    await input.setValue('abc')
    expect(saveButton().attributes('disabled')).toBeDefined()

    await input.setValue('3.5')
    expect(saveButton().attributes('disabled')).toBeDefined()

    expect(invokeMock.mock.calls.some((c) => c[0] === 'settings_set')).toBe(false) // ни одна попытка не отправлена
  })

  it('sends an out-of-range integer (0) to the core and shows its invalidValue text', async () => {
    const wrapper = await mountScreen()
    const input = wrapper.find('#settings-attempts-input')
    await input.setValue('0')

    invokeMock.mockRejectedValueOnce({ kind: 'invalidValue', min: 1, max: 20, message: 'diag' })
    const saveButton = wrapper.findAll('button').filter((b) => b.text() === 'Сохранить')[1]!
    await saveButton.trigger('click')
    await flushPromises()

    expect(invokeMock).toHaveBeenCalledWith('settings_set', { maxAttempts: 0 })
    expect(wrapper.text()).toContain('Число попыток должно быть от 1 до 20.')
  })

  it('disables "+" at the upper bound (20)', async () => {
    const wrapper = await mountScreen(view({ settings: settings({ maxAttempts: 20 }) }))
    const plus = wrapper.findAll('button').find((b) => b.attributes('aria-label') === 'Увеличить число попыток')!
    expect(plus.attributes('disabled')).toBeDefined()
  })

  it('disables "−" at the lower bound (1)', async () => {
    const wrapper = await mountScreen(view({ settings: settings({ maxAttempts: 1 }) }))
    const minus = wrapper.findAll('button').find((b) => b.attributes('aria-label') === 'Уменьшить число попыток')!
    expect(minus.attributes('disabled')).toBeDefined()
  })
})

describe('reset badges (С-9/Ф-9)', () => {
  it('shows a per-field badge for a field reported in resetFields, and not for the others', async () => {
    const wrapper = await mountScreen(view({ resetFields: ['maxAttempts'] }))
    const badges = wrapper.findAll('.settings-screen__field-reset')
    expect(badges).toHaveLength(1)
    expect(wrapper.text()).toContain('Сброшено к значению по умолчанию')
  })

  it('shows one whole-file banner instead of per-field badges when wholeFileReset is true', async () => {
    const wrapper = await mountScreen(view({ wholeFileReset: true, resetFields: [] }))
    expect(wrapper.findAll('.settings-screen__field-reset')).toHaveLength(0)
    expect(wrapper.text()).toContain('Настройки сброшены к умолчаниям: файл не читался')
  })
})
