import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import type { SidecarCheckResult } from '@/types/generated/sidecar'

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

/**
 * `reason` объявлен опциональным во всём контракте (заполняется только
 * при `status === 'launchFailed'`, но тип этого не запрещает) — фикстура
 * на случай, если он всё же не пришёл: до ревью TL-52 такое значение
 * попадало в ту же ветку, что и `corrupted`.
 */
const launchFailedMissingReasonResult: SidecarCheckResult = {
  name: 'yt-dlp',
  path: '/opt/tube-leak/bin/yt-dlp',
  status: 'launchFailed',
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

/**
 * `unrecognizedOutput` (TL-113, ядро TL-109): в отличие от всех остальных
 * причин `launchFailed`, здесь процесс реально запустился и завершился без
 * ошибки — просто в его выводе не нашлось ожидаемой строки версии
 * (например, deno). `osErrorCode` для этой причины ядро не присылает,
 * `stderrTail` содержит вывод отработавшего процесса (stdout, затем
 * stderr), а не диагностику сбоя запуска.
 */
const launchFailedUnrecognizedOutputResult: SidecarCheckResult = {
  name: 'deno',
  path: '/opt/tube-leak/bin/deno',
  status: 'launchFailed',
  reason: 'unrecognizedOutput',
  stderrTail: 'Deno 1.0\n',
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

  it('renders the LaunchFailed(other) state with a neutral explanation, distinct from corrupted (review TL-52)', () => {
    // До ревью TL-52 `other` (и отсутствующий `reason`) молча получали тот
    // же текст, что `corrupted` («похоже, он повреждён») — диагноз,
    // которого ядро не утверждало. Текст для `other` обязан не называть
    // причину и при этом не совпадать дословно с текстом `corrupted`.
    const wrapper = mount(SidecarStatusRow, {
      props: { fallbackName: 'yt-dlp', result: launchFailedOtherResult },
    })

    expect(wrapper.text()).toContain('не удалось запустить')
    expect(wrapper.text()).toContain('Файл yt-dlp найден, но не запустился')
    expect(wrapper.text()).not.toContain('повреждён')
  })

  it('renders the same neutral explanation when `reason` is missing entirely (defensive fallback, review TL-52)', () => {
    const wrapper = mount(SidecarStatusRow, {
      props: { fallbackName: 'yt-dlp', result: launchFailedMissingReasonResult },
    })

    expect(wrapper.text()).toContain('не удалось запустить')
    expect(wrapper.text()).toContain('Файл yt-dlp найден, но не запустился')
    expect(wrapper.text()).not.toContain('повреждён')
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

  it('renders the LaunchFailed(unrecognizedOutput) state truthfully, without "not started" or an OS error code (TL-113)', () => {
    const wrapper = mount(SidecarStatusRow, {
      props: { fallbackName: 'deno', result: launchFailedUnrecognizedOutputResult },
    })

    const text = wrapper.text()
    expect(text).not.toContain('не запустился')
    expect(text).not.toContain('код ошибки ОС')
    expect(text).toContain('неожиданный ответ')
    expect(text).toContain('переустановить tube-leak')
    expect(text).toContain('запустился и завершился без ошибок')
  })

  it('labels the process output as "Вывод" (not "stderr") only for unrecognizedOutput (TL-113)', async () => {
    const wrapper = mount(SidecarStatusRow, {
      props: { fallbackName: 'deno', result: launchFailedUnrecognizedOutputResult },
    })

    const detailsButton = wrapper.findAll('button').find((b) => b.text().includes('Подробнее'))
    await detailsButton?.trigger('click')

    expect(wrapper.text()).toContain('Вывод')
    expect(wrapper.text()).not.toContain('stderr')
    expect(wrapper.text()).toContain(launchFailedUnrecognizedOutputResult.stderrTail?.trim())
  })

  it('keeps the "not started" text and the "stderr" label unchanged for LaunchFailed(other) (TL-113 regression guard)', async () => {
    const wrapper = mount(SidecarStatusRow, {
      props: { fallbackName: 'yt-dlp', result: launchFailedOtherResult },
    })

    expect(wrapper.text()).toContain('не удалось запустить')
    expect(wrapper.text()).not.toContain('неожиданный ответ')
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

  it.each([
    { result: notFoundResult, expectedName: 'yt-dlp' },
    { result: timeoutResult, expectedName: 'ffmpeg' },
    { result: launchFailedUnrecognizedOutputResult, expectedName: 'deno' },
  ])(
    'gives the "Подробнее" button an accessible name that includes the tool name ($expectedName), without changing its visible text',
    ({ result, expectedName }) => {
      const wrapper = mount(SidecarStatusRow, {
        props: { fallbackName: expectedName, result },
      })

      const detailsButton = wrapper.findAll('button').find((b) => b.text().includes('Подробнее'))
      expect(detailsButton?.text()).toBe('Подробнее ▾')
      expect(detailsButton?.attributes('aria-label')).toContain(expectedName)
    },
  )
})
