#!/usr/bin/env node
// Скрипт доставки sidecar-бинарников (yt-dlp, ffmpeg) в src-tauri/binaries/
// по файлу пина версия+SHA256 (Ф-10, Р-1, Р-2 эпика E1, задача TL-6).
//
// Использование:
//   node scripts/fetch-binaries/index.mjs                  # только хост-тройка
//   node scripts/fetch-binaries/index.mjs --target all      # все 4 тройки
//   node scripts/fetch-binaries/index.mjs --target x86_64-unknown-linux-gnu
//   node scripts/fetch-binaries/index.mjs --pin path/to/pin.json --out path/to/dir
//
// Источники и обоснование выбора — см. комментарии в src-tauri/binaries.lock.json.

import { fileURLToPath } from 'node:url'
import { dirname, join, resolve } from 'node:path'
import { mkdir } from 'node:fs/promises'

import { installBinary } from './install.mjs'
import { loadPin } from './pin.mjs'
import { KNOWN_TARGETS, resolveHostTarget } from './targets.mjs'

const __dirname = dirname(fileURLToPath(import.meta.url))
const REPO_ROOT = resolve(__dirname, '..', '..')
const DEFAULT_PIN_PATH = join(REPO_ROOT, 'src-tauri', 'binaries.lock.json')
const DEFAULT_OUT_DIR = join(REPO_ROOT, 'src-tauri', 'binaries')

/**
 * @param {string[]} argv `process.argv.slice(2)`
 */
export function parseArgs(argv) {
  const targets = []
  let pinPath = DEFAULT_PIN_PATH
  let outDir = DEFAULT_OUT_DIR

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--target') {
      const value = argv[i + 1]
      if (!value) throw new Error('--target requires a value')
      i += 1
      if (value === 'all') {
        targets.push(...KNOWN_TARGETS)
      } else {
        targets.push(value)
      }
      continue
    }
    if (arg === '--pin') {
      const value = argv[i + 1]
      if (!value) throw new Error('--pin requires a value')
      i += 1
      pinPath = resolve(value)
      continue
    }
    if (arg === '--out') {
      const value = argv[i + 1]
      if (!value) throw new Error('--out requires a value')
      i += 1
      outDir = resolve(value)
      continue
    }
    throw new Error(`unknown argument: ${arg}`)
  }

  const uniqueTargets = [...new Set(targets.length > 0 ? targets : [resolveHostTarget()])]
  for (const target of uniqueTargets) {
    if (!KNOWN_TARGETS.includes(target)) {
      throw new Error(`unknown target triple: ${target} (known: ${KNOWN_TARGETS.join(', ')})`)
    }
  }

  return { targets: uniqueTargets, pinPath, outDir }
}

/**
 * @param {{ targets: string[]; pinPath: string; outDir: string }} options
 */
export async function run({ targets, pinPath, outDir }) {
  const pin = await loadPin(pinPath)
  await mkdir(outDir, { recursive: true })

  /** @type {Array<{ label: string; ok: boolean; detail: string }>} */
  const results = []

  for (const target of targets) {
    for (const binaryName of ['ytDlp', 'ffmpeg']) {
      const entry = pin[binaryName].targets[target]
      const label = `${binaryName} (${target})`
      try {
        // Загрузка последовательная (не Promise.all) намеренно: экономим
        // сеть/диск, не соревнуемся за пропускную способность между
        // параллельными закачками больших бинарников.
        const finalPath = await installBinary(entry, outDir)
        results.push({ label, ok: true, detail: finalPath })
      } catch (err) {
        results.push({ label, ok: false, detail: err.message })
      }
    }
  }

  for (const result of results) {
    if (result.ok) {
      console.log(`OK    ${result.label} -> ${result.detail}`)
    } else {
      console.error(`ERROR ${result.label}: ${result.detail}`)
    }
  }

  const failed = results.filter((result) => !result.ok)
  if (failed.length > 0) {
    throw new Error(`${failed.length}/${results.length} binaries failed to install`)
  }
}

const isMainModule = process.argv[1] === fileURLToPath(import.meta.url)
if (isMainModule) {
  try {
    const options = parseArgs(process.argv.slice(2))
    await run(options)
  } catch (err) {
    console.error(`fetch-binaries: ${err.message}`)
    process.exitCode = 1
  }
}
