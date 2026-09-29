// Сбор уведомлений об авторских правах (TL-136, замечание Б1 ревью).
//
// Зачем файл вообще появился. `THIRD-PARTY-LICENSES.md` обещал
// получателю: «уведомление каждого пакета лежит в самом пакете (файл
// LICENSE в его исходниках)». Ревью проверило листингом и нашло **19
// пакетов ведра shipped**, у которых нет ни LICENSE, ни LICENCE, ни
// COPYING, ни NOTICE: семейства `objc2-*`, `unic-*`, `webview2-com*`,
// `block2`, `dispatch2`, `dlopen2`, `alloc-stdlib`, `selectors`. Для MIT
// и BSD сохранение уведомления — УСЛОВИЕ ГРАНТА, а не оформление, и мы
// назвали способ его выполнить, которого не существует.
//
// Решение — не чинить формулировку, а выполнить условие буквально:
// собрать уведомления машинно и положить их рядом с приложением
// (`NOTICES.md` едет в бандл).
//
// Три источника, строго в этом порядке, и каждый назван в самой записи:
//
// 1. ФАЙЛ пакета (LICENSE/COPYING/NOTICE) — строки с копирайтом из него.
//    Это то, что апстрим сам считает своим уведомлением.
// 2. МЕТАДАННЫЕ пакета (`authors` манифеста), когда файла нет вовсе.
//    Помечается прямо в записи: апстрим файла не поставляет.
// 3. Ничего — когда нет ни файла, ни авторов (у части крейтов
//    `authors` пуст). Такие пакеты перечисляются явным списком с этой
//    пометкой и ссылкой на репозиторий: честный список того, что
//    физически недоступно, полезнее ссылки в никуда.
//
// Полных текстов лицензий здесь нет намеренно — они в
// THIRD-PARTY-LICENSES.md, по одному на лицензию. Дублировать их 328 раз
// незачем; уведомления же у каждого пакета свои, и вот они здесь.

import { execFileSync } from 'node:child_process'
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'

/**
 * Имена файлов, которые апстримы используют под уведомление.
 * `LICENCE` — британское написание, встречается; без него часть
 * пакетов ложно попала бы в «файла нет».
 */
const LICENSE_FILE = /^(licen[cs]e|copying|notice)/i

/**
 * Строка уведомления. Узко намеренно: широкий `/copyright/i` тащит
 * прозу самих лицензий («...copyright notice shall be included...»),
 * и уведомление утонуло бы в тексте, который и так приведён отдельно.
 */
// Настоящее уведомление отличается от ПРОЗЫ лицензии регистром и
// положением: оно начинает строку словом `Copyright` с заглавной, тогда
// как в тексте лицензии слово `copyright` попадает в начало строки лишь
// переносом посреди предложения — «copyright notice that is included in
// or attached to the work», «copyright license to reproduce, prepare
// Derivative Works of,». Поймано на `ring`: половина его записи
// состояла из таких обрывков текста Apache-2.0.
//
// Проверка РЕГИСТРОЗАВИСИМА намеренно. Первая попытка чинить это
// требованием `(c)`/`©`/года сразу после слова отсекала заодно вполне
// настоящие уведомления без года — `Copyright The Rust Project
// Developers` и подобные: записей «из файла пакета» стало 241 вместо
// 276, а 35 пакетов уехали в восстановленные из метаданных. Регистр
// разделяет эти два случая точно, а год — нет.
const COPYRIGHT_AT_START = /^\s*(?:\/\/|#|\*)?\s*(?:Copyright\b|©|\(c\)\s*\d)/

/**
 * Поле SPDX-документа. Часть пакетов не кладёт текст лицензии вовсе, а
 * поставляет `LICENSE.spdx`, и правообладатель назван там именно так:
 * `@tauri-apps/plugin-dialog` — `PackageCopyrightText: 2019-2022, The
 * Tauri Programme in the Commons Conservancy`. Правило «строка
 * начинается словом Copyright» такую строку теряло, и пакет уходил в
 * «уведомление недоступно» — ложное утверждение перед получателем и
 * невыполненное условие гранта MIT.
 */
const COPYRIGHT_SPDX = /^\s*PackageCopyrightText:\s*\S/

/**
 * Знак копирайта с годом ГДЕ УГОДНО в строке, не только в начале.
 *
 * Правообладатель бывает назван не первым словом: в файлах ICU4X рядом
 * с `Copyright © 2020-2024 Unicode, Inc.` стоит `ICU 1.8.1 to ICU 57.1 ©
 * 1995-2016 International Business Machines Corporation and others.` —
 * это ДРУГОЕ лицо, и для Unicode-3.0 сохранение его уведомления такое же
 * условие гранта. Привязка к началу строки теряла его у 15 крейтов.
 */
const COPYRIGHT_INLINE = /(?:©|\(c\))\s*\d{4}/i

/**
 * Строка вида `Copyright notice…` / `Copyright license…` — это всё ещё
 * проза лицензии, просто начатая с заглавной.
 */
const COPYRIGHT_PROSE = /^Copyright\s+(?:notice|license|holder|owner)\b/i

/**
 * SPDX-заглушка: поле есть, а правообладатель в нём не назван.
 */
const SPDX_UNKNOWN = /^PackageCopyrightText:\s*(?:NOASSERTION|NONE)\s*$/i

/**
 * Похожа ли строка на уведомление об авторских правах.
 *
 * @param {string} raw строка как есть
 * @returns {boolean}
 */
function looksLikeCopyright(raw) {
  return COPYRIGHT_AT_START.test(raw) || COPYRIGHT_SPDX.test(raw) || COPYRIGHT_INLINE.test(raw)
}

/**
 * Шаблонная строка из «как применять эту лицензию», а не уведомление.
 * У Apache-2.0 в приложении стоит `Copyright [yyyy] [name of copyright
 * owner]`; взять её за уведомление значило бы выдать заготовку за имя
 * правообладателя.
 */
const PLACEHOLDER = /\[yyyy]|\[year]|<year>|\[name of copyright owner]|\[fullname]|<name of author>/i

/**
 * @param {string} text
 * @returns {string}
 */
function escapeRegExp(text) {
  return text.replaceAll(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

/**
 * Заголовок записи пакета в NOTICES.md как регулярное выражение.
 *
 * Уровень строгий (ровно три решётки), а вот хвост после имени пакета
 * допускается: в файле заголовок выглядит как
 * `### имя версия — лицензия`. Привязка к концу строки сразу после
 * версии — та ошибка, из-за которой первая редакция сторожа объявила
 * отсутствующими все 328 записей разом.
 *
 * @param {string} entry «имя версия»
 * @returns {RegExp}
 */
export function noticeHeadingPattern(entry) {
  return new RegExp(`^### ${escapeRegExp(entry)}(?= —|[ \\t]*$)`, 'm')
}

/**
 * Строки копирайта из текста лицензии, без повторов и мусора.
 *
 * @param {string} text
 * @returns {string[]}
 */
export function copyrightLinesOf(text) {
  const found = []
  for (const raw of text.split('\n')) {
    const line = raw.replace(/^\s*(?:\/\/|#|\*)\s?/, '').trim()
    if (line === '') continue
    if (!looksLikeCopyright(raw)) continue
    if (COPYRIGHT_PROSE.test(line)) continue
    if (SPDX_UNKNOWN.test(line)) continue
    if (line.length > 300) continue
    if (PLACEHOLDER.test(line)) continue
    if (!found.includes(line)) found.push(line)
  }
  return found
}

/**
 * Уведомление одного пакета и ОТКУДА оно взято.
 *
 * @param {{ licenseTexts?: Array<string | null>; authors: string[] }} input
 * @returns {{ source: 'file' | 'metadata' | 'none'; lines: string[]; hadFiles: boolean }}
 */
export function noticeFrom({ licenseTexts = [], authors }) {
  // Файлов у пакета может быть несколько, и уведомление лежит не всегда
  // в первом: у `ring` в `LICENSE` только объяснение, какая часть кода
  // под какой лицензией, а сам копирайт — в `LICENSE-other-bits`; у
  // `webpki-roots` в `LICENSE` текст соглашения CDLA без строки
  // копирайта вовсе. Брать только первый файл значило бы записать эти
  // пакеты в «уведомление недоступно», хотя оно есть.
  const fromFiles = []
  for (const text of licenseTexts) {
    if (text === null || text === undefined) continue
    for (const line of copyrightLinesOf(text)) {
      if (!fromFiles.includes(line)) fromFiles.push(line)
    }
  }
  // Был ли у пакета файл лицензии вообще — не то же самое, что «нашлось
  // уведомление». Различать обязательно: часть пакетов (`winnow`,
  // `pin-project-lite`, `cargo-platform`, `webpki-roots`) файл
  // ПОСТАВЛЯЕТ, но в нём нет ни одной строки копирайта — текст начинается
  // сразу с «Permission is hereby granted…». Свалив оба случая в один,
  // файл сказал бы получателю неправду: «апстрим не поставляет файла
  // лицензии» про пакет, который его поставляет.
  const hadFiles = licenseTexts.some((text) => typeof text === 'string' && text.trim() !== '')

  // Объединение по всем файлам, а не первый непустой: у пакетов с
  // заимствованным кодом правообладателей несколько, и каждый из них
  // требует сохранения своего уведомления (`ring` — и Brian Smith, и
  // авторы BoringSSL/Go). Потерять любое значило бы не выполнить
  // условие гранта ровно для него.
  if (fromFiles.length > 0) return { source: 'file', lines: fromFiles, hadFiles }
  const named = (authors ?? []).filter((author) => author.trim() !== '')
  if (named.length > 0) return { source: 'metadata', lines: named, hadFiles }
  return { source: 'none', lines: [], hadFiles }
}

/**
 * Текст файла лицензии крейта из локального кэша cargo.
 *
 * Два места, потому что cargo распаковывает не всё: у собранных крейтов
 * есть каталог в `registry/src`, у остальных — только архив `.crate` в
 * `registry/cache`. Читать надо оба, иначе «файла нет» окажется
 * свойством кэша, а не пакета.
 *
 * @param {string} name
 * @param {string} version
 * @returns {string | null}
 */
export function crateLicenseTexts(name, version) {
  const crate = `${name}-${version}`
  for (const root of registryRoots('src')) {
    const dir = join(root, crate)
    let entries
    try {
      entries = readdirSync(dir)
    } catch {
      continue
    }
    return licenseFilesIn(dir, entries).map((file) => readFileSync(join(dir, file), 'utf8'))
  }
  for (const root of registryRoots('cache')) {
    const archive = join(root, `${crate}.crate`)
    let listing
    try {
      listing = execFileSync('tar', ['tzf', archive], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] })
    } catch {
      continue
    }
    const members = listing
      .split('\n')
      .map((entry) => entry.replace(`${crate}/`, ''))
      .filter((entry) => entry !== '' && !entry.includes('/'))
    return licenseFilesIn(null, members).map((file) =>
      execFileSync('tar', ['xzfO', archive, `${crate}/${file}`], {
        encoding: 'utf8',
        stdio: ['ignore', 'pipe', 'ignore'],
      }),
    )
  }
  // Крейта нет НИ В ОДНОМ месте кэша — это не то же самое, что «пакет не
  // поставляет файла лицензии», и путать их нельзя: при пустом кэше все
  // 295 крейтов получили бы пометку «файла лицензии нет вовсе», и
  // `--write` записал бы её получателю молча как факт. `null` заставляет
  // вызывающего отказать (см. index.mjs).
  return null
}

/**
 * @param {string} pkgDir каталог пакета в node_modules
 * @returns {string[]}
 */
export function npmLicenseTexts(pkgDir) {
  let entries
  try {
    entries = readdirSync(pkgDir)
  } catch {
    return null
  }
  return licenseFilesIn(pkgDir, entries).map((file) => readFileSync(join(pkgDir, file), 'utf8'))
}

/**
 * Файлы лицензий пакета: сначала общий (`LICENSE`), затем остальные
 * (`LICENSE-MIT`, `LICENSE-APACHE`, `LICENSE-other-bits`, `NOTICE`) по
 * алфавиту — порядок устойчив, а уведомление ищется по всем.
 *
 * @param {string[]} entries
 * @returns {string[]}
 */
function licenseFilesIn(dir, entries) {
  const candidates = entries
    .filter((entry) => LICENSE_FILE.test(entry))
    // Каталог с именем `licenses/` подходит под ту же маску, и
    // readFileSync на нём падает EISDIR. Проверяем, что это файл, когда
    // есть что проверять (у членов архива каталога нет — там имена уже
    // отфильтрованы по отсутствию `/`).
    .filter((entry) => dir === null || isFile(join(dir, entry)))
    .sort()
  const plain = candidates.filter((entry) => /^licen[cs]e(\.(txt|md))?$/i.test(entry))
  return [...plain, ...candidates.filter((entry) => !plain.includes(entry))]
}

/**
 * @param {string} path
 * @returns {boolean}
 */
function isFile(path) {
  try {
    return statSync(path).isFile()
  } catch {
    return false
  }
}

/**
 * @param {'src' | 'cache'} kind
 * @returns {string[]}
 */
function registryRoots(kind) {
  const home = process.env.HOME ?? ''
  const base = join(home, '.cargo', 'registry', kind)
  try {
    return readdirSync(base).map((entry) => join(base, entry))
  } catch {
    return []
  }
}

/**
 * Заголовок записи пакета. Один и тот же вид у генератора и у сторожа —
 * иначе сторож искал бы не то, что пишется.
 *
 * @param {string} entry «имя версия»
 * @returns {string}
 */
export function noticeHeading(entry) {
  return `### ${entry}`
}

/**
 * Разбирает NOTICES.md обратно в записи.
 *
 * Нужен стороже, который сверяет СОДЕРЖИМОЕ записи с файлом пакета, а не
 * только наличие заголовка. Без него охранялся лишь заголовок: удаление
 * строки копирайта ВНУТРИ записи проходило через `npm test` зелёным.
 *
 * @param {string} text содержимое NOTICES.md
 * @returns {Map<string, { license: string; lines: string[]; body: string }>}
 */
export function parseNotices(text) {
  const records = new Map()
  for (const part of text.split(/^### /m).slice(1)) {
    const newline = part.indexOf('\n')
    if (newline === -1) continue
    const head = part.slice(0, newline).trim()
    const dash = head.indexOf(' — ')
    const entry = dash === -1 ? head : head.slice(0, dash)
    const body = part.slice(newline + 1)
    const block = /```\n([\s\S]*?)\n```/.exec(body)
    records.set(entry, {
      license: dash === -1 ? '' : head.slice(dash + ' — '.length),
      lines: block === null ? [] : block[1].split('\n').filter((line) => line !== ''),
      body,
    })
  }
  return records
}

/**
 * Собирает NOTICES.md.
 *
 * @param {{ rust: Array<{ entry: string; license: string; notice: object; repository: string | null }>;
 *           npm: Array<{ entry: string; license: string; notice: object; repository: string | null }> }} collected
 * @returns {string}
 */
export function renderNotices(collected) {
  const out = []
  const all = [...collected.rust, ...collected.npm]
  const unavailable = all.filter((item) => item.notice.source === 'none')
  const fromMetadata = all.filter((item) => item.notice.source === 'metadata')

  out.push(`# Уведомления об авторских правах

Файл собирается машинно и едет вместе с приложением. Он существует
потому, что MIT, BSD, ISC, Zlib и Apache-2.0 требуют **сохранять
уведомление об авторских правах** при распространении — это условие
гранта, а не оформление. Полные тексты самих лицензий — в
\`THIRD-PARTY-LICENSES.md\` рядом; дублировать их здесь по разу на пакет
незачем, а уведомления у пакетов разные, и они здесь.

**Как собрано.** Для каждого пакета, который едет в поставку (ведро
\`shipped\` в \`licenses.lock.json\`), взяты строки копирайта из файла
лицензии самого пакета. Где апстрим файла не поставляет — взяты авторы
из метаданных пакета, и запись об этом прямо говорит. Где нет ни того,
ни другого — пакет назван в отдельном списке внизу, со ссылкой на его
репозиторий.

Не редактировать руками: пересборка — \`npm run check-licenses -- --write\`.

Итог этой сборки: всего записей ${all.length}, из них из файла пакета
${all.length - fromMetadata.length - unavailable.length}, восстановлено из метаданных ${fromMetadata.length},
недоступно ни одним способом ${unavailable.length}.
`)

  out.push('\n## Rust-крейты\n')
  for (const item of collected.rust) out.push(renderEntry(item))

  out.push('\n## npm-пакеты\n')
  for (const item of collected.npm) out.push(renderEntry(item))

  out.push(`
## Пакеты, для которых уведомление недоступно

Выдумывать правообладателя мы не станем: ниже — честный список с адресом
репозитория, где уведомление можно получить у самого автора. Случая
два, и они разные:

- **файл лицензии есть, копирайта в нём нет** — пакет поставляет текст
  лицензии, но тот начинается сразу с условий, без строки об авторских
  правах, и авторы в метаданных не указаны;
- **файла лицензии нет вовсе** — лицензия заявлена только полем
  \`license\` в манифесте.
`)
  if (unavailable.length === 0) {
    out.push('\nТаких пакетов нет.\n')
  } else {
    for (const item of unavailable) {
      const why = item.notice.hadFiles ? 'файл лицензии есть, копирайта в нём нет' : 'файла лицензии нет вовсе'
      out.push(
        `- \`${item.entry}\` (${item.license}) — ${why}; ${item.repository ?? 'репозиторий не указан в метаданных'}\n`,
      )
    }
  }

  return `${out.join('').trimEnd()}\n`
}

/**
 * @param {{ entry: string; license: string; notice: object; repository: string | null }} item
 * @returns {string}
 */
function renderEntry(item) {
  const head = `\n${noticeHeading(item.entry)} — ${item.license}\n`
  if (item.notice.source === 'none') {
    const why = item.notice.hadFiles
      ? 'файл лицензии в пакете есть, но строки с уведомлением об авторских правах в нём нет, и авторы в метаданных не указаны'
      : 'апстрим не поставляет ни файла лицензии, ни авторов в метаданных'
    return `${head}\nУведомление недоступно: ${why}. Репозиторий: ${item.repository ?? 'не указан'}.\n`
  }
  const note =
    item.notice.source === 'metadata'
      ? '\nУведомление восстановлено из метаданных пакета — апстрим файла лицензии не поставляет.\n'
      : ''
  return `${head}${note}\n\`\`\`\n${item.notice.lines.join('\n')}\n\`\`\`\n`
}
