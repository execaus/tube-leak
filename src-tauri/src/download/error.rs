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

use crate::types::{DownloadError, DownloadErrorDetails, DownloadErrorKind, PartialData};

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

    /// Потоки скачаны, ffmpeg завершился ошибкой (С-11).
    ///
    /// Класс отличим от сетевых сбоев ровно потому, что склейку ведёт
    /// отдельный процесс ffmpeg, запущенный ядром (решение дизайна по
    /// Ф-9): любая ошибка **этого** процесса и есть «не удалось склеить»,
    /// без текстовых эвристик поверх чужого stderr.
    #[error("не удалось склеить видео и звук: ffmpeg завершился ошибкой")]
    MergeFailed { details: DownloadErrorDetails },

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
    #[error("yt-dlp не смог скачать ролик")]
    YtDlpFailure { details: DownloadErrorDetails },
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
            | Self::MergeFailed { details }
            | Self::DestinationUnavailable { details, .. }
            | Self::VideoUnavailable { details }
            | Self::SignInRequired { details }
            | Self::RegionBlocked { details }
            | Self::YtDlpFailure { details } => details,
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
                DownloadFailure::MergeFailed { details: details() },
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
                DownloadFailure::YtDlpFailure { details: details() },
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
        let contract =
            DownloadFailure::MergeFailed { details: details() }.to_contract(PartialData::Kept);

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
            ] {
                assert!(
                    !message.contains(identifier),
                    "«{message}» содержит Rust-идентификатор {identifier}"
                );
            }
        }
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
