#!/usr/bin/env node
// Кладёт в src-tauri/binaries/ файлы-заглушки под именами из пина (TL-25).
//
// Зачем это нужно вместо настоящей доставки:
//
// `cargo test` собирает тестовый бинарник вместе с build.rs, а тот вызывает
// `tauri_build::build()`. Tauri на этом шаге копирует в target/ всё, что
// перечислено в `bundle.externalBin` и `bundle.resources`, и падает
// (`ResourcePathNotFound`), если файла нет. Поэтому просто «собрать тесты в
// свежем клоне» нельзя: в репозитории нет ни binaries/, ни resources/.
//
// Настоящую доставку в тестовом джобе гоняли бы впустую: ни один тест не
// запускает yt-dlp или ffmpeg — в фикстурах используются собственные
// sh-скрипты (см. src-tauri/src/sidecar, src-tauri/src/commands); deno не
// запускает никто вовсе. Зато
// джоб получил бы ~150 МиБ трафика на прогон и зависимость от чужих
// хостингов ассетов: у linux-сборки ffmpeg (BtbN) тег релиза по политике
// ретенции живёт не вечно (см. _note в binaries.lock.json). Красный
// тестовый джоб из-за протухшего чужого ассета обесценивает сам джоб —
// его перестают читать. Настоящая доставка проверяется джобом build,
// который и собирает дистрибутив, а логика самого скрипта доставки —
// юнит-тестами scripts/fetch-binaries/*.test.mjs в `npm test`.
//
// Имена файлов берутся из того же пина, что и у настоящей доставки, —
// переименование в пине не разъедется с заглушками молча.
//
// Заглушки архива yt-dlp и deno не проходят сверку с пином, которую делает
// src-tauri/build.rs (yt-dlp — sha256, TL-25; deno — binarySha256, TL-112):
// иначе они молча уехали бы в бандл, собранный на этой же машине следом за
// тестами, — yt-dlp развалился бы у пользователя на распаковке, а вместо
// deno в Contents/MacOS лежал бы этот текст. Поэтому cargo после этого скрипта нужно
// запускать с TUBE_LEAK_ALLOW_STUB_YTDLP=1 — переменная действует только
// вне профиля release, то есть `npm run tauri build` ею не открыть.
//
// Использование:
//   node scripts/ci/stub-binaries.mjs                 # хост-тройка
//   node scripts/ci/stub-binaries.mjs --target x86_64-unknown-linux-gnu
//   TUBE_LEAK_ALLOW_STUB_YTDLP=1 cargo test           # дальше — так

import { fileURLToPath } from 'node:url'
import { dirname, join, resolve } from 'node:path'
import { mkdir, writeFile, access } from 'node:fs/promises'

import { BINARY_NAMES, loadPin } from '../fetch-binaries/pin.mjs'
import { KNOWN_TARGETS, resolveHostTarget } from '../fetch-binaries/targets.mjs'

const __dirname = dirname(fileURLToPath(import.meta.url))
const REPO_ROOT = resolve(__dirname, '..', '..')
const PIN_PATH = join(REPO_ROOT, 'src-tauri', 'binaries.lock.json')
const OUT_DIR = join(REPO_ROOT, 'src-tauri', 'binaries')

// Содержимое заглушки. Текст, а не пустой файл: объясняет находку тому,
// кто наткнётся на такой файл в src-tauri/binaries/ или в target/.
// Содержимое роли не играет — build.rs сверяет архив yt-dlp и бинарник deno
// с суммами из пина, и заглушка не пройдёт сверку при любом наполнении
// (ffmpeg не сверяется: суммы распакованного его сборщики не публикуют).
const STUB_CONTENT =
  'tube-leak CI stub, not a real binary (scripts/ci/stub-binaries.mjs)\n'

async function exists(path) {
  try {
    await access(path)
    return true
  } catch {
    return false
  }
}

async function main() {
  const argv = process.argv.slice(2)
  let target = null

  for (let i = 0; i < argv.length; i += 1) {
    if (argv[i] === '--target') {
      target = argv[i + 1]
      if (!target) throw new Error('--target requires a value')
      i += 1
      continue
    }
    throw new Error(`unknown argument: ${argv[i]}`)
  }

  target ??= resolveHostTarget()
  if (!KNOWN_TARGETS.includes(target)) {
    throw new Error(`unknown target triple: ${target} (known: ${KNOWN_TARGETS.join(', ')})`)
  }

  const pin = await loadPin(PIN_PATH)
  await mkdir(OUT_DIR, { recursive: true })

  let stubbed = false

  for (const binaryName of BINARY_NAMES) {
    const { binaryName: fileName } = pin[binaryName].targets[target]
    const path = join(OUT_DIR, fileName)

    // Настоящий бинарник не затирается: скрипт безопасно запустить на
    // машине разработчика, где binaries/ уже наполнен доставкой.
    if (await exists(path)) {
      console.log(`SKIP  ${fileName} (уже есть)`)
      continue
    }

    await writeFile(path, STUB_CONTENT, { mode: 0o755 })
    console.log(`STUB  ${fileName}`)
    stubbed = true
  }

  if (stubbed) {
    console.log(
      '\nЗаглушки не совпадают с sha256 из пина, и src-tauri/build.rs это проверяет.\n' +
        'Дальше запускайте cargo с TUBE_LEAK_ALLOW_STUB_YTDLP=1, например:\n' +
        '  TUBE_LEAK_ALLOW_STUB_YTDLP=1 cargo test --manifest-path src-tauri/Cargo.toml\n' +
        'В профиле release переменная не действует: бандл с заглушкой собрать нельзя.',
    )
  }
}

try {
  await main()
} catch (err) {
  console.error(`stub-binaries: ${err.message}`)
  process.exitCode = 1
}
