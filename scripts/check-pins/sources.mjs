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
//   оба 404 — наш приватный репозиторий», и при этом ДВЕ из четырёх
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
 * Раздел пина, ссылки которого обязаны дословно присутствовать в
 * документах. Только ffmpeg: указатель §6d заведён под GPL-компонент, а
 * yt-dlp (Unlicense) и deno (MIT) в нём адресами не перечисляются.
 */
export const DOCUMENTED_SECTION = 'ffmpeg'

/**
 * Граница нашего текста в THIRD-PARTY-LICENSES.md. Ниже первого раздела
 * «Полный текст …» идут ДОСЛОВНЫЕ тексты чужих лицензий и скопированные
 * из них уведомления: адреса там — чужие обещания, которые мы не вправе
 * править. Их в файле сотни, и часть из них мертва десятилетиями
 * (университетские хосты 1990-х, trac Unicode). Сторож, красный от них,
 * — сторож, которого перестают читать; а править текст лицензии, чтобы
 * позеленеть, нельзя.
 */
const OUR_TEXT_END = /^#{1,4} Полный текст/m

// Хвостовая пунктуация, приклеивающаяся к URL в тексте markdown. Скобка
// закрывающая — отдельный случай: она часть синтаксиса `[текст](url)`.
const TRAILING_PUNCTUATION = /[).,;:!?»"'`]+$/

/**
 * Часть документа, за которую отвечаем мы. Для SOURCES-FFMPEG.md это
 * файл целиком (он наш от начала до конца), для THIRD-PARTY-LICENSES.md
 * — до первого дословного текста лицензии.
 *
 * Отсутствие маркера — ошибка, а не повод молча проверить всё: сторож,
 * тихо расширивший собственные границы, — это ровно тот класс дефекта,
 * из-за которого заведён #140.
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
  return text.slice(0, match.index)
}

/**
 * Причина не проверять адрес по HTTP — или null, если проверять нужно.
 * Пропуски ИМЕНОВАННЫЕ и печатаются в отчёте: молча выкинутый адрес
 * ничем не отличается от непроверенного.
 *
 * @param {string} url
 * @returns {string | null}
 */
export function skipReason(url) {
  let parsed
  try {
    parsed = new URL(url)
  } catch {
    return null
  }

  // Эндпоинты систем контроля версий. Они рабочие, но отвечают на
  // `git clone` / `svn checkout`, а не на HTTP-запрос страницы: измерено
  // 2026-09-26 — git.code.sf.net/p/soxr/code и bitbucket .git дают 404,
  // svn.xvid.org — 401, и это не признак пропажи исходников. Проверять
  // их HTTP-статусом — ошибка категории, а не строгость.
  if (parsed.pathname.endsWith('.git') || /^(git|svn)\./.test(parsed.hostname)) {
    return 'VCS-эндпоинт: отвечает на git clone / svn checkout, а не на HTTP'
  }

  // Наш собственный репозиторий: приватный, анонимно отдаёт 404 по
  // устройству. Оба документа это прямо оговаривают. Станет публичным —
  // пропуск исчезнет сам.
  if (parsed.hostname === 'github.com' && parsed.pathname.startsWith('/execaus/tube-leak')) {
    return 'наш репозиторий приватный: анонимно 404 по устройству (оговорено в самих документах)'
  }

  return null
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
      found.push({ url: entry.url, where: `binaries.lock.json ${section}.${target}` })
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
  return matches.map((raw) => ({ url: raw.replace(TRAILING_PUNCTUATION, ''), where }))
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
  /** @type {Map<string, string[]>} */
  const byUrl = new Map()
  for (const { url, where } of entries) {
    const places = byUrl.get(url)
    if (places) {
      if (!places.includes(where)) places.push(where)
    } else {
      byUrl.set(url, [where])
    }
  }
  return [...byUrl].map(([url, where]) => ({ url, where }))
}

/**
 * @param {{ pinPath?: string; repoRoot?: string }} [options]
 * @returns {Promise<Array<{ url: string; where: string[] }>>}
 */
export async function collectAllUrls({ pinPath = PIN_PATH, repoRoot = REPO_ROOT } = {}) {
  const pin = await loadPin(pinPath)
  return mergeByUrl([...collectPinUrls(pin), ...(await collectDocUrls(repoRoot))])
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
