import { chmod, rename, rm } from 'node:fs/promises'
import { join } from 'node:path'

import { extractMember } from './archive.mjs'
import { downloadToFile, sha256File } from './download.mjs'

/**
 * Скачивает, проверяет по SHA-256 и раскладывает один sidecar-бинарник по
 * записи из файла пина (см. pin.mjs).
 *
 * Гарантии:
 * - при несовпадении SHA-256 временный файл удаляется, финальный путь
 *   (`entry.binaryName` в `outDir`) не создаётся и не затрагивается;
 * - финальный файл появляется только через `rename` после успешной
 *   проверки/распаковки — на диске никогда не видно частично записанного
 *   результата под именем sidecar-бинарника.
 *
 * @param {object} entry запись из пина: `{ url, sha256, binaryName, archive? }`
 * @param {string} outDir каталог назначения (`src-tauri/binaries`)
 * @param {{ fetchImpl?: typeof fetch }} [deps] точки подмены для тестов
 * @returns {Promise<string>} абсолютный путь к готовому бинарнику
 */
export async function installBinary(entry, outDir, deps = {}) {
  const downloadPath = join(outDir, `.${entry.binaryName}.download`)
  const finalPath = join(outDir, entry.binaryName)

  await downloadToFile(entry.url, downloadPath, deps)

  const actualSha256 = await sha256File(downloadPath)
  if (actualSha256 !== entry.sha256) {
    await rm(downloadPath, { force: true })
    throw new Error(
      `sha256 mismatch for ${entry.url}: expected ${entry.sha256}, got ${actualSha256} — aborting, no file left in place`,
    )
  }

  if (entry.archive) {
    const extractingPath = join(outDir, `.${entry.binaryName}.extracting`)
    try {
      await extractMember(entry.archive.type, downloadPath, entry.archive.member, extractingPath)
    } catch (err) {
      await rm(extractingPath, { force: true })
      throw err
    } finally {
      await rm(downloadPath, { force: true })
    }
    await rename(extractingPath, finalPath)
  } else {
    await rename(downloadPath, finalPath)
  }

  if (process.platform !== 'win32') {
    await chmod(finalPath, 0o755)
  }

  return finalPath
}
