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
 *
 * Правки третьего возврата: Б4 (блокер) — абзац про `THIRD-PARTY-LICENSES.md`
 * обещал «полный перечень и полные тексты лицензий», а сам файл в разделе
 * «Прочие зависимости» прямо говорит обратное (полный список транзитивных
 * зависимостей получается локально `cargo tree`/`npm ls`, а до закрытия #14
 * весь раздел не считается исчерпывающим). Сторож ниже не запоминает
 * формулировку экрана, а читает настоящий файл рядом (сети в тестах нет,
 * файл лежит в этом же репозитории) и требует: пока файл сам признаёт
 * список неполным, экран не вправе называть его «полным»/«исчерпывающим».
 * Н4 — сторож Б3 (три точечных чёрных списка слов) прошёл мимо мутации
 * ревьюера «включает вспомогательные библиотеки под свободными лицензиями
 * BSD и MIT» — правдоподобно ложное же утверждение (те же 470+ пакетов
 * `Cargo.lock` не только BSD/MIT, см. Б3) другими словами. Побелён:
 * теперь запрещён любой идентификатор лицензии и любая характеристика
 * лицензий вообще в тексте именно этого абзаца, не три фразы. Н3 —
 * маркер Н1 нормализует пробелы перед поиском, чтобы перенос строки
 * внутри `<code>…</code>` в шаблоне не красил тест на ровном месте.
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

  it('does not present the four named components as the full list, and does not name or generalise the licences of the rest (Б2, tightened by Б3, whitened by Н4) — points to THIRD-PARTY-LICENSES.md instead', () => {
    const wrapper = mount(AboutScreen, { props: { appVersion: '0.1.1', report } })
    const text = wrapper.text()

    expect(text).toContain('THIRD-PARTY-LICENSES.md')

    // Н4 (заметка третьего возврата): прежний сторож Б3 запрещал три
    // конкретные формулировки (`/пермиссивн/i`, `/MIT.*Apache-2\.0/`,
    // `/все.*лицензи/i`) и ревьюер прошёл мимо него другими словами —
    // «включает вспомогательные библиотеки под свободными лицензиями BSD
    // и MIT» — тот же неправдивый по сути вывод (Б3: измерение `Cargo.lock`
    // нашло MPL-2.0, `Apache-2.0 AND ISC`, `CDLA-Permissive-2.0`,
    // `Unicode-3.0` и другое в релизном графе), но без единого слова из
    // старого чёрного списка. Побелено: берём именно последний абзац-
    // подсказку в этой секции (тот, что отсылает к
    // `THIRD-PARTY-LICENSES.md`) и требуем, чтобы в нём не было НИ ОДНОГО
    // идентификатора лицензии и НИ ОДНОЙ характеристики лицензий вообще —
    // белый список того, что абзацу разрешено называть (только имя файла
    // и путь к нему), а не список запрещённых фраз.
    const hints = sectionByHeading(wrapper, 'Компоненты и лицензии').findAll('.about-screen__hint')
    const pointerParagraph = hints[hints.length - 1]?.text()
    if (pointerParagraph === undefined) throw new Error('expected a trailing hint paragraph in this section')

    const licenceIdentifiersOrCharacteristics = [
      /\bMIT\b/i,
      /\bApache/i,
      /\bBSD\b/i,
      /\bGPL\b/i,
      /\bLGPL\b/i,
      /\bMPL\b/i,
      /\bISC\b/i,
      /\bCDLA/i,
      /\bUnlicense\b/i,
      /public domain/i,
      /общественное достояние/i,
      /пермиссивн/i,
      /permissive/i,
      /copyleft/i,
      /копилефт/i,
      /свободн\p{L}*\s+лицензи/iu,
      /все.*лицензи/i,
    ]
    for (const pattern of licenceIdentifiersOrCharacteristics) {
      expect(pointerParagraph, `pointer paragraph must not match ${pattern}`).not.toMatch(pattern)
    }

    // Мутация ревьюера дословно (Н4) — обязана краснеть на новом наборе,
    // иначе побелка ничего не доказывает.
    const reviewerMutation =
      'Кроме перечисленного, приложение включает вспомогательные библиотеки под свободными лицензиями BSD и MIT.'
    expect(licenceIdentifiersOrCharacteristics.some((pattern) => pattern.test(reviewerMutation))).toBe(true)
  })

  it('never claims THIRD-PARTY-LICENSES.md is a complete or exhaustive list, tied to what the file itself admits (Б4, mutation guard)', () => {
    // Сторож читает настоящий файл рядом (в тестах — локальный файл,
    // сети нет), а не запоминает формулировку экрана: связь с фактом, а
    // не с текстом. Если раздел «Прочие зависимости» когда-нибудь
    // перестанет признавать список неполным (например, файл станет
    // действительно исчерпывающим), этот же тест перестанет находить
    // маркер ниже и упадёт с понятным сообщением — а не продолжит молча
    // сверять устаревшее утверждение.
    const licensesFile = readFileSync(resolve(process.cwd(), 'THIRD-PARTY-LICENSES.md'), 'utf8')
    const otherDepsHeadingIdx = licensesFile.indexOf('## Прочие зависимости')
    expect(otherDepsHeadingIdx, 'expected a "Прочие зависимости" section in THIRD-PARTY-LICENSES.md').toBeGreaterThan(
      -1,
    )
    const otherDepsSection = licensesFile.slice(otherDepsHeadingIdx)
    expect(
      otherDepsSection,
      'expected the file to still admit the transitive-dependency list is obtained locally, not shipped in full',
    ).toMatch(/получить локально/)

    const wrapper = mount(AboutScreen, { props: { appVersion: '0.1.1', report } })
    const componentsSection = sectionByHeading(wrapper, 'Компоненты и лицензии').text()

    // Пока файл сам признаёт список неполным, экран не вправе называть
    // его «полным перечнем», «полными текстами» или «исчерпывающим» —
    // ни этими словами, ни другими с тем же смыслом (regex по корню
    // «полн» и отдельно «исчерпыв», а не по одной запомненной фразе Б2).
    expect(componentsSection).not.toMatch(/полн\p{L}*\s+перечень/iu)
    expect(componentsSection).not.toMatch(/полн\p{L}*\s+текст/iu)
    expect(componentsSection).not.toMatch(/полн\p{L}*\s+список/iu)
    expect(componentsSection).not.toMatch(/исчерпыв/i)
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
    //
    // Н3 (заметка третьего возврата): маркер ищется в тексте с
    // нормализованными пробелами (`\s+` → один пробел), не в сыром
    // `wrapper.text()`, а связка «имя файла» + «, который» допускает
    // пробел между ними — перенос строки внутри `<code>…</code>` (имя
    // файла и закрывающий тег на разных строках шаблона, смысл не
    // меняется) рендерится Vue как настоящий пробел перед запятой
    // (condense-режим схлопывает перенос в пробел, а не убирает его), и
    // прежняя точная строка без пробела иначе роняет тест сообщением про
    // пропавшую правдивость, хотя вёрстка ни на что не влияет.
    const normalizedText = text.replace(/\s+/g, ' ')
    const mdPointers = ['SOURCES-FFMPEG.md', 'THIRD-PARTY-LICENSES.md']
    expect(mdPointers).toHaveLength(2)
    for (const file of mdPointers) {
      const markerPattern = new RegExp(`${file.replace(/\./g, '\\.')}\\s*, который`)
      const match = markerPattern.exec(normalizedText)
      expect(match, `expected "${file}, который" to appear in the screen text`).not.toBeNull()
      const idx = match!.index + match![0].length
      const after = normalizedText.slice(idx, idx + 60)
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
