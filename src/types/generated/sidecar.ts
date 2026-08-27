// Файл СГЕНЕРИРОВАН из src-tauri/src/types.rs. Руками не править: правка
// живёт в Rust, сюда она приезжает перегенерацией.
//
//   перегенерация:  cd src-tauri && TUBE_LEAK_UPDATE_TS_BINDINGS=1 cargo test --locked
//   сторож:         cargo test падает, если этот файл разошёлся с types.rs (TL-51)
//
// Комментарии ниже — те же doc-комментарии, что стоят у типов в types.rs;
// расходиться с ними этот файл не может по построению.

/**
 * Причина отказа запуска, применима только при `status = launchFailed`.
 *
 * См. пояснение у [`SidecarStatus`] — варианты заполняются в TL-4/TL-5.
 */
export type LaunchFailedReason = "permissionDenied" | "corrupted" | "other";

/**
 * Агрегат результатов проверки обоих sidecar-бинарников, возвращаемый
 * командой `check_sidecar` (Ф-9 эпика E1).
 */
export type SidecarCheckReport = { ytDlp: SidecarCheckResult, ffmpeg: SidecarCheckResult, };

/**
 * Результат проверки одного sidecar-бинарника (yt-dlp или ffmpeg).
 *
 * Поля, специфичные для конкретного `status`, сериализуются только когда
 * заполнены (`version` — при `ok`, `reason` — при `launchFailed`,
 * `exitCode` — при `nonZeroExit`, `timeoutMs` — при `timeout`); остальные
 * диагностические поля опциональны независимо от статуса.
 */
export type SidecarCheckResult = { name: string, path: string, status: SidecarStatus, version?: string, reason?: LaunchFailedReason, exitCode?: number, osErrorCode?: string, stderrTail?: string, timeoutMs?: number, checkedAt?: string, durationMs?: number, };

/**
 * Итог попытки проверить один sidecar-бинарник.
 *
 * Варианты, кроме `Ok`, заполняются реальной логикой в TL-4/TL-5 (запуск
 * процесса, парсинг ошибок ОС и таймаут); здесь они — часть контракта,
 * который зеркалит TS-сторона (TL-2), поэтому не должны исчезать из-за
 * того, что stub-реализация их пока не конструирует.
 */
export type SidecarStatus = "ok" | "notFound" | "launchFailed" | "nonZeroExit" | "timeout";
