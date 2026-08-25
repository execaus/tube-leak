//! Типизированные ошибки домена `sidecar` (Ф-6, Ф-8 эпика E1).
//!
//! Переиспользует [`crate::types::LaunchFailedReason`] из контракта
//! Rust↔TS вместо собственного дублирующего enum — TL-5 конструирует
//! [`crate::types::SidecarCheckResult`] из значений этой ошибки без
//! дополнительной конвертации причины отказа.

use crate::types::LaunchFailedReason;

/// Итог неуспешной попытки запустить или получить версию sidecar-бинарника.
///
/// `unwrap()` на путях, которые могут вернуть эти варианты, недопустим
/// (см. CLAUDE.md, «Конвенции») — вызывающий код (TL-5) обязан обработать
/// каждый вариант явно.
///
/// Варианты, при которых процесс успел стартовать (`LaunchFailed` из-за
/// повреждённого формата/сигнала, `NonZeroExit`, `Timeout`), несут
/// `stderr` — то, что успело накопиться в потоке ошибок процесса
/// (см. `crate::sidecar::process::run`); TL-5 конвертирует его в
/// `SidecarCheckResult::stderr_tail` (обрезанный хвост). Для `NotFound` и
/// `LaunchFailed` из-за отказа самого `spawn` (нет прав на выполнение,
/// не удалось стартовать вовсе) stderr недостижим — пустая строка, это
/// нормально: процесс так и не запустился.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SidecarError {
    /// Бинарник не найден по разрешённому пути (ENOENT при попытке запуска).
    #[error("sidecar binary not found")]
    NotFound,

    /// Процесс не удалось запустить или он не смог корректно стартовать.
    #[error("sidecar launch failed: {reason:?}")]
    LaunchFailed {
        reason: LaunchFailedReason,
        stderr: String,
    },

    /// Процесс запустился и завершился, но с ненулевым кодом выхода.
    #[error("sidecar exited with non-zero code {code}")]
    NonZeroExit { code: i32, stderr: String },

    /// Процесс не завершился в отведённое время и был принудительно убит.
    #[error("sidecar timed out after {ms}ms")]
    Timeout { ms: u64, stderr: String },
}
