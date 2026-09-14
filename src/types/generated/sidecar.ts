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
export type LaunchFailedReason = "permissionDenied" | "corrupted" | "other" | "unrecognizedOutput";

/**
 * Агрегат результатов проверки sidecar-бинарников, возвращаемый командой
 * `check_sidecar` (Ф-9 эпика E1): yt-dlp, ffmpeg и deno — JavaScript-рантайм,
 * который yt-dlp запускает для YouTube-извлечения (TL-110, #114).
 *
 * `deno.version` — нормализованная версия из первой строки `deno --version`
 * (`deno 2.9.6 (stable, …)` даёт `2.9.6`). Одно отличие deno от двух других
 * строк: если бинарник ответил, но версии в выводе не нашлось, приходит не
 * `ok` с выводом вместо версии, а `launchFailed` с причиной
 * `unrecognizedOutput`, и нераспознанный вывод лежит в `stderrTail`. У
 * yt-dlp и ffmpeg в этом случае по-прежнему `ok`. Причина `other` у deno
 * остаётся за отказом до запуска — например, когда не определяется
 * каталог данных приложения для его кэша.
 */
export type SidecarCheckReport = { ytDlp: SidecarCheckResult, ffmpeg: SidecarCheckResult, deno: SidecarCheckResult, };

/**
 * Результат проверки одного sidecar-бинарника (yt-dlp, ffmpeg или deno).
 *
 * Поля, специфичные для конкретного `status`, сериализуются только когда
 * заполнены (`version` и `versionRaw` — при `ok`, `reason` — при `launchFailed`,
 * `exitCode` — при `nonZeroExit`, `timeoutMs` — при `timeout`); остальные
 * диагностические поля опциональны независимо от статуса.
 */
export type SidecarCheckResult = { name: string, path: string, status: SidecarStatus, version?: string, 
/**
 * Полная строка для диагностики в «Подробнее» (TL-15): первая строка
 * вывода версии бинарника, из которой разобрана `version`, — дословно,
 * с тем, что нормализация отбросила. У ffmpeg это
 * `ffmpeg version 9.0.1-https://www.martin-riedl.de Copyright …` (по ней
 * видно, чья сборка), у deno — `deno 2.9.6 (stable, release,
 * aarch64-apple-darwin)`, у yt-dlp — строка его `--version`.
 *
 * Приходит вместе с `version` и только с ней, то есть при `ok`. Длина
 * ограничена на стороне ядра: длиннее предела строка обрезается по
 * символам с `…` в конце — под именем sidecar может лежать бинарник,
 * печатающий мегабайт в одну строку. На экран вместо `version` не
 * выводится: там остаётся нормализованная версия.
 */
versionRaw?: string, reason?: LaunchFailedReason, exitCode?: number, osErrorCode?: string, stderrTail?: string, timeoutMs?: number, checkedAt?: string, durationMs?: number, };

/**
 * Итог попытки проверить один sidecar-бинарник.
 *
 * Варианты, кроме `Ok`, заполняются реальной логикой в TL-4/TL-5 (запуск
 * процесса, парсинг ошибок ОС и таймаут); здесь они — часть контракта,
 * который зеркалит TS-сторона (TL-2), поэтому не должны исчезать из-за
 * того, что stub-реализация их пока не конструирует.
 */
export type SidecarStatus = "ok" | "notFound" | "launchFailed" | "nonZeroExit" | "timeout";
