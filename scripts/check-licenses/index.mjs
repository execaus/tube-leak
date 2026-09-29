#!/usr/bin/env node
// Измерение лицензионного состава поставки (TL-136, #143).
//
// `npm run check-licenses`          — измерить и сверить со снимком;
// `npm run check-licenses -- --write` — измерить и переписать снимок.
//
// Почему отдельной командой, а не тестом: нужен cargo (в тестах его
// звать нельзя — прогон фронтенда не обязан иметь Rust-тулчейн) и
// распакованный node_modules. Офлайн-часть, которая идёт в `npm test`,
// живёт в licenses.test.mjs и сверяет снимок с Cargo.lock,
// package-lock.json и разделами документа.
//
// Как снимается граф Rust. `cargo tree -e normal` — рёбра только
// обычных зависимостей: без dev (их нет в релизе вовсе) и без build
// (они исполняются на сборке и в бинарник не попадают). По каждой из
// четырёх троек отдельно — наборы у них разные.
//
// Отдельным ведром идут proc-macro-крейты: они тоже стоят на обычных
// рёбрах, но исполняются компилятором и в поставку не едут. Признак
// берётся из метаданных (`targets[].kind == "proc-macro"`), а не из
// имени крейта: суффикс `-derive`/`-macros` — соглашение, а не факт.

import { execFile } from 'node:child_process'
import { readFile, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { promisify } from 'node:util'

import {
  checkSnapshot,
  licensesOf,
  packagesNamedBy,
  parseCargoLock,
  parseNpmLock,
  REPO_ROOT,
  SNAPSHOT_PATH,
  TARGETS,
} from './licenses.mjs'

const run = promisify(execFile)
const CARGO_DIR = join(REPO_ROOT, 'src-tauri')

/**
 * Понятная причина вместо пустого объекта ошибки.
 *
 * Измерено на себе: запуск без `export PATH=…/rustup/bin:$PATH` валит
 * команду выводом `{ stdout: '', stderr: '' }` — по нему нельзя понять
 * ни что упало, ни что делать. Сторож, отказывающий неразборчиво, —
 * сторож, которому перестают верить; в этом проекте это уже случалось.
 *
 * @param {NodeJS.ErrnoException} error
 * @returns {Error} исходная ошибка либо названная причина
 */
export function cargoFailure(error) {
  if (error?.code !== 'ENOENT') return error
  return new Error(
    'cargo не найден в PATH, поэтому релизный граф измерить нечем. ' +
      'В этом окружении cargo лежит вне PATH по умолчанию: ' +
      'export PATH="/opt/homebrew/opt/rustup/bin:$PATH". ' +
      'Офлайн-часть сторожа (npm test) cargo не требует — она сверяет снимок ' +
      'licenses.lock.json с Cargo.lock, package-lock.json и разделами документа.',
  )
}

/**
 * @param {string[]} args
 * @param {object} options
 * @returns {Promise<{ stdout: string }>}
 */
async function cargo(args, options) {
  try {
    return await run('cargo', args, options)
  } catch (error) {
    throw cargoFailure(error)
  }
}

/**
 * @param {string[]} argv
 * @returns {{ write: boolean }}
 */
export function parseArgs(argv) {
  return { write: argv.includes('--write') }
}

/**
 * Метаданные пакетов: лицензия и признак proc-macro.
 *
 * @returns {Promise<Map<string, { license: string; procMacro: boolean }>>}
 */
async function readMetadata() {
  const { stdout } = await cargo(['metadata', '--locked', '--format-version', '1'], {
    cwd: CARGO_DIR,
    maxBuffer: 256 * 1024 * 1024,
  })
  const metadata = JSON.parse(stdout)
  const byPackage = new Map()
  for (const pkg of metadata.packages) {
    byPackage.set(`${pkg.name} ${pkg.version}`, {
      license: pkg.license ?? '',
      procMacro: pkg.targets.some((target) => target.kind.includes('proc-macro')),
    })
  }
  return byPackage
}

/**
 * Релизный граф одной тройки.
 *
 * @param {string} target
 * @returns {Promise<string[]>} пакеты «имя версия»
 */
async function releaseGraphOf(target) {
  const { stdout } = await cargo(
    ['tree', '--locked', '--offline', '-e', 'normal', '--target', target, '--prefix', 'none', '--format', '{p}'],
    { cwd: CARGO_DIR, maxBuffer: 64 * 1024 * 1024 },
  )
  const packages = stdout
    .split('\n')
    .map((line) => line.trim())
    .filter((line) => line !== '')
    // `{p}` печатает «имя vВЕРСИЯ [(путь)]»; путь есть только у нашего
    // собственного крейта, он же корень графа.
    .map((line) => line.replace(/\s+\(.*\)$/, ''))
    .map((line) => line.replace(/ v(?=[^ ]+$)/, ' '))
  return [...new Set(packages)]
}

/**
 * @returns {Promise<object>} раздел `rust` снимка
 */
async function measureRust() {
  const metadata = await readMetadata()
  const inGraph = new Set()
  for (const target of TARGETS) {
    for (const pkg of await releaseGraphOf(target)) inGraph.add(pkg)
  }

  const cargoLock = await readFile(join(CARGO_DIR, 'Cargo.lock'), 'utf8')
  const allPackages = parseCargoLock(cargoLock)
  const root = [...inGraph].find((pkg) => pkg.startsWith('tube-leak '))
  if (root !== undefined) inGraph.delete(root)

  /** @type {Record<string, string[]>} */
  const shipped = {}
  const procMacro = []
  for (const pkg of [...inGraph].sort()) {
    const info = metadata.get(pkg)
    if (info === undefined) throw new Error(`нет метаданных для ${pkg}`)
    if (info.procMacro) {
      procMacro.push(pkg)
      continue
    }
    for (const license of licensesOf(info.license)) {
      shipped[license] ??= []
      shipped[license].push(pkg)
    }
  }

  const notInReleaseGraph = allPackages.filter((pkg) => !inGraph.has(pkg) && pkg !== root)
  return {
    _measuredBy: `cargo tree --locked -e normal по тройкам: ${TARGETS.join(', ')}`,
    targets: TARGETS,
    shipped: sortValues(shipped),
    // Наш собственный крейт — корень графа. Он едет в поставку, но
    // третьей стороной не является и раздела в файле лицензий не
    // требует. Отдельным ведром, а не молчанием: пакет, не названный
    // НИ ОДНИМ ведром, обязан красить прогон — иначе дыра в проверке
    // полноты, ради которой снимок и заведён.
    self: root === undefined ? [] : [root],
    procMacro,
    notInReleaseGraph,
  }
}

/**
 * @returns {Promise<object>} раздел `npm` снимка
 */
async function measureNpm() {
  const npmLock = await readFile(join(REPO_ROOT, 'package-lock.json'), 'utf8')
  const production = parseNpmLock(npmLock)

  /** @type {Record<string, string[]>} */
  const shipped = {}
  const withoutLicenseField = []
  for (const entry of production) {
    const name = entry.slice(0, entry.lastIndexOf(' '))
    const manifest = JSON.parse(
      await readFile(join(REPO_ROOT, 'node_modules', name, 'package.json'), 'utf8'),
    )
    const license = typeof manifest.license === 'string' ? manifest.license : ''
    if (license === '') {
      withoutLicenseField.push(entry)
      continue
    }
    for (const id of licensesOf(license)) {
      shipped[id] ??= []
      shipped[id].push(entry)
    }
  }

  return {
    _measuredBy:
      'package-lock.json (записи без dev) + поле license из node_modules/<пакет>/package.json. ' +
      'Это НАДМНОЖЕСТВО: Vite кладёт в бандл только импортированное.',
    shipped: sortValues(shipped),
    procMacro: [],
    notInReleaseGraph: withoutLicenseField,
  }
}

/**
 * @param {Record<string, string[]>} groups
 * @returns {Record<string, string[]>}
 */
function sortValues(groups) {
  /** @type {Record<string, string[]>} */
  const sorted = {}
  for (const key of Object.keys(groups).sort()) sorted[key] = [...groups[key]].sort()
  return sorted
}

/**
 * @returns {Promise<object>}
 */
export async function measure() {
  return {
    _readme: [
      'Снимок лицензионного состава поставки (TL-136, #143). НЕ РЕДАКТИРОВАТЬ РУКАМИ.',
      'Перемер: npm run check-licenses -- --write. Сверка: npm run check-licenses.',
      'Офлайн-сторож (npm test, scripts/check-licenses/licenses.test.mjs) требует, чтобы',
      'КАЖДЫЙ пакет Cargo.lock и package-lock.json был назван здесь одним из вёдер:',
      'shipped (едет в поставку, сгруппировано по лицензии), self (наш собственный крейт —',
      'не третья сторона), procMacro (исполняется компилятором, в бинарник не попадает),',
      'notInReleaseGraph (достижим только через dev-/build-зависимости — в релизном графе его нет).',
      'У каждой лицензии ведра shipped обязан быть раздел в THIRD-PARTY-LICENSES.md;',
      'соответствие «лицензия → заголовок» задано в scripts/check-licenses/licenses.mjs.',
      'Выбор из дизъюнкции (MIT OR Apache-2.0) сделан правилом, а не вручную: см. spdx.mjs.',
    ],
    rust: await measureRust(),
    npm: await measureNpm(),
  }
}

/**
 * @param {string[]} argv
 * @returns {Promise<number>} код возврата
 */
export async function main(argv) {
  const { write } = parseArgs(argv)
  const measured = await measure()

  if (write) {
    await writeFile(SNAPSHOT_PATH, `${JSON.stringify(measured, null, 2)}\n`, 'utf8')
    const rustCount = packagesNamedBy(measured.rust).size
    const npmCount = packagesNamedBy(measured.npm).size
    console.log(`licenses.lock.json переписан: rust ${rustCount} пакетов, npm ${npmCount}.`)
    console.log(`лицензии поставки: ${Object.keys(measured.rust.shipped).join(', ')}`)
    return 0
  }

  const stored = JSON.parse(await readFile(SNAPSHOT_PATH, 'utf8'))
  const problems = []
  if (JSON.stringify(stored.rust) !== JSON.stringify(measured.rust)) {
    problems.push('раздел rust снимка разошёлся с измерением графа')
  }
  if (JSON.stringify(stored.npm) !== JSON.stringify(measured.npm)) {
    problems.push('раздел npm снимка разошёлся с измерением')
  }
  problems.push(
    ...checkSnapshot({
      snapshot: stored,
      cargoLock: await readFile(join(CARGO_DIR, 'Cargo.lock'), 'utf8'),
      npmLock: await readFile(join(REPO_ROOT, 'package-lock.json'), 'utf8'),
      doc: await readFile(join(REPO_ROOT, 'THIRD-PARTY-LICENSES.md'), 'utf8'),
    }),
  )

  if (problems.length > 0) {
    console.error('Лицензионный комплект разошёлся с поставкой:')
    for (const problem of problems) console.error(`  - ${problem}`)
    console.error('\nПересними снимок и заведи недостающие разделы: npm run check-licenses -- --write')
    return 1
  }

  console.log('Лицензионный комплект сошёлся: снимок = измеренный граф, разделы на месте.')
  return 0
}

const invokedDirectly = process.argv[1] !== undefined && import.meta.url.endsWith(process.argv[1].split('/').pop())
if (invokedDirectly) {
  main(process.argv.slice(2))
    .then((code) => {
      process.exitCode = code
    })
    .catch((error) => {
      console.error(error)
      process.exitCode = 1
    })
}
