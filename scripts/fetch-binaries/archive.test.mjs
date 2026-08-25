import { execFileSync } from 'node:child_process'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import { extractMember, listMembers } from './archive.mjs'

let dir
let pkgDir

const FFMPEG_CONTENT = 'pretend-this-is-an-ffmpeg-binary\n'
const README_CONTENT = 'just some docs\n'

beforeEach(async () => {
  dir = await mkdtemp(join(tmpdir(), 'fetch-binaries-archive-test-'))
  pkgDir = join(dir, 'pkg')
  await mkdir(join(pkgDir, 'bin'), { recursive: true })
  await mkdir(join(pkgDir, 'doc'), { recursive: true })
  await writeFile(join(pkgDir, 'bin', 'ffmpeg'), FFMPEG_CONTENT)
  await writeFile(join(pkgDir, 'doc', 'readme.txt'), README_CONTENT)
})

afterEach(async () => {
  await rm(dir, { recursive: true, force: true })
})

function buildZip() {
  const archivePath = join(dir, 'archive.zip')
  execFileSync('zip', ['-r', archivePath, 'pkg'], { cwd: dir })
  return archivePath
}

function buildTarXz() {
  const archivePath = join(dir, 'archive.tar.xz')
  execFileSync('tar', ['-cJf', archivePath, 'pkg'], { cwd: dir })
  return archivePath
}

describe.each([
  ['zip', buildZip],
  ['tar.xz', buildTarXz],
])('archive type %s', (type, buildArchive) => {
  it('lists only files, not directories', async () => {
    const archivePath = buildArchive()

    const members = await listMembers(type, archivePath)

    expect(members).toContain('pkg/bin/ffmpeg')
    expect(members).toContain('pkg/doc/readme.txt')
    expect(members.some((m) => m.endsWith('/'))).toBe(false)
  })

  it('extracts the single member matching the requested basename', async () => {
    const archivePath = buildArchive()
    const dest = join(dir, 'extracted-ffmpeg')

    await extractMember(type, archivePath, 'ffmpeg', dest)

    await expect(readFile(dest, 'utf8')).resolves.toBe(FFMPEG_CONTENT)
  })

  it('throws when no member matches the requested basename', async () => {
    const archivePath = buildArchive()
    const dest = join(dir, 'nope')

    await expect(extractMember(type, archivePath, 'does-not-exist', dest)).rejects.toThrow(
      /no member with that basename found/,
    )
  })

  it('throws when more than one member matches the requested basename', async () => {
    await mkdir(join(pkgDir, 'extra'), { recursive: true })
    await writeFile(join(pkgDir, 'extra', 'ffmpeg'), 'a different ffmpeg\n')
    const archivePath = buildArchive()
    const dest = join(dir, 'ambiguous')

    await expect(extractMember(type, archivePath, 'ffmpeg', dest)).rejects.toThrow(/ambiguous, 2 members match/)
  })
})
