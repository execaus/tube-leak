import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import type { SidecarCheckResult } from '@/types/sidecar'

import SidecarStatusRow from './SidecarStatusRow.vue'

/**
 * Фикстуры результата проверки sidecar — поле в поле повторяют фикстуры
 * `useSidecarCheck.test.ts` (TL-2), плюс варианты, необходимые для
 * различения launchFailed(other) и nonZeroExit отдельно от corrupted.
 */
const okResult: SidecarCheckResult = {
  name: 'ffmpeg',
  path: '/opt/tube-leak/bin/ffmpeg',
  status: 'ok',
  version: '7.1',
}

const notFoundResult: SidecarCheckResult = {
  name: 'yt-dlp',
  path: '/opt/tube-leak/bin/yt-dlp',
  status: 'notFound',
  osErrorCode: 'ENOENT',
}

const launchFailedCorruptedResult: SidecarCheckResult = {
  name: 'ffmpeg',
  path: '/opt/tube-leak/bin/ffmpeg',
  status: 'launchFailed',
  reason: 'corrupted',
  osErrorCode: 'ENOEXEC',
  stderrTail: 'cannot execute binary file',
}

const launchFailedOtherResult: SidecarCheckResult = {
  name: 'yt-dlp',
  path: '/opt/tube-leak/bin/yt-dlp',
  status: 'launchFailed',
  reason: 'other',
  osErrorCode: 'EUNKNOWN',
}

const launchFailedPermissionDeniedResult: SidecarCheckResult = {
  name: 'yt-dlp',
  path: '/opt/tube-leak/bin/yt-dlp',
  status: 'launchFailed',
  reason: 'permissionDenied',
  osErrorCode: 'EACCES',
}

const nonZeroExitResult: SidecarCheckResult = {
  name: 'yt-dlp',
  path: '/opt/tube-leak/bin/yt-dlp',
  status: 'nonZeroExit',
  exitCode: 1,
  stderrTail: 'error: unsupported URL',
}

const timeoutResult: SidecarCheckResult = {
  name: 'ffmpeg',
  path: '/opt/tube-leak/bin/ffmpeg',
  status: 'timeout',
  timeoutMs: 5000,
  stderrTail: 'partial output before kill',
}

describe('SidecarStatusRow', () => {
  it('renders the Checking state before a result arrives, with no buttons or details', () => {
    const wrapper = mount(SidecarStatusRow, { props: { fallbackName: 'yt-dlp' } })

    expect(wrapper.text()).toContain('yt-dlp')
    expect(wrapper.text()).toContain('Проверяем…')
    expect(wrapper.find('button').exists()).toBe(false)
    expect(wrapper.find('dl').exists()).toBe(false)
  })

  it('renders the Ok state with the version and no error affordances', () => {
    const wrapper = mount(SidecarStatusRow, {
      props: { fallbackName: 'ffmpeg', result: okResult },
    })

    expect(wrapper.text()).toContain('ffmpeg')
    expect(wrapper.text()).toContain('7.1')
    expect(wrapper.text()).not.toContain('Подробнее')
    expect(wrapper.find('button').exists()).toBe(false)
  })

  it('renders the NotFound state with icon+text and the expected explanation', () => {
    const wrapper = mount(SidecarStatusRow, {
      props: { fallbackName: 'yt-dlp', result: notFoundResult },
    })

    expect(wrapper.text()).toContain('не найден')
    expect(wrapper.text()).toContain('Не нашли файл yt-dlp по ожидаемому пути.')
    expect(wrapper.text()).toContain('переустановить tube-leak')
    expect(wrapper.text()).toContain('Подробнее')
    expect(wrapper.text()).not.toContain('Инструкция')
  })

  it('renders the LaunchFailed(corrupted) state with the "not started" explanation', () => {
    const wrapper = mount(SidecarStatusRow, {
      props: { fallbackName: 'ffmpeg', result: launchFailedCorruptedResult },
    })

    expect(wrapper.text()).toContain('не удалось запустить')
    expect(wrapper.text()).toContain('Файл ffmpeg найден, но не запустился.')
    expect(wrapper.text()).toContain('повреждён')
  })

  it('renders the LaunchFailed(other) state with the same "not started" explanation as corrupted', () => {
    const wrapper = mount(SidecarStatusRow, {
      props: { fallbackName: 'yt-dlp', result: launchFailedOtherResult },
    })

    expect(wrapper.text()).toContain('не удалось запустить')
    expect(wrapper.text()).toContain('Файл yt-dlp найден, но не запустился.')
  })

  it('renders the LaunchFailed(permissionDenied) state with the chmod explanation', () => {
    const wrapper = mount(SidecarStatusRow, {
      props: { fallbackName: 'yt-dlp', result: launchFailedPermissionDeniedResult },
    })

    expect(wrapper.text()).toContain('не удалось запустить')
    expect(wrapper.text()).toContain('нет прав на выполнение')
    expect(wrapper.text()).toContain('chmod +x')
    expect(wrapper.text()).toContain(launchFailedPermissionDeniedResult.path)
  })

  it('renders the NonZeroExit state with the exit code interpolated', () => {
    const wrapper = mount(SidecarStatusRow, {
      props: { fallbackName: 'yt-dlp', result: nonZeroExitResult },
    })

    expect(wrapper.text()).toContain('не удалось запустить')
    expect(wrapper.text()).toContain('завершился с ошибкой (код выхода: 1)')
  })

  it('renders the Timeout state with the timeout in seconds and an instructions link', () => {
    const wrapper = mount(SidecarStatusRow, {
      props: { fallbackName: 'ffmpeg', result: timeoutResult },
    })

    expect(wrapper.text()).toContain('не отвечает')
    expect(wrapper.text()).toContain('не завершилась за отведённое время (5 с)')

    const link = wrapper.find('a')
    expect(link.exists()).toBe(true)
    expect(link.text()).toContain('Инструкция')
    expect(link.attributes('target')).toBe('_blank')
    expect(link.attributes('href')).toBeTruthy()
  })

  it('keeps the details block collapsed by default and reveals it on click', async () => {
    const wrapper = mount(SidecarStatusRow, {
      props: { fallbackName: 'ffmpeg', result: timeoutResult },
    })

    expect(wrapper.find('dl').exists()).toBe(false)

    const detailsButton = wrapper.findAll('button').find((b) => b.text().includes('Подробнее'))
    expect(detailsButton).toBeDefined()
    expect(detailsButton?.attributes('aria-expanded')).toBe('false')

    await detailsButton?.trigger('click')

    expect(wrapper.find('dl').exists()).toBe(true)
    expect(detailsButton?.attributes('aria-expanded')).toBe('true')
    expect(wrapper.text()).toContain(timeoutResult.path)
    expect(wrapper.text()).toContain('5000 мс')
    expect(wrapper.text()).toContain(timeoutResult.stderrTail)
  })

  it('shows osErrorCode and stderrTail for a corrupted launchFailed result inside details', async () => {
    const wrapper = mount(SidecarStatusRow, {
      props: { fallbackName: 'ffmpeg', result: launchFailedCorruptedResult },
    })

    await wrapper.get('button').trigger('click')

    expect(wrapper.text()).toContain(launchFailedCorruptedResult.osErrorCode)
    expect(wrapper.text()).toContain(launchFailedCorruptedResult.stderrTail)
  })
})
