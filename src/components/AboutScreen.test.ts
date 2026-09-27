import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import type { SidecarCheckReport } from '@/types/generated/sidecar'

import AboutScreen from './AboutScreen.vue'

/**
 * TL-128 (issue execaus/tube-leak#135) — экран «О программе».
 *
 * Критерий приёмки: экран называет версию приложения, перечисляет
 * компоненты с их лицензиями (формулировка ffmpeg обязана называть именно
 * GPL v3, не LGPL), даёт ссылку на исходники ffmpeg и указывает на файлы
 * рядом с установленным приложением, а версии sidecar в разделе «Что
 * установлено сейчас» берутся из отчёта проверки (`SidecarCheckReport`),
 * а не захардкожены — мутация «подставь другую версию в отчёт» обязана
 * менять текст на экране.
 *
 * Правки ревью (возврат TL-128): Б1 — файлы называются лежащими рядом с
 * приложением, не «в репозитории» (репозиторий приватный, получателю
 * недоступен); Б2 — список компонентов не выдаёт себя за полный; М1 —
 * позитивная проверка на GPL v3 дополнена отрицательной на LGPL (иначе
 * подстрока `GPL v3` истинна и для `LGPL v3` — ревьюер воспроизвёл эту
 * подмену и получил зелёный прогон); М2 — версия ffmpeg в тексте сверяется
 * с пином `src-tauri/binaries.lock.json`, а не только сама с собой; М4 —
 * прямая ссылка на архив помечена как источник только macOS-сборки.
 */
const report: SidecarCheckReport = {
  ytDlp: { name: 'yt-dlp', path: '/opt/tube-leak/bin/yt-dlp', status: 'ok', version: '2026.08.20' },
  ffmpeg: { name: 'ffmpeg', path: '/opt/tube-leak/bin/ffmpeg', status: 'ok', version: '9.0.1' },
  deno: { name: 'deno', path: '/opt/tube-leak/bin/deno', status: 'ok', version: '2.9.6' },
}

/**
 * Пин sidecar-бинарников — источник версии ffmpeg для М2: экран называет
 * версию ffmpeg в блоке лицензий текстом (осознанно, doc-комментарий
 * `AboutScreen.vue`, «Два разных источника версии ffmpeg»), и это
 * единственный способ поймать её расхождение с тем, что мы реально
 * раздаём, — сравнением с тем же файлом, что уже сверяют
 * `scripts/check-pins/sources.mjs` и `src-tauri/build.rs`.
 */
const pin = JSON.parse(readFileSync(resolve(process.cwd(), 'src-tauri/binaries.lock.json'), 'utf8')) as {
  ffmpeg: { version: string }
}

describe('AboutScreen', () => {
  it('shows the app version passed by prop', () => {
    const wrapper = mount(AboutScreen, { props: { appVersion: '0.1.1', report } })
    expect(wrapper.text()).toContain('0.1.1')
  })

  it('names every distributed component with its licence, ffmpeg naming GPL v3 explicitly — and never LGPL (mutation guard, М1)', () => {
    const wrapper = mount(AboutScreen, { props: { appVersion: '0.1.1', report } })
    const text = wrapper.text()

    expect(text).toContain('ffmpeg')
    // Проверка не подстрокой `GPL v3` (истинна и для `LGPL v3`), а точной
    // фразой плюс отдельным запретом на LGPL/Lesser где бы то ни было в
    // тексте экрана — ревью показало, что первая версия этого теста
    // (только `toContain('GPL v3')`) осталась зелёной при подмене GPL на
    // LGPL в обоих местах разметки.
    expect(text).toContain('GNU General Public License версии 3 (GPL v3)')
    expect(text).not.toMatch(/LGPL/i)
    expect(text).not.toMatch(/Lesser/i)
    expect(text).toContain('yt-dlp')
    expect(text).toContain('Unlicense')
    expect(text).toContain('deno')
    expect(text).toContain('MIT')
    expect(text).toContain('V8')
    expect(text).toContain('BSD 3-Clause')
    expect(text).toContain('ICU')
    expect(text).toContain('Unicode License v3')
    expect(text).toContain('TypeScript')
    expect(text).toContain('Apache License 2.0')
    expect(text).toContain('SQLite')
    expect(text).toContain('Public Domain')
  })

  it('does not present the four named components as the full list (Б2) — points to THIRD-PARTY-LICENSES.md for the rest', () => {
    const wrapper = mount(AboutScreen, { props: { appVersion: '0.1.1', report } })
    const text = wrapper.text()

    expect(text).toContain('THIRD-PARTY-LICENSES.md')
    // Формулировка честно называет остальные компоненты пермиссивными, а
    // не выдаёт список из четырёх пунктов за исчерпывающий (ревью Б2:
    // THIRD-PARTY-LICENSES.md документирует ещё tauri-plugin-dialog, rfd,
    // tauri-plugin-fs, windows-sys/windows-targets и раздел «Прочие
    // зависимости» — придумывать их точный список в UI не нужно).
    expect(text).toMatch(/MIT.*Apache-2\.0|Apache-2\.0.*MIT/)
  })

  it('gives a plain, retypeable pointer to the ffmpeg sources, and points at files installed next to the app (Б1) — not the private repository', () => {
    const wrapper = mount(AboutScreen, { props: { appVersion: '0.1.1', report } })
    const text = wrapper.text()

    expect(text).toContain('https://ffmpeg.org/releases/ffmpeg-9.0.1.tar.bz2')
    expect(text).toContain('SOURCES-FFMPEG.md')
    expect(text).toContain('THIRD-PARTY-LICENSES.md')

    // Б1: файлы названы лежащими рядом с установленным приложением — не
    // «репозитория проекта» как единственного места. Доступ к репозиторию
    // упомянут отдельно, не как единственный способ их получить. Регекс
    // узкий и намеренно: он ловит именно старую формулировку («в файле
    // ИМЯ.md репозитория проекта», слова впритык), а не любое упоминание
    // слова «репозиторий» рядом — оно и так есть в фразе про тех, у кого
    // есть к нему доступ.
    expect(text).toMatch(/ставится вместе с этим приложением/)
    expect(text).not.toMatch(/\.md репозитория проекта/)

    // Ссылки — читаемый текст, не `<a href>` (задача TL-128: репозиторий
    // приватный, у получателя сборки он не откроется, обещать рабочую
    // ссылку на него нельзя; см. doc-комментарий SOURCES-FFMPEG.md).
    expect(wrapper.findAll('a')).toHaveLength(0)
  })

  it('marks the direct ffmpeg archive link as the macOS build source, and points elsewhere for Windows/Linux (М4)', () => {
    const wrapper = mount(AboutScreen, { props: { appVersion: '0.1.1', report } })
    const text = wrapper.text()

    // М4: неточность SOURCES-FFMPEG.md («один источник на все платформы»)
    // не воспроизводится на экране — прямая ссылка помечена как источник
    // конкретно macOS-сборки, а Windows/Linux названы прямо как случаи,
    // где эта ссылка не является точным источником их сборки.
    expect(text).toMatch(/источник macOS-сборки/)
    expect(text).toContain('Windows')
    expect(text).toContain('Linux')
  })

  it('every ffmpeg version named on screen matches the pinned build version (mutation guard, М2)', () => {
    const wrapper = mount(AboutScreen, { props: { appVersion: '0.1.1', report } })
    const text = wrapper.text()

    expect(pin.ffmpeg.version).toMatch(/^\d+\.\d+(\.\d+)?$/)

    // Не `toContain` одного вхождения (первая версия этого теста прошла
    // мимо мутации `9.0.1` → `9.0.2` в самой лицензионной строке: URL и
    // абзац «Исходный код ffmpeg» по-прежнему называли старую версию
    // отдельной константой `FFMPEG_SOURCES_URL`, и `toContain('ffmpeg
    // 9.0.1')` оставался истинным за их счёт). Здесь собраны ВСЕ версии,
    // упомянутые рядом со словом «ffmpeg» на экране (лицензионная строка,
    // абзац про сборку, имя файла в ссылке на исходники), и каждая обязана
    // совпасть с пином — расхождение в любом из трёх мест красит тест.
    const namedVersions = [...text.matchAll(/ffmpeg[ -](\d+\.\d+\.\d+)/gi)].map((m) => m[1])
    expect(namedVersions.length).toBeGreaterThanOrEqual(3)
    for (const version of namedVersions) {
      expect(version).toBe(pin.ffmpeg.version)
    }
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
