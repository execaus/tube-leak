import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { Settings, SettingsView } from '@/types/generated/settings'

const invokeMock = vi.fn()

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}))

const { useSettingsStore } = await import('./settings')

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

beforeEach(() => {
  vi.useFakeTimers()
  invokeMock.mockReset()
  setActivePinia(createPinia())
})

afterEach(() => {
  vi.useRealTimers()
})

describe('fetchSettings', () => {
  it('reads settings_get and applies the whole view', async () => {
    invokeMock.mockResolvedValueOnce(view({ resetFields: ['maxAttempts'] }))
    const store = useSettingsStore()

    await store.fetchSettings()

    expect(invokeMock).toHaveBeenCalledExactlyOnceWith('settings_get')
    expect(store.settings).toStrictEqual(settings())
    expect(store.resetFields).toStrictEqual(['maxAttempts'])
    expect(store.loaded).toBe(true)
    expect(store.ipcFailure).toBe(false)
  })

  it('marks ipcFailure on a rejected call, without crashing (settings_get has no typed Result)', async () => {
    invokeMock.mockRejectedValueOnce(new Error('boom'))
    const store = useSettingsStore()

    await store.fetchSettings()

    expect(store.ipcFailure).toBe(true)
    expect(store.loaded).toBe(true)
    expect(store.settings).toBeUndefined()
  })
})

describe('setDestinationFolder', () => {
  it('sends { kind: "custom", path } and applies the returned view on success', async () => {
    const nextView = view({ settings: settings({ destinationFolder: { kind: 'custom', path: '/x' } }) })
    invokeMock.mockResolvedValueOnce(nextView)
    const store = useSettingsStore()

    const ok = await store.setDestinationFolder({ kind: 'custom', path: '/x' })

    expect(invokeMock).toHaveBeenCalledExactlyOnceWith('settings_set', { destinationFolder: { kind: 'custom', path: '/x' } })
    expect(ok).toBe(true)
    expect(store.settings?.destinationFolder).toStrictEqual({ kind: 'custom', path: '/x' })
    expect(store.folderError).toBeUndefined()
  })

  it('sends { kind: "system" } on reset', async () => {
    invokeMock.mockResolvedValueOnce(view())
    const store = useSettingsStore()

    await store.setDestinationFolder({ kind: 'system' })

    expect(invokeMock).toHaveBeenCalledExactlyOnceWith('settings_set', { destinationFolder: { kind: 'system' } })
  })

  it('records a typed notADirectory failure without touching settings', async () => {
    invokeMock.mockRejectedValueOnce({ kind: 'notADirectory', problem: 'notFound', message: 'diag' })
    const store = useSettingsStore()

    const ok = await store.setDestinationFolder({ kind: 'custom', path: '/gone' })

    expect(ok).toBe(false)
    expect(store.folderError).toStrictEqual({ kind: 'notADirectory', problem: 'notFound', message: 'diag' })
    expect(store.settings).toBeUndefined()
  })
})

describe('setNameTemplate / setMaxAttempts', () => {
  it('setNameTemplate sends the field under its own key and clears a previous error on success', async () => {
    const store = useSettingsStore()
    invokeMock.mockRejectedValueOnce({ kind: 'invalidTemplate', problem: { kind: 'noVariables' }, message: 'diag' })
    await store.setNameTemplate('plain')
    expect(store.templateError).toStrictEqual({ kind: 'invalidTemplate', problem: { kind: 'noVariables' }, message: 'diag' })

    invokeMock.mockResolvedValueOnce(view({ settings: settings({ nameTemplate: '{id}' }) }))
    const ok = await store.setNameTemplate('{id}')

    expect(invokeMock).toHaveBeenLastCalledWith('settings_set', { nameTemplate: '{id}' })
    expect(ok).toBe(true)
    expect(store.templateError).toBeUndefined()
    expect(store.settings?.nameTemplate).toBe('{id}')
  })

  it('setMaxAttempts sends the field under its own key and records invalidValue on failure', async () => {
    invokeMock.mockRejectedValueOnce({ kind: 'invalidValue', min: 1, max: 20, message: 'diag' })
    const store = useSettingsStore()

    const ok = await store.setMaxAttempts(0)

    expect(invokeMock).toHaveBeenCalledExactlyOnceWith('settings_set', { maxAttempts: 0 })
    expect(ok).toBe(false)
    expect(store.attemptsError).toStrictEqual({ kind: 'invalidValue', min: 1, max: 20, message: 'diag' })
  })
})

describe('requestPreview — debounce', () => {
  it('does not call preview_name_template before ~400ms of silence', async () => {
    const store = useSettingsStore()
    store.requestPreview('{title}')
    await vi.advanceTimersByTimeAsync(399)
    expect(invokeMock).not.toHaveBeenCalled()
    await vi.advanceTimersByTimeAsync(1)
    expect(invokeMock).toHaveBeenCalledExactlyOnceWith('preview_name_template', { template: '{title}' })
  })

  it('does not restart the debounce timer on every call while the draft keeps changing, and only the last one is dispatched', async () => {
    invokeMock.mockResolvedValueOnce({ result: 'Как приручить дракона' })
    const store = useSettingsStore()

    store.requestPreview('{t')
    await vi.advanceTimersByTimeAsync(300)
    store.requestPreview('{ti')
    await vi.advanceTimersByTimeAsync(300)
    store.requestPreview('{title}')
    await vi.advanceTimersByTimeAsync(400)

    expect(invokeMock).toHaveBeenCalledExactlyOnceWith('preview_name_template', { template: '{title}' })
    expect(store.previewResult).toBe('Как приручить дракона')
  })
})

describe('requestPreview — race guard (mutation: applying any response makes this red)', () => {
  it('keeps only the result of the most recently requested preview, even if an earlier request resolves later', async () => {
    const store = useSettingsStore()

    let resolveFirst!: (v: { result: string }) => void
    invokeMock.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveFirst = resolve
        }),
    )
    store.requestPreview('{id}')
    await vi.advanceTimersByTimeAsync(400)

    invokeMock.mockResolvedValueOnce({ result: 'second' })
    store.requestPreview('{title}')
    await vi.advanceTimersByTimeAsync(400)

    expect(store.previewResult).toBe('second')

    // Первый (более старый) запрос разрешается только теперь — позже второго.
    resolveFirst({ result: 'first' })
    await Promise.resolve()
    await Promise.resolve()

    expect(store.previewResult).toBe('second')
  })
})

describe('cancelPreview (правка ревью TL-94, R6 — debounce timer переживает размонтирование экрана)', () => {
  it('prevents a pending debounced preview_name_template from firing at all', async () => {
    const store = useSettingsStore()

    store.requestPreview('{title}')
    store.cancelPreview()
    await vi.advanceTimersByTimeAsync(400)

    expect(invokeMock).not.toHaveBeenCalled()
  })

  it('makes an in-flight response of the cancelled request a no-op (мутация: убрать проворот поколения — тест краснеет)', async () => {
    const store = useSettingsStore()
    let resolveInFlight!: (v: { result: string }) => void
    invokeMock.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveInFlight = resolve
        }),
    )

    store.requestPreview('{id}', { immediate: true })
    store.cancelPreview()
    resolveInFlight({ result: 'stale' })
    await Promise.resolve()
    await Promise.resolve()

    expect(store.previewResult).toBeUndefined()
  })
})

describe('clearTemplateError / clearAttemptsError (правка ревью TL-94, п. 8)', () => {
  it('clearTemplateError resets templateError without touching anything else', async () => {
    const store = useSettingsStore()
    invokeMock.mockRejectedValueOnce({ kind: 'invalidTemplate', problem: { kind: 'noVariables' }, message: 'diag' })
    await store.setNameTemplate('plain')
    expect(store.templateError).toBeDefined()

    store.clearTemplateError()

    expect(store.templateError).toBeUndefined()
  })

  it('clearAttemptsError resets attemptsError', async () => {
    const store = useSettingsStore()
    invokeMock.mockRejectedValueOnce({ kind: 'invalidValue', min: 1, max: 20, message: 'diag' })
    await store.setMaxAttempts(0)
    expect(store.attemptsError).toBeDefined()

    store.clearAttemptsError()

    expect(store.attemptsError).toBeUndefined()
  })
})

describe('requestPreview — writeFailed (TL-91 stub) shows "unavailable", not a save failure (mutation guard)', () => {
  it('sets previewUnavailable, not previewProblem, on a writeFailed response', async () => {
    invokeMock.mockRejectedValueOnce({ kind: 'writeFailed', message: 'not implemented yet' })
    const store = useSettingsStore()

    store.requestPreview('{title}')
    await vi.advanceTimersByTimeAsync(400)

    expect(store.previewUnavailable).toBe(true)
    expect(store.previewProblem).toBeUndefined()
    expect(store.previewResult).toBeUndefined()
  })

  it('sets previewProblem (not previewUnavailable) on an invalidTemplate response', async () => {
    invokeMock.mockRejectedValueOnce({ kind: 'invalidTemplate', problem: { kind: 'noVariables' }, message: 'diag' })
    const store = useSettingsStore()

    store.requestPreview('plain')
    await vi.advanceTimersByTimeAsync(400)

    expect(store.previewProblem).toStrictEqual({ kind: 'noVariables' })
    expect(store.previewUnavailable).toBe(false)
  })
})
