//! `#[tauri::command]` для служебного экрана: проверка sidecar-бинарников
//! (Ф-9, Н-6 эпика E1).
//!
//! Тонкий слой над `crate::sidecar`: резолвит путь к каждому бинарнику,
//! запускает его с аргументом версии и конвертирует
//! [`crate::sidecar::SidecarError`] в контрактный
//! [`crate::types::SidecarCheckResult`]. Обе проверки (yt-dlp, ffmpeg)
//! идут параллельно (`tokio::join!`, Н-6 «Экономия вызовов» — общее время
//! ожидания близко к максимуму из двух проверок, а не к их сумме).

use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tauri::State;

use crate::sidecar::{self, ChildRegistry, SidecarError};
use crate::types::{LaunchFailedReason, SidecarCheckReport, SidecarCheckResult, SidecarStatus};

/// Верхняя граница времени служебного экрана по Н-2: версия должна
/// появиться не позже, чем через 10 секунд после старта. Обе проверки идут
/// параллельно (`tokio::join!`), поэтому экран ждёт `max` таймаутов, а не их
/// сумму — значит ни один отдельный таймаут не может быть больше этого
/// бюджета, иначе требование нарушается самой конструкцией, независимо от
/// того, как быстро работают бинарники.
///
/// Единица измерения — секунды; используется только внутри этого модуля —
/// для вывода [`CHECK_TIMEOUT_SECS`] и для `const`-проверки, сторожащей
/// это соотношение на этапе компиляции.
const SERVICE_SCREEN_BUDGET_SECS: u64 = 10;

/// Запас бюджета Н-2, который не отдаётся под ожидание процесса: время на
/// invoke-round-trip, сериализацию отчёта и отрисовку экрана. Одна секунда
/// на порядок больше фактического round-trip (`invoke` на локальном IPC —
/// единицы миллисекунд) и покрывает медленный первый рендер WebView.
const SERVICE_SCREEN_RESERVE_SECS: u64 = 1;

/// Таймаут одной проверки: бюджет Н-2 минус запас. Оба sidecar-бинарника
/// проверяются с одним и тем же значением — экран всё равно ждёт максимум
/// из двух параллельных проверок, поэтому индивидуально более короткий
/// таймаут ничего не экономит, а только повышает шанс ложного «не
/// отвечает» на машине медленнее той, на которой снимались замеры.
const CHECK_TIMEOUT_SECS: u64 = SERVICE_SCREEN_BUDGET_SECS - SERVICE_SCREEN_RESERVE_SECS;

// Сторожит калибровку К-4 на этапе компиляции: таймаут отдельной проверки
// не может ни выродиться в ноль (мгновенный таймаут), ни превысить бюджет
// служебного экрана — второе нарушало бы Н-2 самой конструкцией, ещё до
// того, как что-то запустится.
const _: () = assert!(CHECK_TIMEOUT_SECS > 0 && CHECK_TIMEOUT_SECS <= SERVICE_SCREEN_BUDGET_SECS);

/// Таймаут проверки yt-dlp. Калибровка К-4 по фактическим замерам TL-12
/// (Apple Silicon, macOS 26.6, APFS/NVMe; все цифры — `/usr/bin/time -p`,
/// `real`):
///
/// | поставка yt-dlp                                      | первый запуск | повторные   |
/// |------------------------------------------------------|---------------|-------------|
/// | onefile `yt-dlp_macos` (текущая)                      | 25,2–39,4 с   | 25,2–34,0 с |
/// | onedir `yt-dlp_macos.zip`, каталог сам по себе        | 27,8–39,1 с   | 0,41–0,58 с |
/// | onedir внутри подписанного `.app` (`codesign` bundle) | 2,39–3,87 с   | 0,36–1,24 с |
///
/// Ключевое: у onefile-поставки «повторные» не отличаются от «первого» —
/// бутлоадер PyInstaller распаковывает 128 файлов (103 из них Mach-O) в
/// новый `$TMPDIR/_MEIxxxxxx` на каждом запуске, а macOS берёт
/// ~0,42 с за первую загрузку каждого только что созданного Mach-O-файла
/// (проверка подписи в `dyld4::Loader::mapSegments` → `fcntl`, обслуживает
/// `syspolicyd`). Одни и те же inode во второй раз стоят 0,08 с на все 84
/// файла — разница в ~490 раз. Подробности замеров — в отчёте по TL-12.
///
/// Отсюда значение: любое число, при котором текущая onefile-поставка
/// успевает, больше бюджета Н-2 в 3–4 раза, то есть требование всё равно
/// нарушено. Поэтому таймаут задаётся не «чтобы успевало», а как
/// исполнение Н-2: бюджет минус запас. При поставке, укладывающейся в Н-2
/// (onedir внутри подписанного бандла, худший замер 3,87 с), это даёт
/// запас 2,3×.
const YT_DLP_TIMEOUT: Duration = Duration::from_secs(CHECK_TIMEOUT_SECS);

/// Таймаут проверки ffmpeg. Калибровка К-4 по фактическим замерам TL-12 на
/// нативной arm64-сборке, поставленной в TL-11 (единый файл 66 МиБ):
/// холодный запуск (файл только что создан, inode новый) 1,58–2,52 с,
/// повторные 0,03–0,08 с. Природа холодной надбавки та же, что у yt-dlp,
/// но платится один раз на файл, а не на каждый запуск.
///
/// Значение — то же [`CHECK_TIMEOUT_SECS`], что и у [`YT_DLP_TIMEOUT`]
/// (обоснование единой величины — там же); к худшему измеренному
/// холодному запуску это запас 3,6×. Прежние 5 с тоже покрывали замер, но
/// были взяты из дизайна, а не из него.
const FFMPEG_TIMEOUT: Duration = Duration::from_secs(CHECK_TIMEOUT_SECS);

/// Максимальная длина `stderrTail` в Unicode-символах (не байтах) — по
/// контракту TL-1 достаточно «~1000 символов или ~20 строк» для «Подробнее»
/// на служебном экране; полный stderr в DTO не попадает, только хвост.
const STDERR_TAIL_MAX_CHARS: usize = 1000;

/// Возвращает результат проверки обоих sidecar-бинарников (yt-dlp, ffmpeg).
///
/// `registry` — реестр PID выполняющихся процессов (TL-10), внедряется
/// Tauri автоматически из состояния, управляемого в `main.rs`
/// (`app.manage(ChildRegistry::new())`); не часть JS-видимого контракта
/// команды — фронтенд по-прежнему вызывает `invoke("check_sidecar")` без
/// аргументов.
///
/// Возвращает `Result`, хотя сама проверка не может завершиться ошибкой на
/// этом уровне (все ошибки уже
/// конвертированы в поля [`SidecarCheckReport`], `Err` здесь никогда не
/// конструируется) — это требование самого Tauri: async-команда,
/// принимающая ссылки (`State<'_, T>`), обязана возвращать `Result`,
/// иначе сгенерированный future не может быть `'static`
/// (`AsyncCommandMustReturnResult`). На JS-стороне это не меняет поведение:
/// промис `invoke("check_sidecar")` всегда резолвится тем же отчётом, что
/// и раньше, и никогда не реджектится.
#[tauri::command]
pub async fn check_sidecar(registry: State<'_, ChildRegistry>) -> Result<SidecarCheckReport, ()> {
    Ok(check_report(
        sidecar::resolve_sidecar_path("yt-dlp"),
        sidecar::resolve_sidecar_path("ffmpeg"),
        &registry,
    )
    .await)
}

/// Собирает отчёт по уже резолвленным (или неуспешно резолвленным) путям —
/// вынесено из [`check_sidecar`] отдельно от резолва, чтобы тесты могли
/// подставлять пути к фикстурным скриптам вместо реальных sidecar-бинарников
/// (см. `crate::sidecar::process` тесты TL-4).
async fn check_report(
    yt_dlp_path: Result<PathBuf, SidecarError>,
    ffmpeg_path: Result<PathBuf, SidecarError>,
    registry: &ChildRegistry,
) -> SidecarCheckReport {
    let (yt_dlp, ffmpeg) = tokio::join!(
        check_binary(
            "yt-dlp",
            yt_dlp_path,
            &["--version"],
            YT_DLP_TIMEOUT,
            sidecar::parse_ytdlp_version,
            registry,
        ),
        check_binary(
            "ffmpeg",
            ffmpeg_path,
            &["-version"],
            FFMPEG_TIMEOUT,
            sidecar::parse_ffmpeg_version,
            registry,
        ),
    );

    SidecarCheckReport { yt_dlp, ffmpeg }
}

/// Проверяет один sidecar-бинарник: запускает `program` с аргументом
/// версии `args`, ждёт не дольше `timeout`, разбирает версию `parse_version`
/// и конвертирует итог в [`SidecarCheckResult`].
///
/// `resolved_path` уже несёт исход резолва (`Ok` — путь; `Err` — почему не
/// удалось определить путь, см. `crate::sidecar::resolve_sidecar_path`) —
/// если резолв не удался, дальше запускать нечего, ошибка конвертируется
/// напрямую, а `path` в результате — сам запрошенный `name` (место, где
/// физически не смогли даже начать искать, единственное осмысленное «здесь
/// искали» в этом случае).
async fn check_binary(
    name: &str,
    resolved_path: Result<PathBuf, SidecarError>,
    args: &[&str],
    timeout: Duration,
    parse_version: fn(&str) -> Option<sidecar::SidecarVersion>,
    registry: &ChildRegistry,
) -> SidecarCheckResult {
    let checked_at = now_iso8601();
    let started = Instant::now();

    let path = match resolved_path {
        Ok(path) => path,
        Err(error) => {
            let duration_ms = elapsed_ms(started);
            return error_to_result(name, name.to_string(), checked_at, duration_ms, error);
        }
    };
    let path_string = path.display().to_string();

    let run_result: Result<sidecar::RunOutput, SidecarError> =
        sidecar::run(&path, args, timeout, registry).await;
    let duration_ms = elapsed_ms(started);

    match run_result {
        Ok(output) => {
            let version = match parse_version(&output.stdout) {
                Some(parsed) => {
                    // Полная строка сборки в DTO не уходит: `version` в
                    // контракте один, и новое поле потянуло бы за собой
                    // TS-зеркало и область ui. На экране — нормализованный
                    // semver, полная строка пишется в лог, чтобы по ней можно
                    // было опознать сборку при разборе бага постобработки.
                    if parsed.is_normalized() {
                        eprintln!(
                            "sidecar {name}: версия сборки {raw}, на служебном экране показывается {display}",
                            name = name,
                            raw = parsed.raw,
                            display = parsed.display,
                        );
                    }
                    parsed.display
                }
                None => output.stdout.trim().to_string(),
            };

            SidecarCheckResult {
                name: name.to_string(),
                path: path_string,
                status: SidecarStatus::Ok,
                version: Some(version),
                reason: None,
                exit_code: None,
                os_error_code: None,
                stderr_tail: None,
                timeout_ms: None,
                checked_at: Some(checked_at),
                duration_ms: Some(duration_ms),
            }
        }
        Err(error) => error_to_result(name, path_string, checked_at, duration_ms, error),
    }
}

/// Конвертирует [`SidecarError`] в [`SidecarCheckResult`] по контракту
/// TL-1: соответствие `status`/`osErrorCode` для каждого варианта
/// зафиксировано в дизайне эпика E1 (см. `epics/E1-karkas-i-sidecar.md`).
fn error_to_result(
    name: &str,
    path: String,
    checked_at: String,
    duration_ms: u64,
    error: SidecarError,
) -> SidecarCheckResult {
    let (status, reason, exit_code, os_error_code, stderr_tail, timeout_ms) = match error {
        SidecarError::NotFound => (
            SidecarStatus::NotFound,
            None,
            None,
            Some("ENOENT".to_string()),
            None,
            None,
        ),
        SidecarError::LaunchFailed { reason, stderr } => {
            let os_error_code = match reason {
                LaunchFailedReason::PermissionDenied => Some("EACCES".to_string()),
                // ENOEXEC — см. doc `crate::sidecar::process`: ядро отказывается
                // выполнить файл неопознанного формата; на Unix это и есть код
                // ошибки, которым он в итоге проявляется (через shell-фолбэк,
                // код выхода 126, классифицированный как `Corrupted`).
                LaunchFailedReason::Corrupted => Some("ENOEXEC".to_string()),
                LaunchFailedReason::Other => None,
            };
            (
                SidecarStatus::LaunchFailed,
                Some(reason),
                None,
                os_error_code,
                tail(&stderr),
                None,
            )
        }
        SidecarError::NonZeroExit { code, stderr } => (
            SidecarStatus::NonZeroExit,
            None,
            Some(code),
            None,
            tail(&stderr),
            None,
        ),
        SidecarError::Timeout { ms, stderr } => (
            SidecarStatus::Timeout,
            None,
            None,
            None,
            tail(&stderr),
            Some(ms),
        ),
    };

    SidecarCheckResult {
        name: name.to_string(),
        path,
        status,
        version: None,
        reason,
        exit_code,
        os_error_code,
        stderr_tail,
        timeout_ms,
        checked_at: Some(checked_at),
        duration_ms: Some(duration_ms),
    }
}

/// Обрезает stderr до последних [`STDERR_TAIL_MAX_CHARS`] символов;
/// `None`, если после `trim()` пусто (нечего показывать в «Подробнее»).
fn tail(stderr: &str) -> Option<String> {
    let trimmed = stderr.trim();
    if trimmed.is_empty() {
        return None;
    }

    let char_count = trimmed.chars().count();
    if char_count <= STDERR_TAIL_MAX_CHARS {
        Some(trimmed.to_string())
    } else {
        let skip = char_count - STDERR_TAIL_MAX_CHARS;
        Some(trimmed.chars().skip(skip).collect())
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Текущее время в UTC, отформатированное как RFC 3339
/// (`2026-08-25T09:15:30.123Z`), без внешних зависимостей — только
/// `std::time` плюс алгоритм перевода дней с эпохи Unix в календарную дату
/// Говарда Хайнанта (`civil_from_days`, общественное достояние, см.
/// http://howardhinnant.github.io/date_algorithms.html). Добавлять `chrono`
/// или `time` ради одного поля лога — решение о новой зависимости, не
/// принимается на уровне этой задачи (см. CLAUDE.md, «owner-level
/// questions»).
fn now_iso8601() -> String {
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format_unix_timestamp(since_epoch)
}

fn format_unix_timestamp(since_epoch: Duration) -> String {
    let total_secs = since_epoch.as_secs();
    let millis = since_epoch.subsec_millis();
    let days = (total_secs / 86_400) as i64;
    let secs_of_day = total_secs % 86_400;

    let (year, month, day) = civil_from_days(days);
    let hour = secs_of_day / 3600;
    let minute = (secs_of_day % 3600) / 60;
    let second = secs_of_day % 60;

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

/// Переводит число дней с эпохи Unix (1970-01-01) в григорианскую дату
/// `(год, месяц 1..=12, день 1..=31)`. Алгоритм Говарда Хайнанта, корректен
/// для всего диапазона дат, поддерживаемых `i64`, включая años до эпохи.
fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let year = if month <= 2 { y + 1 } else { y };
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use tempfile::tempdir;

    fn write_script(dir: &tempfile::TempDir, name: &str, contents: &str, mode: u32) -> PathBuf {
        let path = dir.path().join(name);
        fs::write(&path, contents).expect("failed to write fixture script");
        fs::set_permissions(&path, fs::Permissions::from_mode(mode))
            .expect("failed to chmod fixture script");
        path
    }

    #[test]
    fn formats_the_unix_epoch_as_rfc3339() {
        assert_eq!(
            format_unix_timestamp(Duration::from_secs(0)),
            "1970-01-01T00:00:00.000Z"
        );
    }

    #[test]
    fn formats_a_known_date_with_milliseconds() {
        // 2000-01-01T00:00:00Z == 946684800 (справочная точка).
        assert_eq!(
            format_unix_timestamp(Duration::new(946_684_800, 123_000_000)),
            "2000-01-01T00:00:00.123Z"
        );
    }

    #[tokio::test]
    async fn converts_a_successful_run_into_ok_status_with_parsed_version() {
        let dir = tempdir().expect("failed to create temp dir");
        let script = write_script(
            &dir,
            "yt-dlp.sh",
            "#!/bin/sh\necho 2026.08.19\nexit 0\n",
            0o755,
        );

        let registry = ChildRegistry::new();
        let result = check_binary(
            "yt-dlp",
            Ok(script.clone()),
            &["--version"],
            Duration::from_secs(5),
            sidecar::parse_ytdlp_version,
            &registry,
        )
        .await;

        assert_eq!(result.name, "yt-dlp");
        assert_eq!(result.path, script.display().to_string());
        assert_eq!(result.status, SidecarStatus::Ok);
        assert_eq!(result.version.as_deref(), Some("2026.08.19"));
        assert!(result.reason.is_none());
        assert!(result.exit_code.is_none());
        assert!(result.os_error_code.is_none());
        assert!(result.stderr_tail.is_none());
        assert!(result.timeout_ms.is_none());
        assert!(result.checked_at.is_some());
        assert!(result.duration_ms.is_some());
    }

    #[tokio::test]
    async fn converts_a_missing_binary_into_not_found_status_with_enoent() {
        let dir = tempdir().expect("failed to create temp dir");
        let missing = dir.path().join("does-not-exist");

        let registry = ChildRegistry::new();
        let result = check_binary(
            "ffmpeg",
            Ok(missing.clone()),
            &["-version"],
            Duration::from_secs(5),
            sidecar::parse_ffmpeg_version,
            &registry,
        )
        .await;

        assert_eq!(result.status, SidecarStatus::NotFound);
        assert_eq!(result.path, missing.display().to_string());
        assert_eq!(result.os_error_code.as_deref(), Some("ENOENT"));
        assert!(result.version.is_none());
    }

    #[tokio::test]
    async fn converts_a_non_executable_binary_into_launch_failed_status_with_eacces() {
        let dir = tempdir().expect("failed to create temp dir");
        let script = write_script(&dir, "not-executable.sh", "#!/bin/sh\nexit 0\n", 0o644);

        let registry = ChildRegistry::new();
        let result = check_binary(
            "yt-dlp",
            Ok(script),
            &["--version"],
            Duration::from_secs(5),
            sidecar::parse_ytdlp_version,
            &registry,
        )
        .await;

        assert_eq!(result.status, SidecarStatus::LaunchFailed);
        assert_eq!(result.reason, Some(LaunchFailedReason::PermissionDenied));
        assert_eq!(result.os_error_code.as_deref(), Some("EACCES"));
        // Отказ на уровне spawn — процесс не стартовал, stderr недостижим.
        assert!(result.stderr_tail.is_none());
    }

    #[tokio::test]
    async fn converts_a_corrupted_binary_into_launch_failed_status_with_enoexec() {
        let dir = tempdir().expect("failed to create temp dir");
        let script = write_script(&dir, "garbage", "not a real executable\x00\x01\x02", 0o755);

        let registry = ChildRegistry::new();
        let result = check_binary(
            "ffmpeg",
            Ok(script),
            &["-version"],
            Duration::from_secs(5),
            sidecar::parse_ffmpeg_version,
            &registry,
        )
        .await;

        assert_eq!(result.status, SidecarStatus::LaunchFailed);
        assert_eq!(result.reason, Some(LaunchFailedReason::Corrupted));
        assert_eq!(result.os_error_code.as_deref(), Some("ENOEXEC"));
    }

    #[tokio::test]
    async fn converts_a_non_zero_exit_into_non_zero_exit_status_with_exit_code_and_stderr_tail() {
        let dir = tempdir().expect("failed to create temp dir");
        let script = write_script(
            &dir,
            "fail.sh",
            "#!/bin/sh\necho 'error: unsupported URL' >&2\nexit 1\n",
            0o755,
        );

        let registry = ChildRegistry::new();
        let result = check_binary(
            "yt-dlp",
            Ok(script),
            &["--version"],
            Duration::from_secs(5),
            sidecar::parse_ytdlp_version,
            &registry,
        )
        .await;

        assert_eq!(result.status, SidecarStatus::NonZeroExit);
        assert_eq!(result.exit_code, Some(1));
        assert_eq!(
            result.stderr_tail.as_deref(),
            Some("error: unsupported URL")
        );
        assert!(result.version.is_none());
    }

    #[tokio::test]
    async fn converts_a_slow_process_into_timeout_status_with_timeout_ms() {
        let dir = tempdir().expect("failed to create temp dir");
        let script = write_script(&dir, "slow.sh", "#!/bin/sh\nsleep 5\n", 0o755);

        let registry = ChildRegistry::new();
        let result = check_binary(
            "ffmpeg",
            Ok(script),
            &["-version"],
            Duration::from_millis(150),
            sidecar::parse_ffmpeg_version,
            &registry,
        )
        .await;

        assert_eq!(result.status, SidecarStatus::Timeout);
        assert_eq!(result.timeout_ms, Some(150));
        assert!(result.version.is_none());
        assert!(
            registry.is_empty(),
            "check_binary must not leave a pid registered after the timeout kill"
        );
    }

    #[tokio::test]
    async fn converts_a_resolve_failure_into_launch_failed_status_using_the_binary_name_as_path() {
        let registry = ChildRegistry::new();
        let result = check_binary(
            "yt-dlp",
            Err(SidecarError::LaunchFailed {
                reason: LaunchFailedReason::Other,
                stderr: String::new(),
            }),
            &["--version"],
            Duration::from_secs(5),
            sidecar::parse_ytdlp_version,
            &registry,
        )
        .await;

        assert_eq!(result.status, SidecarStatus::LaunchFailed);
        assert_eq!(result.reason, Some(LaunchFailedReason::Other));
        assert!(result.os_error_code.is_none());
        // Резолв не дал пути — единственное осмысленное «здесь искали» это
        // само запрошенное имя.
        assert_eq!(result.path, "yt-dlp");
    }

    #[tokio::test]
    async fn truncates_a_long_stderr_tail_to_the_configured_character_limit() {
        let dir = tempdir().expect("failed to create temp dir");
        // Печатает заметно больше STDERR_TAIL_MAX_CHARS символов на stderr.
        let script = write_script(
            &dir,
            "verbose-fail.sh",
            "#!/bin/sh\ni=0\nwhile [ $i -lt 2000 ]; do printf 'x' >&2; i=$((i + 1)); done\nexit 1\n",
            0o755,
        );

        let registry = ChildRegistry::new();
        let result = check_binary(
            "yt-dlp",
            Ok(script),
            &["--version"],
            Duration::from_secs(5),
            sidecar::parse_ytdlp_version,
            &registry,
        )
        .await;

        assert_eq!(result.status, SidecarStatus::NonZeroExit);
        let tail = result.stderr_tail.expect("stderr tail must be present");
        assert_eq!(tail.chars().count(), STDERR_TAIL_MAX_CHARS);
        assert!(tail.chars().all(|c| c == 'x'));
    }

    /// Прогревает фикстурный скрипт синхронным холостым запуском вне
    /// измеряемого окна теста — самый первый запуск нового исполняемого
    /// файла на некоторых машинах/песочницах несёт одноразовый фиксированный
    /// оверхед (например, проверка Gatekeeper на macOS для файла, который
    /// ещё не запускался), не имеющий отношения к параллельности самой
    /// проверки; без прогрева он на порядок превышает `sleep 0.2` в
    /// фикстуре и маскирует разницу между параллельным и последовательным
    /// выполнением. Второй и последующие запуски того же файла эту
    /// надбавку уже не платят.
    async fn warm_up(script: &std::path::Path) {
        let registry = ChildRegistry::new();
        let _ = check_binary(
            "warm-up",
            Ok(script.to_path_buf()),
            &["--version"],
            Duration::from_secs(20),
            sidecar::parse_ytdlp_version,
            &registry,
        )
        .await;
    }

    #[tokio::test]
    async fn checks_yt_dlp_and_ffmpeg_concurrently_not_sequentially() {
        let dir = tempdir().expect("failed to create temp dir");
        let script_contents = "#!/bin/sh\nsleep 0.2\necho 2026.08.19\nexit 0\n";
        let yt_dlp_script = write_script(&dir, "slow-yt-dlp.sh", script_contents, 0o755);
        let ffmpeg_script = write_script(&dir, "slow-ffmpeg.sh", script_contents, 0o755);

        warm_up(&yt_dlp_script).await;
        warm_up(&ffmpeg_script).await;

        let registry = ChildRegistry::new();
        let started = Instant::now();
        let report = check_report(Ok(yt_dlp_script), Ok(ffmpeg_script), &registry).await;
        let elapsed = started.elapsed();

        assert_eq!(report.yt_dlp.status, SidecarStatus::Ok);
        assert_eq!(report.ffmpeg.status, SidecarStatus::Ok);
        assert!(
            elapsed < Duration::from_millis(350),
            "two ~200ms checks must run concurrently (well under ~400ms), took {elapsed:?}"
        );
    }
}
