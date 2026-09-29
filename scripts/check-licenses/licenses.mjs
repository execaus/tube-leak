// Сторож лицензионного комплекта (TL-136, #143).
//
// Дефект, из-за которого он заведён: `THIRD-PARTY-LICENSES.md`
// утверждал, что всё, кроме четырёх названных компонентов,
// распространяется «под пермиссивными лицензиями (преимущественно MIT
// и/или Apache-2.0)». Измерение релизного графа показало в нём MPL-2.0
// (слабый copyleft), `Apache-2.0 AND ISC`, CDLA-Permissive-2.0,
// Unicode-3.0, Zlib и BSD — ни одной строки о них в документе не было.
// Утверждение о полноте оказалось неправдой; это третий случай того же
// класса за сессию.
//
// Устройство сторожа — две части, разнесённые по цене, как у check-pins:
//
// 1. ОФЛАЙН (этот модуль + licenses.test.mjs, идёт в `npm test`):
//    сверяет снимок `licenses.lock.json` с тем, что лежит в
//    репозитории, и с разделами документа. Сети и cargo не требует.
// 2. ИЗМЕРЕНИЕ (`npm run check-licenses`, scripts/check-licenses/index.mjs):
//    заново строит граф через `cargo tree -e normal` по четырём тройкам
//    и через node_modules, и сверяет с снимком. Требует cargo — поэтому
//    отдельной командой, а не тестом.
//
// Почему офлайн-части хватает, чтобы снимок не протух молча. Лицензия
// пары «имя + версия» на crates.io неизменна, а любой новый крейт или
// смена версии МЕНЯЮТ `Cargo.lock`. Поэтому офлайн-проверка требует,
// чтобы КАЖДЫЙ пакет `Cargo.lock` был назван в снимке — в одном из
// четырёх вёдер (поставляется / наш собственный крейт / proc-macro /
// не в релизном графе). Новая
// зависимость с новой лицензией не может появиться, не покраснев здесь:
// её нет ни в одном ведре. То же для npm через `package-lock.json`.
//
// Чего сторож НЕ проверяет и не притворяется, что проверяет:
// - правильность поля `license` у самого апстрима (мы верим метаданным
//   crates.io/npm — иного машиночитаемого источника нет);
// - что раздел документа СОДЕРЖАТЕЛЬНО верен: проверяется наличие
//   раздела под лицензию, а не качество его текста;
// - долю пакетов npm, реально попадающую в бандл Vite: снимок берёт
//   весь production-замыкание `package-lock.json`, то есть НАДМНОЖЕСТВО
//   (Vite вырезает неиспользованное). Для лицензий это безопасная
//   сторона ошибки: разделов получается больше, чем строго нужно.

import { readFile } from 'node:fs/promises'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { effectiveLicenses } from './spdx.mjs'

const __dirname = dirname(fileURLToPath(import.meta.url))
export const REPO_ROOT = resolve(__dirname, '..', '..')
export const SNAPSHOT_PATH = join(REPO_ROOT, 'licenses.lock.json')
export const LICENSES_DOC = 'THIRD-PARTY-LICENSES.md'

/**
 * Тройки, по которым снимается релизный граф. Наборы крейтов у них
 * РАЗНЫЕ (windows-*, gtk-*, objc2-*), поэтому одной тройки мало: так
 * прятался дефект `flate2`, невидимый на macOS.
 */
export const TARGETS = Object.freeze([
  'aarch64-apple-darwin',
  'x86_64-apple-darwin',
  'x86_64-pc-windows-msvc',
  'x86_64-unknown-linux-gnu',
])

/**
 * Заголовок раздела документа, который закрывает лицензию.
 *
 * Это НЕ список известных лицензий и не белый список: отсутствие
 * лицензии в этой таблице — красное (сторож не знает, чем её закрыть), и
 * наличие строки тоже ничего не доказывает, пока такого заголовка нет в
 * самом файле. Проверяются обе стороны.
 */
export const SECTION_BY_LICENSE = Object.freeze({
  'MIT': '## MIT',
  'Apache-2.0': '## Apache-2.0',
  'ISC': '## ISC',
  'BSD-2-Clause': '## BSD-2-Clause',
  'BSD-3-Clause': '## BSD-3-Clause',
  'MPL-2.0': '## MPL-2.0',
  'CDLA-Permissive-2.0': '## CDLA-Permissive-2.0',
  'Unicode-3.0': '## Unicode-3.0',
  'Zlib': '## Zlib',
})

/**
 * Пакеты `Cargo.lock` по парам «имя версия».
 *
 * Разбор регулярным выражением, а не TOML-парсером: зависимость ради
 * трёх полей не нужна, а формат `[[package]]` у cargo стабилен и
 * генерируется им самим (файл помечен «not intended for manual
 * editing»). Порядок полей внутри записи cargo тоже пишет сам.
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
 * Записи с `dev: true` — только сборочный инструмент (vite, vitest,
 * eslint), в дистрибутив они не едут.
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
 * Расхождения снимка с репозиторием и с документом. Пустой список —
 * всё сошлось.
 *
 * @param {object} options
 * @param {object} options.snapshot содержимое licenses.lock.json
 * @param {string} options.cargoLock содержимое Cargo.lock
 * @param {string} options.npmLock содержимое package-lock.json
 * @param {string} options.doc содержимое THIRD-PARTY-LICENSES.md
 * @returns {string[]} человекочитаемые расхождения
 */
export function checkSnapshot({ snapshot, cargoLock, npmLock, doc }) {
  const problems = []

  problems.push(
    ...accountedFor({
      what: 'Cargo.lock',
      all: parseCargoLock(cargoLock),
      named: packagesNamedBy(snapshot.rust),
      regenerate: 'npm run check-licenses -- --write',
    }),
  )
  problems.push(
    ...accountedFor({
      what: 'package-lock.json (production)',
      all: parseNpmLock(npmLock),
      named: packagesNamedBy(snapshot.npm),
      regenerate: 'npm run check-licenses -- --write',
    }),
  )

  for (const license of licensesRequiringSection(snapshot)) {
    const heading = SECTION_BY_LICENSE[license]
    if (heading === undefined) {
      problems.push(
        `${license}: лицензия есть в релизном графе, но сторож не знает, каким разделом ` +
          `${LICENSES_DOC} она закрывается. Заведи раздел и назови его в SECTION_BY_LICENSE — ` +
          'молча пропускать новую лицензию нельзя, ровно это и было дефектом #143.',
      )
      continue
    }
    if (!doc.includes(heading)) {
      problems.push(
        `${license}: SECTION_BY_LICENSE обещает раздел «${heading}», но в ${LICENSES_DOC} его нет. ` +
          'Либо раздел переименовали, либо его не завели вовсе.',
      )
    }
  }

  for (const [license, heading] of Object.entries(SECTION_BY_LICENSE)) {
    if (!doc.includes(heading)) {
      problems.push(
        `${license}: раздел «${heading}» назван сторожем, но в ${LICENSES_DOC} отсутствует. ` +
          'Сторож обязан краснеть и на собственной устаревшей таблице, не только на графе.',
      )
    }
  }

  return problems
}

/**
 * Каждый пакет репозитория обязан быть назван снимком, и наоборот.
 *
 * Обе стороны, а не одна: пропавший пакет — это снимок, описывающий
 * несуществующий граф, и такой снимок так же лжёт, как и неполный.
 *
 * @param {{ what: string; all: string[]; named: Set<string>; regenerate: string }} options
 * @returns {string[]}
 */
function accountedFor({ what, all, named, regenerate }) {
  const problems = []
  const missing = all.filter((entry) => !named.has(entry))
  const extra = [...named].filter((entry) => !all.includes(entry)).sort()

  if (missing.length > 0) {
    problems.push(
      `${what}: ${missing.length} пакет(ов) не названы в licenses.lock.json ни одним ведром — ` +
        `${preview(missing)}. Новая зависимость приносит и новую лицензию; пересними снимок: ${regenerate}`,
    )
  }
  if (extra.length > 0) {
    problems.push(
      `${what}: снимок называет ${extra.length} пакет(ов), которых в нём больше нет — ` +
        `${preview(extra)}. Пересними снимок: ${regenerate}`,
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
 * Лицензии пакета по его выражению — тонкая обёртка, чтобы правило
 * выбора жило в одном месте и у измерения, и у проверки.
 *
 * @param {string} expression
 * @returns {string[]}
 */
export function licensesOf(expression) {
  return effectiveLicenses(expression)
}

/**
 * @param {string} [repoRoot]
 * @returns {Promise<{ snapshot: object; cargoLock: string; npmLock: string; doc: string }>}
 */
export async function readInputs(repoRoot = REPO_ROOT) {
  const [snapshot, cargoLock, npmLock, doc] = await Promise.all([
    readFile(join(repoRoot, 'licenses.lock.json'), 'utf8'),
    readFile(join(repoRoot, 'src-tauri', 'Cargo.lock'), 'utf8'),
    readFile(join(repoRoot, 'package-lock.json'), 'utf8'),
    readFile(join(repoRoot, LICENSES_DOC), 'utf8'),
  ])
  return { snapshot: JSON.parse(snapshot), cargoLock, npmLock, doc }
}
