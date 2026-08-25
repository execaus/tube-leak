import { chmod, rename, rm } from 'node:fs/promises'
import { join } from 'node:path'

import { extractMember } from './archive.mjs'
import { downloadToFile, sha256File } from './download.mjs'
import { entryKind } from './pin.mjs'

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
 * `entry.kind` (см. pin.mjs) различает два вида итогового файла:
 * - `binary` (умолчание) — исполняемый файл, который резолвит Tauri из
 *   `externalBin`; на Unix ему ставится бит выполнения;
 * - `archive` — архив, который кладётся как есть и распаковывается уже
 *   приложением на рантайме (yt-dlp после TL-12, см. `src-tauri/src/ytdlp`);
 *   бит выполнения ему не ставится — исполнять предстоит не его, а файлы
 *   внутри распакованного дерева.
 *
 * Не путать `entry.kind` с `entry.archive`: второе — указание извлечь ОДИН
 * файл ИЗ скачанного архива (так поставляется ffmpeg). Комбинация
 * `kind: 'archive'` + `archive: {...}` осмысленна и допустима (архив внутри
 * архива), но в текущем пине не встречается.
 *
 * @param {object} entry запись из пина: `{ url, sha256, binaryName, kind?, archive? }`
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
    await chmod(finalPath, entryKind(entry) === 'archive' ? 0o644 : 0o755)
  }

  return finalPath
}
