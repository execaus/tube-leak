// Минимальные заголовки исполняемых файлов для тестов проверки архитектуры
// (arch.mjs). Не настоящие бинарники: ровно столько байт, сколько читает
// разбор, плюс немного хвоста. Не тест сам по себе — vitest подхватывает
// только *.test.mjs.

const MACHO_CPU = {
  x86: 7,
  arm: 12,
  x86_64: 0x01000007,
  aarch64: 0x0100000c,
  arm64_32: 0x0200000c,
  powerpc: 18,
  powerpc64: 0x01000012,
}
const ELF_MACHINE = { x86: 3, arm: 40, x86_64: 62, aarch64: 183, powerpc: 20, powerpc64: 21 }
const PE_MACHINE = { x86: 0x014c, arm: 0x01c4, x86_64: 0x8664, aarch64: 0xaa64 }

function known(table, arch) {
  const value = table[arch]
  if (value === undefined) throw new Error(`test-headers: no constant for ${arch}`)
  return value
}

/**
 * Thin Mach-O. По умолчанию 64-битный little-endian — как у всех
 * настоящих macOS-бинарников под наши тройки.
 *
 * @param {string} arch
 * @param {{ bits?: 32 | 64; endian?: 'little' | 'big' }} [options]
 *   `bits` — magic MH_MAGIC (32) или MH_MAGIC_64; `endian` — порядок байт
 *   всего заголовка (big — MH_CIGAM*, как у PowerPC-сборок).
 */
export function machoThin(arch, { bits = 64, endian = 'little' } = {}) {
  const bytes = Buffer.alloc(64)
  const magic = bits === 32 ? 0xfeedface : 0xfeedfacf
  if (endian === 'big') {
    bytes.writeUInt32BE(magic, 0)
    bytes.writeUInt32BE(known(MACHO_CPU, arch), 4)
  } else {
    bytes.writeUInt32LE(magic, 0)
    bytes.writeUInt32LE(known(MACHO_CPU, arch), 4)
  }
  return bytes
}

/**
 * Fat/universal Mach-O (32-битные fat_arch). `lie` — подменить cputype в
 * записи заголовка, не трогая сам срез: битое оглавление.
 *
 * @param {string[]} archs
 * @param {{ lie?: { index: number; arch: string } }} [options]
 */
export function machoFat(archs, { lie } = {}) {
  const sliceSize = 64
  const firstSlice = 4096
  const bytes = Buffer.alloc(firstSlice + archs.length * sliceSize)
  bytes.writeUInt32BE(0xcafebabe, 0)
  bytes.writeUInt32BE(archs.length, 4)
  archs.forEach((arch, i) => {
    const base = 8 + i * 20
    const offset = firstSlice + i * sliceSize
    const headerArch = lie && lie.index === i ? lie.arch : arch
    bytes.writeUInt32BE(known(MACHO_CPU, headerArch), base)
    bytes.writeUInt32BE(0, base + 4)
    bytes.writeUInt32BE(offset, base + 8)
    bytes.writeUInt32BE(sliceSize, base + 12)
    bytes.writeUInt32BE(12, base + 16)
    machoThin(arch).copy(bytes, offset)
  })
  return bytes
}

/**
 * ELF; `elfClass` 2 — 64 бит, 1 — 32 бит; `endian` — EI_DATA и порядок байт
 * полей заголовка.
 *
 * @param {string} arch
 * @param {{ elfClass?: 1 | 2; endian?: 'little' | 'big' }} [options]
 */
export function elf(arch, { elfClass = 2, endian = 'little' } = {}) {
  const bytes = Buffer.alloc(64)
  const big = endian === 'big'
  bytes.set([0x7f, 0x45, 0x4c, 0x46, elfClass, big ? 2 : 1, 1], 0)
  if (big) {
    bytes.writeUInt16BE(2, 16) // e_type ET_EXEC
    bytes.writeUInt16BE(known(ELF_MACHINE, arch), 18)
  } else {
    bytes.writeUInt16LE(2, 16)
    bytes.writeUInt16LE(known(ELF_MACHINE, arch), 18)
  }
  return bytes
}

/** PE: DOS-заглушка MZ, e_lfanew = 0x80, сигнатура PE\0\0 и Machine. */
export function pe(arch, { signature = true } = {}) {
  const bytes = Buffer.alloc(0x100)
  bytes.write('MZ', 0, 'latin1')
  bytes.writeUInt32LE(0x80, 0x3c)
  if (signature) bytes.write('PE\0\0', 0x80, 'latin1')
  bytes.writeUInt16LE(known(PE_MACHINE, arch), 0x84)
  return bytes
}

/** Заголовок, который доставка обязана принять для тройки. */
export function nativeHeaderFor(target) {
  const [arch] = target.split('-')
  if (target.endsWith('-apple-darwin')) return machoThin(arch)
  if (target.includes('-windows-')) return pe(arch)
  if (target.includes('-linux-')) return elf(arch)
  throw new Error(`test-headers: unknown target ${target}`)
}
