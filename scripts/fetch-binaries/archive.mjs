import { execFile, spawn } from 'node:child_process'
import { createWriteStream } from 'node:fs'
import { basename } from 'node:path'
import { promisify } from 'node:util'

const execFileAsync = promisify(execFile)

// 64 МиБ с запасом хватает под текстовый листинг архива (сотни файлов
// документации ffmpeg); сами бинарники никогда не буферизуются целиком —
// извлечение идёт потоково (см. extractMember).
const LISTING_MAX_BUFFER = 64 * 1024 * 1024

/**
 * Возвращает список файлов (без директорий) внутри архива.
 *
 * @param {'zip' | 'tar.xz'} type
 * @param {string} archivePath
 * @returns {Promise<string[]>} пути внутри архива, каждый — обычный файл
 */
export async function listMembers(type, archivePath) {
  if (type === 'zip') {
    const { stdout } = await execFileAsync('unzip', ['-Z1', archivePath], {
      maxBuffer: LISTING_MAX_BUFFER,
    }).catch((err) => {
      throw new Error(`listing zip members of ${archivePath}: ${err.message}`, { cause: err })
    })
    return stdout.split('\n').filter((line) => line.length > 0 && !line.endsWith('/'))
  }

  if (type === 'tar.xz') {
    const { stdout } = await execFileAsync('tar', ['-tJf', archivePath], {
      maxBuffer: LISTING_MAX_BUFFER,
    }).catch((err) => {
      throw new Error(`listing tar.xz members of ${archivePath}: ${err.message}`, { cause: err })
    })
    return stdout.split('\n').filter((line) => line.length > 0 && !line.endsWith('/'))
  }

  throw new Error(`unsupported archive type: ${type}`)
}

/**
 * Находит внутри архива ровно один файл с заданным basename и извлекает
 * его потоково в `destPath`. Специально не угадывает при неоднозначности:
 * ноль или больше одного совпадения — ошибка (явное поведение вместо
 * неявного выбора "первого попавшегося").
 *
 * @param {'zip' | 'tar.xz'} type
 * @param {string} archivePath
 * @param {string} member basename искомого файла, например `ffmpeg.exe`
 * @param {string} destPath
 */
export async function extractMember(type, archivePath, member, destPath) {
  const members = await listMembers(type, archivePath)
  const matches = members.filter((entry) => basename(entry) === member)

  if (matches.length === 0) {
    throw new Error(
      `extracting ${member} from ${archivePath}: no member with that basename found (archive has ${members.length} files)`,
    )
  }
  if (matches.length > 1) {
    throw new Error(
      `extracting ${member} from ${archivePath}: ambiguous, ${matches.length} members match: ${matches.join(', ')}`,
    )
  }

  const [matchedPath] = matches
  await extractToFile(type, archivePath, matchedPath, destPath)
}

/**
 * @param {'zip' | 'tar.xz'} type
 * @param {string} archivePath
 * @param {string} matchedPath точный путь внутри архива
 * @param {string} destPath
 */
function extractToFile(type, archivePath, matchedPath, destPath) {
  const [command, args] =
    type === 'zip'
      ? ['unzip', ['-p', archivePath, matchedPath]]
      : ['tar', ['-xJf', archivePath, '-O', matchedPath]]

  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { stdio: ['ignore', 'pipe', 'pipe'] })
    const out = createWriteStream(destPath)
    let stderr = ''

    child.stderr.on('data', (chunk) => {
      stderr += chunk.toString('utf8')
    })
    child.on('error', (err) => {
      reject(
        new Error(`extracting ${matchedPath} from ${archivePath}: spawning ${command}: ${err.message}`, {
          cause: err,
        }),
      )
    })

    child.stdout.pipe(out)
    out.on('error', (err) => {
      reject(
        new Error(`extracting ${matchedPath} from ${archivePath}: writing ${destPath}: ${err.message}`, {
          cause: err,
        }),
      )
    })

    child.on('close', (code) => {
      if (code !== 0) {
        reject(
          new Error(
            `extracting ${matchedPath} from ${archivePath}: ${command} exited with code ${code}: ${stderr.trim()}`,
          ),
        )
        return
      }
      resolve()
    })
  })
}
