import { open } from 'node:fs/promises'

// Проверка архитектуры доставленного исполняемого файла (TL-108, долг #15).
//
// sha256 из пина доказывает только одно: скачан ровно тот файл, который
// человек вписал в пин. Что это файл нужной архитектуры, он не доказывает.
// TL-11 — ровно этот класс дефекта: ffmpeg для aarch64-apple-darwin был
// x86_64-сборкой, проходил сверку суммы и на Apple Silicon молча уходил в
// Rosetta. У deno macOS-ассеты раздельные по архитектуре, так что
// перепутанный адрес в пине воспроизвёл бы тот же дефект.
//
// Поэтому заголовок файла читается здесь, без внешних утилит (`lipo`,
// `file` есть не на каждой машине доставки), и сверяется с целевой тройкой
// записи пина. Разбираются три формата — ровно те, в которых приходят
// бинарники под известные тройки (targets.mjs): Mach-O (включая fat/
// universal), PE и ELF. Всё прочее — отказ, а не пропуск: неизвестный
// формат под именем исполняемого sidecar хуже любой известной архитектуры.

/** Сколько байт начала файла читается за один раз. Fat-заголовок на
 *  MAX_FAT_ARCHS записей по 32 байта и ELF/Mach-O-заголовок сюда
 *  помещаются с запасом; PE-заголовок дочитывается по смещению отдельно. */
const HEAD_BYTES = 4096

// Mach-O: magic в порядке байт самого файла.
const MH_MAGIC = 0xfeedface
const MH_MAGIC_64 = 0xfeedfacf
const MH_CIGAM = 0xcefaedfe
const MH_CIGAM_64 = 0xcffaedfe
// Fat-заголовок всегда big-endian.
const FAT_MAGIC = 0xcafebabe
const FAT_MAGIC_64 = 0xcafebabf
const FAT_ARCH_SIZE = 20
const FAT_ARCH_64_SIZE = 32
/**
 * Верхняя граница числа срезов fat-файла. Нужна не только как защита от
 * мусора: у Java class-файла тот же magic 0xcafebabe, а на месте nfat_arch
 * у него лежит версия формата (major ≥ 45). Граница ниже 45 отличает одно
 * от другого.
 */
const MAX_FAT_ARCHS = 32
const CPU_ARCH_ABI64 = 0x01000000

/** @type {ReadonlyMap<number, string>} cputype → имя архитектуры в терминах троек */
const MACHO_CPU_TYPES = new Map([
  [7, 'x86'],
  [7 | CPU_ARCH_ABI64, 'x86_64'],
  [12, 'arm'],
  [12 | CPU_ARCH_ABI64, 'aarch64'],
  [0x0200000c, 'arm64_32'],
  [18, 'powerpc'],
  [18 | CPU_ARCH_ABI64, 'powerpc64'],
])

/** @type {ReadonlyMap<number, string>} ELF e_machine → имя архитектуры */
const ELF_MACHINES = new Map([
  [3, 'x86'],
  [8, 'mips'],
  [20, 'powerpc'],
  [21, 'powerpc64'],
  [40, 'arm'],
  [62, 'x86_64'],
  [183, 'aarch64'],
  [243, 'riscv'],
])

/** @type {ReadonlyMap<number, string>} PE Machine → имя архитектуры */
const PE_MACHINES = new Map([
  [0x014c, 'x86'],
  [0x01c0, 'arm'],
  [0x01c4, 'arm'],
  [0x8664, 'x86_64'],
  [0xaa64, 'aarch64'],
])

/** Архитектуры, у которых ELF-класс обязан быть 64-битным. */
const ARCHS_64 = new Set(['x86_64', 'aarch64', 'powerpc64'])

/**
 * Ожидаемые формат и архитектура исполняемого файла для тройки.
 *
 * Выводится из самой тройки, а не таблицей «тройка → ожидание»: таблица
 * была бы второй правдой рядом с KNOWN_TARGETS и молча не покрыла бы
 * добавленную тройку. Незнакомая ОС или архитектура — отказ.
 *
 * @param {string} target
 * @returns {{ format: 'mach-o' | 'pe' | 'elf'; arch: string }}
 */
export function expectedExecutable(target) {
  const [arch] = target.split('-')
  if (arch !== 'x86_64' && arch !== 'aarch64') {
    throw new Error(`cannot derive the expected executable architecture from target triple ${target}`)
  }

  if (target.endsWith('-apple-darwin')) return { format: 'mach-o', arch }
  if (target.includes('-windows-')) return { format: 'pe', arch }
  if (target.includes('-linux-')) return { format: 'elf', arch }

  throw new Error(`cannot derive the expected executable format from target triple ${target}`)
}

/**
 * Читает заголовок исполняемого файла и возвращает его формат и список
 * архитектур (у fat Mach-O их несколько, у остальных — одна).
 *
 * @param {string} filePath
 * @returns {Promise<{ format: 'mach-o' | 'pe' | 'elf'; archs: string[] }>}
 */
export async function readExecutableHeader(filePath) {
  const handle = await open(filePath, 'r').catch((err) => {
    throw new Error(`reading executable header of ${filePath}: ${err.message}`, { cause: err })
  })
  try {
    const head = Buffer.alloc(HEAD_BYTES)
    const { bytesRead } = await handle.read(head, 0, HEAD_BYTES, 0)
    const bytes = head.subarray(0, bytesRead)
    const readAt = async (position, length) => {
      const buffer = Buffer.alloc(length)
      const { bytesRead: got } = await handle.read(buffer, 0, length, position)
      return buffer.subarray(0, got)
    }
    return await parseHeader(bytes, readAt, filePath)
  } finally {
    await handle.close()
  }
}

/**
 * @param {Buffer} bytes начало файла
 * @param {(position: number, length: number) => Promise<Buffer>} readAt
 * @param {string} filePath только для сообщений
 */
async function parseHeader(bytes, readAt, filePath) {
  const unrecognised = (why) =>
    new Error(
      `${filePath} is not a recognised executable (Mach-O, PE or ELF): ${why}; first bytes: ${
        bytes.subarray(0, 16).toString('hex') || '<empty file>'
      }`,
    )

  if (bytes.length < 8) throw unrecognised(`only ${bytes.length} bytes long`)

  // ELF
  if (bytes[0] === 0x7f && bytes[1] === 0x45 && bytes[2] === 0x4c && bytes[3] === 0x46) {
    if (bytes.length < 20) throw unrecognised('truncated ELF header')
    const elfClass = bytes[4]
    const elfData = bytes[5]
    if (elfClass !== 1 && elfClass !== 2) throw unrecognised(`unknown ELF class ${elfClass}`)
    if (elfData !== 1 && elfData !== 2) throw unrecognised(`unknown ELF data encoding ${elfData}`)
    const machine = elfData === 1 ? bytes.readUInt16LE(18) : bytes.readUInt16BE(18)
    let arch = ELF_MACHINES.get(machine) ?? `unknown (e_machine ${machine})`
    // x32 ABI: e_machine x86_64 при 32-битном классе — под x86_64-тройку
    // такой бинарник не годится.
    if (ARCHS_64.has(arch) && elfClass !== 2) arch = `${arch} (ELFCLASS32)`
    return { format: 'elf', archs: [arch] }
  }

  // PE: MZ-заглушка DOS, по смещению e_lfanew — сигнатура PE\0\0 и Machine.
  if (bytes[0] === 0x4d && bytes[1] === 0x5a) {
    if (bytes.length < 0x40) throw unrecognised('truncated DOS header')
    const peOffset = bytes.readUInt32LE(0x3c)
    const pe = await readAt(peOffset, 6)
    if (pe.length < 6 || pe.readUInt32BE(0) !== 0x50450000) {
      throw unrecognised(`no PE signature at e_lfanew 0x${peOffset.toString(16)}`)
    }
    const machine = pe.readUInt16LE(4)
    return {
      format: 'pe',
      archs: [PE_MACHINES.get(machine) ?? `unknown (Machine 0x${machine.toString(16)})`],
    }
  }

  // Fat/universal Mach-O
  const magicBE = bytes.readUInt32BE(0)
  if (magicBE === FAT_MAGIC || magicBE === FAT_MAGIC_64) {
    const count = bytes.readUInt32BE(4)
    if (count === 0 || count > MAX_FAT_ARCHS) {
      throw unrecognised(`fat header declares ${count} architectures`)
    }
    const entrySize = magicBE === FAT_MAGIC_64 ? FAT_ARCH_64_SIZE : FAT_ARCH_SIZE
    if (bytes.length < 8 + count * entrySize) throw unrecognised('truncated fat header')

    const archs = []
    for (let i = 0; i < count; i += 1) {
      const base = 8 + i * entrySize
      const cputype = bytes.readUInt32BE(base)
      const offset =
        entrySize === FAT_ARCH_64_SIZE
          ? Number(bytes.readBigUInt64BE(base + 8))
          : bytes.readUInt32BE(base + 8)
      // Срез сверяется со своей записью в fat-заголовке: заголовок, который
      // обещает arm64, а указывает на x86_64-срез, — не universal-файл, а
      // битый, и доверять его оглавлению нельзя.
      const slice = parseThinMachO(await readAt(offset, 8))
      if (slice === null || slice.cputype !== cputype) {
        throw unrecognised(
          `fat slice ${i} at offset ${offset} does not match its header entry (cputype 0x${cputype.toString(16)})`,
        )
      }
      archs.push(slice.arch)
    }
    return { format: 'mach-o', archs }
  }

  const thin = parseThinMachO(bytes)
  if (thin !== null) return { format: 'mach-o', archs: [thin.arch] }

  throw unrecognised('unknown magic')
}

/**
 * @param {Buffer} bytes не меньше 8 байт начала среза
 * @returns {{ cputype: number; arch: string } | null}
 */
function parseThinMachO(bytes) {
  if (bytes.length < 8) return null
  const magic = bytes.readUInt32LE(0)
  let cputype
  let is64Header
  if (magic === MH_MAGIC || magic === MH_MAGIC_64) {
    cputype = bytes.readUInt32LE(4)
    is64Header = magic === MH_MAGIC_64
  } else if (magic === MH_CIGAM || magic === MH_CIGAM_64) {
    cputype = bytes.readUInt32BE(4)
    is64Header = magic === MH_CIGAM_64
  } else {
    return null
  }
  let arch = MACHO_CPU_TYPES.get(cputype) ?? `unknown (cputype 0x${cputype.toString(16)})`
  if ((cputype & CPU_ARCH_ABI64) !== 0 && !is64Header) arch = `${arch} (32-bit header)`
  return { cputype, arch }
}

/**
 * Сверяет формат и архитектуру файла с целевой тройкой. Бросает ошибку,
 * называющую и ожидаемое, и фактическое (критерий #15).
 *
 * У fat Mach-O достаточно, чтобы среди срезов была нужная архитектура:
 * universal2-сборка (yt-dlp_macos) законно обслуживает обе macOS-тройки.
 *
 * @param {string} filePath
 * @param {string} target
 * @param {string} [label] как назвать файл в сообщении (по умолчанию путь)
 */
export async function verifyExecutableArch(filePath, target, label = filePath) {
  const expected = expectedExecutable(target)
  const actual = await readExecutableHeader(filePath)

  if (actual.format !== expected.format || !actual.archs.includes(expected.arch)) {
    throw new Error(
      `architecture mismatch for ${label}: target ${target} expects ${expected.format} ${expected.arch}, ` +
        `got ${actual.format} ${actual.archs.join(' + ')} — aborting, no file left in place`,
    )
  }
  return actual
}
