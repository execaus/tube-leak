import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

import { type VueWrapper, mount } from '@vue/test-utils'
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
 * Правки первого возврата: Б1 — файлы называются лежащими рядом с
 * приложением, не «в репозитории» (репозиторий приватный, получателю
 * недоступен); Б2 — список компонентов не выдаёт себя за полный; М1 —
 * позитивная проверка на GPL v3 дополнена отрицательной на LGPL (иначе
 * подстрока `GPL v3` истинна и для `LGPL v3` — ревьюер воспроизвёл эту
 * подмену и получил зелёный прогон); М2 — версия ffmpeg в тексте сверяется
 * с пином `src-tauri/binaries.lock.json`, а не только сама с собой; М4 —
 * прямая ссылка на архив помечена как источник только macOS-сборки.
 *
 * Правки второго возврата: Б3 — фраза «остальное — пермиссивные
 * MIT/Apache-2.0» из Б2 оказалась неправдой (измерение `Cargo.lock`,
 * doc-комментарий `AboutScreen.vue`, «Список компонентов… не обобщаются»):
 * лицензии остальных библиотек на экране больше не называются и не
 * обобщаются вовсе, только отсылка к файлу; Н1 — проверка Б1 сузилась до
 * per-file (раньше позитив/негатив читали `wrapper.text()` целиком и не
 * заметили бы регресс ровно одного из двух абзацев — воспроизведено и
 * проверено при этой правке); Н2 — сторож М2 сужен до секций
 * «Компоненты и лицензии»/«Исходный код ffmpeg», не всего экрана, чтобы
 * будущее слияние `<dt>`/`<dd>` в блоке «Что установлено сейчас» в одну
 * строку не превратило законное расхождение живой версии sidecar с пином
 * в ложное падение этого теста.
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

/**
 * Секция экрана по заголовку `<h3>` (Н2) — узкий доступ к разметке вместо
 * `wrapper.text()` целиком, нужен там, где сторож обязан не дотягиваться
 * до соседних секций (версии sidecar в «Что установлено сейчас» —
 * законно другие числа, не расхождение с пином).
 */
function sectionByHeading(wrapper: VueWrapper, heading: string) {
  const section = wrapper.findAll('.about-screen__section').find((s) => s.get('h3').text() === heading)
  if (!section) throw new Error(`section not found: ${heading}`)
  return section
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

  it('does not present the four named components as the full list, and does not name or generalise the licences of the rest (Б2, tightened by Б3) — points to THIRD-PARTY-LICENSES.md instead', () => {
    const wrapper = mount(AboutScreen, { props: { appVersion: '0.1.1', report } })
    const text = wrapper.text()

    expect(text).toContain('THIRD-PARTY-LICENSES.md')

    // Б3 (блокер второго возврата): «остальное — пермиссивные
    // MIT/Apache-2.0» было неправдой — измерение `Cargo.lock` нашло
    // MPL-2.0 (слабый copyleft) и другие лицензии в релизном графе. Экран
    // не вправе обобщать то, что не проверял и что наш собственный
    // THIRD-PARTY-LICENSES.md сам называет лишь «преимущественно»
    // MIT/Apache-2.0. Проверяется явный запрет обобщающих слов ОБО ВСЕХ
    // компонентах сразу — не только замена самого текста Б2 на дословно
    // то же самое.
    expect(text).not.toMatch(/пермиссивн/i)
    expect(text).not.toMatch(/MIT.*Apache-2\.0|Apache-2\.0.*MIT/)
    expect(text).not.toMatch(/все.*лицензи/i)
  })

  it('gives a plain, retypeable pointer to the ffmpeg sources, and points at files installed next to the app (Б1) — not the private repository', () => {
    const wrapper = mount(AboutScreen, { props: { appVersion: '0.1.1', report } })
    const text = wrapper.text()

    expect(text).toContain('https://ffmpeg.org/releases/ffmpeg-9.0.1.tar.bz2')

    // Н1 (заметка первого возврата): проверка идёт ОТДЕЛЬНО на каждый из
    // двух `.md`-указателей, не на `wrapper.text()` целиком — первая
    // версия этого теста требовала «фраза где-то в тексте» и «старая
    // фраза нигде», и обе стороны выполнялись, даже когда только ВТОРОЙ
    // абзац (SOURCES-FFMPEG.md) откатился на «который лежит в
    // репозитории проекта»: первый абзац (THIRD-PARTY-LICENSES.md)
    // по-прежнему нёс правильную фразу и закрывал позитивную проверку, а
    // старая формулировка негативной проверки не совпадала с новым
    // текстом отката дословно. Здесь для каждого файла проверяется
    // ровно та фраза, что стоит сразу после его имени в разметке.
    const mdPointers = ['SOURCES-FFMPEG.md', 'THIRD-PARTY-LICENSES.md']
    expect(mdPointers).toHaveLength(2)
    for (const file of mdPointers) {
      const marker = `${file}, который`
      const idx = text.indexOf(marker)
      expect(idx, `expected "${marker}" to appear in the screen text`).toBeGreaterThan(-1)
      const after = text.slice(idx + marker.length, idx + marker.length + 60)
      expect(after).toMatch(/^\s*ставится вместе с этим приложением/)
    }

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

  it('every ffmpeg version named in the licence sections matches the pinned build version (mutation guard, М2)', () => {
    const wrapper = mount(AboutScreen, { props: { appVersion: '0.1.1', report } })

    expect(pin.ffmpeg.version).toMatch(/^\d+\.\d+(\.\d+)?$/)

    // Н2 (заметка первого возврата): область сужена до секций
    // «Компоненты и лицензии»/«Исходный код ffmpeg» — не всего экрана.
    // Полный текст экрана включает и «Что установлено сейчас», где та же
    // подстрока `ffmpeg<версия>` — ЖИВАЯ версия из отчёта проверки
    // (`report.ffmpeg.version`), и её расхождение с пином ЗАКОННО (doc
    // `AboutScreen.vue`, «Два разных источника версии ffmpeg»): сборка на
    // машине пользователя может отставать от актуального пина. Раньше
    // регэксп её не задевал только по случайности вёрстки (`<dt>`/`<dd>`
    // на разных строках не оставляют пробела между «ffmpeg» и версией в
    // отрендеренном тексте) — сузил явно, а не полагаюсь на этот побочный
    // эффект переноса строк.
    const licenceSectionsText = [
      sectionByHeading(wrapper, 'Компоненты и лицензии').text(),
      sectionByHeading(wrapper, 'Исходный код ffmpeg').text(),
    ].join(' ')

    // Не `toContain` одного вхождения (первая версия этого теста прошла
    // мимо мутации `9.0.1` → `9.0.2` в самой лицензионной строке: URL и
    // абзац «Исходный код ffmpeg» по-прежнему называли старую версию
    // отдельной константой `FFMPEG_SOURCES_URL`, и `toContain('ffmpeg
    // 9.0.1')` оставался истинным за их счёт). Здесь собраны ВСЕ версии,
    // упомянутые рядом со словом «ffmpeg» в этих двух секциях
    // (лицензионная строка, абзац про сборку, имя файла в ссылке на
    // исходники), и каждая обязана совпасть с пином — расхождение в
    // любом из трёх мест красит тест.
    const namedVersions = [...licenceSectionsText.matchAll(/ffmpeg[ -](\d+\.\d+\.\d+)/gi)].map((m) => m[1])
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
