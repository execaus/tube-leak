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
 * Страницы issues нашего репозитория — и только они. Намеренно НЕ
 * `startsWith('/execaus/tube-leak')`: такое правило накрыло бы и
 * `tube-leak-docs`, и будущий `/execaus/tube-leak/releases/download/…`,
 * а в день, когда мы заведём собственное зеркало ассета ffmpeg
 * (исследование §2.4), 404 по пин-адресу молча перестал бы считаться
 * пропажей — то есть главный охраняемый адрес перестал бы охраняться
 * (замечание Н1). Для зеркала ассета 404 обязан остаться смертью.
 */
const OWN_ISSUES_PATH = /^\/execaus\/tube-leak\/issues(\/\d+)?\/?$/

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
 * У проверяемого адреса может быть оговорка `notFoundMeans`: причина, по
 * которой именно 404 по этому адресу не доказывает отсутствия ресурса.
 * Её применяет `checkAll` — и только к 404, и только к ней.
 *
 * @param {string} url
 * @returns {{ kind: 'check'; probeUrl: string; why: string | null; notFoundMeans?: string } | { kind: 'skip'; reason: string }}
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

  // Наш собственный репозиторий ПРОВЕРЯЕТСЯ, как все прочие: запрос
  // уходит, повторы и канарейка работают. Особый здесь только разбор
  // 404 — пока репозиторий приватный, GitHub отвечает 404 анонимно и на
  // существующий issue (измерено 2026-09-27 тем же клиентом, которым
  // ходит сторож: и `issues`, и `issues/14` — HTTP 404), и снаружи «нет
  // доступа» от «нет ресурса» не отличить. Поэтому 404 здесь читается
  // как «не подтверждён», а не как «мёртв»: у адреса документа это
  // терпимо и гейт не краснеет, у адреса пина — по-прежнему отказ (в
  // пине таких адресов нет, проверено).
  //
  // Прежде здесь стоял безусловный пропуск, а рядом — обещание «станет
  // публичным, пропуск исчезнет сам». Обещание было неверным: пропуск по
  // хосту и пути не исчез бы никогда, а адрес не проверялся вовсе.
  // Теперь после открытия репозитория придёт 200, и эта оговорка просто
  // перестанет срабатывать, ничего не пряча.
  if (parsed.hostname === 'github.com' && OWN_ISSUES_PATH.test(parsed.pathname)) {
    return {
      kind: 'check',
      probeUrl: url,
      why: null,
      notFoundMeans:
        'наш репозиторий пока приватный, и анонимно GitHub отвечает 404 даже на существующий ' +
        'issue — отсутствия ресурса этот 404 не доказывает (существование issue #14 ' +
        'подтверждено авторизованным `gh api`)',
    }
  }

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
