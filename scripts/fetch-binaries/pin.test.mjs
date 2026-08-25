import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import { loadPin } from './pin.mjs'
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
    })
    const path = await writePin(pin)

    await expect(loadPin(path)).resolves.toStrictEqual(pin)
  })
})
