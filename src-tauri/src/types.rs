//! Типы, пересекающие границу Rust↔TS: результат проверки sidecar-бинарников
//! (yt-dlp, ffmpeg) и ход подготовки yt-dlp при первом запуске (TL-12).
//! Объявлены здесь один раз; TS-зеркало в `src/types/` поддерживает точное
//! соответствие полей и значений enum-строк — расхождение с этим файлом
//! дорого чинить постфактум (см. TL-1/TL-2 в эпике E1).

use serde::Serialize;

/// Итог попытки проверить один sidecar-бинарник.
///
/// Варианты, кроме `Ok`, заполняются реальной логикой в TL-4/TL-5 (запуск
/// процесса, парсинг ошибок ОС и таймаут); здесь они — часть контракта,
/// который зеркалит TS-сторона (TL-2), поэтому не должны исчезать из-за
/// того, что stub-реализация их пока не конструирует.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SidecarStatus {
    Ok,
    NotFound,
    LaunchFailed,
    NonZeroExit,
    Timeout,
}

/// Причина отказа запуска, применима только при `status = launchFailed`.
///
/// См. пояснение у [`SidecarStatus`] — варианты заполняются в TL-4/TL-5.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LaunchFailedReason {
    PermissionDenied,
    Corrupted,
    Other,
}

/// Результат проверки одного sidecar-бинарника (yt-dlp или ffmpeg).
///
/// Поля, специфичные для конкретного `status`, сериализуются только когда
/// заполнены (`version` — при `ok`, `reason` — при `launchFailed`,
/// `exitCode` — при `nonZeroExit`, `timeoutMs` — при `timeout`); остальные
/// диагностические поля опциональны независимо от статуса.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarCheckResult {
    pub name: String,
    pub path: String,
    pub status: SidecarStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<LaunchFailedReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os_error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr_tail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

/// Агрегат результатов проверки обоих sidecar-бинарников, возвращаемый
/// командой `check_sidecar` (Ф-9 эпика E1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarCheckReport {
    pub yt_dlp: SidecarCheckResult,
    pub ffmpeg: SidecarCheckResult,
}

// ───────────────────────── подготовка yt-dlp (TL-12) ─────────────────────────

/// Этап подготовки yt-dlp, отображаемый пользователю.
///
/// Это не внутренние шаги реализации, а то, что видно снаружи: сначала
/// приложение раскладывает вложенный onedir-архив в каталог данных
/// (`unpacking`), потом один раз прогоняет распакованное дерево, чтобы ОС
/// проверила подписи всех его файлов (`warmingUp`) — именно этот прогон
/// стоит ~35 с и ради него этап вообще показывается пользователю. Дальше —
/// терминальные состояния: `ready` или `failed`.
///
/// Почему прогрев вообще нужен — см. doc [`crate::ytdlp`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum YtDlpPrepareStage {
    Unpacking,
    WarmingUp,
    Ready,
    Failed,
}

/// Типизированная причина отказа подготовки (CLAUDE.md, «Ошибки
/// типизированные, не строки»): по `kind` фронтенд решает, что предложить
/// пользователю, `message` — диагностика для «Подробнее», не для решения.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum YtDlpPrepareErrorKind {
    /// Каталог данных приложения недоступен (нет прав, нет тома).
    DataDirUnavailable,
    /// В бандле нет вложенного архива yt-dlp — сломанная установка.
    ArchiveMissing,
    /// Архив есть, но не читается как zip либо не проходит проверку
    /// целостности (CRC32 записи не сошёлся).
    ArchiveCorrupted,
    /// Не удалось записать распакованное дерево (нет места, нет прав).
    UnpackFailed,
    /// Дерево распаковалось, но выглядит не так, как ожидается: в корне не
    /// нашлось ровно одного исполняемого файла.
    LayoutUnexpected,
    /// Дерево на месте, но yt-dlp не запускается или не отвечает.
    WarmupFailed,
}

/// Отказ подготовки в сериализуемом виде.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YtDlpPrepareError {
    pub kind: YtDlpPrepareErrorKind,
    pub message: String,
}

/// Событие хода подготовки yt-dlp, эмитится под именем `ytdlp://prepare`
/// (константа `PREPARE_EVENT` в `crate::ytdlp`).
///
/// `percent` — сквозной прогресс всей подготовки (0..=100), а не прогресс
/// текущего этапа: этапы стоят несопоставимо (распаковка — единицы секунд,
/// прогрев — десятки), и отдельные шкалы на них выглядели бы как зависший
/// индикатор. `etaSecs` — оценка оставшегося времени; на этапе прогрева она
/// выводится из числа файлов дерева и измеренной цены одного файла, а не
/// из наблюдаемого прогресса: прогрев — один процесс, который до самого
/// конца ничего не сообщает о себе.
///
/// Ни одно событие не является обязательным для получения результата:
/// команда `prepare_ytdlp` возвращает итог сама. События нужны, чтобы
/// показать ход, и приходят **только если работа действительно
/// понадобилась** — на «тёплом» запуске (подготовка уже сделана раньше) не
/// приходит ни одного, и экран подготовки показывать не нужно.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YtDlpPrepareEvent {
    pub stage: YtDlpPrepareStage,
    pub percent: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eta_secs: Option<u64>,
    /// Версия yt-dlp, полученная запуском — только при `stage = ready`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Причина отказа — только при `stage = failed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<YtDlpPrepareError>,
}

/// Итог подготовки, возвращаемый командой `prepare_ytdlp`.
///
/// `prepared = false` означает, что делать ничего не потребовалось: дерево
/// уже лежало в каталоге данных и отозвалось за доли секунды. Именно этот
/// случай — обычный запуск приложения; `true` бывает при первом запуске
/// после установки, после обновления приложения с новой версией yt-dlp и
/// после того, как ОС забыла результат проверки подписей.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YtDlpPrepared {
    /// Версия yt-dlp, полученная запуском (не из пина).
    pub version: String,
    /// Абсолютный путь к исполняемому файлу внутри распакованного дерева.
    pub path: String,
    /// Выполнялась ли фактическая работа (распаковка и/или прогрев).
    pub prepared: bool,
    /// Сколько заняла команда целиком.
    pub duration_ms: u64,
}

/// Фиксированные stub-данные: реальный запуск и разбор бинарников теперь
/// реализованы (`crate::commands::sidecar::check_sidecar`, TL-5), эта
/// функция больше не используется как продакшен-заглушка — оставлена ради
/// собственных тестов ниже (форма ответа для TS-зеркала, TL-2) и как
/// готовый фикстурный `SidecarCheckReport` для будущих тестов на стороне
/// вызывающего кода, если понадобится. `#[allow(dead_code)]` — не контракт,
/// а именно эта функция вне `#[cfg(test)]`.
#[allow(dead_code)]
pub fn stub_report() -> SidecarCheckReport {
    let ok = |name: &str, path: &str| SidecarCheckResult {
        name: name.to_string(),
        path: path.to_string(),
        status: SidecarStatus::Ok,
        version: Some("stub".to_string()),
        reason: None,
        exit_code: None,
        os_error_code: None,
        stderr_tail: None,
        timeout_ms: None,
        checked_at: None,
        duration_ms: None,
    };

    SidecarCheckReport {
        yt_dlp: ok("yt-dlp", "yt-dlp"),
        ffmpeg: ok("ffmpeg", "ffmpeg"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn serializes_ok_status_with_version() {
        let result = SidecarCheckResult {
            name: "yt-dlp".to_string(),
            path: "/opt/tube-leak/bin/yt-dlp".to_string(),
            status: SidecarStatus::Ok,
            version: Some("2026.08.01".to_string()),
            reason: None,
            exit_code: None,
            os_error_code: None,
            stderr_tail: None,
            timeout_ms: None,
            checked_at: None,
            duration_ms: None,
        };

        let value = serde_json::to_value(&result).expect("serialization must not fail");

        assert_eq!(
            value,
            json!({
                "name": "yt-dlp",
                "path": "/opt/tube-leak/bin/yt-dlp",
                "status": "ok",
                "version": "2026.08.01",
            })
        );
    }

    #[test]
    fn serializes_not_found_status_with_os_error_code() {
        let result = SidecarCheckResult {
            name: "ffmpeg".to_string(),
            path: "/opt/tube-leak/bin/ffmpeg".to_string(),
            status: SidecarStatus::NotFound,
            version: None,
            reason: None,
            exit_code: None,
            os_error_code: Some("ENOENT".to_string()),
            stderr_tail: None,
            timeout_ms: None,
            checked_at: None,
            duration_ms: None,
        };

        let value = serde_json::to_value(&result).expect("serialization must not fail");

        assert_eq!(
            value,
            json!({
                "name": "ffmpeg",
                "path": "/opt/tube-leak/bin/ffmpeg",
                "status": "notFound",
                "osErrorCode": "ENOENT",
            })
        );
    }

    #[test]
    fn serializes_launch_failed_status_with_permission_denied_reason() {
        let result = SidecarCheckResult {
            name: "yt-dlp".to_string(),
            path: "/opt/tube-leak/bin/yt-dlp".to_string(),
            status: SidecarStatus::LaunchFailed,
            version: None,
            reason: Some(LaunchFailedReason::PermissionDenied),
            exit_code: None,
            os_error_code: Some("EACCES".to_string()),
            stderr_tail: None,
            timeout_ms: None,
            checked_at: None,
            duration_ms: None,
        };

        let value = serde_json::to_value(&result).expect("serialization must not fail");

        assert_eq!(
            value,
            json!({
                "name": "yt-dlp",
                "path": "/opt/tube-leak/bin/yt-dlp",
                "status": "launchFailed",
                "reason": "permissionDenied",
                "osErrorCode": "EACCES",
            })
        );
    }

    #[test]
    fn serializes_launch_failed_status_with_corrupted_reason() {
        let result = SidecarCheckResult {
            name: "ffmpeg".to_string(),
            path: "/opt/tube-leak/bin/ffmpeg".to_string(),
            status: SidecarStatus::LaunchFailed,
            version: None,
            reason: Some(LaunchFailedReason::Corrupted),
            exit_code: None,
            os_error_code: Some("ENOEXEC".to_string()),
            stderr_tail: None,
            timeout_ms: None,
            checked_at: None,
            duration_ms: None,
        };

        let value = serde_json::to_value(&result).expect("serialization must not fail");

        assert_eq!(
            value,
            json!({
                "name": "ffmpeg",
                "path": "/opt/tube-leak/bin/ffmpeg",
                "status": "launchFailed",
                "reason": "corrupted",
                "osErrorCode": "ENOEXEC",
            })
        );
    }

    #[test]
    fn serializes_non_zero_exit_status_with_exit_code() {
        let result = SidecarCheckResult {
            name: "yt-dlp".to_string(),
            path: "/opt/tube-leak/bin/yt-dlp".to_string(),
            status: SidecarStatus::NonZeroExit,
            version: None,
            reason: None,
            exit_code: Some(1),
            os_error_code: None,
            stderr_tail: Some("error: unsupported URL".to_string()),
            timeout_ms: None,
            checked_at: None,
            duration_ms: None,
        };

        let value = serde_json::to_value(&result).expect("serialization must not fail");

        assert_eq!(
            value,
            json!({
                "name": "yt-dlp",
                "path": "/opt/tube-leak/bin/yt-dlp",
                "status": "nonZeroExit",
                "exitCode": 1,
                "stderrTail": "error: unsupported URL",
            })
        );
    }

    #[test]
    fn serializes_timeout_status_with_timeout_ms() {
        let result = SidecarCheckResult {
            name: "ffmpeg".to_string(),
            path: "/opt/tube-leak/bin/ffmpeg".to_string(),
            status: SidecarStatus::Timeout,
            version: None,
            reason: None,
            exit_code: None,
            os_error_code: None,
            stderr_tail: None,
            timeout_ms: Some(5000),
            checked_at: None,
            duration_ms: None,
        };

        let value = serde_json::to_value(&result).expect("serialization must not fail");

        assert_eq!(
            value,
            json!({
                "name": "ffmpeg",
                "path": "/opt/tube-leak/bin/ffmpeg",
                "status": "timeout",
                "timeoutMs": 5000,
            })
        );
    }

    #[test]
    fn stub_report_marks_both_sidecars_as_ok() {
        let report = stub_report();

        assert_eq!(report.yt_dlp.status, SidecarStatus::Ok);
        assert_eq!(report.ffmpeg.status, SidecarStatus::Ok);
        assert!(report.yt_dlp.version.is_some());
        assert!(report.ffmpeg.version.is_some());
    }

    #[test]
    fn serializes_report_with_camel_case_field_names() {
        let value = serde_json::to_value(stub_report()).expect("serialization must not fail");
        let object = value
            .as_object()
            .expect("report must serialize to an object");

        assert!(object.contains_key("ytDlp"));
        assert!(object.contains_key("ffmpeg"));
    }
}
