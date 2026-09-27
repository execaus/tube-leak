import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import type { SidecarCheckReport } from '@/types/generated/sidecar'

import AboutScreen from './AboutScreen.vue'

/**
 * TL-128 (issue execaus/tube-leak#135) — экран «О программе».
 *
 * Критерий приёмки: экран называет версию приложения, перечисляет все
 * компоненты с их лицензиями (формулировка ffmpeg обязана называть именно
 * GPL v3), даёт ссылку на исходники ffmpeg, а версии sidecar в разделе
 * «Что установлено сейчас» берутся из отчёта проверки (`SidecarCheckReport`),
 * а не захардкожены — мутация «подставь другую версию в отчёт» обязана
 * менять текст на экране.
 */
const report: SidecarCheckReport = {
  ytDlp: { name: 'yt-dlp', path: '/opt/tube-leak/bin/yt-dlp', status: 'ok', version: '2026.08.20' },
  ffmpeg: { name: 'ffmpeg', path: '/opt/tube-leak/bin/ffmpeg', status: 'ok', version: '9.0.1' },
  deno: { name: 'deno', path: '/opt/tube-leak/bin/deno', status: 'ok', version: '2.9.6' },
}

describe('AboutScreen', () => {
  it('shows the app version passed by prop', () => {
    const wrapper = mount(AboutScreen, { props: { appVersion: '0.1.1', report } })
    expect(wrapper.text()).toContain('0.1.1')
  })

  it('names every distributed component with its licence, ffmpeg naming GPL v3 explicitly', () => {
    const wrapper = mount(AboutScreen, { props: { appVersion: '0.1.1', report } })
    const text = wrapper.text()

    expect(text).toContain('ffmpeg')
    expect(text).toContain('GPL v3')
    expect(text).toContain('yt-dlp')
    expect(text).toContain('Unlicense')
    expect(text).toContain('deno')
    expect(text).toContain('MIT')
    expect(text).toContain('V8')
    expect(text).toContain('BSD 3-Clause')
    expect(text).toContain('ICU')
    expect(text).toContain('Unicode License')
    expect(text).toContain('TypeScript')
    expect(text).toContain('Apache License 2.0')
    expect(text).toContain('SQLite')
    expect(text).toContain('Public Domain')
  })

  it('gives a plain, retypeable pointer to the ffmpeg sources and to the licence texts file', () => {
    const wrapper = mount(AboutScreen, { props: { appVersion: '0.1.1', report } })
    const text = wrapper.text()

    expect(text).toContain('https://ffmpeg.org/releases/ffmpeg-9.0.1.tar.bz2')
    expect(text).toContain('SOURCES-FFMPEG.md')
    expect(text).toContain('THIRD-PARTY-LICENSES.md')

    // Ссылки — читаемый текст, не `<a href>` (задача TL-128: репозиторий
    // приватный, у получателя сборки он не откроется, обещать рабочую
    // ссылку на него нельзя; см. doc-комментарий SOURCES-FFMPEG.md).
    expect(wrapper.findAll('a')).toHaveLength(0)
  })

  it('reads sidecar versions from the report prop, not from a hardcoded constant (mutation guard)', () => {
    const wrapper = mount(AboutScreen, { props: { appVersion: '0.1.1', report } })
    expect(wrapper.text()).toContain('2026.08.20')
    expect(wrapper.text()).toContain('9.0.1')
    expect(wrapper.text()).toContain('2.9.6')

    const changedReport: SidecarCheckReport = {
      ytDlp: { ...report.ytDlp, version: '2099.01.01' },
      ffmpeg: { ...report.ffmpeg, version: '99.9.9' },
      deno: { ...report.deno, version: '9.9.9' },
    }
    const changedWrapper = mount(AboutScreen, { props: { appVersion: '0.1.1', report: changedReport } })
    const text = changedWrapper.text()
    expect(text).toContain('2099.01.01')
    expect(text).toContain('99.9.9')
    expect(text).toContain('9.9.9')
    expect(text).not.toContain('2026.08.20')
  })

  it('shows a neutral placeholder while the sidecar report has not arrived yet', () => {
    const wrapper = mount(AboutScreen, { props: { appVersion: '0.1.1' } })
    expect(wrapper.text()).toContain('проверяется')
  })

  it('does not claim a version for a sidecar whose check did not succeed', () => {
    const failedReport: SidecarCheckReport = {
      ytDlp: { name: 'yt-dlp', path: '/opt/tube-leak/bin/yt-dlp', status: 'notFound' },
      ffmpeg: report.ffmpeg,
      deno: report.deno,
    }
    const wrapper = mount(AboutScreen, { props: { appVersion: '0.1.1', report: failedReport } })
    expect(wrapper.text()).not.toContain('undefined')
  })
})
