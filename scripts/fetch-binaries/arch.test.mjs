import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import { expectedExecutable, readExecutableHeader, verifyExecutableArch } from './arch.mjs'
import { KNOWN_TARGETS } from './targets.mjs'
import { elf, machoFat, machoThin, nativeHeaderFor, pe } from './test-headers.mjs'

let dir

beforeEach(async () => {
  dir = await mkdtemp(join(tmpdir(), 'fetch-binaries-arch-test-'))
})

afterEach(async () => {
  await rm(dir, { recursive: true, force: true })
})

async function fileWith(bytes, name = 'bin') {
  const path = join(dir, name)
  await writeFile(path, bytes)
  return path
}

describe('expectedExecutable', () => {
  it('derives format and architecture for every known target', () => {
    expect(Object.fromEntries(KNOWN_TARGETS.map((t) => [t, expectedExecutable(t)]))).toStrictEqual({
      'x86_64-pc-windows-msvc': { format: 'pe', arch: 'x86_64' },
      'x86_64-apple-darwin': { format: 'mach-o', arch: 'x86_64' },
      'aarch64-apple-darwin': { format: 'mach-o', arch: 'aarch64' },
      'x86_64-unknown-linux-gnu': { format: 'elf', arch: 'x86_64' },
    })
  })

  it('refuses a triple it cannot interpret instead of guessing', () => {
    expect(() => expectedExecutable('sparc-sun-solaris')).toThrow(/cannot derive/)
    expect(() => expectedExecutable('x86_64-unknown-freebsd')).toThrow(/cannot derive/)
  })
})

describe('readExecutableHeader', () => {
  it('reads a thin Mach-O', async () => {
    await expect(readExecutableHeader(await fileWith(machoThin('aarch64')))).resolves.toStrictEqual({
      format: 'mach-o',
      archs: ['aarch64'],
    })
  })

  it('reads every slice of a fat Mach-O', async () => {
    await expect(
      readExecutableHeader(await fileWith(machoFat(['x86_64', 'aarch64']))),
    ).resolves.toStrictEqual({ format: 'mach-o', archs: ['x86_64', 'aarch64'] })
  })

  it('reads PE Machine through e_lfanew', async () => {
    await expect(readExecutableHeader(await fileWith(pe('x86_64')))).resolves.toStrictEqual({
      format: 'pe',
      archs: ['x86_64'],
    })
  })

  it('reads ELF e_machine', async () => {
    await expect(readExecutableHeader(await fileWith(elf('aarch64')))).resolves.toStrictEqual({
      format: 'elf',
      archs: ['aarch64'],
    })
  })

  it('rejects the text stub from scripts/ci/stub-binaries.mjs as not an executable', async () => {
    const stub = 'tube-leak CI stub, not a real binary (scripts/ci/stub-binaries.mjs)\n'
    await expect(readExecutableHeader(await fileWith(stub))).rejects.toThrow(
      /not a recognised executable.*unknown magic/,
    )
  })

  it('rejects an empty or tiny file', async () => {
    await expect(readExecutableHeader(await fileWith(''))).rejects.toThrow(/only 0 bytes long/)
  })

  it('does not mistake a Java class file (same 0xcafebabe magic) for a fat Mach-O', async () => {
    const classFile = Buffer.alloc(64)
    classFile.writeUInt32BE(0xcafebabe, 0)
    classFile.writeUInt16BE(0, 4) // minor
    classFile.writeUInt16BE(65, 6) // major (Java 21)
    await expect(readExecutableHeader(await fileWith(classFile))).rejects.toThrow(
      /fat header declares 65 architectures/,
    )
  })

  it('rejects a fat header whose table of contents lies about a slice', async () => {
    const lying = machoFat(['x86_64'], { lie: { index: 0, arch: 'aarch64' } })
    await expect(readExecutableHeader(await fileWith(lying))).rejects.toThrow(
      /fat slice 0 .* does not match its header entry/,
    )
  })

  it('rejects an MZ file without a PE signature', async () => {
    await expect(readExecutableHeader(await fileWith(pe('x86_64', { signature: false })))).rejects.toThrow(
      /no PE signature/,
    )
  })
})

describe('verifyExecutableArch — own header accepted, every foreign one refused', () => {
  const candidates = {
    'mach-o x86_64': machoThin('x86_64'),
    'mach-o aarch64': machoThin('aarch64'),
    'mach-o x86': machoThin('x86'),
    'pe x86_64': pe('x86_64'),
    'pe aarch64': pe('aarch64'),
    'pe x86': pe('x86'),
    'elf x86_64': elf('x86_64'),
    'elf aarch64': elf('aarch64'),
    'elf x86': elf('x86'),
  }

  for (const target of KNOWN_TARGETS) {
    const { format, arch } = expectedExecutable(target)
    const own = `${format} ${arch}`

    it(`${target}: accepts ${own}`, async () => {
      await expect(verifyExecutableArch(await fileWith(nativeHeaderFor(target)), target)).resolves.toBeDefined()
    })

    for (const [name, bytes] of Object.entries(candidates)) {
      if (name === own) continue
      it(`${target}: refuses ${name}, naming expected and actual`, async () => {
        const path = await fileWith(bytes)
        await expect(verifyExecutableArch(path, target, 'deno-under-test')).rejects.toThrow(
          new RegExp(`architecture mismatch for deno-under-test: target ${target} expects ${own}, got ${name}`),
        )
      })
    }
  }

  it('accepts a universal2 Mach-O for both macOS targets (yt-dlp_macos)', async () => {
    const path = await fileWith(machoFat(['x86_64', 'aarch64']))
    await expect(verifyExecutableArch(path, 'x86_64-apple-darwin')).resolves.toBeDefined()
    await expect(verifyExecutableArch(path, 'aarch64-apple-darwin')).resolves.toBeDefined()
  })

  it('refuses a fat Mach-O that lacks the target architecture', async () => {
    const path = await fileWith(machoFat(['x86_64', 'x86']))
    await expect(verifyExecutableArch(path, 'aarch64-apple-darwin')).rejects.toThrow(
      /expects mach-o aarch64, got mach-o x86_64 \+ x86/,
    )
  })

  it('refuses an x32-ABI ELF (x86_64 machine, 32-bit class) for the 64-bit Linux target', async () => {
    const path = await fileWith(elf('x86_64', { elfClass: 1 }))
    await expect(verifyExecutableArch(path, 'x86_64-unknown-linux-gnu')).rejects.toThrow(
      /got elf x86_64 \(ELFCLASS32\)/,
    )
  })
})
