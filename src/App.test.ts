import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { SidecarCheckReport, SidecarCheckResult } from '@/types/sidecar'

const invokeMock = vi.fn()

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}))

// Импортируется после мока `invoke` (тот же приём, что и в
// useSidecarCheck.test.ts), т.к. App.vue использует composable как есть.
const { default: App } = await import('./App.vue')

const okYtDlp: SidecarCheckResult = {
  name: 'yt-dlp',
  path: '/opt/tube-leak/bin/yt-dlp',
  status: 'ok',
  version: '2026.08.20',
}

const okFfmpeg: SidecarCheckResult = {
  name: 'ffmpeg',
  path: '/opt/tube-leak/bin/ffmpeg',
  status: 'ok',
  version: '7.1',
}

const timeoutFfmpeg: SidecarCheckResult = {
  name: 'ffmpeg',
  path: '/opt/tube-leak/bin/ffmpeg',
  status: 'timeout',
  timeoutMs: 5000,
}

const notFoundYtDlp: SidecarCheckResult = {
  name: 'yt-dlp',
  path: '/opt/tube-leak/bin/yt-dlp',
  status: 'notFound',
  osErrorCode: 'ENOENT',
}

beforeEach(() => {
  invokeMock.mockReset()
})

describe('App', () => {
  it('renders the title and version immediately, with both rows Checking, before invoke resolves (Н-6)', () => {
    let resolveInvoke: (value: SidecarCheckReport) => void = () => {}
    invokeMock.mockReturnValueOnce(
      new Promise<SidecarCheckReport>((resolve) => {
        resolveInvoke = resolve
      }),
    )

    const wrapper = mount(App)

    expect(wrapper.text()).toContain('tube-leak')
    expect(wrapper.text()).toContain('версия 0.1.0')
    expect(wrapper.text()).toContain('yt-dlp')
    expect(wrapper.text()).toContain('ffmpeg')
    // Обе строки ещё не получили ответ — обе в состоянии «Проверяем…».
    expect(wrapper.text().match(/Проверяем…/g)).toHaveLength(2)
    // Кнопка повтора не показывается, пока отчёта ещё нет.
    expect(wrapper.find('button').exists()).toBe(false)
    // invoke уже вызван (асинхронно, после монтирования), не заблокировав рендер.
    expect(invokeMock).toHaveBeenCalledExactlyOnceWith('check_sidecar')

    resolveInvoke({ ytDlp: okYtDlp, ffmpeg: okFfmpeg })
  })

  it('hides the retry button when both rows resolve Ok', async () => {
    const report: SidecarCheckReport = { ytDlp: okYtDlp, ffmpeg: okFfmpeg }
    invokeMock.mockResolvedValueOnce(report)

    const wrapper = mount(App)
    await flushPromises()

    expect(wrapper.text()).toContain('2026.08.20')
    expect(wrapper.text()).toContain('7.1')
    expect(wrapper.find('button').exists()).toBe(false)
  })

  it('shows the retry button when at least one row is not Ok, for a mixed ok/timeout report', async () => {
    const report: SidecarCheckReport = { ytDlp: okYtDlp, ffmpeg: timeoutFfmpeg }
    invokeMock.mockResolvedValueOnce(report)

    const wrapper = mount(App)
    await flushPromises()

    // yt-dlp: Ok
    expect(wrapper.text()).toContain('2026.08.20')
    // ffmpeg: Timeout, отдельно и одновременно с yt-dlp Ok на одном экране
    expect(wrapper.text()).toContain('не отвечает')

    const retryButton = wrapper.findAll('button').find((b) => b.text().includes('Повторить проверку'))
    expect(retryButton).toBeDefined()
  })

  it('shows the retry button when both rows are in error states', async () => {
    const report: SidecarCheckReport = { ytDlp: notFoundYtDlp, ffmpeg: timeoutFfmpeg }
    invokeMock.mockResolvedValueOnce(report)

    const wrapper = mount(App)
    await flushPromises()

    const retryButton = wrapper.findAll('button').find((b) => b.text().includes('Повторить проверку'))
    expect(retryButton).toBeDefined()
  })

  it('re-invokes check_sidecar when the retry button is clicked', async () => {
    invokeMock.mockResolvedValueOnce({ ytDlp: notFoundYtDlp, ffmpeg: okFfmpeg })

    const wrapper = mount(App)
    await flushPromises()

    invokeMock.mockResolvedValueOnce({ ytDlp: okYtDlp, ffmpeg: okFfmpeg })

    const retryButton = wrapper.findAll('button').find((b) => b.text().includes('Повторить проверку'))
    await retryButton?.trigger('click')
    await flushPromises()

    expect(invokeMock).toHaveBeenCalledTimes(2)
    expect(wrapper.find('button').exists()).toBe(false)
  })
})
