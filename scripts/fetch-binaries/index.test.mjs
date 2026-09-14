import { createHash } from 'node:crypto'
import { execFileSync } from 'node:child_process'
import { mkdir, mkdtemp, readdir, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { parseArgs, run } from './index.mjs'
import { BINARY_NAMES } from './pin.mjs'
import { KNOWN_TARGETS, resolveHostTarget } from './targets.mjs'
import { machoThin } from './test-headers.mjs'

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
  const target = 'x86_64-apple-darwin'
  let dir
  let outDir
  let ytDlpContent
  let archives

  async function zipWith(name, member, content) {
    const src = join(dir, `${name}-src`)
    await mkdir(join(src, 'bin'), { recursive: true })
    await writeFile(join(src, 'bin', member), content)
    const archivePath = join(dir, `${name}.zip`)
    execFileSync('zip', ['-r', archivePath, '.'], { cwd: src })
    return readFile(archivePath)
  }

  beforeEach(async () => {
    dir = await mkdtemp(join(tmpdir(), 'fetch-binaries-run-test-'))
    outDir = join(dir, 'binaries')
    await mkdir(outDir, { recursive: true })

    ytDlpContent = machoThin('x86_64')
    archives = {
      ffmpeg: { member: machoThin('x86_64'), bytes: null },
      deno: { member: machoThin('x86_64'), bytes: null },
    }
    archives.ffmpeg.bytes = await zipWith('ffmpeg', 'ffmpeg', archives.ffmpeg.member)
    archives.deno.bytes = await zipWith('deno', 'deno', archives.deno.member)

    vi.stubGlobal(
      'fetch',
      vi.fn(async (url) => {
        if (url === 'https://example.invalid/yt-dlp') return new Response(ytDlpContent)
        if (url === 'https://example.invalid/ffmpeg.zip') return new Response(archives.ffmpeg.bytes)
        if (url === 'https://example.invalid/deno.zip') return new Response(archives.deno.bytes)
        throw new Error(`unexpected test URL: ${url}`)
      }),
    )
  })

  afterEach(async () => {
    vi.unstubAllGlobals()
    await rm(dir, { recursive: true, force: true })
  })

  function buildPin({ ffmpegSha256, denoSha256 } = {}) {
    // Пин обязан покрывать все известные тройки (см. pin.mjs), поэтому
    // остальные три получают инертные заглушки-плейсхолдеры — run() в
    // этом тесте запрашивается только для `target`, они не скачиваются.
    const otherTargets = KNOWN_TARGETS.filter((t) => t !== target)
    const placeholders = (tool) =>
      Object.fromEntries(
        otherTargets.map((t) => [
          t,
          { url: `https://example.invalid/${tool}-${t}`, sha256: 'a'.repeat(64), binaryName: `${tool}-${t}` },
        ]),
      )

    return {
      ytDlp: {
        version: '2026.01.01',
        targets: {
          ...placeholders('yt-dlp'),
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
          ...placeholders('ffmpeg'),
          [target]: {
            url: 'https://example.invalid/ffmpeg.zip',
            sha256: ffmpegSha256 ?? sha256Of(archives.ffmpeg.bytes),
            binaryName: `ffmpeg-${target}`,
            archive: { type: 'zip', member: 'ffmpeg' },
          },
        },
      },
      deno: {
        version: '2.9.6',
        targets: {
          ...placeholders('deno'),
          [target]: {
            url: 'https://example.invalid/deno.zip',
            sha256: denoSha256 ?? sha256Of(archives.deno.bytes),
            binaryName: `deno-${target}`,
            archive: { type: 'zip', member: 'deno' },
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

  it('delivers every sidecar section of the pin — deno included', () => {
    expect(BINARY_NAMES).toStrictEqual(['ytDlp', 'ffmpeg', 'deno'])
  })

  it('installs all three binaries when every checksum and architecture in the pin matches', async () => {
    const pinPath = await writeTempPin(buildPin())

    await run({ targets: [target], pinPath, outDir })

    await expect(readFile(join(outDir, `yt-dlp-${target}`))).resolves.toStrictEqual(ytDlpContent)
    await expect(readFile(join(outDir, `ffmpeg-${target}`))).resolves.toStrictEqual(archives.ffmpeg.member)
    await expect(readFile(join(outDir, `deno-${target}`))).resolves.toStrictEqual(archives.deno.member)
    expect((await readdir(outDir)).sort()).toStrictEqual([`deno-${target}`, `ffmpeg-${target}`, `yt-dlp-${target}`])
  })

  it('fails the run and leaves no file for the target whose pinned sha256 was tampered with', async () => {
    // Заведомо неверная контрольная сумма — имитирует подмену пина.
    const tamperedSha256 = 'f'.repeat(64)
    const pinPath = await writeTempPin(buildPin({ denoSha256: tamperedSha256 }))

    await expect(run({ targets: [target], pinPath, outDir })).rejects.toThrow(/1\/3 binaries failed to install/)

    // yt-dlp и ffmpeg с верными суммами всё же установлены...
    // ...а deno с подменённой суммой — нет, и никакого частичного файла не осталось.
    expect((await readdir(outDir)).sort()).toStrictEqual([`ffmpeg-${target}`, `yt-dlp-${target}`])
  })

  it('fails the run when deno of a foreign architecture is pinned under this target', async () => {
    // Перепутанный macOS-ассет deno: сумма верная, архитектура — нет.
    archives.deno.member = machoThin('aarch64')
    archives.deno.bytes = await zipWith('deno-arm', 'deno', archives.deno.member)
    const pinPath = await writeTempPin(buildPin())
    const errors = []
    const spy = vi.spyOn(console, 'error').mockImplementation((line) => errors.push(line))

    try {
      await expect(run({ targets: [target], pinPath, outDir })).rejects.toThrow(/1\/3 binaries failed to install/)
    } finally {
      spy.mockRestore()
    }

    expect(errors.join('\n')).toMatch(/deno \(x86_64-apple-darwin\): architecture mismatch .*expects mach-o x86_64, got mach-o aarch64/)
    expect((await readdir(outDir)).sort()).toStrictEqual([`ffmpeg-${target}`, `yt-dlp-${target}`])
  })
})
