import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
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

// TL-112: мутации ревью TL-108, которые прежние тесты пропускали (каждая
// давала зелёный прогон). Тесты сверяют точное имя архитектуры, а не только
// факт отказа: подмена разбора на «неизвестную архитектуру» тоже отказ, и
// по одному отказу её не отличить.
describe('readExecutableHeader — the details a mutation used to slip past', () => {
  it('marks a 64-bit CPU type under a 32-bit Mach-O magic, and the target refuses it', async () => {
    const path = await fileWith(machoThin('aarch64', { bits: 32 }))
    await expect(readExecutableHeader(path)).resolves.toStrictEqual({
      format: 'mach-o',
      archs: ['aarch64 (32-bit header)'],
    })
    await expect(verifyExecutableArch(path, 'aarch64-apple-darwin')).rejects.toThrow(
      /expects mach-o aarch64, got mach-o aarch64 \(32-bit header\)$/,
    )
  })

  it('does not take arm64_32 (watchOS ILP32) for aarch64', async () => {
    const path = await fileWith(machoThin('arm64_32', { bits: 32 }))
    await expect(readExecutableHeader(path)).resolves.toStrictEqual({ format: 'mach-o', archs: ['arm64_32'] })
    await expect(verifyExecutableArch(path, 'aarch64-apple-darwin')).rejects.toThrow(
      /expects mach-o aarch64, got mach-o arm64_32$/,
    )
  })

  it('reads ELF e_machine in the byte order EI_DATA declares', async () => {
    // 183 (aarch64) в обратном порядке байт — 0xb700, то есть разбор не в
    // том порядке дал бы «unknown», а не случайное совпадение.
    await expect(readExecutableHeader(await fileWith(elf('aarch64', { endian: 'big' })))).resolves.toStrictEqual({
      format: 'elf',
      archs: ['aarch64'],
    })
    await expect(
      readExecutableHeader(await fileWith(elf('powerpc64', { endian: 'big' }), 'ppc64')),
    ).resolves.toStrictEqual({ format: 'elf', archs: ['powerpc64'] })
  })

  it('refuses a big-endian ELF for the Linux target, naming its real machine', async () => {
    const path = await fileWith(elf('powerpc64', { endian: 'big' }))
    await expect(verifyExecutableArch(path, 'x86_64-unknown-linux-gnu')).rejects.toThrow(
      /expects elf x86_64, got elf powerpc64$/,
    )
  })

  it('refuses a big-endian Mach-O for both macOS targets, naming its real CPU', async () => {
    const path = await fileWith(machoThin('powerpc64', { endian: 'big' }))
    await expect(readExecutableHeader(path)).resolves.toStrictEqual({ format: 'mach-o', archs: ['powerpc64'] })
    for (const target of ['x86_64-apple-darwin', 'aarch64-apple-darwin']) {
      await expect(verifyExecutableArch(path, target)).rejects.toThrow(/got mach-o powerpc64$/)
    }
  })

  it('refuses a degenerate 8-byte Mach-O (magic and cputype, no header)', async () => {
    await expect(readExecutableHeader(await fileWith(machoThin('aarch64').subarray(0, 8)))).rejects.toThrow(
      /truncated Mach-O header \(8 of 32 bytes\)/,
    )
    await expect(
      readExecutableHeader(await fileWith(machoThin('aarch64', { bits: 32 }).subarray(0, 27), 'short32')),
    ).rejects.toThrow(/truncated Mach-O header \(27 of 28 bytes\)/)
    // Граница: ровно полный mach_header_64 — уже заголовок.
    await expect(
      readExecutableHeader(await fileWith(machoThin('aarch64').subarray(0, 32), 'exact')),
    ).resolves.toStrictEqual({ format: 'mach-o', archs: ['aarch64'] })
  })

  it('names the path when a directory stands where the executable should be', async () => {
    const path = join(dir, 'deno-aarch64-apple-darwin')
    await mkdir(path)
    await expect(readExecutableHeader(path)).rejects.toThrow(`reading executable header of ${path}:`)
  })

  it('promises nothing about files left in place — only installBinary can', async () => {
    const err = await verifyExecutableArch(await fileWith(machoThin('x86_64')), 'aarch64-apple-darwin').catch((e) => e)
    expect(err).toBeInstanceOf(Error)
    expect(err.message).toMatch(/^architecture mismatch for .*got mach-o x86_64$/)
    expect(err.message).not.toMatch(/no file left in place/)
  })
})
