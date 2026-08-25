/**
 * TS-зеркало Rust-контракта подготовки yt-dlp из `src-tauri/src/types.rs`
 * (секция «подготовка yt-dlp (TL-12)», TL-17).
 *
 * Поля и значения enum-строк должны совпадать один в один с Rust-стороной
 * (сериализация там — `serde(rename_all = "camelCase")`). Расхождение с
 * `types.rs` — баг, а не «улучшение»: изменения контракта делаются сначала
 * в Rust, это зеркало обновляется следом.
 */

/**
 * Этап подготовки yt-dlp, отображаемый пользователю.
 *
 * `unpacking` — раскладка вложенного onedir-архива в каталог данных;
 * `warmingUp` — прогон распакованного дерева, чтобы ОС проверила подписи
 * всех его файлов (это и есть тот самый долгий шаг ~35 с); `ready` и
 * `failed` — терминальные состояния.
 */
export type YtDlpPrepareStage = 'unpacking' | 'warmingUp' | 'ready' | 'failed'

/**
 * Типизированная причина отказа подготовки (CLAUDE.md, «Ошибки
 * типизированные, не строки»): по `kind` фронтенд решает, что предложить
 * пользователю, `message` — диагностика, не для решения.
 */
export type YtDlpPrepareErrorKind =
  | 'dataDirUnavailable'
  | 'archiveMissing'
  | 'archiveCorrupted'
  | 'unpackFailed'
  | 'layoutUnexpected'
  | 'warmupFailed'

/** Отказ подготовки в сериализуемом виде. */
export interface YtDlpPrepareError {
  kind: YtDlpPrepareErrorKind
  message: string
}

/**
 * Событие хода подготовки yt-dlp, эмитится под именем `ytdlp://prepare`.
 *
 * `percent` — сквозной прогресс всей подготовки (0..100), а не прогресс
 * текущего этапа. `etaSecs` присутствует только когда заполнена на
 * Rust-стороне (не приходит как `null`); `version` — только при
 * `stage === 'ready'`; `error` — только при `stage === 'failed'`.
 */
export interface YtDlpPrepareEvent {
  stage: YtDlpPrepareStage
  percent: number
  etaSecs?: number
  version?: string
  error?: YtDlpPrepareError
}

/**
 * Итог подготовки, возвращаемый командой `prepare_ytdlp`.
 *
 * `prepared: false` означает, что делать ничего не потребовалось: дерево
 * уже лежало в каталоге данных и отозвалось за доли секунды — обычный
 * тёплый запуск, экран подготовки для него показывать не нужно.
 */
export interface YtDlpPrepared {
  version: string
  path: string
  prepared: boolean
  durationMs: number
}
