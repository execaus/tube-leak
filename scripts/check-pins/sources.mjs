// Сбор адресов, которые обязаны отвечать: и ссылки пина
// (src-tauri/binaries.lock.json), и ссылки указателя исходников
// SOURCES-FFMPEG.md / THIRD-PARTY-LICENSES.md (TL-133, #140).
//
// Почему оба списка, а не один. #140 сломал ровно тот случай, который
// каждая из двух проверок по отдельности не видит:
//
// - сторож, читающий только пин, не заметит, что указатель §6d GPL
//   разошёлся с тем, что мы раздаём на самом деле;
// - сторож, читающий только SOURCES-FFMPEG.md, не заметил бы мёртвые
//   сборки: на 2026-09-26 проверка 44 ссылок этого файла давала «42/44,
//   оба 404 — наш приватный репозиторий» (тогда репозиторий кода ещё был
//   приватным; теперь он открыт, а 404 у пары адресов был именно этим), и при этом ДВЕ из четырёх
//   раздаваемых сборок ffmpeg были недоступны. Адреса сборок в таблице
//   были записаны ИМЕНАМИ ФАЙЛОВ, а не ссылками, поэтому в список 44 не
//   попадали вовсе — сторож был слеп на собственной границе.
//
// Отсюда две части, разнесённые по цене:
//
// 1. Офлайн-сверка (crossCheckDocs) — идёт в обычном `npm test`: каждый
//    URL ffmpeg из пина обязан встречаться ДОСЛОВНО в обоих документах.
//    Она и держит дыру закрытой: записать сборку именем файла снова
//    нельзя — тест покраснеет, а раз в документе стоит URL, его
//    подхватывает и сетевая часть.
// 2. Сетевая проверка (scripts/check-pins/index.mjs, `npm run check-pins`)
//    — отдельной командой перед выпуском: в тестах сети нет (правило
//    проекта), поэтому в `npm test` её быть не может.

import { readFile } from 'node:fs/promises'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { BINARY_NAMES, loadPin } from '../fetch-binaries/pin.mjs'
import { KNOWN_TARGETS } from '../fetch-binaries/targets.mjs'

const __dirname = dirname(fileURLToPath(import.meta.url))
export const REPO_ROOT = resolve(__dirname, '..', '..')
export const PIN_PATH = join(REPO_ROOT, 'src-tauri', 'binaries.lock.json')

/**
 * Документы, которые сопровождают раздаваемые бинарники по §6d GPL v3.
 * Оба называют сборки ffmpeg, и оба обязаны называть их АДРЕСАМИ.
 */
export const DOC_FILES = Object.freeze(['SOURCES-FFMPEG.md', 'THIRD-PARTY-LICENSES.md'])

/**
 * Документы, которые обязаны ехать в бандл рядом с приложением.
 *
 * Шире, чем DOC_FILES, и список отдельный намеренно: DOC_FILES — это
 * указатели §6d, и каждый из них обязан называть адреса сборок ffmpeg
 * (crossCheckDocs). У NOTICES.md такой обязанности нет — он про
 * уведомления об авторских правах, — и попади он в DOC_FILES, сверка
 * потребовала бы от него ffmpeg-адресов, которых там взяться неоткуда.
 */
export const BUNDLED_DOCS = Object.freeze([...DOC_FILES, 'NOTICES.md'])

/**
 * Снимок лицензионного состава: из него берутся адреса исходников
 * MPL-крейтов (TL-136, замечание Н6 ревью).
 */
export const LICENSE_SNAPSHOT_FILE = 'licenses.lock.json'

/**
 * Раздел пина, ссылки которого обязаны дословно присутствовать в
 * документах. Только ffmpeg: указатель §6d заведён под GPL-компонент, а
 * yt-dlp (Unlicense) и deno (MIT) в нём адресами не перечисляются.
 */
export const DOCUMENTED_SECTION = 'ffmpeg'

/**
 * Граница нашего текста в THIRD-PARTY-LICENSES.md. Ниже первого раздела
 * «Полный текст …» идут ДОСЛОВНЫЕ тексты чужих лицензий и скопированные
 * из них уведомления: адреса там — чужие обещания, которые мы не вправе
 * править. Их в файле сотни, и часть мертва десятилетиями
 * (университетские хосты 1990-х, trac Unicode). Сторож, красный от них,
 * — сторож, которого перестают читать; а править текст лицензии, чтобы
 * позеленеть, нельзя.
 */
const OUR_TEXT_END = /^#{1,4} Полный текст/m

/**
 * Отпечатки дословных текстов чужих лицензий. Нужны потому, что
 * проверять НАЛИЧИЕ маркера недостаточно: маркеров в файле три, и если
 * пропадает ПЕРВЫЙ, граница молча съезжает на следующий — измерено
 * 2026-09-26, наша часть выросла с 6 290 до 42 500 знаков (14 → 20
 * адресов), втянув текст GPL v3. Ни одного из отпечатков в нашем тексте
 * нет, а в съехавшей границе они появляются — это и ловится.
 *
 * Прежний комментарий здесь обещал «отсутствие маркера — ошибка» и был
 * неверен для частичного удаления: сторож лгал о собственной границе
 * (замечание Б2 ревью TL-133).
 */
const VERBATIM_LICENSE_FINGERPRINTS = Object.freeze([
  'TERMS AND CONDITIONS',
  'Preamble',
  'THE SOFTWARE IS PROVIDED',
  'WITHOUT WARRANTY OF ANY KIND',
])

// Хвостовая пунктуация, приклеивающаяся к URL в тексте markdown. Скобка
// закрывающая — отдельный случай: она часть синтаксиса `[текст](url)`.
const TRAILING_PUNCTUATION = /[).,;:!?»"'`]+$/

/**
 * Путь канарейки. Достаточно одного сегмента: именно такой формой
 * измерено, что code.videolan.org отвечает 200 на несуществующее.
 */
const CANARY_PATH = '/tube-leak-check-pins-canary-does-not-exist-9f3a2b7c'

/**
 * Репозиторий документов `execaus/tube-leak-docs` — и только он. Владелец
 * открывает репозиторий КОДА (`execaus/tube-leak`), а документы остаются
 * закрытыми, поэтому анонимный 404 у них — устройство, а не пропажа, и
 * подтвердить существование снаружи нечем.
 *
 * Сравнение по СЕГМЕНТАМ пути, а не префиксом, и это не педантизм:
 * `tube-leak-docs` начинается с `tube-leak`, поэтому
 * `startsWith('/execaus/tube-leak')` накрыл бы оба репозитория разом —
 * то есть снова спрятал бы адреса открытого репозитория кода, включая
 * будущее зеркало ассета ffmpeg (замечание Н1). Для всего, что лежит в
 * репозитории кода, 404 обязан остаться смертью.
 *
 * @param {URL} parsed
 * @returns {boolean}
 */
function isOwnDocsRepo(parsed) {
  if (parsed.hostname !== 'github.com') return false
  const [owner, repo] = parsed.pathname.split('/').filter(Boolean)
  return owner === 'execaus' && repo === 'tube-leak-docs'
}

/**
 * Часть документа, за которую отвечаем мы. Для SOURCES-FFMPEG.md это
 * файл целиком (он наш от начала до конца), для THIRD-PARTY-LICENSES.md
 * — до первого дословного текста лицензии.
 *
 * Отказывает в двух случаях, а не в одном:
 * - маркера нет вовсе — граница неизвестна;
 * - граница съехала, и в нашу часть попал дословный текст чужой лицензии
 *   (ловится отпечатками). Именно этот случай прежняя версия пропускала.
 *
 * @param {string} name имя файла
 * @param {string} text содержимое
 * @returns {string}
 */
export function ourTextOf(name, text) {
  if (name !== 'THIRD-PARTY-LICENSES.md') return text

  const match = OUR_TEXT_END.exec(text)
  if (!match) {
    throw new Error(
      `${name}: не найден раздел «Полный текст …», по которому проходит граница нашего текста. ` +
        'Пока граница не восстановлена, проверять файл нельзя: ниже неё лежат дословные ' +
        'тексты чужих лицензий, адреса в которых нам не принадлежат.',
    )
  }

  const ours = text.slice(0, match.index)
  const leaked = VERBATIM_LICENSE_FINGERPRINTS.filter((fingerprint) => ours.includes(fingerprint))
  if (leaked.length > 0) {
    throw new Error(
      `${name}: граница нашего текста съехала — в неё попал дословный текст чужой лицензии ` +
        `(отпечатки: ${leaked.join(', ')}). Скорее всего, пропал один из заголовков ` +
        '«Полный текст …»: тогда граница уезжает на следующий, и сторож начинает проверять ' +
        'чужие адреса, которые мы не вправе править.',
    )
  }
  return ours
}

/**
 * Что делать с адресом: проверять как есть, проверять замену или
 * пропустить с названной причиной.
 *
 * Правило сужено по замечанию Б1 ревью. Прежнее
 * (`endsWith('.git') || /^(git|svn)\./`) было обосновано измерением лишь
 * для трёх хостов, а применялось ко всем: пропускался **71 адрес из
 * 129**, включая строки Linux-таблицы, которыми закрыт #136, и заведомо
 * мёртвый `github.com/…/this-repo-does-not-exist-xyz123.git` проходил
 * как SKIP. Измерено 2026-09-26 — отвечают **200**:
 * `github.com/google/snappy.git`, `gitlab.com/AOMediaCodec/SVT-AV1.git`,
 * `code.videolan.org/videolan/x264.git`,
 * `git.savannah.gnu.org/git/libiconv.git`,
 * `svn.code.sf.net/p/lame/svn/trunk/lame`. Все они теперь проверяются.
 *
 * @param {string} url
 * @returns {{ kind: 'check'; probeUrl: string; why: string | null } | { kind: 'skip'; reason: string }}
 */
export function planProbe(url) {
  let parsed
  try {
    parsed = new URL(url)
  } catch {
    return { kind: 'check', probeUrl: url, why: null }
  }

  // SourceForge: git-эндпоинт отдаёт 404 на HTTP (измерено для soxr и
  // opencore-amr), но у того же репозитория есть страница, которая
  // отвечает 200. Пропускать незачем — проверяем замену.
  if (parsed.hostname === 'git.code.sf.net') {
    const project = /^\/p\/([^/]+)\/code\/?$/.exec(parsed.pathname)
    if (project) {
      return {
        kind: 'check',
        probeUrl: `https://sourceforge.net/p/${project[1]}/code/`,
        why: 'git-эндпоинт SourceForge отвечает на HTTP 404; проверяется страница того же репозитория',
      }
    }
    return {
      kind: 'skip',
      reason: 'git-эндпоинт SourceForge неизвестной формы: HTTP-статус о наличии исходников не говорит',
    }
  }

  // Bitbucket: 404 анонимно и на `.git`, и на страницу репозитория
  // (измерено 2026-09-26 на x265_git) — заменить нечем.
  if (parsed.hostname === 'bitbucket.org' && parsed.pathname.endsWith('.git')) {
    return {
      kind: 'skip',
      reason:
        'Bitbucket анонимно отвечает 404 и на .git-эндпоинт, и на страницу репозитория (измерено); ' +
        'адрес рабочий для git clone, HTTP-статус о наличии исходников не говорит',
    }
  }

  // SVN-сервер xvid требует анонимный логин: сборщик ходит
  // `svn checkout --username anonymous`, а обычный HTTP-запрос — 401.
  if (parsed.hostname === 'svn.xvid.org') {
    return {
      kind: 'skip',
      reason: 'SVN-сервер xvid требует анонимный логин: HTTP-запрос отвечает 401 (измерено)',
    }
  }

  // Репозиторий документов приватный и остаётся таким (решение владельца:
  // открывается только репозиторий кода). Анонимно он отвечает 404 по
  // устройству — измерено 2026-09-27 тем же клиентом, которым ходит
  // сторож, — и отличить «нет доступа» от «нет ресурса» снаружи нельзя,
  // поэтому адрес пропускается с названной причиной.
  //
  // Правило стоит на будущее: URL на tube-leak-docs в документах §6d
  // сегодня нет ни одного (проверено), и указатель §6d ссылаться на него
  // не должен — по такой ссылке получатель установщика не пройдёт.
  if (isOwnDocsRepo(parsed)) {
    return {
      kind: 'skip',
      reason:
        'репозиторий документов execaus/tube-leak-docs приватный и остаётся таким: анонимно 404 ' +
        'по устройству (владелец открывает только репозиторий кода). Указатель §6d ссылаться ' +
        'на него не должен — получатель установщика по такой ссылке не пройдёт',
    }
  }

  // Адреса самого репозитория КОДА никакого правила не получают: он
  // открыт, и 404 по любому его адресу — настоящая пропажа. Прежде здесь
  // стоял безусловный пропуск с обещанием «станет публичным — пропуск
  // исчезнет сам»; обещание было неверным (пропуск по хосту и пути не
  // исчез бы никогда), а заменившая его оговорка про 404 после открытия
  // репозитория стала бы вечным укрытием для удалённого issue.

  return { kind: 'check', probeUrl: url, why: null }
}

/**
 * Адрес назван пином? У таких адресов нет права на «не подтверждён»:
 * пин — это то, что мы СКАЧИВАЕМ на сборке, и неподтверждённый адрес
 * там означает несобираемый установщик (#140). Решение ведущего по
 * замечанию Б4 ревью: любой неподтверждённый адрес пина валит гейт, а
 * для адресов документов класс «не подтверждён» допустим.
 *
 * @param {{ origins: string[] }} entry
 * @returns {boolean}
 */
export function isPinAddress(entry) {
  return entry.origins.includes('pin')
}

/**
 * Путь-канарейка: заведомо несуществующий адрес на том же хосте.
 *
 * Зачем (замечание Б5 ревью). Измерено: code.videolan.org отвечает 200 и
 * на `no-such-project-xyz123`, и на `no-such-xyz123.git` — то есть «200»
 * от такого хоста не значит, что ресурс существует, и восемь адресов
 * стояли зелёными независимо от их наличия. Список таких хостов вести
 * нельзя — он устареет молча; поэтому спрашиваем каждый хост сами.
 *
 * @param {string} url
 * @returns {string}
 */
export function canaryUrlFor(url) {
  const { protocol, host } = new URL(url)
  return `${protocol}//${host}${CANARY_PATH}`
}

/**
 * Адреса из пина: по одному на каждую пару (sidecar, тройка).
 *
 * @param {object} pin результат loadPin
 * @returns {Array<{ url: string; where: string }>}
 */
export function collectPinUrls(pin) {
  const found = []
  for (const section of BINARY_NAMES) {
    for (const target of KNOWN_TARGETS) {
      const entry = pin[section].targets[target]
      found.push({
        url: entry.url,
        where: `binaries.lock.json ${section}.${target}`,
        origin: 'pin',
      })
    }
  }
  return found
}

/**
 * Адреса из текста markdown. Ловит и голые ссылки, и ссылки в скобках
 * `[текст](url)`, и адреса внутри `обратных кавычек`.
 *
 * @param {string} text
 * @param {string} where имя файла для сообщений
 * @returns {Array<{ url: string; where: string }>}
 */
export function extractMarkdownUrls(text, where) {
  const matches = text.match(/https?:\/\/[^\s<>()[\]"'`|]+/g) ?? []
  return matches.map((raw) => ({
    url: raw.replace(TRAILING_PUNCTUATION, ''),
    where,
    origin: 'docs',
  }))
}

/**
 * @param {string} [repoRoot]
 * @returns {Promise<Array<{ url: string; where: string }>>}
 */
export async function collectDocUrls(repoRoot = REPO_ROOT) {
  const found = []
  for (const name of DOC_FILES) {
    const text = await readFile(join(repoRoot, name), 'utf8')
    found.push(...extractMarkdownUrls(ourTextOf(name, text), name))
  }
  return found
}

/**
 * Сводит адреса в список без повторов, сохраняя все места, где встретился
 * каждый. Повторы — норма, а не аномалия: один ассет yt-dlp_macos.zip
 * стоит сразу у двух macOS-троек, и дважды дёргать сеть за него незачем.
 *
 * @param {Array<{ url: string; where: string }>} entries
 * @returns {Array<{ url: string; where: string[] }>}
 */
export function mergeByUrl(entries) {
  /** @type {Map<string, { where: string[]; origins: Set<string> }>} */
  const byUrl = new Map()
  for (const { url, where, origin } of entries) {
    const found = byUrl.get(url)
    if (found) {
      if (!found.where.includes(where)) found.where.push(where)
      found.origins.add(origin)
    } else {
      byUrl.set(url, { where: [where], origins: new Set([origin]) })
    }
  }
  return [...byUrl].map(([url, { where, origins }]) => ({
    url,
    where,
    origins: [...origins].sort(),
  }))
}

/**
 * @param {{ pinPath?: string; repoRoot?: string }} [options]
 * @returns {Promise<Array<{ url: string; where: string[] }>>}
 */
export async function collectAllUrls({ pinPath = PIN_PATH, repoRoot = REPO_ROOT } = {}) {
  const pin = await loadPin(pinPath)
  return mergeByUrl([
    ...collectPinUrls(pin),
    ...(await collectDocUrls(repoRoot)),
    ...(await collectLicenseSourceUrls(repoRoot)),
  ])
}

/**
 * Адреса исходников MPL-крейтов из снимка лицензий.
 *
 * Зачем отдельный источник, а не текст документа (замечание Н6 ревью).
 * Эти четыре адреса стоят в THIRD-PARTY-LICENSES.md НИЖЕ границы
 * «нашего текста» (первого заголовка «Полный текст …»), потому что
 * соседствуют с дословными текстами чужих лицензий. Граница законна и
 * ломать её нельзя — но эти адреса не чужое обещание, а НАШЕ
 * обязательство по §3.2 MPL: по ним получатель забирает исходный код
 * покрытых файлов. Обещание, которое никто никогда не проверяет, —
 * ровно тот класс дыры, что уже дважды ловился в этом стороже.
 *
 * Поэтому адреса берутся из машиночитаемого снимка, а не вычитываются
 * из markdown: тогда проверка не зависит от того, по какую сторону
 * границы они оказались в тексте. Что текст и снимок не разошлись,
 * проверяет офлайн-сторож лицензий отдельно.
 *
 * @param {string} [repoRoot]
 * @returns {Promise<Array<{ url: string; where: string; origin: string }>>}
 */
export async function collectLicenseSourceUrls(repoRoot = REPO_ROOT) {
  let snapshot
  try {
    snapshot = JSON.parse(await readFile(join(repoRoot, LICENSE_SNAPSHOT_FILE), 'utf8'))
  } catch {
    return []
  }
  return (snapshot.rust?.sourceUrls ?? []).map(({ package: pkg, url }) => ({
    url,
    where: `${LICENSE_SNAPSHOT_FILE} (исходники ${pkg} по §3.2 MPL)`,
    origin: 'licenses',
  }))
}

/**
 * Офлайн-часть сторожа: каждый адрес раздаваемой сборки ffmpeg из пина
 * обязан встречаться дословно в каждом документе §6d. Возвращает список
 * расхождений (пустой — всё сошлось).
 *
 * @param {object} pin результат loadPin
 * @param {Record<string, string>} docs содержимое документов по имени
 * @returns {string[]} человекочитаемые расхождения
 */
export function crossCheckDocs(pin, docs) {
  const problems = []
  for (const target of KNOWN_TARGETS) {
    const { url } = pin[DOCUMENTED_SECTION].targets[target]
    for (const name of DOC_FILES) {
      const text = docs[name]
      if (text === undefined) {
        problems.push(`${name}: документ не прочитан`)
        continue
      }
      if (!text.includes(url)) {
        problems.push(
          `${name}: нет адреса сборки ${DOCUMENTED_SECTION} для ${target} (${url}). ` +
            'Записывать сборку именем файла нельзя: так дыра #140 и осталась невидимой — ' +
            'проверка ссылок документа имён файлов не видит.',
        )
      }
    }
  }
  return problems
}

/**
 * @param {string} [repoRoot]
 * @returns {Promise<Record<string, string>>}
 */
export async function readDocs(repoRoot = REPO_ROOT) {
  /** @type {Record<string, string>} */
  const docs = {}
  for (const name of DOC_FILES) {
    docs[name] = await readFile(join(repoRoot, name), 'utf8')
  }
  return docs
}
