import { describe, expect, it, vi } from 'vitest'

const openMock = vi.fn()

vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: (...args: unknown[]) => openMock(...args),
}))

const { pickDestinationFolder } = await import('./usePickFolder')

describe('pickDestinationFolder', () => {
  it('opens a directory-selection dialog', async () => {
    openMock.mockResolvedValueOnce('/Users/execaus/Movies/YouTube')
    await pickDestinationFolder()
    expect(openMock).toHaveBeenCalledExactlyOnceWith({ directory: true })
  })

  it('returns the absolute path on a successful pick', async () => {
    openMock.mockResolvedValueOnce('/Users/execaus/Movies/YouTube')
    await expect(pickDestinationFolder()).resolves.toBe('/Users/execaus/Movies/YouTube')
  })

  it('returns null when the dialog is cancelled, without calling settings_set (checked at the call site)', async () => {
    openMock.mockResolvedValueOnce(null)
    await expect(pickDestinationFolder()).resolves.toBeNull()
  })
})
