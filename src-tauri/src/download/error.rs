//! Типизированная ошибка скачивания (Ф-10 эпика E3, TL-38).
//!
//! Девять вариантов — ровно девять классов таблицы ошибок дизайна, ни
//! больше ни меньше: фронтенд по классу выбирает заголовок и пояснение,
//! поэтому появление десятого класса или исчезновение любого из девяти —
//! изменение контракта, а не деталь реализации. Пять классов свои,
//! четыре переиспользуют смысловые классы разбора E2 (см. doc
//! [`DownloadErrorKind`] — там же перечислено, каких классов E2 в девятке
//! сознательно нет и почему).
//!
//! Технические детали ([`DownloadErrorDetails`]) взяты из контракта, а не
//! продублированы своим типом домена — тот же приём, что в
//! [`crate::probe::ProbeFailure`]: детали одинаковы по обе стороны
//! границы, и лишняя конвертация только добавила бы места, где они могут
//! разойтись.

use crate::types::{
    DownloadCommandError, DownloadCommandErrorKind, DownloadError, DownloadErrorDetails,
    DownloadErrorKind, PartialData, QueueTaskRef, YtDlpFailureReason,
};

/// Почему скачивание не дошло до готового файла.
///
/// Варианты конструирует классификация вывода yt-dlp и политика повторов
/// (TL-43) и оркестрация задачи (TL-44: недоступная папка назначения,
/// исчерпание попыток, исход склейки).
///
/// Судьба частично скачанного здесь **не** хранится: она не свойство
/// класса, а результат того, что ядро успело сделать с диском к моменту
/// отказа, и потому передаётся отдельным аргументом в [`to_contract`].
/// Ожидаемое значение для каждого класса — в таблице ниже; отличаться от
/// неё имеет право не «когда захочется», а ровно в двух случаях, которые
/// таблица дизайна и описывает:
///
/// | Класс | Частичное на диске |
/// |---|---|
/// | `ConnectionLost` | [`PartialData::Kept`] — повтор докачивает с места |
/// | `DiskFull` | [`PartialData::Kept`] — освободить место и продолжить |
/// | `StaleFormat` | [`PartialData::Removed`]: докачка того же формата невозможна по построению |
/// | `MergeFailed` | [`PartialData::Kept`] — оба потока целы, повтор пересобирает файл |
/// | `DestinationUnavailable` | [`PartialData::Kept`], если папка ещё доступна; иначе честнее [`PartialData::Removed`] |
/// | `VideoUnavailable` | [`PartialData::Removed`] — докачивать больше нечего |
/// | `SignInRequired` | [`PartialData::Removed`] |
/// | `RegionBlocked` | [`PartialData::Removed`] |
/// | `YtDlpFailure` | [`PartialData::Kept`] — неизвестно, поможет ли повтор, но удалять вслепую опаснее |
///
/// Второй случай общий для всех девяти: отказ, случившийся до первого
/// принятого байта (обычная судьба классов E2 — они всплывают ещё в фазе
/// `fetching`), даёт [`PartialData::NothingCreated`] — удалять было
/// нечего, и говорить пользователю «данные удалены» было бы неправдой.
///
/// [`to_contract`]: DownloadFailure::to_contract
// Конструировать варианты начнут TL-43 и TL-44; до тех пор контракт живёт
// без вызывающего — так же, как жил контракт E2 до своей оркестрации.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DownloadFailure {
    /// Соединение потеряно, попытки исчерпаны (С-7).
    ///
    /// `attempts` — сколько попыток подряд не дали ни одного байта
    /// продвижения; на экран это число не идёт (текст класса про попытки
    /// говорит без цифр), но в логе и в «Подробнее» отличает «сеть
    /// пропала на минуту» от «её не было всё время».
    #[error("соединение потеряно: {attempts} попыток подряд без продвижения")]
    ConnectionLost {
        attempts: u32,
        details: DownloadErrorDetails,
    },

    /// Место на диске кончилось по ходу записи (С-9).
    #[error("на диске не осталось места для файла загрузки")]
    DiskFull { details: DownloadErrorDetails },

    /// Выбранный формат больше не отдаётся (С-10).
    ///
    /// Тихой подмены качества «ближайшим» не делается: пользователь выбрал
    /// конкретную строку лестницы, и скачать вместо неё другую — обман.
    /// Чинится новым разбором по действию пользователя (Н-2 E2 запрещает
    /// фоновые повторы), а не повтором этой же задачи.
    #[error("выбранный формат больше не доступен: данные разбора устарели")]
    StaleFormat { details: DownloadErrorDetails },

    /// Склеить не вышло (С-11).
    ///
    /// Класс отличим от сетевых сбоев ровно потому, что склейку ведёт
    /// отдельный процесс ffmpeg, запущенный ядром (решение дизайна по
    /// Ф-9): любая ошибка **этого** процесса и есть «не удалось склеить»,
    /// без текстовых эвристик поверх чужого stderr.
    ///
    /// `reason` — почему именно, и он не украшение (TL-130). Под этим
    /// классом живут два разных события: отказ ffmpeg и «файлов потоков к
    /// склейке нет», где ffmpeg не запускался вовсе. Пока причина была
    /// одна на двоих, второму случаю выдавался текст первого, и
    /// пользователь на живой Windows читал про ошибку ffmpeg, которого не
    /// было. Класс при этом общий намеренно: судьба частичного
    /// ([`PartialData::Kept`]) и польза повтора у обоих совпадают, а
    /// девятка классов Ф-10 от подпричины не растёт — как `reason` у
    /// [`DownloadFailure::YtDlpFailure`].
    #[error("не удалось склеить видео и звук: {reason}")]
    MergeFailed {
        reason: MergeFailedReason,
        details: DownloadErrorDetails,
    },

    /// Папка назначения недоступна: нет прав либо её не существует.
    ///
    /// `reason` — формулировка ОС (или ядра), уезжающая в `message`:
    /// процесса за этим отказом может не быть вовсе, и без неё в логе
    /// осталась бы одна общая фраза без различия «нет прав» и «папки
    /// нет». Момент обнаружения (проверка перед стартом или классификация
    /// ошибки записи по факту) контракт не фиксирует — дизайн оставил его
    /// реализации (TL-44); в любом случае это отказ **задачи**, попадающий
    /// в панель, а не отказ команды старта: id задачи выдаётся раньше.
    #[error("папка назначения недоступна: {reason}")]
    DestinationUnavailable {
        reason: String,
        details: DownloadErrorDetails,
    },

    /// Ролик удалён, снят с публикации или не существует (класс E2).
    #[error("ролик недоступен: удалён, снят с публикации или не существует")]
    VideoUnavailable { details: DownloadErrorDetails },

    /// Нужен вход в аккаунт YouTube (класс E2).
    #[error("ролик требует входа в аккаунт YouTube")]
    SignInRequired { details: DownloadErrorDetails },

    /// Ролик заблокирован для страны пользователя (класс E2).
    #[error("ролик недоступен в регионе пользователя")]
    RegionBlocked { details: DownloadErrorDetails },

    /// Сбой yt-dlp, не отнесённый ни к одному классу выше (класс E2).
    ///
    /// Сюда же попадает и таймаут фазы «Подготовка»: отдельного класса
    /// `timeout`, как в E2, девятка E3 не содержит.
    ///
    /// `reason` — обязательное поле, а не `Option`, дословно как у
    /// [`crate::probe::ProbeFailure`]: сигнатура устаревшего yt-dlp либо
    /// опознана, либо нет, третьего состояния не бывает, и
    /// [`YtDlpFailureReason::Generic`] — честное «не опознана». Маркеры
    /// распознавания написаны и покрыты фикстурами ещё в E2 (TL-31) —
    /// классификации E3 (TL-43) их переиспользовать, а не изобретать
    /// заново.
    ///
    /// В текст ошибки под-причина не интерполируется: он уезжает в
    /// `message`, то есть в «Подробнее» на экран, а имя Rust-варианта
    /// пользователю не говорит ничего. Фронтенду под-причина приходит
    /// отдельным полем `reason`.
    #[error("yt-dlp не смог скачать ролик")]
    YtDlpFailure {
        reason: YtDlpFailureReason,
        details: DownloadErrorDetails,
    },
}

/// Почему склейка не состоялась ([`DownloadFailure::MergeFailed`], TL-130).
///
/// Подпричина домена, а не контракта: границу она пересекает только
/// текстом `message`, поле `reason` у [`crate::types::DownloadError`]
/// остаётся за `ytDlpFailure`. Фронтенду различать эти два случая незачем
/// — действия пользователя (повтор, «Подробнее») у них совпадают, — а вот
/// говорить ему неправду про чужой процесс нельзя.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MergeFailedReason {
    /// Процесс ffmpeg запускался и завершился ошибкой.
    #[error("ffmpeg завершился ошибкой")]
    FfmpegFailed,

    /// Файлов потоков к склейке не нашлось: yt-dlp не назвал файл потока
    /// либо названный файл пропал с диска до склейки. Склеивать было
    /// нечего, **ffmpeg не запускался** — и упоминать его в тексте
    /// значило бы отправить пользователя искать несуществующую поломку.
    #[error("файлов скачанных потоков нет, склеивать было нечего")]
    StreamsMissing,
}

#[allow(dead_code)]
impl DownloadFailure {
    /// Класс ошибки для фронтенда.
    pub fn kind(&self) -> DownloadErrorKind {
        match self {
            Self::ConnectionLost { .. } => DownloadErrorKind::ConnectionLost,
            Self::DiskFull { .. } => DownloadErrorKind::DiskFull,
            Self::StaleFormat { .. } => DownloadErrorKind::StaleFormat,
            Self::MergeFailed { .. } => DownloadErrorKind::MergeFailed,
            Self::DestinationUnavailable { .. } => DownloadErrorKind::DestinationUnavailable,
            Self::VideoUnavailable { .. } => DownloadErrorKind::VideoUnavailable,
            Self::SignInRequired { .. } => DownloadErrorKind::SignInRequired,
            Self::RegionBlocked { .. } => DownloadErrorKind::RegionBlocked,
            Self::YtDlpFailure { .. } => DownloadErrorKind::YtDlpFailure,
        }
    }

    /// Проекция на контракт Rust↔TS — единственный способ получить
    /// [`DownloadError`].
    ///
    /// `partial_data` передаётся аргументом, а не берётся из ошибки: что
    /// осталось на диске, знает не тот, кто классифицировал вывод
    /// процесса, а тот, кто уже закончил подчистку (таблица в doc
    /// [`DownloadFailure`]).
    ///
    /// `retryable` не аргумент и не поле: оно выводится из класса
    /// ([`DownloadErrorKind::is_retryable`]), потому что таблица
    /// «Повторить?» безусловна. Иначе на месте вызова можно было бы
    /// предложить повтор там, где дизайн его запретил.
    ///
    /// `message` — `Display` этой ошибки: она идёт в «Подробнее» и в лог,
    /// но не на экран как основной текст — тексты по классам задаёт UI
    /// (таблица дизайна E3, Н-4).
    pub fn to_contract(&self, partial_data: PartialData) -> DownloadError {
        let kind = self.kind();

        DownloadError {
            kind,
            message: self.to_string(),
            retryable: kind.is_retryable(),
            reason: match self {
                Self::YtDlpFailure { reason, .. } => Some(*reason),
                _ => None,
            },
            partial_data,
            // Пустые детали границу не пересекают: «Подробнее», за
            // которым ничего нет, — это состояние без содержания, а не
            // диагностика (Н-4, doc [`DownloadErrorDetails`]).
            details: Some(self.details())
                .filter(|details| !details.is_empty())
                .cloned(),
        }
    }

    /// Технические детали, если процессу было что о себе сообщить.
    ///
    /// В отличие от [`crate::probe::ProbeFailure`], где `NotAUrl` возникал
    /// до запуска процесса и деталей не имел вовсе, здесь поле есть у всех
    /// девяти вариантов: отказ без процесса за спиной
    /// (`DestinationUnavailable` при проверке до старта) выражается
    /// пустыми деталями, которые [`to_contract`] и отбрасывает.
    ///
    /// [`to_contract`]: DownloadFailure::to_contract
    fn details(&self) -> &DownloadErrorDetails {
        match self {
            Self::ConnectionLost { details, .. }
            | Self::DiskFull { details }
            | Self::StaleFormat { details }
            | Self::MergeFailed { details, .. }
            | Self::DestinationUnavailable { details, .. }
            | Self::VideoUnavailable { details }
            | Self::SignInRequired { details }
            | Self::RegionBlocked { details }
            | Self::YtDlpFailure { details, .. } => details,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn details() -> DownloadErrorDetails {
        DownloadErrorDetails {
            stderr_tail: Some("ERROR: unable to download video data".to_string()),
            exit_code: Some(1),
        }
    }

    /// Все девять классов Ф-10 с представителем каждого и той судьбой
    /// частичного, которую для него называет таблица дизайна.
    fn all_variants() -> Vec<(DownloadFailure, DownloadErrorKind, PartialData)> {
        vec![
            (
                DownloadFailure::ConnectionLost {
                    attempts: 6,
                    details: details(),
                },
                DownloadErrorKind::ConnectionLost,
                PartialData::Kept,
            ),
            (
                DownloadFailure::DiskFull { details: details() },
                DownloadErrorKind::DiskFull,
                PartialData::Kept,
            ),
            (
                DownloadFailure::StaleFormat { details: details() },
                DownloadErrorKind::StaleFormat,
                PartialData::Removed,
            ),
            (
                DownloadFailure::MergeFailed {
                    reason: MergeFailedReason::FfmpegFailed,
                    details: details(),
                },
                DownloadErrorKind::MergeFailed,
                PartialData::Kept,
            ),
            (
                DownloadFailure::DestinationUnavailable {
                    reason: "нет прав на запись".to_string(),
                    details: DownloadErrorDetails {
                        stderr_tail: None,
                        exit_code: None,
                    },
                },
                DownloadErrorKind::DestinationUnavailable,
                PartialData::Kept,
            ),
            (
                DownloadFailure::VideoUnavailable { details: details() },
                DownloadErrorKind::VideoUnavailable,
                PartialData::Removed,
            ),
            (
                DownloadFailure::SignInRequired { details: details() },
                DownloadErrorKind::SignInRequired,
                PartialData::Removed,
            ),
            (
                DownloadFailure::RegionBlocked { details: details() },
                DownloadErrorKind::RegionBlocked,
                PartialData::Removed,
            ),
            (
                DownloadFailure::YtDlpFailure {
                    reason: YtDlpFailureReason::Generic,
                    details: details(),
                },
                DownloadErrorKind::YtDlpFailure,
                PartialData::Kept,
            ),
        ]
    }

    #[test]
    fn maps_every_variant_to_its_own_contract_kind() {
        let variants = all_variants();

        // Ф-10 плюс декомпозиция E3 — ровно девять классов; и лишний, и
        // потерянный ломают таблицу текстов на стороне UI.
        assert_eq!(variants.len(), 9);

        for (failure, expected_kind, partial) in variants {
            assert_eq!(failure.kind(), expected_kind);

            let contract = failure.to_contract(partial);
            assert_eq!(contract.kind, expected_kind);
            assert!(
                !contract.message.is_empty(),
                "сообщение обязано быть непустым: по нему пишется лог и \
                 наполняется «Подробнее»"
            );
        }
    }

    #[test]
    fn retryability_comes_from_the_class_not_from_the_call_site() {
        // Единственный конструктор контрактной ошибки не даёт предложить
        // повтор там, где дизайн его запретил: значение не аргумент.
        for (failure, kind, partial) in all_variants() {
            assert_eq!(failure.to_contract(partial).retryable, kind.is_retryable());
        }

        let stale =
            DownloadFailure::StaleFormat { details: details() }.to_contract(PartialData::Removed);
        assert!(
            !stale.retryable,
            "повтор с тем же форматом провалится тем же образом — чинит \
             только новый разбор"
        );
    }

    #[test]
    fn what_is_left_on_disk_is_reported_not_guessed_from_the_class() {
        for (failure, _, partial) in all_variants() {
            assert_eq!(failure.to_contract(partial).partial_data, partial);
        }

        // Тот самый класс, для которого таблица дизайна не постоянна:
        // «сохраняется, если технически осталось доступным». Если папка
        // исчезла вместе с данными, ядро отчитывается фактом, а не
        // таблицей.
        let vanished = DownloadFailure::DestinationUnavailable {
            reason: "папка не существует".to_string(),
            details: DownloadErrorDetails {
                stderr_tail: None,
                exit_code: None,
            },
        }
        .to_contract(PartialData::Removed);
        assert_eq!(vanished.partial_data, PartialData::Removed);

        // И общий для всех девяти случай: отказ до первого принятого
        // байта (классы E2 всплывают ещё в фазе «Подготовка») ничего не
        // удаляет — удалять нечего.
        let early = DownloadFailure::VideoUnavailable { details: details() }
            .to_contract(PartialData::NothingCreated);
        assert_eq!(early.partial_data, PartialData::NothingCreated);
    }

    #[test]
    fn passes_stderr_tail_and_exit_code_through_to_the_contract() {
        let contract = DownloadFailure::MergeFailed {
            reason: MergeFailedReason::FfmpegFailed,
            details: details(),
        }
        .to_contract(PartialData::Kept);

        assert_eq!(contract.details, Some(details()));
    }

    #[test]
    fn empty_details_do_not_cross_the_command_boundary() {
        // За недоступной папкой назначения процесса может не быть вовсе:
        // показывать в «Подробнее» нечего, и пустой объект туда не
        // уезжает — иначе UI нарисует раскрывашку, за которой пусто.
        let contract = DownloadFailure::DestinationUnavailable {
            reason: "нет прав на запись".to_string(),
            details: DownloadErrorDetails {
                stderr_tail: None,
                exit_code: None,
            },
        }
        .to_contract(PartialData::Kept);

        assert_eq!(contract.kind, DownloadErrorKind::DestinationUnavailable);
        assert_eq!(contract.details, None);
        // Причина отказа при этом не теряется: она в `message`, то есть в
        // логе и в «Подробнее», если детали всё же появятся.
        assert!(contract.message.contains("нет прав на запись"));
    }

    #[test]
    fn the_merge_failure_without_streams_does_not_blame_ffmpeg() {
        // TL-130, дефект живой Windows (#137): «файлов потоков к склейке
        // нет» выдавалось текстом «ffmpeg завершился ошибкой», хотя ffmpeg
        // не запускался. Класс у двух причин общий, текст — нет.
        let missing = DownloadFailure::MergeFailed {
            reason: MergeFailedReason::StreamsMissing,
            details: details(),
        }
        .to_contract(PartialData::Kept);
        let ffmpeg = DownloadFailure::MergeFailed {
            reason: MergeFailedReason::FfmpegFailed,
            details: details(),
        }
        .to_contract(PartialData::Kept);

        assert!(
            !missing.message.contains("ffmpeg"),
            "ffmpeg не запускался, а текст про него: «{}»",
            missing.message
        );
        assert!(
            ffmpeg.message.contains("ffmpeg"),
            "отказ самого ffmpeg обязан его называть: «{}»",
            ffmpeg.message
        );
        assert_ne!(
            missing.message, ffmpeg.message,
            "две причины — два текста, иначе различать их незачем"
        );
        assert_eq!(
            (missing.kind, missing.retryable, missing.partial_data),
            (ffmpeg.kind, ffmpeg.retryable, ffmpeg.partial_data),
            "класс, повторяемость и судьба частичного у причин общие"
        );
    }

    #[test]
    fn no_message_leaks_a_rust_identifier_to_the_user() {
        // `message` виден в «Подробнее» (Н-4): в нём не должно быть имён
        // вариантов и типов Rust — класс едет отдельным полем `kind`.
        for (failure, _, partial) in all_variants() {
            let message = failure.to_contract(partial).message;
            for identifier in [
                "DownloadFailure",
                "DownloadErrorKind",
                "PartialData",
                "ConnectionLost",
                "DiskFull",
                "StaleFormat",
                "MergeFailed",
                "DestinationUnavailable",
                "VideoUnavailable",
                "SignInRequired",
                "RegionBlocked",
                "YtDlpFailure",
                "Generic",
                "Outdated",
                "YtDlpFailureReason",
                "MergeFailedReason",
                "FfmpegFailed",
                "StreamsMissing",
            ] {
                assert!(
                    !message.contains(identifier),
                    "«{message}» содержит Rust-идентификатор {identifier}"
                );
            }
        }
    }

    #[test]
    fn carries_the_failure_reason_only_for_the_yt_dlp_failure_class() {
        for (failure, kind, partial) in all_variants() {
            let contract = failure.to_contract(partial);
            if kind == DownloadErrorKind::YtDlpFailure {
                assert_eq!(contract.reason, Some(YtDlpFailureReason::Generic));
            } else {
                assert_eq!(contract.reason, None);
            }
        }
    }

    #[test]
    fn recognised_outdated_signature_changes_the_reason_not_the_class() {
        // Девятка классов от под-причины не растёт — меняется только текст
        // пояснения на экране (тот же приём, что в E2).
        let generic = DownloadFailure::YtDlpFailure {
            reason: YtDlpFailureReason::Generic,
            details: details(),
        }
        .to_contract(PartialData::Kept);
        let outdated = DownloadFailure::YtDlpFailure {
            reason: YtDlpFailureReason::Outdated,
            details: details(),
        }
        .to_contract(PartialData::Kept);

        assert_eq!(generic.kind, outdated.kind);
        assert_ne!(generic.reason, outdated.reason);
        // Повтор при устаревшем yt-dlp дизайн не запрещает: ценность
        // под-причины в честном объяснении, а не в другой кнопке.
        assert_eq!(generic.retryable, outdated.retryable);
    }

    #[test]
    fn the_number_of_exhausted_attempts_stays_in_the_diagnostics() {
        // Текст класса на экране про попытки говорит без цифр, но в логе
        // «сеть пропала на минуту» и «её не было всё время» — разные
        // истории.
        let contract = DownloadFailure::ConnectionLost {
            attempts: 6,
            details: details(),
        }
        .to_contract(PartialData::Kept);

        assert!(contract.message.contains('6'));
    }
}

/// Почему команда управления загрузкой отклонена — доменная сторона
/// [`DownloadCommandError`].
///
/// Отдельный тип, а не строки на месте вызова, по той же причине, что и
/// [`DownloadFailure`]: оркестрация (TL-44) отклоняет вызовы в шести
/// разных местах, и без общего типа текст собирался бы руками шесть раз —
/// а отладочное форматирование варианта, случайно попавшее в такой текст,
/// прошло бы незамеченным. Здесь его ловит тот же сторож, что и у отказа
/// задачи.
///
/// Классы отказа **команды** и классы отказа **задачи** не смешиваются:
/// первые описывают отказ выполнить вызов (до которого исправный фронтенд
/// не доводит — кнопок, которых нельзя нажать, он не показывает), вторые —
/// судьбу задачи, которую рисует панель.
// Конструировать варианты начнёт TL-44 вместе с самими командами.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DownloadCommandRejection {
    /// Задачи с таким идентификатором ядро не знает.
    ///
    /// Идентификатор уезжает в `message`, то есть в лог: он выдан самим
    /// ядром и непрозрачен, разбирать в нём нечего, но по нему видно,
    /// какой именно вызов промахнулся.
    #[error("задача {task_id} ядру неизвестна")]
    UnknownTask { task_id: String },

    /// Повтор запрошен для задачи, которая не завершилась ошибкой.
    #[error("повтор доступен только для задачи, завершившейся ошибкой")]
    NotFailed,

    /// Повтор запрошен для класса, где он заведомо бесполезен
    /// ([`DownloadErrorKind::is_retryable`] = `false`).
    ///
    /// Класс в текст не интерполируется: он и так известен фронтенду из
    /// последнего события, а имя Rust-варианта в логе ничего не добавило
    /// бы к уже записанному там отказу задачи.
    #[error("для этого класса ошибки повтор заведомо бесполезен")]
    NotRetryable,

    /// В запросе нет ни одного идентификатора потока: скачивать нечего
    /// (инвариант [`crate::types::QualityStreams::has_any`]).
    #[error("в запросе нет ни одного идентификатора потока")]
    NoStreamsSelected,

    /// Ссылка не является http(s)-адресом (Ф-1).
    #[error("ссылка не является http(s)-адресом")]
    InvalidUrl,

    /// Тот же ролик с тем же пунктом качества уже стоит в очереди
    /// нетерминальной задачей (Ф-8, Р-5 эпика E4).
    ///
    /// Ссылка на существующую задачу едет на провод, а не в текст:
    /// текст отказа собирает UI по структурным полям, а `message`
    /// остаётся диагностикой для лога (дизайн E4, «Отказ по дублю»).
    /// Идентификатор в текст всё же попадает — по нему в логе видно,
    /// какую именно задачу ядро посчитало дублем.
    #[error("задача {} уже стоит в очереди с тем же качеством", existing.task_id)]
    DuplicateTask { existing: QueueTaskRef },

    /// Скрыть просят задачу, которая ещё не дошла до терминальной фазы
    /// (Ф-1 эпика E4, дизайн: «скрывать можно только завершённое»).
    #[error("скрыть можно только завершённую задачу")]
    TaskNotFinished,
}

#[allow(dead_code)]
impl DownloadCommandRejection {
    /// Класс отказа для фронтенда.
    pub fn kind(&self) -> DownloadCommandErrorKind {
        match self {
            Self::UnknownTask { .. } => DownloadCommandErrorKind::UnknownTask,
            Self::NotFailed => DownloadCommandErrorKind::NotFailed,
            Self::NotRetryable => DownloadCommandErrorKind::NotRetryable,
            Self::NoStreamsSelected => DownloadCommandErrorKind::NoStreamsSelected,
            Self::InvalidUrl => DownloadCommandErrorKind::InvalidUrl,
            Self::DuplicateTask { existing } => DownloadCommandErrorKind::DuplicateTask {
                existing: existing.clone(),
            },
            Self::TaskNotFinished => DownloadCommandErrorKind::TaskNotFinished,
        }
    }

    /// Проекция на контракт — единственный способ получить
    /// [`DownloadCommandError`].
    pub fn to_contract(&self) -> DownloadCommandError {
        DownloadCommandError {
            kind: self.kind(),
            message: self.to_string(),
        }
    }
}

#[cfg(test)]
mod command_tests {
    use super::*;

    /// Ссылка на существующую задачу — образец для отказа по дублю.
    fn existing_task() -> QueueTaskRef {
        QueueTaskRef {
            task_id: "dl-1".to_string(),
            title: "Летний влог".to_string(),
            quality: crate::types::SelectedQuality {
                kind: crate::types::QualityKind::Standard,
                height_px: Some(720),
            },
        }
    }

    /// Все семь классов отказа команд с представителем каждого.
    fn all_rejections() -> Vec<(DownloadCommandRejection, DownloadCommandErrorKind)> {
        vec![
            (
                DownloadCommandRejection::UnknownTask {
                    task_id: "task-1".to_string(),
                },
                DownloadCommandErrorKind::UnknownTask,
            ),
            (
                DownloadCommandRejection::NotFailed,
                DownloadCommandErrorKind::NotFailed,
            ),
            (
                DownloadCommandRejection::NotRetryable,
                DownloadCommandErrorKind::NotRetryable,
            ),
            (
                DownloadCommandRejection::NoStreamsSelected,
                DownloadCommandErrorKind::NoStreamsSelected,
            ),
            (
                DownloadCommandRejection::InvalidUrl,
                DownloadCommandErrorKind::InvalidUrl,
            ),
            (
                DownloadCommandRejection::DuplicateTask {
                    existing: existing_task(),
                },
                DownloadCommandErrorKind::DuplicateTask {
                    existing: existing_task(),
                },
            ),
            (
                DownloadCommandRejection::TaskNotFinished,
                DownloadCommandErrorKind::TaskNotFinished,
            ),
        ]
    }

    #[test]
    fn maps_every_rejection_to_its_own_contract_kind() {
        let rejections = all_rejections();

        assert_eq!(rejections.len(), 7);

        for (rejection, expected_kind) in rejections {
            assert_eq!(rejection.kind(), expected_kind);

            let contract = rejection.to_contract();
            assert_eq!(contract.kind, expected_kind);
            assert!(
                !contract.message.is_empty(),
                "сообщение обязано быть непустым: по нему пишется лог"
            );
        }
    }

    #[test]
    fn no_rejection_message_leaks_a_rust_identifier() {
        // Тот же сторож, что у отказа задачи: отладочное форматирование
        // варианта не должно доехать ни до лога, ни до «Подробнее».
        for (rejection, _) in all_rejections() {
            let message = rejection.to_contract().message;
            for identifier in [
                "DownloadCommandRejection",
                "DownloadCommandErrorKind",
                "DownloadErrorKind",
                "DuplicateTask",
                "TaskNotFinished",
                "QueueTaskRef",
                "UnknownTask",
                "NotFailed",
                "NotRetryable",
                "NoStreamsSelected",
                "InvalidUrl",
            ] {
                assert!(
                    !message.contains(identifier),
                    "«{message}» содержит Rust-идентификатор {identifier}"
                );
            }
        }
    }

    #[test]
    fn an_unknown_task_keeps_its_identifier_in_the_diagnostics() {
        let contract = DownloadCommandRejection::UnknownTask {
            task_id: "task-7".to_string(),
        }
        .to_contract();

        assert!(contract.message.contains("task-7"));
    }
}
