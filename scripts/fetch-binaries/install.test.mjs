import { createHash } from 'node:crypto'
import { execFileSync } from 'node:child_process'
import { mkdir, mkdtemp, readdir, readFile, rm, stat, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import { installBinary } from './install.mjs'

let dir
let outDir

beforeEach(async () => {
  dir = await mkdtemp(join(tmpdir(), 'fetch-binaries-install-test-'))
  outDir = join(dir, 'binaries')
  await mkdir(outDir, { recursive: true })
})

afterEach(async () => {
  await rm(dir, { recursive: true, force: true })
})

function sha256Of(content) {
  return createHash('sha256').update(content).digest('hex')
}

describe('installBinary — direct binary entries (no archive)', () => {
  it('places the verified download at binaryName inside outDir', async () => {
    const content = 'a fake yt-dlp binary\n'
    const entry = {
      url: 'https://example.invalid/yt-dlp',
      sha256: sha256Of(content),
      binaryName: 'yt-dlp-x86_64-apple-darwin',
    }
    const fetchImpl = async () => new Response(content)

    const finalPath = await installBinary(entry, outDir, { fetchImpl })

    expect(finalPath).toBe(join(outDir, entry.binaryName))
    await expect(readFile(finalPath, 'utf8')).resolves.toBe(content)
  })

  it('leaves no file in outDir when the downloaded content does not match the pinned sha256', async () => {
    const content = 'a fake yt-dlp binary\n'
    const entry = {
      url: 'https://example.invalid/yt-dlp',
      sha256: 'f'.repeat(64), // заведомо неверная контрольная сумма
      binaryName: 'yt-dlp-x86_64-apple-darwin',
    }
    const fetchImpl = async () => new Response(content)

    await expect(installBinary(entry, outDir, { fetchImpl })).rejects.toThrow(/sha256 mismatch/)

    const entries = await readdir(outDir)
    expect(entries).toStrictEqual([])
  })

  it('reports both the expected and the actual sha256 in the error message', async () => {
    const content = 'a fake yt-dlp binary\n'
    const wrongSha = 'f'.repeat(64)
    const entry = {
      url: 'https://example.invalid/yt-dlp',
      sha256: wrongSha,
      binaryName: 'yt-dlp-x86_64-apple-darwin',
    }
    const fetchImpl = async () => new Response(content)

    await expect(installBinary(entry, outDir, { fetchImpl })).rejects.toThrow(
      new RegExp(`expected ${wrongSha}, got ${sha256Of(content)}`),
    )
  })
})

describe('installBinary — archive entries', () => {
  async function buildZipFixture(memberContent) {
    const pkgDir = join(dir, 'pkg')
    await mkdir(join(pkgDir, 'bin'), { recursive: true })
    await writeFile(join(pkgDir, 'bin', 'ffmpeg'), memberContent)
    const archivePath = join(dir, 'archive.zip')
    execFileSync('zip', ['-r', archivePath, 'pkg'], { cwd: dir })
    return readFile(archivePath)
  }

  it('extracts the matched member and places it at binaryName', async () => {
    const memberContent = 'pretend-ffmpeg-binary\n'
    const archiveBytes = await buildZipFixture(memberContent)
    const entry = {
      url: 'https://example.invalid/ffmpeg.zip',
      sha256: sha256Of(archiveBytes),
      binaryName: 'ffmpeg-x86_64-apple-darwin',
      archive: { type: 'zip', member: 'ffmpeg' },
    }
    const fetchImpl = async () => new Response(archiveBytes)

    const finalPath = await installBinary(entry, outDir, { fetchImpl })

    await expect(readFile(finalPath, 'utf8')).resolves.toBe(memberContent)
  })

  it('places an archive entry as-is, without the executable bit and without extracting it', async () => {
    // yt-dlp после TL-12 поставляется onedir-архивом: скрипт кладёт архив
    // целиком, распаковывает его уже приложение при первом запуске
    // (см. src-tauri/src/ytdlp).
    const archiveBytes = await buildZipFixture('pretend-yt-dlp-tree\n')
    const entry = {
      url: 'https://example.invalid/yt-dlp_macos.zip',
      sha256: sha256Of(archiveBytes),
      kind: 'archive',
      binaryName: 'yt-dlp-aarch64-apple-darwin.zip',
    }
    const fetchImpl = async () => new Response(archiveBytes)

    const finalPath = await installBinary(entry, outDir, { fetchImpl })

    expect(finalPath).toBe(join(outDir, entry.binaryName))
    await expect(readFile(finalPath)).resolves.toStrictEqual(archiveBytes)
    const mode = (await stat(finalPath)).mode & 0o777
    expect(mode & 0o111).toBe(0)
  })

  it('does not leave the downloaded archive or a partial file behind on sha256 mismatch', async () => {
    const archiveBytes = await buildZipFixture('irrelevant\n')
    const entry = {
      url: 'https://example.invalid/ffmpeg.zip',
      sha256: 'f'.repeat(64),
      binaryName: 'ffmpeg-x86_64-apple-darwin',
      archive: { type: 'zip', member: 'ffmpeg' },
    }
    const fetchImpl = async () => new Response(archiveBytes)

    await expect(installBinary(entry, outDir, { fetchImpl })).rejects.toThrow(/sha256 mismatch/)

    const entries = await readdir(outDir)
    expect(entries).toStrictEqual([])
  })
})
