import { createHash } from 'node:crypto'
import { execFileSync } from 'node:child_process'
import { mkdir, mkdtemp, readdir, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { parseArgs, run } from './index.mjs'
import { KNOWN_TARGETS, resolveHostTarget } from './targets.mjs'

function sha256Of(content) {
  return createHash('sha256').update(content).digest('hex')
}

describe('parseArgs', () => {
  it('defaults to the host target with the default pin and out paths', () => {
    const options = parseArgs([])

    expect(options.targets).toStrictEqual([resolveHostTarget()])
    expect(options.pinPath.endsWith('src-tauri/binaries.lock.json')).toBe(true)
    expect(options.outDir.endsWith('src-tauri/binaries')).toBe(true)
  })

  it('expands --target all to every known target exactly once', () => {
    const options = parseArgs(['--target', 'all'])

    expect(options.targets).toStrictEqual([
      'x86_64-pc-windows-msvc',
      'x86_64-apple-darwin',
      'aarch64-apple-darwin',
      'x86_64-unknown-linux-gnu',
    ])
  })

  it('accepts a single explicit target', () => {
    const options = parseArgs(['--target', 'x86_64-unknown-linux-gnu'])

    expect(options.targets).toStrictEqual(['x86_64-unknown-linux-gnu'])
  })

  it('deduplicates repeated --target flags', () => {
    const options = parseArgs(['--target', 'x86_64-apple-darwin', '--target', 'x86_64-apple-darwin'])

    expect(options.targets).toStrictEqual(['x86_64-apple-darwin'])
  })

  it('rejects an unknown target triple', () => {
    expect(() => parseArgs(['--target', 'sparc-sun-solaris'])).toThrow(/unknown target triple/)
  })

  it('rejects an unknown flag', () => {
    expect(() => parseArgs(['--bogus'])).toThrow(/unknown argument: --bogus/)
  })

  it('requires a value after --target/--pin/--out', () => {
    expect(() => parseArgs(['--target'])).toThrow(/--target requires a value/)
    expect(() => parseArgs(['--pin'])).toThrow(/--pin requires a value/)
    expect(() => parseArgs(['--out'])).toThrow(/--out requires a value/)
  })

  it('honours explicit --pin and --out overrides', () => {
    const options = parseArgs(['--pin', '/tmp/custom-pin.json', '--out', '/tmp/custom-out'])

    expect(options.pinPath).toBe('/tmp/custom-pin.json')
    expect(options.outDir).toBe('/tmp/custom-out')
  })
})

describe('run — end to end against a temporary pin file (real repo pin is never touched)', () => {
  let dir
  let outDir
  let ytDlpContent
  let ffmpegArchiveBytes
  let ffmpegMemberContent

  beforeEach(async () => {
    dir = await mkdtemp(join(tmpdir(), 'fetch-binaries-run-test-'))
    outDir = join(dir, 'binaries')
    await mkdir(outDir, { recursive: true })

    ytDlpContent = 'fake yt-dlp payload\n'

    ffmpegMemberContent = 'fake ffmpeg payload\n'
    const pkgDir = join(dir, 'pkg')
    await mkdir(join(pkgDir, 'bin'), { recursive: true })
    await writeFile(join(pkgDir, 'bin', 'ffmpeg'), ffmpegMemberContent)
    const archivePath = join(dir, 'ffmpeg.zip')
    execFileSync('zip', ['-r', archivePath, 'pkg'], { cwd: dir })
    ffmpegArchiveBytes = await readFile(archivePath)

    vi.stubGlobal(
      'fetch',
      vi.fn(async (url) => {
        if (url === 'https://example.invalid/yt-dlp') return new Response(ytDlpContent)
        if (url === 'https://example.invalid/ffmpeg.zip') return new Response(ffmpegArchiveBytes)
        throw new Error(`unexpected test URL: ${url}`)
      }),
    )
  })

  afterEach(async () => {
    vi.unstubAllGlobals()
    await rm(dir, { recursive: true, force: true })
  })

  function buildPin({ ffmpegSha256 } = {}) {
    const target = 'x86_64-apple-darwin'
    // Пин обязан покрывать все известные тройки (см. pin.mjs), поэтому
    // остальные три получают инертные заглушки-плейсхолдеры — run() в
    // этом тесте запрашивается только для `target`, они не скачиваются.
    const otherTargets = KNOWN_TARGETS.filter((t) => t !== target)
    const placeholderYtDlp = Object.fromEntries(
      otherTargets.map((t) => [
        t,
        { url: `https://example.invalid/yt-dlp-${t}`, sha256: 'a'.repeat(64), binaryName: `yt-dlp-${t}` },
      ]),
    )
    const placeholderFfmpeg = Object.fromEntries(
      otherTargets.map((t) => [
        t,
        { url: `https://example.invalid/ffmpeg-${t}`, sha256: 'a'.repeat(64), binaryName: `ffmpeg-${t}` },
      ]),
    )

    return {
      ytDlp: {
        version: '2026.01.01',
        targets: {
          ...placeholderYtDlp,
          [target]: {
            url: 'https://example.invalid/yt-dlp',
            sha256: sha256Of(ytDlpContent),
            binaryName: `yt-dlp-${target}`,
          },
        },
      },
      ffmpeg: {
        version: '9.0.1',
        targets: {
          ...placeholderFfmpeg,
          [target]: {
            url: 'https://example.invalid/ffmpeg.zip',
            sha256: ffmpegSha256 ?? sha256Of(ffmpegArchiveBytes),
            binaryName: `ffmpeg-${target}`,
            archive: { type: 'zip', member: 'ffmpeg' },
          },
        },
      },
    }
  }

  async function writeTempPin(pin) {
    const path = join(dir, 'pin.json')
    await writeFile(path, JSON.stringify(pin), 'utf8')
    return path
  }

  it('installs both binaries when every checksum in the pin matches', async () => {
    const pinPath = await writeTempPin(buildPin())

    await run({ targets: ['x86_64-apple-darwin'], pinPath, outDir })

    await expect(readFile(join(outDir, 'yt-dlp-x86_64-apple-darwin'), 'utf8')).resolves.toBe(ytDlpContent)
    await expect(readFile(join(outDir, 'ffmpeg-x86_64-apple-darwin'), 'utf8')).resolves.toBe(
      ffmpegMemberContent,
    )
  })

  it('fails the run and leaves no file for the target whose pinned sha256 was tampered with', async () => {
    // Заведомо неверная контрольная сумма — имитирует подмену пина.
    const tamperedSha256 = 'f'.repeat(64)
    const pinPath = await writeTempPin(buildPin({ ffmpegSha256: tamperedSha256 }))

    await expect(run({ targets: ['x86_64-apple-darwin'], pinPath, outDir })).rejects.toThrow(
      /1\/2 binaries failed to install/,
    )

    // yt-dlp с верной суммой всё же установлен...
    await expect(readFile(join(outDir, 'yt-dlp-x86_64-apple-darwin'), 'utf8')).resolves.toBe(ytDlpContent)
    // ...а ffmpeg с подменённой суммой — нет, и никакого частичного файла не осталось.
    const entries = await readdir(outDir)
    expect(entries).toStrictEqual(['yt-dlp-x86_64-apple-darwin'])
  })
})
