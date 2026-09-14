import { chmod, rename, rm } from 'node:fs/promises'
import { join } from 'node:path'

import { extractMember } from './archive.mjs'
import { verifyExecutableArch } from './arch.mjs'
import { downloadToFile, sha256File } from './download.mjs'
import { deliveredArchiveType, entryKind } from './pin.mjs'

/**
 * Скачивает, проверяет по SHA-256 и по архитектуре и раскладывает один
 * sidecar-бинарник по записи из файла пина (см. pin.mjs).
 *
 * Гарантии:
 * - при несовпадении SHA-256 или архитектуры временные файлы удаляются,
 *   финальный путь (`entry.binaryName` в `outDir`) не создаётся и не
 *   затрагивается;
 * - финальный файл появляется только через `rename` после успешной
 *   проверки/распаковки — на диске никогда не видно частично записанного
 *   или непроверенного результата под именем sidecar-бинарника.
 *
 * `entry.kind` (см. pin.mjs) различает два вида итогового файла:
 * - `binary` (умолчание) — исполняемый файл, который резолвит Tauri из
 *   `externalBin`; на Unix ему ставится бит выполнения;
 * - `archive` — архив, который кладётся как есть и распаковывается уже
 *   приложением на рантайме (yt-dlp после TL-12, см. `src-tauri/src/ytdlp`);
 *   бит выполнения ему не ставится — исполнять предстоит не его, а файлы
 *   внутри распакованного дерева.
 *
 * Архитектура (TL-108, долг #15) сверяется с `target` по заголовку
 * исполняемого файла (см. arch.mjs): у `binary` — самого итогового файла,
 * у `archive` — члена `entry.executableMember`, который для этого
 * извлекается во временный файл и удаляется.
 *
 * Не путать `entry.kind` с `entry.archive`: второе — указание извлечь ОДИН
 * файл ИЗ скачанного архива (так поставляются ffmpeg и deno). Комбинация
 * `kind: 'archive'` + `archive: {...}` осмысленна и допустима (архив внутри
 * архива), но в текущем пине не встречается.
 *
 * @param {object} entry запись из пина: `{ url, sha256, binaryName, kind?, archive?, executableMember? }`
 * @param {string} outDir каталог назначения (`src-tauri/binaries`)
 * @param {string} target целевая тройка записи — с ней сверяется архитектура
 * @param {{ fetchImpl?: typeof fetch }} [deps] точки подмены для тестов
 * @returns {Promise<string>} абсолютный путь к готовому бинарнику
 */
export async function installBinary(entry, outDir, target, deps = {}) {
  if (typeof target !== 'string' || target.length === 0) {
    // Без тройки проверять архитектуру не с чем, а молча пропустить
    // проверку — значит вернуть долг #15.
    throw new Error(`installBinary(${entry.binaryName}): target triple is required for the architecture check`)
  }

  const downloadPath = join(outDir, `.${entry.binaryName}.download`)
  const extractingPath = join(outDir, `.${entry.binaryName}.extracting`)
  const archCheckPath = join(outDir, `.${entry.binaryName}.archcheck`)
  const finalPath = join(outDir, entry.binaryName)

  await downloadToFile(entry.url, downloadPath, deps)

  try {
    const actualSha256 = await sha256File(downloadPath)
    if (actualSha256 !== entry.sha256) {
      throw new Error(
        `sha256 mismatch for ${entry.url}: expected ${entry.sha256}, got ${actualSha256} — aborting, no file left in place`,
      )
    }

    let candidatePath = downloadPath
    if (entry.archive) {
      await extractMember(entry.archive.type, downloadPath, entry.archive.member, extractingPath)
      await rm(downloadPath, { force: true })
      candidatePath = extractingPath
    }

    if (entryKind(entry) === 'archive') {
      const type = deliveredArchiveType(entry)
      if (type === null || !entry.executableMember) {
        throw new Error(
          `${entry.binaryName}: kind "archive" entry needs executableMember and a .zip/.tar.xz name for the architecture check`,
        )
      }
      await extractMember(type, candidatePath, entry.executableMember, archCheckPath)
      await verifyExecutableArch(archCheckPath, target, `${entry.executableMember} inside ${entry.binaryName}`)
      await rm(archCheckPath, { force: true })
    } else {
      await verifyExecutableArch(candidatePath, target, entry.binaryName)
    }

    await rename(candidatePath, finalPath)
  } catch (err) {
    await Promise.all(
      [downloadPath, extractingPath, archCheckPath].map((path) => rm(path, { force: true })),
    )
    throw err
  }

  if (process.platform !== 'win32') {
    await chmod(finalPath, entryKind(entry) === 'archive' ? 0o644 : 0o755)
  }

  return finalPath
}
