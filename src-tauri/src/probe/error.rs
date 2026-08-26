//! Типизированная ошибка разбора ссылки (Ф-6 эпика E2, TL-27).
//!
//! Девять вариантов — ровно девять классов Ф-6, ни больше ни меньше:
//! фронтенд по классу выбирает заголовок, пояснение и наличие кнопки
//! «Повторить» (таблица текстов — в дизайне E2), поэтому появление
//! десятого класса или исчезновение любого из девяти — изменение
//! контракта, а не деталь реализации.
//!
//! Технические детали ([`crate::types::ProbeErrorDetails`]) взяты из
//! контракта, а не продублированы своим типом домена — тот же приём, что
//! в [`crate::sidecar::SidecarError`], который переиспользует
//! `LaunchFailedReason`: детали одинаковы по обе стороны границы, и лишняя
//! конвертация только добавила бы места, где они могут разойтись.

use crate::types::{ProbeError, ProbeErrorDetails, ProbeErrorKind, YtDlpFailureReason};

/// Почему не удалось разобрать ссылку.
///
/// Варианты конструирует TL-31 (классификация вывода yt-dlp) и TL-32
/// (`NotAUrl` — до запуска процесса, `Timeout` — по сработавшему порогу);
/// здесь они объявлены заранее, потому что от них зависят обе задачи
/// сразу и контракт должен существовать до них.
///
/// Детали (`details`) несут все варианты, кроме [`ProbeFailure::NotAUrl`]:
/// он возникает до запуска yt-dlp (Ф-2, С-4), процесса не существует, и
/// говорить в «Подробнее» просто нечего.
///
/// `#[allow(dead_code)]` — по той же причине, что и у типов контракта в
/// [`crate::types`]: объявление опережает код, который его конструирует,
/// и снимается вместе с TL-31/TL-32.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProbeFailure {
    /// Ввод не является http(s)-ссылкой. Процесс не порождался вовсе.
    #[error("ввод не является http(s)-ссылкой на ролик")]
    NotAUrl,

    /// Ролик удалён, снят с публикации или никогда не существовал.
    ///
    /// Приватного ролика здесь нет, хотя Ф-6 приписывает «скрыт» именно
    /// сюда: на пине 2026.08.19 один и тот же приватный ролик отвечает в
    /// соседних запусках то развёрнутым «Private video. Sign in if you've
    /// been granted access…», то безликим «Please sign in.», неотличимым
    /// от честного «требуется вход» (обе формулировки сняты в фикстуры
    /// TL-31). Детерминированного правила, которое кладёт приватный ролик
    /// сюда, попросту не существует — есть только правило, кладущее его в
    /// [`ProbeFailure::SignInRequired`]. К-3(в) эпика допускает для
    /// приватного ролика оба исхода.
    #[error("ролик недоступен: удалён, снят с публикации или не существует")]
    VideoUnavailable { details: ProbeErrorDetails },

    /// Нужен вход в аккаунт YouTube: возрастное ограничение, подписка,
    /// приватный ролик (см. [`ProbeFailure::VideoUnavailable`] — почему
    /// приватный попадает сюда, а не туда).
    #[error("ролик требует входа в аккаунт YouTube")]
    SignInRequired { details: ProbeErrorDetails },

    /// Ролик заблокирован для страны пользователя.
    #[error("ролик недоступен в регионе пользователя")]
    RegionBlocked { details: ProbeErrorDetails },

    /// Нет соединения с интернетом.
    #[error("нет соединения с интернетом")]
    NetworkUnavailable { details: ProbeErrorDetails },

    /// Ссылка ведёт на плейлист или канал, а не на отдельный ролик.
    #[error("ссылка ведёт на плейлист или канал, а не на отдельный ролик")]
    PlaylistUnsupported { details: ProbeErrorDetails },

    /// Идущий эфир или запланированная премьера.
    #[error("ролик — идущая прямая трансляция или запланированная премьера")]
    LiveUnsupported { details: ProbeErrorDetails },

    /// Сбой yt-dlp, не отнесённый ни к одному классу выше.
    ///
    /// `reason` — обязательное поле, а не `Option`: сигнатура устаревшего
    /// yt-dlp либо опознана, либо нет, третьего состояния не бывает, и
    /// [`YtDlpFailureReason::Generic`] — честное «не опознана».
    ///
    /// В текст ошибки под-причина не интерполируется: этот текст уезжает в
    /// `message` и оттуда — в свёрнутое «Подробнее», то есть на экран, а
    /// имя Rust-варианта пользователю не говорит ничего. Фронтенду
    /// под-причина приходит отдельным полем `reason`; в лог её пишет
    /// вызывающий вместе с классом (как это делает `commands::ytdlp`).
    #[error("yt-dlp не смог получить данные о ролике")]
    YtDlpFailure {
        reason: YtDlpFailureReason,
        details: ProbeErrorDetails,
    },

    /// Разбор не уложился в отведённое время; процесс убит.
    #[error("разбор не уложился в отведённое время ({secs} с) и был прерван")]
    Timeout {
        secs: u64,
        details: ProbeErrorDetails,
    },
}

#[allow(dead_code)]
impl ProbeFailure {
    /// Класс ошибки для фронтенда.
    pub fn kind(&self) -> ProbeErrorKind {
        match self {
            Self::NotAUrl => ProbeErrorKind::NotAUrl,
            Self::VideoUnavailable { .. } => ProbeErrorKind::VideoUnavailable,
            Self::SignInRequired { .. } => ProbeErrorKind::SignInRequired,
            Self::RegionBlocked { .. } => ProbeErrorKind::RegionBlocked,
            Self::NetworkUnavailable { .. } => ProbeErrorKind::NetworkUnavailable,
            Self::PlaylistUnsupported { .. } => ProbeErrorKind::PlaylistUnsupported,
            Self::LiveUnsupported { .. } => ProbeErrorKind::LiveUnsupported,
            Self::YtDlpFailure { .. } => ProbeErrorKind::YtDlpFailure,
            Self::Timeout { .. } => ProbeErrorKind::Timeout,
        }
    }

    /// Проекция на контракт Rust↔TS.
    ///
    /// `message` — формулировка ядра (`Display` этой ошибки): она идёт в
    /// «Подробнее» и в лог, но не на экран как основной текст — тексты по
    /// классам задаёт UI (дизайн E2, Н-4).
    pub fn to_contract(&self) -> ProbeError {
        ProbeError {
            kind: self.kind(),
            message: self.to_string(),
            reason: match self {
                Self::YtDlpFailure { reason, .. } => Some(*reason),
                _ => None,
            },
            timeout_secs: match self {
                Self::Timeout { secs, .. } => Some(*secs),
                _ => None,
            },
            // Пустые детали границу не пересекают: «Подробнее», за
            // которым ничего нет, — это состояние без содержания, а не
            // диагностика (Н-4, doc [`ProbeErrorDetails`]).
            details: self
                .details()
                .filter(|details| !details.is_empty())
                .cloned(),
        }
    }

    /// Технические детали, если процесс успел что-то о себе сообщить.
    fn details(&self) -> Option<&ProbeErrorDetails> {
        match self {
            Self::NotAUrl => None,
            Self::VideoUnavailable { details }
            | Self::SignInRequired { details }
            | Self::RegionBlocked { details }
            | Self::NetworkUnavailable { details }
            | Self::PlaylistUnsupported { details }
            | Self::LiveUnsupported { details }
            | Self::YtDlpFailure { details, .. }
            | Self::Timeout { details, .. } => Some(details),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn details() -> ProbeErrorDetails {
        ProbeErrorDetails {
            stderr_tail: Some("ERROR: Video unavailable".to_string()),
            exit_code: Some(1),
        }
    }

    /// Все девять классов Ф-6 с представителем каждого.
    fn all_variants() -> Vec<(ProbeFailure, ProbeErrorKind)> {
        vec![
            (ProbeFailure::NotAUrl, ProbeErrorKind::NotAUrl),
            (
                ProbeFailure::VideoUnavailable { details: details() },
                ProbeErrorKind::VideoUnavailable,
            ),
            (
                ProbeFailure::SignInRequired { details: details() },
                ProbeErrorKind::SignInRequired,
            ),
            (
                ProbeFailure::RegionBlocked { details: details() },
                ProbeErrorKind::RegionBlocked,
            ),
            (
                ProbeFailure::NetworkUnavailable { details: details() },
                ProbeErrorKind::NetworkUnavailable,
            ),
            (
                ProbeFailure::PlaylistUnsupported { details: details() },
                ProbeErrorKind::PlaylistUnsupported,
            ),
            (
                ProbeFailure::LiveUnsupported { details: details() },
                ProbeErrorKind::LiveUnsupported,
            ),
            (
                ProbeFailure::YtDlpFailure {
                    reason: YtDlpFailureReason::Generic,
                    details: details(),
                },
                ProbeErrorKind::YtDlpFailure,
            ),
            (
                ProbeFailure::Timeout {
                    secs: 30,
                    // Убитый по таймауту процесс не оставляет своего кода
                    // завершения, но успевает что-то написать в stderr.
                    details: ProbeErrorDetails {
                        stderr_tail: Some("[youtube] Downloading player".to_string()),
                        exit_code: None,
                    },
                },
                ProbeErrorKind::Timeout,
            ),
        ]
    }

    #[test]
    fn maps_every_variant_to_its_own_contract_kind() {
        let variants = all_variants();

        // Ф-6 — ровно девять классов; и лишний, и потерянный ломают
        // таблицу текстов на стороне UI.
        assert_eq!(variants.len(), 9);

        for (failure, expected_kind) in variants {
            assert_eq!(failure.kind(), expected_kind);

            let contract = failure.to_contract();
            assert_eq!(contract.kind, expected_kind);
            assert!(
                !contract.message.is_empty(),
                "сообщение обязано быть непустым: по нему пишется лог и \
                 наполняется «Подробнее»"
            );
        }
    }

    #[test]
    fn carries_the_failure_reason_only_for_the_yt_dlp_failure_class() {
        for (failure, kind) in all_variants() {
            let contract = failure.to_contract();
            if kind == ProbeErrorKind::YtDlpFailure {
                assert_eq!(contract.reason, Some(YtDlpFailureReason::Generic));
            } else {
                assert_eq!(contract.reason, None);
            }
        }
    }

    #[test]
    fn carries_the_timeout_threshold_only_for_the_timeout_class() {
        for (failure, kind) in all_variants() {
            let contract = failure.to_contract();
            if kind == ProbeErrorKind::Timeout {
                assert_eq!(contract.timeout_secs, Some(30));
            } else {
                assert_eq!(contract.timeout_secs, None);
            }
        }
    }

    #[test]
    fn not_a_url_is_the_only_class_without_technical_details() {
        for (failure, kind) in all_variants() {
            let contract = failure.to_contract();
            if kind == ProbeErrorKind::NotAUrl {
                assert_eq!(
                    contract.details, None,
                    "процесс не запускался (Ф-2), «Подробнее» показывать нечего"
                );
            } else {
                assert!(contract.details.is_some());
            }
        }
    }

    #[test]
    fn passes_stderr_tail_and_exit_code_through_to_the_contract() {
        let contract = ProbeFailure::VideoUnavailable { details: details() }.to_contract();

        assert_eq!(contract.details, Some(details()));
    }

    #[test]
    fn empty_details_do_not_cross_the_command_boundary() {
        // Процесс убит раньше, чем что-либо сказал: показывать в
        // «Подробнее» нечего, и пустой объект туда не уезжает.
        let contract = ProbeFailure::Timeout {
            secs: 30,
            details: ProbeErrorDetails {
                stderr_tail: None,
                exit_code: None,
            },
        }
        .to_contract();

        assert_eq!(contract.kind, ProbeErrorKind::Timeout);
        assert_eq!(contract.timeout_secs, Some(30));
        assert_eq!(contract.details, None);
    }

    #[test]
    fn no_message_leaks_a_rust_identifier_to_the_user() {
        // `message` виден в «Подробнее» (Н-4): в нём не должно быть имён
        // вариантов Rust — под-причина едет отдельным полем `reason`.
        for (failure, _) in all_variants() {
            let message = failure.to_contract().message;
            for identifier in ["Generic", "Outdated", "ProbeFailure", "YtDlpFailureReason"] {
                assert!(
                    !message.contains(identifier),
                    "«{message}» содержит Rust-идентификатор {identifier}"
                );
            }
        }
    }

    #[test]
    fn recognised_outdated_signature_changes_the_reason_not_the_class() {
        let generic = ProbeFailure::YtDlpFailure {
            reason: YtDlpFailureReason::Generic,
            details: details(),
        }
        .to_contract();
        let outdated = ProbeFailure::YtDlpFailure {
            reason: YtDlpFailureReason::Outdated,
            details: details(),
        }
        .to_contract();

        assert_eq!(generic.kind, outdated.kind);
        assert_ne!(generic.reason, outdated.reason);
    }
}
