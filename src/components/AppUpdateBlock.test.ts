import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const checkMock = vi.fn()
vi.mock('@tauri-apps/plugin-updater', () => ({ check: (...args: unknown[]) => checkMock(...args) }))

const { default: AppUpdateBlock } = await import('./AppUpdateBlock.vue')

beforeEach(() => checkMock.mockReset())

describe('manual application updates', () => {
  it('does not contact the updater before the user clicks and reports no update', async () => {
    const wrapper = mount(AppUpdateBlock, { props: { currentVersion: '0.1.3', queueBusy: false } })
    expect(checkMock).not.toHaveBeenCalled()
    checkMock.mockResolvedValueOnce(null)
    await wrapper.get('button').trigger('click')
    await flushPromises()
    expect(checkMock).toHaveBeenCalledOnce()
    expect(wrapper.text()).toContain('У вас последняя версия')
    wrapper.unmount()
  })

  it('offers a found update but waits for consent and an empty queue', async () => {
    const close = vi.fn().mockResolvedValue(undefined)
    const downloadAndInstall = vi.fn().mockResolvedValue(undefined)
    checkMock.mockResolvedValueOnce({ version: '0.1.4', body: 'Исправления', close, downloadAndInstall })
    const wrapper = mount(AppUpdateBlock, { props: { currentVersion: '0.1.3', queueBusy: true } })
    await wrapper.get('button').trigger('click')
    await flushPromises()
    expect(wrapper.text()).toContain('Доступна версия 0.1.4')
    expect(downloadAndInstall).not.toHaveBeenCalled()
    expect(wrapper.findAll('button')[1]?.attributes('disabled')).toBeDefined()
    await wrapper.setProps({ queueBusy: false })
    await wrapper.findAll('button')[1]?.trigger('click')
    await flushPromises()
    expect(downloadAndInstall).toHaveBeenCalledOnce()
    expect(wrapper.text()).toContain('Обновление установлено')
    wrapper.unmount()
  })
})
