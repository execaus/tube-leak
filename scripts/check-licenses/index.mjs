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
  NOTICES_DOC,
  packagesNamedBy,
  parseCargoLock,
  parseNpmLock,
  REPO_ROOT,
  SNAPSHOT_PATH,
  TARGETS,
} from './licenses.mjs'
import { crateLicenseTexts, noticeFrom, npmLicenseTexts, renderNotices } from './notices.mjs'

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
 * Места, где в этом окружении лежит cargo, когда его нет в PATH.
 *
 * Зачем список, а не требование «экспортируй PATH сам». Измерение графа
 * вызывается из `npm test` (иначе оно не вызывается никогда — см. Н4), а
 * cargo в этом окружении вне PATH по умолчанию: так записано в CLAUDE.md
 * проекта. Без поиска обычный `npm test` краснел бы не от дефекта, а от
 * переменной окружения — ровно тот ложный красный, который в этом
 * проекте неотличим от настоящей поломки и потому запрещён.
 *
 * Пропускать измерение при отсутствии cargo нельзя: пропуск вернул бы
 * слепое пятно, ради закрытия которого измерение сюда и переехало.
 * Поэтому не «тише», а «надёжнее»: сначала ищем, и только не найдя —
 * отказываем с названной причиной.
 */
const CARGO_FALLBACK_DIRS = Object.freeze([
  '/opt/homebrew/opt/rustup/bin',
  join(process.env.HOME ?? '', '.cargo', 'bin'),
])

/**
 * Окружение с cargo в PATH, если он там ещё не оказался.
 *
 * @param {NodeJS.ProcessEnv} [env]
 * @returns {NodeJS.ProcessEnv}
 */
export function cargoEnv(env = process.env) {
  const parts = (env.PATH ?? '').split(':')
  const missing = CARGO_FALLBACK_DIRS.filter((dir) => dir !== '' && !parts.includes(dir))
  if (missing.length === 0) return env
  return { ...env, PATH: [...parts, ...missing].filter((part) => part !== '').join(':') }
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
    if (error?.code !== 'ENOENT') throw error
    try {
      return await run('cargo', args, { ...options, env: cargoEnv() })
    } catch (retried) {
      throw cargoFailure(retried)
    }
  }
}

/**
 * @param {string[]} argv
 * @returns {{ write: boolean }}
 */
export function parseArgs(argv) {
  return { write: argv.includes('--write') }
}

/** @type {Map<string, object> | null} */
let metadataCache = null

/**
 * Метаданные пакетов: лицензия, авторы, репозиторий и признак
 * proc-macro. Авторы нужны для NOTICES.md — у части крейтов файла
 * лицензии нет вовсе, и уведомление берётся отсюда.
 *
 * @returns {Promise<Map<string, { license: string; procMacro: boolean; authors: string[]; repository: string | null }>>}
 */
async function readMetadata() {
  if (metadataCache !== null) return metadataCache
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
      authors: pkg.authors ?? [],
      repository: pkg.repository ?? null,
    })
  }
  metadataCache = byPackage
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

  // Адреса исходников по §3.2 MPL. Только MPL: у пермиссивных лицензий
  // обязательства предоставить исходник нет, и раздувать проверяемый
  // список нечем. Версия в адресе — та самая, что влинкована.
  const sourceUrls = (shipped['MPL-2.0'] ?? []).map((entry) => {
    const [name, version] = entry.split(' ')
    return { package: entry, url: `https://crates.io/crates/${name}/${version}` }
  })

  return {
    sourceUrls,
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
 * Уведомления об авторских правах для всего, что едет в поставку.
 *
 * Читает файлы лицензий из локального кэша cargo и из node_modules;
 * где файла нет — берёт авторов из метаданных, и запись об этом
 * говорит прямо (см. notices.mjs).
 *
 * @param {object} snapshot
 * @returns {Promise<{ rust: object[]; npm: object[] }>}
 */
async function collectNoticesData(snapshot) {
  const metadata = await readMetadata()

  const rust = []
  for (const entry of [...new Set(Object.values(snapshot.rust.shipped).flat())].sort()) {
    const [name, version] = entry.split(' ')
    const info = metadata.get(entry) ?? { license: '', authors: [], repository: null }
    const licenseTexts = crateLicenseTexts(name, version)
    // `null` — крейта нет в кэше cargo. Промах кэша НЕ ДОЛЖЕН молча
    // превращаться в «у пакета нет файла лицензии»: при пустом кэше эту
    // пометку получили бы все 295 крейтов, и `--write` записал бы её
    // получателю как установленный факт.
    if (licenseTexts === null) {
      throw new Error(
        `${entry}: исходников нет ни в registry/src, ни в registry/cache — уведомление собрать не из чего. ` +
          'Это промах кэша, а не отсутствие файла лицензии у пакета, и записывать его в NOTICES.md как ' +
          'факт нельзя. Наполни кэш: cd src-tauri && cargo fetch --locked',
      )
    }
    rust.push({
      entry,
      license: info.license === '' ? 'лицензия не указана в метаданных' : info.license,
      notice: noticeFrom({ licenseTexts, authors: info.authors }),
      repository: info.repository,
    })
  }

  const npm = []
  for (const entry of [...new Set(Object.values(snapshot.npm.shipped).flat())].sort()) {
    const name = entry.slice(0, entry.lastIndexOf(' '))
    const dir = join(REPO_ROOT, 'node_modules', name)
    const manifest = JSON.parse(await readFile(join(dir, 'package.json'), 'utf8'))
    const authors = [manifest.author, ...(manifest.contributors ?? [])]
      .filter((author) => author !== undefined && author !== null)
      .map((author) =>
        typeof author === 'string'
          ? author
          : `${author.name ?? ''}${author.email === undefined ? '' : ` <${author.email}>`}`.trim(),
      )
    const licenseTexts = npmLicenseTexts(dir)
    if (licenseTexts === null) {
      throw new Error(
        `${entry}: каталога ${dir} нет — уведомление собрать не из чего. Это отсутствие установки, ` +
          'а не отсутствие файла лицензии у пакета. Выполни npm ci',
      )
    }
    npm.push({
      entry,
      license: manifest.license ?? 'лицензия не указана в манифесте',
      notice: noticeFrom({ licenseTexts, authors }),
      repository:
        typeof manifest.repository === 'string' ? manifest.repository : (manifest.repository?.url ?? null),
    })
  }

  return { rust, npm }
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
    const notices = renderNotices(await collectNoticesData(measured))
    await writeFile(join(REPO_ROOT, NOTICES_DOC), notices, 'utf8')

    const rustCount = packagesNamedBy(measured.rust).size
    const npmCount = packagesNamedBy(measured.npm).size
    console.log(`licenses.lock.json переписан: rust ${rustCount} пакетов, npm ${npmCount}.`)
    console.log(`лицензии поставки: ${Object.keys(measured.rust.shipped).join(', ')}`)
    console.log(`${NOTICES_DOC} пересобран: ${notices.split('\n### ').length - 1} записей.`)
    return 0
  }

  const stored = JSON.parse(await readFile(SNAPSHOT_PATH, 'utf8'))
  const storedNotices = await readFile(join(REPO_ROOT, NOTICES_DOC), 'utf8')
  const problems = []
  if (JSON.stringify(stored.rust) !== JSON.stringify(measured.rust)) {
    problems.push('раздел rust снимка разошёлся с измерением графа')
  }
  if (JSON.stringify(stored.npm) !== JSON.stringify(measured.npm)) {
    problems.push('раздел npm снимка разошёлся с измерением')
  }
  if (storedNotices !== renderNotices(await collectNoticesData(stored))) {
    problems.push(
      `${NOTICES_DOC} разошёлся с тем, что собирается из пакетов (правка руками либо устаревший файл)`,
    )
  }
  problems.push(
    ...checkSnapshot({
      snapshot: stored,
      cargoLock: await readFile(join(CARGO_DIR, 'Cargo.lock'), 'utf8'),
      npmLock: await readFile(join(REPO_ROOT, 'package-lock.json'), 'utf8'),
      doc: await readFile(join(REPO_ROOT, 'THIRD-PARTY-LICENSES.md'), 'utf8'),
      notices: storedNotices,
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
