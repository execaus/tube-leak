import { createHash } from 'node:crypto'
import { execFileSync } from 'node:child_process'
import { mkdir, mkdtemp, readdir, readFile, rm, stat, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'

import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import { installBinary } from './install.mjs'
import { KNOWN_TARGETS } from './targets.mjs'
import { elf, machoFat, machoThin, nativeHeaderFor, pe } from './test-headers.mjs'

const TARGET = 'x86_64-apple-darwin'

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

/**
 * Собирает zip из `{ путь внутри архива: содержимое }`.
 *
 * @param {Record<string, string | Buffer>} files
 */
async function buildZipFixture(files) {
  const pkgRoot = join(dir, 'zip-src')
  await rm(pkgRoot, { recursive: true, force: true })
  for (const [path, content] of Object.entries(files)) {
    await mkdir(dirname(join(pkgRoot, path)), { recursive: true })
    await writeFile(join(pkgRoot, path), content)
  }
  const archivePath = join(dir, 'archive.zip')
  await rm(archivePath, { force: true })
  execFileSync('zip', ['-r', archivePath, '.'], { cwd: pkgRoot })
  return readFile(archivePath)
}

describe('installBinary — direct binary entries (no archive)', () => {
  it('places the verified download at binaryName inside outDir', async () => {
    const content = machoThin('x86_64')
    const entry = {
      url: 'https://example.invalid/yt-dlp',
      sha256: sha256Of(content),
      binaryName: 'yt-dlp-x86_64-apple-darwin',
    }
    const fetchImpl = async () => new Response(content)

    const finalPath = await installBinary(entry, outDir, TARGET, { fetchImpl })

    expect(finalPath).toBe(join(outDir, entry.binaryName))
    await expect(readFile(finalPath)).resolves.toStrictEqual(content)
  })

  it('leaves no file in outDir when the downloaded content does not match the pinned sha256', async () => {
    const content = machoThin('x86_64')
    const entry = {
      url: 'https://example.invalid/yt-dlp',
      sha256: 'f'.repeat(64), // заведомо неверная контрольная сумма
      binaryName: 'yt-dlp-x86_64-apple-darwin',
    }
    const fetchImpl = async () => new Response(content)

    await expect(installBinary(entry, outDir, TARGET, { fetchImpl })).rejects.toThrow(/sha256 mismatch/)

    const entries = await readdir(outDir)
    expect(entries).toStrictEqual([])
  })

  it('reports both the expected and the actual sha256 in the error message', async () => {
    const content = machoThin('x86_64')
    const wrongSha = 'f'.repeat(64)
    const entry = {
      url: 'https://example.invalid/yt-dlp',
      sha256: wrongSha,
      binaryName: 'yt-dlp-x86_64-apple-darwin',
    }
    const fetchImpl = async () => new Response(content)

    await expect(installBinary(entry, outDir, TARGET, { fetchImpl })).rejects.toThrow(
      new RegExp(`expected ${wrongSha}, got ${sha256Of(content)}`),
    )
  })

  // Проверка обязана сверять с тройкой ЗАПИСИ, а не с какой-то одной: тест
  // на единственной тройке не заметил бы, что тройка подставлена жёстко.
  for (const target of KNOWN_TARGETS) {
    it(`accepts the native executable for ${target}, both direct and extracted from an archive`, async () => {
      const header = nativeHeaderFor(target)
      const direct = { url: 'https://example.invalid/direct', sha256: sha256Of(header), binaryName: `direct-${target}` }
      await expect(
        installBinary(direct, outDir, target, { fetchImpl: async () => new Response(header) }),
      ).resolves.toBe(join(outDir, direct.binaryName))

      const archiveBytes = await buildZipFixture({ deno: header })
      const extracted = {
        url: 'https://example.invalid/deno.zip',
        sha256: sha256Of(archiveBytes),
        binaryName: `deno-${target}`,
        archive: { type: 'zip', member: 'deno' },
      }
      await expect(
        installBinary(extracted, outDir, target, { fetchImpl: async () => new Response(archiveBytes) }),
      ).resolves.toBe(join(outDir, extracted.binaryName))

      expect((await readdir(outDir)).sort()).toStrictEqual([`deno-${target}`, `direct-${target}`])
    })
  }

  it('refuses a binary of a foreign architecture even though its sha256 matches, leaving nothing behind', async () => {
    // TL-11: пин с правдоподобным ассетом чужой архитектуры проходит сверку
    // суммы — именно поэтому сумма здесь верная.
    const content = machoThin('aarch64')
    const entry = {
      url: 'https://example.invalid/deno',
      sha256: sha256Of(content),
      binaryName: 'deno-x86_64-apple-darwin',
    }
    const fetchImpl = async () => new Response(content)

    await expect(installBinary(entry, outDir, TARGET, { fetchImpl })).rejects.toThrow(
      /architecture mismatch for deno-x86_64-apple-darwin: target x86_64-apple-darwin expects mach-o x86_64, got mach-o aarch64/,
    )
    expect(await readdir(outDir)).toStrictEqual([])
  })

  it('refuses a non-executable under a binary name (e.g. an HTML error page)', async () => {
    const content = '<!doctype html><title>404</title>\n'
    const entry = {
      url: 'https://example.invalid/deno',
      sha256: sha256Of(content),
      binaryName: 'deno-x86_64-apple-darwin',
    }
    const fetchImpl = async () => new Response(content)

    await expect(installBinary(entry, outDir, TARGET, { fetchImpl })).rejects.toThrow(/not a recognised executable/)
    expect(await readdir(outDir)).toStrictEqual([])
  })

  it('requires the target triple instead of silently skipping the architecture check', async () => {
    const content = machoThin('x86_64')
    const entry = { url: 'https://example.invalid/x', sha256: sha256Of(content), binaryName: 'x' }
    let fetched = false
    const fetchImpl = async () => {
      fetched = true
      return new Response(content)
    }

    await expect(installBinary(entry, outDir, undefined, { fetchImpl })).rejects.toThrow(/target triple is required/)
    expect(fetched).toBe(false)
    expect(await readdir(outDir)).toStrictEqual([])
  })
})

describe('installBinary — archive entries', () => {
  it('extracts the matched member and places it at binaryName', async () => {
    const memberContent = machoThin('x86_64')
    const archiveBytes = await buildZipFixture({ 'pkg/bin/ffmpeg': memberContent, 'pkg/README': 'docs\n' })
    const entry = {
      url: 'https://example.invalid/ffmpeg.zip',
      sha256: sha256Of(archiveBytes),
      binaryName: 'ffmpeg-x86_64-apple-darwin',
      archive: { type: 'zip', member: 'ffmpeg' },
    }
    const fetchImpl = async () => new Response(archiveBytes)

    const finalPath = await installBinary(entry, outDir, TARGET, { fetchImpl })

    await expect(readFile(finalPath)).resolves.toStrictEqual(memberContent)
    expect(await readdir(outDir)).toStrictEqual([entry.binaryName])
  })

  it('refuses an extracted member of a foreign architecture and leaves neither archive nor extraction behind', async () => {
    // deno для Windows по ошибке вписан под Linux-тройку: sha256 от «того»
    // архива верный, член с нужным basename есть, но это PE, а не ELF.
    const archiveBytes = await buildZipFixture({ deno: pe('x86_64') })
    const entry = {
      url: 'https://example.invalid/deno.zip',
      sha256: sha256Of(archiveBytes),
      binaryName: 'deno-x86_64-unknown-linux-gnu',
      archive: { type: 'zip', member: 'deno' },
    }
    const fetchImpl = async () => new Response(archiveBytes)

    await expect(installBinary(entry, outDir, 'x86_64-unknown-linux-gnu', { fetchImpl })).rejects.toThrow(
      /expects elf x86_64, got pe x86_64/,
    )
    expect(await readdir(outDir)).toStrictEqual([])
  })

  it('places an archive entry as-is, without the executable bit and without extracting it', async () => {
    // yt-dlp после TL-12 поставляется onedir-архивом: скрипт кладёт архив
    // целиком, распаковывает его уже приложение при первом запуске
    // (см. src-tauri/src/ytdlp).
    const archiveBytes = await buildZipFixture({
      'yt-dlp_macos': machoFat(['x86_64', 'aarch64']),
      '_internal/base_library.zip': 'pretend-tree\n',
    })
    const entry = {
      url: 'https://example.invalid/yt-dlp_macos.zip',
      sha256: sha256Of(archiveBytes),
      kind: 'archive',
      binaryName: 'yt-dlp-aarch64-apple-darwin.zip',
      executableMember: 'yt-dlp_macos',
    }
    const fetchImpl = async () => new Response(archiveBytes)

    const finalPath = await installBinary(entry, outDir, 'aarch64-apple-darwin', { fetchImpl })

    expect(finalPath).toBe(join(outDir, entry.binaryName))
    await expect(readFile(finalPath)).resolves.toStrictEqual(archiveBytes)
    const mode = (await stat(finalPath)).mode & 0o777
    expect(mode & 0o111).toBe(0)
    // Временный файл проверки архитектуры убран.
    expect(await readdir(outDir)).toStrictEqual([entry.binaryName])
  })

  it('refuses an as-is archive whose executable member is of a foreign architecture', async () => {
    const archiveBytes = await buildZipFixture({
      'yt-dlp_linux': elf('aarch64'),
      '_internal/base_library.zip': 'pretend-tree\n',
    })
    const entry = {
      url: 'https://example.invalid/yt-dlp_linux.zip',
      sha256: sha256Of(archiveBytes),
      kind: 'archive',
      binaryName: 'yt-dlp-x86_64-unknown-linux-gnu.zip',
      executableMember: 'yt-dlp_linux',
    }
    const fetchImpl = async () => new Response(archiveBytes)

    await expect(installBinary(entry, outDir, 'x86_64-unknown-linux-gnu', { fetchImpl })).rejects.toThrow(
      /architecture mismatch for yt-dlp_linux inside yt-dlp-x86_64-unknown-linux-gnu\.zip: .*expects elf x86_64, got elf aarch64/,
    )
    expect(await readdir(outDir)).toStrictEqual([])
  })

  it('refuses an as-is archive that does not contain the declared executable member', async () => {
    const archiveBytes = await buildZipFixture({ 'something-else': machoThin('x86_64') })
    const entry = {
      url: 'https://example.invalid/yt-dlp_macos.zip',
      sha256: sha256Of(archiveBytes),
      kind: 'archive',
      binaryName: 'yt-dlp-x86_64-apple-darwin.zip',
      executableMember: 'yt-dlp_macos',
    }
    const fetchImpl = async () => new Response(archiveBytes)

    await expect(installBinary(entry, outDir, TARGET, { fetchImpl })).rejects.toThrow(/no member with that basename/)
    expect(await readdir(outDir)).toStrictEqual([])
  })

  it('does not leave the downloaded archive or a partial file behind on sha256 mismatch', async () => {
    const archiveBytes = await buildZipFixture({ 'pkg/bin/ffmpeg': machoThin('x86_64') })
    const entry = {
      url: 'https://example.invalid/ffmpeg.zip',
      sha256: 'f'.repeat(64),
      binaryName: 'ffmpeg-x86_64-apple-darwin',
      archive: { type: 'zip', member: 'ffmpeg' },
    }
    const fetchImpl = async () => new Response(archiveBytes)

    await expect(installBinary(entry, outDir, TARGET, { fetchImpl })).rejects.toThrow(/sha256 mismatch/)

    const entries = await readdir(outDir)
    expect(entries).toStrictEqual([])
  })
})
