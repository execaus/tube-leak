// Известные тройки таргетов, под которые доставляются sidecar-бинарники
// (Tauri `externalBin`, см. src-tauri/tauri.conf.json и CLAUDE.md/Структура).
export const KNOWN_TARGETS = Object.freeze([
  'x86_64-pc-windows-msvc',
  'x86_64-apple-darwin',
  'aarch64-apple-darwin',
  'x86_64-unknown-linux-gnu',
])

/**
 * Определяет тройку таргета хост-машины по `process.platform`/`process.arch`.
 * Бросает ошибку на неподдерживаемой комбинации вместо угадывания —
 * лучше явный отказ, чем скачивание бинарника не под ту платформу.
 *
 * @returns {string} одна из {@link KNOWN_TARGETS}
 */
export function resolveHostTarget() {
  const { platform, arch } = process

  if (platform === 'darwin') {
    if (arch === 'arm64') return 'aarch64-apple-darwin'
    if (arch === 'x64') return 'x86_64-apple-darwin'
    throw new Error(`unsupported host architecture on macOS: ${arch}`)
  }

  if (platform === 'linux') {
    if (arch === 'x64') return 'x86_64-unknown-linux-gnu'
    throw new Error(`unsupported host architecture on Linux: ${arch}`)
  }

  if (platform === 'win32') {
    if (arch === 'x64') return 'x86_64-pc-windows-msvc'
    throw new Error(`unsupported host architecture on Windows: ${arch}`)
  }

  throw new Error(`unsupported host platform: ${platform}`)
}
