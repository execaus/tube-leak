import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import { parseArgs } from './index.mjs'
import { BINARY_NAMES, entryKind, loadPin } from './pin.mjs'
import { KNOWN_TARGETS } from './targets.mjs'

const VALID_SHA = 'a'.repeat(64)

function makeEntry(overrides = {}) {
  return { url: 'https://example.invalid/asset', sha256: VALID_SHA, binaryName: 'thing', ...overrides }
}

function makeTargets(entryFactory = makeEntry) {
  return Object.fromEntries(KNOWN_TARGETS.map((target) => [target, entryFactory()]))
}

function makeValidPin() {
  return {
    ytDlp: { version: '1.0.0', targets: makeTargets() },
    ffmpeg: { version: '1.0.0', targets: makeTargets() },
    deno: { version: '1.0.0', targets: makeTargets() },
  }
}

let dir

beforeEach(async () => {
  dir = await mkdtemp(join(tmpdir(), 'fetch-binaries-pin-test-'))
})

afterEach(async () => {
  await rm(dir, { recursive: true, force: true })
})

async function writePin(pin) {
  const path = join(dir, 'pin.json')
  await writeFile(path, JSON.stringify(pin), 'utf8')
  return path
}

describe('loadPin', () => {
  it('loads and returns a structurally valid pin file', async () => {
    const pin = makeValidPin()
    const path = await writePin(pin)

    await expect(loadPin(path)).resolves.toStrictEqual(pin)
  })

  it('rejects when the file does not exist', async () => {
    await expect(loadPin(join(dir, 'missing.json'))).rejects.toThrow(/reading pin file/)
  })

  it('rejects invalid JSON', async () => {
    const path = join(dir, 'broken.json')
    await writeFile(path, '{ not json', 'utf8')

    await expect(loadPin(path)).rejects.toThrow(/parsing pin file/)
  })

  it('rejects a pin missing the ffmpeg section', async () => {
    const pin = makeValidPin()
    delete pin.ffmpeg
    const path = await writePin(pin)

    await expect(loadPin(path)).rejects.toThrow(/missing "ffmpeg" section/)
  })

  it('rejects a pin missing the deno section', async () => {
    const pin = makeValidPin()
    delete pin.deno
    const path = await writePin(pin)

    await expect(loadPin(path)).rejects.toThrow(/missing "deno" section/)
  })

  it('rejects a pin missing a known target triple', async () => {
    const pin = makeValidPin()
    delete pin.ytDlp.targets['x86_64-unknown-linux-gnu']
    const path = await writePin(pin)

    await expect(loadPin(path)).rejects.toThrow(
      /ytDlp\.targets\.x86_64-unknown-linux-gnu.* is missing/,
    )
  })

  it('rejects an entry with a malformed sha256 (wrong length)', async () => {
    const pin = makeValidPin()
    pin.ffmpeg.targets['x86_64-apple-darwin'] = makeEntry({ sha256: 'deadbeef' })
    const path = await writePin(pin)

    await expect(loadPin(path)).rejects.toThrow(/sha256.* must be a 64-char lowercase hex/)
  })

  it('rejects an entry with an uppercase sha256', async () => {
    const pin = makeValidPin()
    pin.ffmpeg.targets['x86_64-apple-darwin'] = makeEntry({ sha256: 'A'.repeat(64) })
    const path = await writePin(pin)

    await expect(loadPin(path)).rejects.toThrow(/sha256.* must be a 64-char lowercase hex/)
  })

  it('rejects an archive entry with an unsupported archive type', async () => {
    const pin = makeValidPin()
    pin.ffmpeg.targets['x86_64-pc-windows-msvc'] = makeEntry({
      archive: { type: '7z', member: 'ffmpeg.exe' },
    })
    const path = await writePin(pin)

    await expect(loadPin(path)).rejects.toThrow(/archive\.type.* must be "zip" or "tar\.xz"/)
  })

  it('rejects an archive entry missing a member basename', async () => {
    const pin = makeValidPin()
    pin.ffmpeg.targets['x86_64-pc-windows-msvc'] = makeEntry({
      archive: { type: 'zip' },
    })
    const path = await writePin(pin)

    await expect(loadPin(path)).rejects.toThrow(/archive\.member.* must be the basename/)
  })

  it('accepts a valid archive entry unchanged', async () => {
    const pin = makeValidPin()
    pin.ffmpeg.targets['x86_64-pc-windows-msvc'] = makeEntry({
      archive: { type: 'zip', member: 'ffmpeg.exe' },
      binarySha256: 'b'.repeat(64),
    })
    const path = await writePin(pin)

    await expect(loadPin(path)).resolves.toStrictEqual(pin)
  })

  it('rejects an extracted entry without binarySha256 — nothing would cover the placed file', async () => {
    // TL-134 (#141): sha256 архива после извлечения не охраняет ничего, и
    // подмену распакованного файла между доставкой и сборкой без этой
    // суммы не ловит никто.
    const pin = makeValidPin()
    pin.ffmpeg.targets['x86_64-pc-windows-msvc'] = makeEntry({
      archive: { type: 'zip', member: 'ffmpeg.exe' },
    })
    const path = await writePin(pin)

    await expect(loadPin(path)).rejects.toThrow(
      /ffmpeg\.targets\.x86_64-pc-windows-msvc\.binarySha256.* is required whenever the delivered file is extracted/,
    )
  })

  it('rejects an entry with an unknown kind', async () => {
    const pin = makeValidPin()
    pin.ytDlp.targets['aarch64-apple-darwin'] = makeEntry({ kind: 'directory' })
    const path = await writePin(pin)

    await expect(loadPin(path)).rejects.toThrow(/kind.* must be one of binary, archive/)
  })

  it('accepts an entry declared as an archive delivered as-is', async () => {
    const pin = makeValidPin()
    pin.ytDlp.targets['aarch64-apple-darwin'] = makeEntry({
      kind: 'archive',
      binaryName: 'yt-dlp-aarch64-apple-darwin.zip',
      executableMember: 'yt-dlp_macos',
    })
    const path = await writePin(pin)

    await expect(loadPin(path)).resolves.toStrictEqual(pin)
  })

  it('rejects an as-is archive entry without executableMember — its architecture could not be checked', async () => {
    const pin = makeValidPin()
    pin.ytDlp.targets['aarch64-apple-darwin'] = makeEntry({
      kind: 'archive',
      binaryName: 'yt-dlp-aarch64-apple-darwin.zip',
    })
    const path = await writePin(pin)

    await expect(loadPin(path)).rejects.toThrow(/ytDlp\.targets\.aarch64-apple-darwin\.executableMember.* must be the basename/)
  })

  it('rejects an executableMember that is a path rather than a basename', async () => {
    const pin = makeValidPin()
    pin.ytDlp.targets['aarch64-apple-darwin'] = makeEntry({
      kind: 'archive',
      binaryName: 'yt-dlp-aarch64-apple-darwin.zip',
      executableMember: '_internal/yt-dlp_macos',
    })
    const path = await writePin(pin)

    await expect(loadPin(path)).rejects.toThrow(/executableMember.* must be the basename/)
  })

  it('rejects an as-is archive entry whose name does not tell the archive type', async () => {
    const pin = makeValidPin()
    pin.ytDlp.targets['aarch64-apple-darwin'] = makeEntry({
      kind: 'archive',
      binaryName: 'yt-dlp-aarch64-apple-darwin.7z',
      executableMember: 'yt-dlp_macos',
    })
    const path = await writePin(pin)

    await expect(loadPin(path)).rejects.toThrow(/binaryName.* must end with \.zip or \.tar\.xz/)
  })

  it('rejects a malformed binarySha256 and accepts a well-formed one', async () => {
    const bad = makeValidPin()
    bad.deno.targets['aarch64-apple-darwin'] = makeEntry({ binarySha256: 'B'.repeat(64) })
    await expect(loadPin(await writePin(bad))).rejects.toThrow(
      /deno\.targets\.aarch64-apple-darwin\.binarySha256.* must be a 64-char lowercase hex/,
    )

    const good = makeValidPin()
    good.deno.targets['aarch64-apple-darwin'] = makeEntry({ binarySha256: 'b'.repeat(64) })
    await expect(loadPin(await writePin(good))).resolves.toStrictEqual(good)
  })
})

describe('the real repository pin', () => {
  it('validates, names deno 2.9.6 for every target and lets every as-is archive be architecture-checked', async () => {
    // Тот же путь по умолчанию, по которому читает пин настоящая доставка.
    const pin = await loadPin(parseArgs([]).pinPath)

    expect(pin.deno.version).toBe('2.9.6')
    for (const target of KNOWN_TARGETS) {
      const entry = pin.deno.targets[target]
      expect(entry.url).toBe(`https://github.com/denoland/deno/releases/download/v2.9.6/deno-${target}.zip`)
      expect(entry.archive).toStrictEqual({ type: 'zip', member: target.includes('windows') ? 'deno.exe' : 'deno' })
      expect(entry.binaryName).toBe(`deno-${target}${target.includes('windows') ? '.exe' : ''}`)
      // TL-112: сумма распакованного бинарника — отдельная от суммы архива.
      expect(entry.binarySha256).toMatch(/^[0-9a-f]{64}$/)
      expect(entry.binarySha256).not.toBe(entry.sha256)
    }
    expect(pin.deno.targets['aarch64-apple-darwin'].binarySha256).toBe(
      'b3ac3bd206e48c26026cadd80c1367e96c149f9c66130952382a642b09fa8a71',
    )
    for (const section of BINARY_NAMES) {
      for (const target of KNOWN_TARGETS) {
        const entry = pin[section].targets[target]
        if (entryKind(entry) === 'archive') expect(entry.executableMember).toBeTruthy()
      }
    }
  })

  it('carries the unpacked ffmpeg sum for every target (TL-134)', async () => {
    const pin = await loadPin(parseArgs([]).pinPath)
    const sums = new Set()

    for (const target of KNOWN_TARGETS) {
      const entry = pin.ffmpeg.targets[target]
      expect(entry.binarySha256).toMatch(/^[0-9a-f]{64}$/)
      // Сумма распакованного — не сумма архива: перепутанное поле
      // пропустило бы в binaries/ любой файл, кроме настоящего.
      expect(entry.binarySha256).not.toBe(entry.sha256)
      sums.add(entry.binarySha256)
    }
    // Четыре разные сборки — четыре разные суммы; скопированное поле
    // охраняло бы чужую тройку.
    expect(sums.size).toBe(KNOWN_TARGETS.length)

    // Снято `shasum -a 256` с файла, доставленного на этой машине, а не
    // кодом под тестом.
    expect(pin.ffmpeg.targets['aarch64-apple-darwin'].binarySha256).toBe(
      '393e4c395020a1cb7cbd77fbe00599ce69d1c6466fee0dbd59d13f86a81a1611',
    )
  })
})

describe('entryKind', () => {
  it('defaults to binary for entries written before TL-12 introduced the field', () => {
    expect(entryKind(makeEntry())).toBe('binary')
  })

  it('returns the declared kind when present', () => {
    expect(entryKind(makeEntry({ kind: 'archive' }))).toBe('archive')
  })
})
