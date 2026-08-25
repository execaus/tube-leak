import { readFile } from 'node:fs/promises'

import { KNOWN_TARGETS } from './targets.mjs'

const BINARY_NAMES = Object.freeze(['ytDlp', 'ffmpeg'])
/** Допустимые значения `kind` у записи пина; см. entryKind(). */
export const ENTRY_KINDS = Object.freeze(['binary', 'archive'])
const SHA256_HEX_RE = /^[0-9a-f]{64}$/

/**
 * Загружает и валидирует файл пина версий (по умолчанию
 * `src-tauri/binaries.lock.json`). Бросает подробную ошибку при любом
 * структурном несоответствии — скрипт не должен молча продолжать со
 * сломанным пином.
 *
 * @param {string} pinPath абсолютный путь к файлу пина
 * @returns {Promise<object>} провалидированный объект пина
 */
export async function loadPin(pinPath) {
  let raw
  try {
    raw = await readFile(pinPath, 'utf8')
  } catch (err) {
    throw new Error(`reading pin file ${pinPath}: ${err.message}`, { cause: err })
  }

  let pin
  try {
    pin = JSON.parse(raw)
  } catch (err) {
    throw new Error(`parsing pin file ${pinPath} as JSON: ${err.message}`, { cause: err })
  }

  validatePin(pin, pinPath)
  return pin
}

/**
 * @param {unknown} pin
 * @param {string} pinPath только для сообщений об ошибках
 */
function validatePin(pin, pinPath) {
  if (typeof pin !== 'object' || pin === null) {
    throw new Error(`pin file ${pinPath}: top-level value must be an object`)
  }

  for (const binaryName of BINARY_NAMES) {
    const section = pin[binaryName]
    if (typeof section !== 'object' || section === null) {
      throw new Error(`pin file ${pinPath}: missing "${binaryName}" section`)
    }
    if (typeof section.version !== 'string' || section.version.length === 0) {
      throw new Error(`pin file ${pinPath}: "${binaryName}.version" must be a non-empty string`)
    }
    if (typeof section.targets !== 'object' || section.targets === null) {
      throw new Error(`pin file ${pinPath}: "${binaryName}.targets" must be an object`)
    }

    for (const target of KNOWN_TARGETS) {
      const entry = section.targets[target]
      if (typeof entry !== 'object' || entry === null) {
        throw new Error(
          `pin file ${pinPath}: "${binaryName}.targets.${target}" is missing`,
        )
      }
      validateEntry(entry, `${binaryName}.targets.${target}`, pinPath)
    }
  }
}

/**
 * @param {unknown} entry
 * @param {string} label путь до записи в структуре пина, для сообщений
 * @param {string} pinPath
 */
function validateEntry(entry, label, pinPath) {
  if (typeof entry.url !== 'string' || entry.url.length === 0) {
    throw new Error(`pin file ${pinPath}: "${label}.url" must be a non-empty string`)
  }
  if (typeof entry.sha256 !== 'string' || !SHA256_HEX_RE.test(entry.sha256)) {
    throw new Error(
      `pin file ${pinPath}: "${label}.sha256" must be a 64-char lowercase hex string`,
    )
  }
  if (typeof entry.binaryName !== 'string' || entry.binaryName.length === 0) {
    throw new Error(`pin file ${pinPath}: "${label}.binaryName" must be a non-empty string`)
  }

  // `kind` описывает, ЧЕМ является итоговый файл в src-tauri/binaries/:
  // исполняемым файлом (externalBin Tauri) или архивом, который приложение
  // распаковывает само на рантайме (yt-dlp после TL-12). Отсутствие поля —
  // "binary": так записаны все прежние entries, и молчаливая смена смысла
  // существующего пина недопустима.
  if (entry.kind !== undefined && !ENTRY_KINDS.includes(entry.kind)) {
    throw new Error(
      `pin file ${pinPath}: "${label}.kind" must be one of ${ENTRY_KINDS.join(', ')} when present`,
    )
  }

  if (entry.archive !== undefined) {
    if (typeof entry.archive !== 'object' || entry.archive === null) {
      throw new Error(`pin file ${pinPath}: "${label}.archive" must be an object when present`)
    }
    const { type, member } = entry.archive
    if (type !== 'zip' && type !== 'tar.xz') {
      throw new Error(`pin file ${pinPath}: "${label}.archive.type" must be "zip" or "tar.xz"`)
    }
    if (typeof member !== 'string' || member.length === 0) {
      throw new Error(
        `pin file ${pinPath}: "${label}.archive.member" must be the basename of the file to extract`,
      )
    }
  }
}

/**
 * Возвращает `kind` записи пина с подстановкой умолчания.
 *
 * @param {{ kind?: string }} entry
 * @returns {'binary' | 'archive'}
 */
export function entryKind(entry) {
  return entry.kind ?? 'binary'
}
