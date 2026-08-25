import { createHash } from 'node:crypto'
import { access, mkdtemp, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import { downloadToFile, sha256File } from './download.mjs'

let dir

beforeEach(async () => {
  dir = await mkdtemp(join(tmpdir(), 'fetch-binaries-download-test-'))
})

afterEach(async () => {
  await rm(dir, { recursive: true, force: true })
})

async function exists(path) {
  try {
    await access(path)
    return true
  } catch {
    return false
  }
}

describe('downloadToFile', () => {
  it('writes the response body to the destination file', async () => {
    const dest = join(dir, 'out.bin')
    const fetchImpl = async () => new Response('hello world')

    await downloadToFile('https://example.invalid/asset', dest, { fetchImpl })

    await expect(readFile(dest, 'utf8')).resolves.toBe('hello world')
  })

  it('creates parent directories that do not exist yet', async () => {
    const dest = join(dir, 'nested', 'deeper', 'out.bin')
    const fetchImpl = async () => new Response('content')

    await downloadToFile('https://example.invalid/asset', dest, { fetchImpl })

    await expect(readFile(dest, 'utf8')).resolves.toBe('content')
  })

  it('rejects and leaves no file behind on a non-OK HTTP status', async () => {
    const dest = join(dir, 'out.bin')
    const fetchImpl = async () => new Response('not found', { status: 404 })

    await expect(downloadToFile('https://example.invalid/missing', dest, { fetchImpl })).rejects.toThrow(
      /unexpected HTTP status 404/,
    )
    await expect(exists(dest)).resolves.toBe(false)
  })

  it('rejects when the network request itself fails', async () => {
    const dest = join(dir, 'out.bin')
    const fetchImpl = async () => {
      throw new Error('DNS resolution failed')
    }

    await expect(downloadToFile('https://example.invalid/asset', dest, { fetchImpl })).rejects.toThrow(
      /network request failed: DNS resolution failed/,
    )
    await expect(exists(dest)).resolves.toBe(false)
  })

  it('removes a partially written file if the body stream errors mid-transfer', async () => {
    const dest = join(dir, 'out.bin')
    const body = new ReadableStream({
      start(controller) {
        controller.enqueue(new TextEncoder().encode('partial-chunk-'))
      },
      pull(controller) {
        controller.error(new Error('connection reset'))
      },
    })
    const fetchImpl = async () => new Response(body)

    await expect(downloadToFile('https://example.invalid/asset', dest, { fetchImpl })).rejects.toThrow(
      /writing to .*out\.bin failed/,
    )
    await expect(exists(dest)).resolves.toBe(false)
  })
})

describe('sha256File', () => {
  it('computes the correct sha256 for known content', async () => {
    const dest = join(dir, 'known.bin')
    const content = 'the quick brown fox'
    await downloadToFile('https://example.invalid/asset', dest, {
      fetchImpl: async () => new Response(content),
    })
    const expected = createHash('sha256').update(content).digest('hex')

    await expect(sha256File(dest)).resolves.toBe(expected)
  })

  it('rejects with context when the file does not exist', async () => {
    await expect(sha256File(join(dir, 'missing.bin'))).rejects.toThrow(/hashing/)
  })
})
