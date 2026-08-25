/**
 * TS-зеркало Rust-контракта из `src-tauri/src/types.rs` (TL-1).
 *
 * Поля и значения enum-строк должны совпадать один в один с Rust-стороной
 * (сериализация там — `serde(rename_all = "camelCase")`). Расхождение с
 * `types.rs` — баг, а не «улучшение»: изменения контракта делаются сначала
 * в Rust, это зеркало обновляется следом.
 */

/** Итог попытки проверить один sidecar-бинарник. */
export type SidecarStatus = 'ok' | 'notFound' | 'launchFailed' | 'nonZeroExit' | 'timeout'

/** Причина отказа запуска, применима только при `status === 'launchFailed'`. */
export type LaunchFailedReason = 'permissionDenied' | 'corrupted' | 'other'

/**
 * Результат проверки одного sidecar-бинарника (yt-dlp или ffmpeg).
 *
 * Поля, специфичные для конкретного `status`, присутствуют только когда
 * заполнены на Rust-стороне (`version` — при `ok`, `reason` — при
 * `launchFailed`, `exitCode` — при `nonZeroExit`, `timeoutMs` — при
 * `timeout`); остальные диагностические поля опциональны независимо от
 * статуса.
 */
export interface SidecarCheckResult {
  name: string
  path: string
  status: SidecarStatus
  version?: string
  reason?: LaunchFailedReason
  exitCode?: number
  osErrorCode?: string
  stderrTail?: string
  timeoutMs?: number
  checkedAt?: string
  durationMs?: number
}

/**
 * Агрегат результатов проверки обоих sidecar-бинарников, возвращаемый
 * командой `check_sidecar` (Ф-9 эпика E1).
 */
export interface SidecarCheckReport {
  ytDlp: SidecarCheckResult
  ffmpeg: SidecarCheckResult
}
