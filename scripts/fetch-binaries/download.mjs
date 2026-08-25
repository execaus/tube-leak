import { createHash } from 'node:crypto'
import { createReadStream, createWriteStream } from 'node:fs'
import { mkdir, rm } from 'node:fs/promises'
import { dirname } from 'node:path'
import { Readable } from 'node:stream'
import { pipeline } from 'node:stream/promises'

/**
 * Скачивает `url` в `destPath`, создавая родительские каталоги по мере
 * необходимости. Не глотает ошибки сети/HTTP: бросает с контекстом.
 * Если запись не удалась на любом этапе, частично записанный файл
 * удаляется, прежде чем ошибка всплывёт наружу.
 *
 * @param {string} url
 * @param {string} destPath
 * @param {{ fetchImpl?: typeof fetch }} [options] `fetchImpl` — точка
 *   подмены для тестов, чтобы не ходить в сеть.
 */
export async function downloadToFile(url, destPath, { fetchImpl = fetch } = {}) {
  await mkdir(dirname(destPath), { recursive: true })

  let response
  try {
    response = await fetchImpl(url)
  } catch (err) {
    throw new Error(`downloading ${url}: network request failed: ${err.message}`, { cause: err })
  }

  if (!response.ok) {
    throw new Error(`downloading ${url}: unexpected HTTP status ${response.status}`)
  }
  if (!response.body) {
    throw new Error(`downloading ${url}: response has no body`)
  }

  try {
    await pipeline(Readable.fromWeb(response.body), createWriteStream(destPath))
  } catch (err) {
    await rm(destPath, { force: true })
    throw new Error(`downloading ${url}: writing to ${destPath} failed: ${err.message}`, {
      cause: err,
    })
  }
}

/**
 * Считает SHA-256 файла в шестнадцатеричном виде (в нижнем регистре).
 *
 * @param {string} filePath
 * @returns {Promise<string>}
 */
export function sha256File(filePath) {
  return new Promise((resolve, reject) => {
    const hash = createHash('sha256')
    const stream = createReadStream(filePath)
    stream.on('error', (err) =>
      reject(new Error(`hashing ${filePath}: ${err.message}`, { cause: err })),
    )
    stream.on('data', (chunk) => hash.update(chunk))
    stream.on('end', () => resolve(hash.digest('hex')))
  })
}
