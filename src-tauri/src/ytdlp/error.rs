//! Типизированные ошибки подготовки yt-dlp (TL-12).
//!
//! Домейн говорит `thiserror`-ошибкой, граница `#[tauri::command]`
//! конвертирует её в сериализуемый [`crate::types::YtDlpPrepareError`]
//! (CLAUDE.md, «Конвенции»). Разделение не формальность: `PrepareError`
//! несёт полные пути и системные сообщения, полезные в логе, а контрактный
//! тип — только классификацию и уже урезанное сообщение.

use crate::types::{YtDlpPrepareError, YtDlpPrepareErrorKind};

/// Почему не удалось подготовить yt-dlp к работе.
#[derive(Debug, thiserror::Error)]
pub enum PrepareError {
    /// Каталог данных приложения недоступен: не резолвится, не создаётся
    /// или в нём нельзя писать.
    #[error("каталог данных приложения недоступен: {reason}")]
    DataDirUnavailable { reason: String },

    /// В бандле нет вложенного onedir-архива yt-dlp. Это сломанная
    /// установка приложения, а не состояние, из которого можно выйти
    /// повторной попыткой.
    #[error("в дистрибутиве нет архива yt-dlp ({path})")]
    ArchiveMissing { path: String },

    /// Архив есть, но не читается как zip либо не проходит проверку
    /// целостности: CRC32 записи не сошёлся при распаковке.
    #[error("архив yt-dlp повреждён: {reason}")]
    ArchiveCorrupted { reason: String },

    /// Не удалось записать распакованное дерево: нет места, нет прав,
    /// отказ файловой системы.
    #[error("не удалось распаковать yt-dlp: {reason}")]
    UnpackFailed { reason: String },

    /// Дерево распаковалось, но выглядит не так, как ожидается: в корне
    /// не нашлось ровно одного исполняемого файла. Гадать, что запускать,
    /// нельзя — лучше явный отказ.
    #[error("распакованное дерево yt-dlp выглядит неожиданно: {reason}")]
    LayoutUnexpected { reason: String },

    /// Дерево на месте, но yt-dlp не запустился, завершился с ошибкой или
    /// не ответил за отведённое время.
    #[error("yt-dlp не отвечает после подготовки: {reason}")]
    WarmupFailed { reason: String },
}

impl PrepareError {
    /// Классификация для фронтенда.
    pub fn kind(&self) -> YtDlpPrepareErrorKind {
        match self {
            Self::DataDirUnavailable { .. } => YtDlpPrepareErrorKind::DataDirUnavailable,
            Self::ArchiveMissing { .. } => YtDlpPrepareErrorKind::ArchiveMissing,
            Self::ArchiveCorrupted { .. } => YtDlpPrepareErrorKind::ArchiveCorrupted,
            Self::UnpackFailed { .. } => YtDlpPrepareErrorKind::UnpackFailed,
            Self::LayoutUnexpected { .. } => YtDlpPrepareErrorKind::LayoutUnexpected,
            Self::WarmupFailed { .. } => YtDlpPrepareErrorKind::WarmupFailed,
        }
    }

    /// Проекция на контракт Rust↔TS.
    pub fn to_contract(&self) -> YtDlpPrepareError {
        YtDlpPrepareError {
            kind: self.kind(),
            message: self.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_every_variant_to_its_own_contract_kind() {
        let variants = [
            (
                PrepareError::DataDirUnavailable {
                    reason: "нет прав".to_string(),
                },
                YtDlpPrepareErrorKind::DataDirUnavailable,
            ),
            (
                PrepareError::ArchiveMissing {
                    path: "/x".to_string(),
                },
                YtDlpPrepareErrorKind::ArchiveMissing,
            ),
            (
                PrepareError::ArchiveCorrupted {
                    reason: "crc".to_string(),
                },
                YtDlpPrepareErrorKind::ArchiveCorrupted,
            ),
            (
                PrepareError::UnpackFailed {
                    reason: "ENOSPC".to_string(),
                },
                YtDlpPrepareErrorKind::UnpackFailed,
            ),
            (
                PrepareError::LayoutUnexpected {
                    reason: "два файла".to_string(),
                },
                YtDlpPrepareErrorKind::LayoutUnexpected,
            ),
            (
                PrepareError::WarmupFailed {
                    reason: "timeout".to_string(),
                },
                YtDlpPrepareErrorKind::WarmupFailed,
            ),
        ];

        for (error, expected_kind) in variants {
            assert_eq!(error.kind(), expected_kind);
            let contract = error.to_contract();
            assert_eq!(contract.kind, expected_kind);
            assert!(
                !contract.message.is_empty(),
                "сообщение обязано быть непустым: по нему пользователь \
                 открывает «Подробнее»"
            );
        }
    }
}
