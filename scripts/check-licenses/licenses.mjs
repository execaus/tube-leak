// Сторож лицензионного комплекта (TL-136, #143).
//
// Дефект, из-за которого он заведён: `THIRD-PARTY-LICENSES.md`
// утверждал, что всё, кроме четырёх названных компонентов,
// распространяется «под пермиссивными лицензиями (преимущественно MIT
// и/или Apache-2.0)». Измерение релизного графа показало в нём MPL-2.0
// (слабый copyleft), `Apache-2.0 AND ISC`, CDLA-Permissive-2.0,
// Unicode-3.0, Zlib и BSD — ни одной строки о них в документе не было.
//
// Устройство сторожа — две части, и ГРАНИЦА МЕЖДУ НИМИ ВАЖНА:
//
// 1. ОФЛАЙН (этот модуль, идёт в `npm test` без cargo): сверяет снимок
//    `licenses.lock.json` с тем, что лежит в репозитории, и с разделами
//    документа. Ловит появление и пропажу ПАКЕТОВ (через Cargo.lock и
//    package-lock.json), пропажу РАЗДЕЛА и расхождение таблицы
//    «лицензия → раздел».
// 2. ИЗМЕРЕНИЕ (`measure()` из index.mjs, требует cargo): заново строит
//    граф по четырём тройкам и сверяет с снимком целиком, включая
//    РАСПРЕДЕЛЕНИЕ ПАКЕТОВ ПО ВЁДРАМ.
//
// Чего офлайн-часть НЕ умеет и почему это не оговорка, а устройство.
// Перенос пакета из `shipped` в `notInReleaseGraph` меняет обе стороны
// сверки согласованно: пакет по-прежнему назван снимком, Cargo.lock
// по-прежнему сходится. Офлайн отличить такой перенос от правды нечем —
// нужен сам граф. Ревью воспроизвело это мутацией: четыре MPL-крейта
// уехали в `notInReleaseGraph`, строка ушла из таблицы, раздел вырезан
// из документа — офлайн-часть вернула ноль проблем.
//
// Поэтому измерение больше НЕ отдельная команда «на всякий случай»:
// оно вызывается из `npm test` (licenses.test.mjs) и обязано сойтись.
// Цена названа прямо: `npm test` теперь требует cargo в PATH. Для этого
// репозитория это не новое требование — `cargo test` и так в обычном
// прогоне, — но сторож, который «никогда не запускается автоматически»,
// охраняет ровно ничего, и прежний комментарий здесь утверждал обратное.
//
// Чего сторож не проверяет и не притворяется, что проверяет:
// - правильность поля `license` у апстрима (иного машиночитаемого
//   источника нет);
// - содержательную верность раздела: проверяется, что раздел есть и что
//   он настоящий заголовок нужного уровня, а не качество его текста;
// - долю пакетов npm, реально попадающую в бандл Vite: берётся всё
//   production-замыкание, то есть НАДМНОЖЕСТВО.

import { readFile } from 'node:fs/promises'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { noticeHeading, noticeHeadingPattern } from './notices.mjs'
import { effectiveLicenses } from './spdx.mjs'

const __dirname = dirname(fileURLToPath(import.meta.url))
export const REPO_ROOT = resolve(__dirname, '..', '..')
export const SNAPSHOT_PATH = join(REPO_ROOT, 'licenses.lock.json')
export const LICENSES_DOC = 'THIRD-PARTY-LICENSES.md'
export const NOTICES_DOC = 'NOTICES.md'

/**
 * Тройки, по которым снимается релизный граф. Наборы крейтов у них
 * РАЗНЫЕ (windows-*, gtk-*, objc2-*), поэтому одной тройки мало.
 */
export const TARGETS = Object.freeze([
  'aarch64-apple-darwin',
  'x86_64-apple-darwin',
  'x86_64-pc-windows-msvc',
  'x86_64-unknown-linux-gnu',
])

/**
 * Уровень заголовка, на котором обязан стоять раздел лицензии.
 * Разделы лицензий — второго уровня, вровень с остальными разделами
 * документа.
 */
export const SECTION_LEVEL = 2

/**
 * Заголовок раздела, который закрывает лицензию, — ТОЛЬКО НАЗВАНИЕ,
 * без решёток.
 *
 * Решётки убраны по замечанию ревью: раньше здесь лежала строка
 * `'## BSD-3-Clause'`, а наличие проверялось `doc.includes(...)`.
 * Подстрока `## BSD-3-Clause` содержится и в `#### BSD-3-Clause`,
 * поэтому раздел можно было «спрятать», понизив уровень заголовка, —
 * сторож этого не замечал. Теперь уровень задан отдельно и проверяется
 * настоящим заголовком (см. `hasSection`).
 */
export const SECTION_BY_LICENSE = Object.freeze({
  'MIT': 'MIT',
  'Apache-2.0': 'Apache-2.0',
  'ISC': 'ISC',
  'BSD-2-Clause': 'BSD-2-Clause',
  'BSD-3-Clause': 'BSD-3-Clause',
  'MPL-2.0': 'MPL-2.0',
  'CDLA-Permissive-2.0': 'CDLA-Permissive-2.0',
  'Unicode-3.0': 'Unicode-3.0',
  'Zlib': 'Zlib',
})

/**
 * @param {string} text
 * @returns {string}
 */
function escapeRegExp(text) {
  return text.replaceAll(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

/**
 * Регулярное выражение настоящего заголовка нужного уровня.
 *
 * Именно заголовка, а не подстроки: строка должна начинаться ровно с
 * `SECTION_LEVEL` решёток (ни больше, ни меньше), дальше пробел,
 * название и конец строки.
 *
 * @param {string} title
 * @param {number} [level]
 * @returns {RegExp}
 */
export function headingPattern(title, level = SECTION_LEVEL) {
  return new RegExp(`^#{${level}} ${escapeRegExp(title)}[ \\t]*$`, 'm')
}

/**
 * Есть ли в документе раздел с таким названием на нужном уровне.
 *
 * @param {string} doc
 * @param {string} title
 * @returns {boolean}
 */
export function hasSection(doc, title) {
  return headingPattern(title).test(doc)
}

/**
 * Смещение заголовка раздела или -1.
 *
 * @param {string} doc
 * @param {string} title
 * @returns {number}
 */
export function sectionIndex(doc, title) {
  return doc.search(headingPattern(title))
}

/**
 * Пакеты `Cargo.lock` по парам «имя версия».
 *
 * Разбор регулярным выражением, а не TOML-парсером: формат
 * `[[package]]` генерирует сам cargo, файл помечен «not intended for
 * manual editing».
 *
 * @param {string} text содержимое Cargo.lock
 * @returns {string[]} отсортированный список «имя версия»
 */
export function parseCargoLock(text) {
  const found = [...text.matchAll(/\[\[package\]\]\nname = "([^"]+)"\nversion = "([^"]+)"/g)]
  return found.map(([, name, version]) => `${name} ${version}`).sort()
}

/**
 * Production-замыкание `package-lock.json` по парам «имя версия».
 *
 * @param {string} text содержимое package-lock.json
 * @returns {string[]} отсортированный список «имя версия»
 */
export function parseNpmLock(text) {
  const lock = JSON.parse(text)
  const found = []
  for (const [path, entry] of Object.entries(lock.packages ?? {})) {
    if (!path.startsWith('node_modules/')) continue
    if (entry.dev === true || entry.devOptional === true) continue
    if (entry.version === undefined) continue
    const name = path.slice(path.lastIndexOf('node_modules/') + 'node_modules/'.length)
    found.push(`${name} ${entry.version}`)
  }
  return [...new Set(found)].sort()
}

/**
 * Все пакеты, названные снимком, — из всех вёдер сразу.
 *
 * @param {object} section раздел снимка (`rust` или `npm`)
 * @returns {Set<string>}
 */
export function packagesNamedBy(section) {
  const named = new Set()
  for (const packages of Object.values(section.shipped ?? {})) {
    for (const entry of packages) named.add(entry)
  }
  for (const entry of section.self ?? []) named.add(entry)
  for (const entry of section.procMacro ?? []) named.add(entry)
  for (const entry of section.notInReleaseGraph ?? []) named.add(entry)
  return named
}

/**
 * Пакеты, которые едут в поставку, — из обоих разделов снимка.
 *
 * @param {object} snapshot
 * @returns {string[]}
 */
export function shippedPackages(snapshot) {
  const found = new Set()
  for (const section of [snapshot.rust, snapshot.npm]) {
    for (const packages of Object.values(section.shipped ?? {})) {
      for (const entry of packages) found.add(entry)
    }
  }
  return [...found].sort()
}

/**
 * Лицензии, под которые снимок требует раздела.
 *
 * @param {object} snapshot
 * @returns {string[]} отсортированный список без повторов
 */
export function licensesRequiringSection(snapshot) {
  const found = new Set()
  for (const section of [snapshot.rust, snapshot.npm]) {
    for (const license of Object.keys(section.shipped ?? {})) found.add(license)
  }
  return [...found].sort()
}

/**
 * Расхождения снимка с репозиторием и с документами. Пустой список —
 * всё сошлось.
 *
 * @param {object} options
 * @param {object} options.snapshot содержимое licenses.lock.json
 * @param {string} options.cargoLock содержимое Cargo.lock
 * @param {string} options.npmLock содержимое package-lock.json
 * @param {string} options.doc содержимое THIRD-PARTY-LICENSES.md
 * @param {string} options.notices содержимое NOTICES.md
 * @returns {string[]} человекочитаемые расхождения
 */
export function checkSnapshot({ snapshot, cargoLock, npmLock, doc, notices }) {
  const problems = []

  problems.push(
    ...accountedFor({
      what: 'Cargo.lock',
      all: parseCargoLock(cargoLock),
      named: packagesNamedBy(snapshot.rust),
    }),
  )
  problems.push(
    ...accountedFor({
      what: 'package-lock.json (production)',
      all: parseNpmLock(npmLock),
      named: packagesNamedBy(snapshot.npm),
    }),
  )

  for (const license of licensesRequiringSection(snapshot)) {
    const title = SECTION_BY_LICENSE[license]
    if (title === undefined) {
      problems.push(
        `${license}: лицензия есть в релизном графе, но сторож не знает, каким разделом ` +
          `${LICENSES_DOC} она закрывается. Заведи раздел и назови его в SECTION_BY_LICENSE — ` +
          'молча пропускать новую лицензию нельзя, ровно это и было дефектом #143.',
      )
      continue
    }
    if (!hasSection(doc, title)) {
      problems.push(
        `${license}: в ${LICENSES_DOC} нет раздела «${title}» заголовком ${SECTION_LEVEL}-го уровня. ` +
          'Проверяется именно заголовок: понижённый уровень (#### вместо ##) прячет раздел от ' +
          'читателя и от оглавления, а подстрокой такая подмена не ловится.',
      )
    }
  }

  for (const [license, title] of Object.entries(SECTION_BY_LICENSE)) {
    if (!hasSection(doc, title)) {
      problems.push(
        `${license}: раздел «${title}» назван сторожем, но в ${LICENSES_DOC} его нет ` +
          `заголовком ${SECTION_LEVEL}-го уровня. Сторож обязан краснеть и на собственной ` +
          'устаревшей таблице, не только на графе.',
      )
    }
  }

  problems.push(...checkNotices({ snapshot, notices }))
  problems.push(...checkSourceUrls({ snapshot, doc }))

  return problems
}

/**
 * Адреса исходников по §3.2 MPL обязаны стоять и в снимке, и в
 * документе — дословно.
 *
 * Снимок читает `npm run check-pins` (он ходит по этим адресам), а
 * документ читает получатель. Разойдись они — получатель пошёл бы по
 * непроверяемому адресу, а проверялся бы адрес, которого он не видит.
 *
 * @param {{ snapshot: object; doc: string }} options
 * @returns {string[]}
 */
export function checkSourceUrls({ snapshot, doc }) {
  const problems = []
  for (const { package: pkg, url } of snapshot.rust?.sourceUrls ?? []) {
    if (!doc.includes(url)) {
      problems.push(
        `${LICENSES_DOC}: нет адреса исходников ${pkg} (${url}), названного снимком. ` +
          'По §3.2 MPL это наше обязательство перед получателем, и текст с проверяемым ' +
          'списком расходиться не вправе.',
      )
    }
  }
  return problems
}

/**
 * Каждый пакет, который едет в поставку, обязан иметь запись в
 * NOTICES.md.
 *
 * Зачем (замечание Б1 ревью). MIT, BSD, ISC и Zlib требуют СОХРАНЯТЬ
 * уведомление об авторских правах — это условие гранта. Документ
 * обещал, что уведомление «лежит в самом пакете», а у 19 пакетов ведра
 * `shipped` файла лицензии нет вовсе: обещан был путь, которого не
 * существует. Теперь уведомления собраны машинно, и их полнота
 * охраняется здесь.
 *
 * @param {{ snapshot: object; notices: string }} options
 * @returns {string[]}
 */
export function checkNotices({ snapshot, notices }) {
  if (notices === undefined) return []
  const problems = []
  const missing = shippedPackages(snapshot).filter((entry) => !noticeHeadingPattern(entry).test(notices))
  if (missing.length > 0) {
    problems.push(
      `${NOTICES_DOC}: ${missing.length} пакет(ов) едут в поставку, но записи об авторских ` +
        `правах у них нет — ${preview(missing)}. Сохранение уведомления — условие гранта MIT/BSD/ISC/Zlib, ` +
        'а не оформление. Пересобери: npm run check-licenses -- --write',
    )
  }
  return problems
}

/**
 * Каждый пакет репозитория обязан быть назван снимком, и наоборот.
 *
 * @param {{ what: string; all: string[]; named: Set<string> }} options
 * @returns {string[]}
 */
function accountedFor({ what, all, named }) {
  const problems = []
  const missing = all.filter((entry) => !named.has(entry))
  const extra = [...named].filter((entry) => !all.includes(entry)).sort()

  if (missing.length > 0) {
    problems.push(
      `${what}: ${missing.length} пакет(ов) не названы в licenses.lock.json ни одним ведром — ` +
        `${preview(missing)}. Новая зависимость приносит и новую лицензию; ` +
        'пересними снимок: npm run check-licenses -- --write',
    )
  }
  if (extra.length > 0) {
    problems.push(
      `${what}: снимок называет ${extra.length} пакет(ов), которых в нём больше нет — ` +
        `${preview(extra)}. Пересними снимок: npm run check-licenses -- --write`,
    )
  }
  return problems
}

/**
 * @param {string[]} entries
 * @returns {string}
 */
function preview(entries) {
  const head = entries.slice(0, 5).join(', ')
  return entries.length > 5 ? `${head} и ещё ${entries.length - 5}` : head
}

/**
 * Лицензии пакета по его выражению — обёртка, чтобы правило выбора
 * жило в одном месте и у измерения, и у проверки.
 *
 * @param {string} expression
 * @returns {string[]}
 */
export function licensesOf(expression) {
  return effectiveLicenses(expression)
}

/**
 * @param {string} [repoRoot]
 * @returns {Promise<{ snapshot: object; cargoLock: string; npmLock: string; doc: string; notices: string }>}
 */
export async function readInputs(repoRoot = REPO_ROOT) {
  const [snapshot, cargoLock, npmLock, doc, notices] = await Promise.all([
    readFile(join(repoRoot, 'licenses.lock.json'), 'utf8'),
    readFile(join(repoRoot, 'src-tauri', 'Cargo.lock'), 'utf8'),
    readFile(join(repoRoot, 'package-lock.json'), 'utf8'),
    readFile(join(repoRoot, LICENSES_DOC), 'utf8'),
    readFile(join(repoRoot, NOTICES_DOC), 'utf8'),
  ])
  return { snapshot: JSON.parse(snapshot), cargoLock, npmLock, doc, notices }
}

export { noticeHeading }
