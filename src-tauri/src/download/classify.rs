//! Классификация исхода одной попытки скачивания (Ф-10, С-6…С-10) —
//! TL-43.
//!
//! Функция [`classify_attempt`] чистая: на входе — то, что осталось от
//! одного запуска yt-dlp (код завершения и stderr), на выходе — вердикт
//! о **попытке**, а не сразу о задаче. Процессов здесь не запускается,
//! таймеров не отсчитывается и решения «повторять ли» не принимается:
//! повторы — соседний модуль [`crate::download::retry`], запуск и
//! подчистка — оркестрация (TL-44).
//!
//! # Почему вердикт трёхзначный, а классов девять
//!
//! У разбора ссылки (E2) любой неуспех процесса — сразу класс отказа. У
//! скачивания это не так, и разница принципиальная: **транспортный сбой
//! отказом не является** (см. doc [`crate::types::DownloadErrorKind`]:
//! класса `networkUnavailable` в девятке нет намеренно). Оборвавшаяся
//! связь — повод уйти в цикл повторов С-6, и отказом она становится
//! только когда повторы исчерпаны, — тогда это `connectionLost`, и
//! собирает его политика повторов, а не эта функция. Отсюда
//! [`AttemptVerdict::Interrupted`]: «попытка не удалась, но повторять
//! осмысленно».
//!
//! # Что переиспользовано у разбора ссылки, а не написано заново
//!
//! Четыре класса девятки (`videoUnavailable`, `signInRequired`,
//! `regionBlocked`, `ytDlpFailure`) носят на проводе те же значения, что
//! классы E2, и распознаются **теми же** маркерами: извлечение текста
//! отказа ([`crate::probe::fatal_text`]), три общих класса с их порядком
//! ([`crate::probe::shared_failure_class`]), транспортный сбой
//! ([`crate::probe::is_transport_failure`]) и под-причина «устарел»
//! ([`crate::probe::yt_dlp_failure_reason`]) приезжают из `probe`
//! целиком. Своих здесь ровно три группы — «нет места», «формат
//! недоступен» и «папка назначения недоступна», то есть то, чего на
//! пути разбора не бывает по построению: разбор ничего не пишет на диск.
//!
//! Что это переиспользование не теоретическое, проверено живьём: те же
//! удалённый ролик и ролик с возрастным ограничением, что сняты для E2
//! разбором (`-J`), сняты ещё раз **скачиванием** (фикстуры
//! `video-unavailable`, `sign-in-required`) и дали дословно ту же
//! формулировку.

use crate::download::error::DownloadFailure;
use crate::probe::{
    fatal_text, is_transport_failure, normalize_typography, shared_failure_class,
    yt_dlp_failure_reason, SharedFailureClass,
};
use crate::sidecar::stderr_tail;
use crate::types::DownloadErrorDetails;

/// Всё, что осталось от одной попытки скачивания.
///
/// stdout сюда не входит, и это решение, а не упущение. У разбора ссылки
/// stdout — предмет классификации (успех определяется по метаданным); у
/// скачивания в stdout едет прогресс, который уже разобран построчно
/// ([`crate::download::progress`]) и агрегирован
/// ([`crate::download::aggregate`]) задолго до конца процесса. Отдавать
/// его сюда вторым потоком значило бы завести второе место, где решается
/// «скачалось ли», — а оно ровно одно: [`crate::download::aggregate::ProgressAggregator::is_complete`].
#[derive(Debug, Clone, Copy)]
pub struct AttemptOutcome<'a> {
    /// Код завершения процесса; `None` — процесс убит сигналом и своего
    /// кода не оставил (в том числе сторожем «нет продвижения», С-8).
    pub exit_code: Option<i32>,
    /// stderr процесса целиком. Наружу уходит только хвост (Н-4).
    pub stderr: &'a str,
}

/// Чем кончилась попытка.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttemptVerdict {
    /// Процесс дошёл до конца сам. Скачаны ли все потоки — вопрос не к
    /// коду завершения, а к агрегатору прогресса.
    Completed,

    /// Попытка оборвалась так, что повторить её осмысленно (С-6, С-8):
    /// транспорт умер или процесс убит сторожем.
    ///
    /// Детали едут с вердиктом, потому что пригодятся не здесь и не
    /// сейчас: когда попытки кончатся, из последних таких деталей
    /// соберётся [`DownloadFailure::ConnectionLost`] — иначе в
    /// «Подробнее» у исчерпания попыток не было бы ничего, кроме слова
    /// «попытки закончились».
    Interrupted { details: DownloadErrorDetails },

    /// Класс, который повтором той же попытки не чинится: решение по
    /// задаче принято, цикл повторов не начинается.
    Failed(DownloadFailure),
}

/// Разобрать исход попытки.
///
/// # Порядок проверок
///
/// Порядок нагружен ровно так же, как в [`crate::probe`]: группы
/// маркеров пересекаются словами, и первая совпавшая решает исход.
///
/// 1. **Код 0 — попытка дошла до конца.** Непустой stderr успеху не
///    мешает: yt-dlp печатает предупреждения в **каждом** снятом
///    запуске, включая успешный. Обычно это жалоба на отсутствие
///    JS-рантайма (восемь фикстур из девяти); девятая снята с
///    принудительным клиентом `web` и жалуется на своё
///    («n challenge solving failed»), но пустого stderr нет ни у одной.
/// 2. **Нет места на диске** — раньше всего остального. Повторять такую
///    попытку не только бесполезно: пять повторов с растущей паузой
///    закончились бы сообщением «соединение потеряно», то есть прямой
///    неправдой о причине.
/// 3. **Папка назначения недоступна** — по той же причине и с той же
///    ценой ошибки: «нет прав на запись» повторами не лечится.
/// 4. **Транспортный сбой → [`AttemptVerdict::Interrupted`]**, и здесь
///    же кончается сходство с E2, где это класс. Стоит перед
///    содержательными классами по тому же правилу, что в разборе: при
///    транспортном сбое ответа от YouTube не было вовсе, и выводы о
///    самом ролике сделаны не будут.
/// 5. **Формат недоступен** (С-10) — данные карточки устарели.
/// 6. **Общие с E2 классы**: регион → вход → «ролик недоступен», порядок
///    внутри — [`crate::probe::shared_failure_class`].
/// 7. **Процесс убит сигналом** (кода завершения нет) — тоже
///    [`AttemptVerdict::Interrupted`]. Так выглядит попытка, которую
///    оборвал сторож «ни байта за двадцать секунд» (С-8): в stderr при
///    этом нет ни одной строки `ERROR:` — снято живьём, фикстура
///    `stalled-killed-by-watchdog`. Шаг стоит **после** маркеров, а не
///    до: если сторож убил процесс, который уже успел напечатать «нет
///    места», правда — «нет места».
/// 8. **Всё остальное** — сбой yt-dlp; под-причина меняет только текст
///    пояснения на экране, но не класс.
pub fn classify_attempt(outcome: &AttemptOutcome<'_>) -> AttemptVerdict {
    if outcome.exit_code == Some(0) {
        return AttemptVerdict::Completed;
    }

    let text = fatal_text(outcome.stderr);
    let details = details(outcome);

    if contains_any(&text, &DISK_FULL_MARKERS) {
        return AttemptVerdict::Failed(DownloadFailure::DiskFull { details });
    }

    if let Some(reason) = destination_reason(&text) {
        return AttemptVerdict::Failed(DownloadFailure::DestinationUnavailable {
            reason: reason.to_owned(),
            details,
        });
    }

    if is_transport_failure(&text) || contains_any(&text, &DOWNLOAD_TRANSPORT_MARKERS) {
        return AttemptVerdict::Interrupted { details };
    }

    if contains_any(&text, &STALE_FORMAT_MARKERS) {
        return AttemptVerdict::Failed(DownloadFailure::StaleFormat { details });
    }

    if let Some(shared) = shared_failure_class(&text) {
        return AttemptVerdict::Failed(match shared {
            SharedFailureClass::RegionBlocked => DownloadFailure::RegionBlocked { details },
            SharedFailureClass::SignInRequired => DownloadFailure::SignInRequired { details },
            SharedFailureClass::VideoUnavailable => DownloadFailure::VideoUnavailable { details },
        });
    }

    if outcome.exit_code.is_none() {
        return AttemptVerdict::Interrupted { details };
    }

    AttemptVerdict::Failed(DownloadFailure::YtDlpFailure {
        reason: yt_dlp_failure_reason(&text),
        details,
    })
}

/// Технические детали отказа для «Подробнее» (Н-4).
fn details(outcome: &AttemptOutcome<'_>) -> DownloadErrorDetails {
    DownloadErrorDetails {
        stderr_tail: stderr_tail(outcome.stderr),
        exit_code: outcome.exit_code,
    }
}

/// Та же проверка маркеров, что у разбора ссылки, и **той же**
/// нормализацией типографских знаков (TL-122): своя копия сравнения
/// здесь есть только потому, что списки маркеров свои, а правило
/// сравнения — общее и приезжает из [`crate::probe`].
fn contains_any(text: &str, markers: &[&str]) -> bool {
    let text = normalize_typography(text);

    markers
        .iter()
        .any(|marker| text.contains(normalize_typography(marker).as_ref()))
}

/// Почему папка назначения недоступна — в тех же словах, которыми об
/// этом говорит контракт ([`DownloadFailure::DestinationUnavailable`]).
///
/// Формулировка короткая и своя, а не хвост чужого текста: полный текст
/// yt-dlp вместе с путём уже едет в `details.stderrTail`, а `reason`
/// уходит в `message`, то есть в лог и в «Подробнее» строкой, которую
/// читают глазами.
///
/// Признак делится надвое, и обе половины обязаны совпасть: сначала
/// **контекст записи** (yt-dlp сообщил, что не смог открыть файл, создать
/// каталог или переименовать результат), потом **причина** от ОС. Одной
/// причины мало: «no such file or directory» встречается в выводе по
/// поводам, к папке назначения отношения не имеющим, и без контекста
/// класс «Папка «Загрузки» недоступна» показывался бы наугад.
fn destination_reason(text: &str) -> Option<&'static str> {
    if !contains_any(text, &WRITE_CONTEXT_MARKERS) {
        return None;
    }

    if contains_any(text, &PERMISSION_MARKERS) {
        return Some("нет прав на запись");
    }
    if contains_any(text, &MISSING_PATH_MARKERS) {
        return Some("папка не существует");
    }
    None
}

/// Место на диске кончилось по ходу записи (С-9).
///
/// Первый маркер снят живьём: загрузка потока 134 весом 18,3 МБ на
/// образ диска в 6 МиБ дала `ERROR: unable to write data: [Errno 28] No
/// space left on device` (фикстура `disk-full`). Номер ошибки взят
/// отдельным маркером, потому что текст к нему приписывает libc, а не
/// yt-dlp, и на локализованной системе он может приехать переведённым.
///
/// Windows-формулировка **живьём не проверена** (Р-6, машины нет): взята
/// из штатного текста `ERROR_DISK_FULL`.
const DISK_FULL_MARKERS: [&str; 3] = [
    "no space left on device",
    "[errno 28]",
    // Windows: «There is not enough space on the disk».
    "not enough space on the disk",
];

/// Выбранный формат больше не отдаётся (С-10).
///
/// Снято живьём: `-f 999+140` по живому ролику даёт `ERROR: [youtube]
/// aqz-KE-bpKQ: Requested format is not available. Use --list-formats for
/// a list of available formats` (фикстура `stale-format`). Второй маркер
/// — та же ошибка без артикля, историческая формулировка yt-dlp;
/// **живьём не проверена**.
const STALE_FORMAT_MARKERS: [&str; 2] = [
    "requested format is not available",
    "requested format not available",
];

/// Контекст записи: yt-dlp говорит про файл или каталог назначения.
///
/// Первый маркер снят живьём (каталог с правами 555): `ERROR: unable to
/// open for writing: [Errno 13] Permission denied: '…/….mp4.part'`.
/// Остальные **живьём не проверены**: это соседние формулировки того же
/// семейства сообщений yt-dlp о записи. «unable to write data» попадает
/// сюда же, но до этой проверки не доходит — его перехватывает более
/// частный маркер «нет места».
const WRITE_CONTEXT_MARKERS: [&str; 4] = [
    "unable to open for writing",
    "unable to write data",
    "unable to create directory",
    "unable to rename file",
];

/// Прав на запись нет.
///
/// `[errno 13]` снят живьём; текст рядом с ним пишет libc, поэтому он
/// отдельным маркером. Windows-формулировка **живьём не проверена**.
const PERMISSION_MARKERS: [&str; 3] = [
    "[errno 13]",
    "permission denied",
    // Windows: «Access is denied».
    "access is denied",
];

/// Папки назначения нет.
///
/// **ЖИВЬЁМ НЕ ПРОВЕРЕНО**: воспроизвести исчезновение папки посреди
/// загрузки на macOS не вышло — открытый файловый дескриптор переживает
/// удаление каталога, и запись продолжается в файл, которого уже нет в
/// дереве. Маркеры взяты из штатных текстов ОС.
const MISSING_PATH_MARKERS: [&str; 4] = [
    "[errno 2]",
    "no such file or directory",
    // Windows.
    "the system cannot find the path",
    "cannot find the file specified",
];

/// Транспортные сбои, которых не бывает на пути разбора ссылки.
///
/// Разбор — один короткий запрос: он либо доехал, либо нет, и его отказы
/// уже перечислены в [`crate::probe`]. Скачивание держит соединение
/// минутами, и рвётся оно посреди чтения тела ответа — другими словами
/// и другими исключениями Python.
///
/// Снято живьём (транспорт оборван убийством прокси, через который шла
/// загрузка, — сеть не выключалась; см. README фикстур):
///
/// - «1327104 bytes read, 8897345 more expected» — тело ответа кончилось
///   раньше объявленной длины.
/// - «Read timed out» — транспорт замолчал, не закрыв сокет (модель
///   выключенного Wi-Fi). Эта строка приезжает в **stdout** и до
///   классификации доходит, только если процесс успел завершиться сам;
///   обычно раньше срабатывает сторож С-8.
///
/// «Connection reset by peer» и `[errno 54]` **живьём не проверены**:
/// подходящего разрыва воспроизвести не удалось, формулировки штатные.
///
/// # Чего здесь нет и не будет: «Giving up after N retries»
///
/// Соблазнительный маркер — им кончается снятая живьём фикстура обрыва,
/// и выглядит он как готовый признак «связь не восстановилась». Но эту
/// строку печатает менеджер повторов yt-dlp после **любого** повторяемого
/// семейства ошибок, включая состоявшийся ответ сервера (`HTTP Error
/// 4xx`) и неудачу открытия файла. Разбор ссылки исключает такие ответы
/// из транспортных маркеров сознательно и с записанным обоснованием
/// («это как раз состоявшийся ответ»), а через этот маркер они вернулись
/// бы обратно — и пользователь получил бы пять пауз и «соединение
/// потеряно» на ошибке, к сети отношения не имеющей. Ровно та цена,
/// которой обоснован порядок проверок выше.
///
/// Настоящий обрыв от снятия маркера ничего не теряет: в той же фикстуре
/// внутри обёртки стоит «Connection refused» — маркер разбора, который и
/// решает исход. Сторож — тест
/// `a_server_answer_wrapped_in_retries_is_not_a_transport_failure`.
const DOWNLOAD_TRANSPORT_MARKERS: [&str; 5] = [
    "more expected",
    "read timed out",
    "connection reset by peer",
    "[errno 54]",
    // Классическая формулировка yt-dlp, когда тело потока не удалось
    // получить целиком. ЖИВЬЁМ НЕ ПРОВЕРЕНО.
    "unable to download video data",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::download::fixtures::{self, OUTCOME_FIXTURES};
    use crate::download::progress::PROGRESS_TEMPLATE;
    use crate::sidecar::STDERR_TAIL_MAX_CHARS;
    use crate::types::{DownloadErrorKind, PartialData, YtDlpFailureReason};

    /// Что классификация обязана сказать про снятый исход.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Expected {
        /// Процесс дошёл до конца сам.
        Completed,
        /// Попытка оборвалась, повтор осмыслен (С-6/С-8).
        Interrupted,
        /// Класс отказа Ф-10.
        Failed(DownloadErrorKind),
        /// Класс «сбой yt-dlp» с конкретной под-причиной.
        YtDlpFailure(YtDlpFailureReason),
    }

    /// Снятые исходы и вердикт каждого.
    ///
    /// Таблица — источник истины теста, фикстуры хранят только факты
    /// («что напечатал процесс»), не ответ. Как снята каждая и что в ней
    /// живое, а что смоделировано, — в README рядом с фикстурами.
    const OUTCOMES: [(&str, Expected); 9] = [
        ("success-audio-only.json", Expected::Completed),
        (
            "disk-full.json",
            Expected::Failed(DownloadErrorKind::DiskFull),
        ),
        (
            "stale-format.json",
            Expected::Failed(DownloadErrorKind::StaleFormat),
        ),
        (
            "destination-read-only.json",
            Expected::Failed(DownloadErrorKind::DestinationUnavailable),
        ),
        (
            "video-unavailable.json",
            Expected::Failed(DownloadErrorKind::VideoUnavailable),
        ),
        (
            "sign-in-required.json",
            Expected::Failed(DownloadErrorKind::SignInRequired),
        ),
        (
            "ytdlp-failure-outdated.json",
            Expected::YtDlpFailure(YtDlpFailureReason::Outdated),
        ),
        ("connection-lost-mid-download.json", Expected::Interrupted),
        ("stalled-killed-by-watchdog.json", Expected::Interrupted),
    ];

    fn verdict(verdict: &AttemptVerdict) -> Expected {
        match verdict {
            AttemptVerdict::Completed => Expected::Completed,
            AttemptVerdict::Interrupted { .. } => Expected::Interrupted,
            AttemptVerdict::Failed(DownloadFailure::YtDlpFailure { reason, .. }) => {
                Expected::YtDlpFailure(*reason)
            }
            AttemptVerdict::Failed(failure) => Expected::Failed(failure.kind()),
        }
    }

    fn classify_fixture(name: &str) -> AttemptVerdict {
        let outcome = fixtures::outcome(name);
        classify_attempt(&AttemptOutcome {
            exit_code: outcome.exit_code,
            stderr: &outcome.stderr,
        })
    }

    #[test]
    fn classifies_every_captured_outcome_into_its_declared_verdict() {
        for (name, expected) in OUTCOMES {
            assert_eq!(
                verdict(&classify_fixture(name)),
                expected,
                "{name}: классификация разошлась с таблицей (см. README фикстур)"
            );
        }
    }

    #[test]
    fn the_list_of_outcomes_matches_the_files_on_disk() {
        // Фикстура, о которой таблица не знает, выглядела бы покрытым
        // случаем, не будучи им; удалённая — молча уменьшила бы набор.
        let mut declared: Vec<String> = OUTCOMES.iter().map(|(name, _)| (*name).into()).collect();
        declared.sort();

        assert_eq!(declared, fixtures::outcome_files_on_disk());
        assert_eq!(declared, {
            let mut listed: Vec<String> = OUTCOME_FIXTURES.iter().map(|&n| n.into()).collect();
            listed.sort();
            listed
        });
    }

    #[test]
    fn fixtures_are_real_output_of_the_pinned_yt_dlp() {
        // Та же связь, что у фикстур прогресса (TL-41) и лестницы
        // (TL-30): фикстуры заморожены, а yt-dlp — нет. Формулировки
        // ошибок задаёт апстрим, и смена пина обязана громко ломать этот
        // тест, а не тихо оставлять набор зелёным.
        let pinned = fixtures::pinned_yt_dlp_version();

        for (name, _) in OUTCOMES {
            let outcome = fixtures::outcome(name);

            assert_eq!(
                outcome.capture.yt_dlp_version, pinned,
                "{name}: исход снят не тем yt-dlp, который вложен в приложение \
                 ({pinned} по binaries.lock.json). Пин сменили — переснимите \
                 фикстуры по README, а не правьте эту строку"
            );
            assert_eq!(
                outcome.capture.progress_template, PROGRESS_TEMPLATE,
                "{name}: исход снят другим --progress-template. Шаблон сменили — \
                 переснимите фикстуры по README"
            );
            assert!(
                outcome
                    .capture
                    .argv
                    .iter()
                    .any(|arg| arg == "<progressTemplate>"),
                "{name}: в argv потерян плейсхолдер шаблона — команду съёмки \
                 больше не воспроизвести"
            );
            assert!(
                outcome.stderr.contains("WARNING:"),
                "{name}: yt-dlp предупреждает о чём-нибудь в каждом запуске по \
                 ролику YouTube (обычно об отсутствии JS-рантайма, а в \
                 фикстуре с принудительным клиентом — о своём) — stderr без \
                 предупреждений означает, что фикстура снята не с ролика"
            );
        }
    }

    #[test]
    fn every_fixture_says_out_loud_whether_it_was_captured_live() {
        // Разделение «живое / смоделированное» в этом проекте ценнее
        // самого набора, поэтому оно не в комментарии, а в поле, и
        // список смоделированных закреплён здесь: новая модель обязана
        // попасть в этот список руками, а не проехать незамеченной.
        let modelled: Vec<&str> = OUTCOMES
            .iter()
            .map(|(name, _)| *name)
            .filter(|name| !fixtures::outcome(name).capture.is_live())
            .collect();

        assert_eq!(
            modelled,
            vec![
                "connection-lost-mid-download.json",
                "stalled-killed-by-watchdog.json"
            ],
            "смоделированы ровно два исхода — оба про обрыв транспорта, \
             который на настоящем Wi-Fi не воспроизводится без выключения \
             сети у владельца машины"
        );
    }

    #[test]
    fn a_successful_process_is_not_judged_by_its_stderr() {
        // Предупреждения yt-dlp печатает в каждом запуске по ролику
        // YouTube, в том числе в успешном (здесь — про JS-рантайм).
        // Классификация, смотрящая на stderr раньше кода завершения,
        // объявила бы отказом любую удачную загрузку.
        let outcome = fixtures::outcome("success-audio-only.json");
        assert_eq!(outcome.exit_code, Some(0));
        assert!(outcome.stderr.contains("WARNING:"));

        assert_eq!(
            classify_fixture("success-audio-only.json"),
            AttemptVerdict::Completed
        );
    }

    #[test]
    fn a_transport_death_is_a_retry_and_not_a_failure() {
        // Главное отличие от E2: «нет сети» здесь не класс отказа.
        // Задача не падает, а уходит в цикл повторов — падать ей
        // предстоит классом `connectionLost`, и только когда попытки
        // кончатся (С-7).
        let outcome = fixtures::outcome("connection-lost-mid-download.json");
        assert_eq!(outcome.exit_code, Some(1));

        let AttemptVerdict::Interrupted { details } =
            classify_fixture("connection-lost-mid-download.json")
        else {
            panic!("обрыв транспорта обязан быть поводом к повтору, а не отказом");
        };
        assert_eq!(details.exit_code, Some(1));
        assert!(
            details
                .stderr_tail
                .expect("хвост stderr есть")
                .contains("Giving up after 10 retries"),
            "детали обязаны донести до «Подробнее» причину, по которой \
             попытка кончилась"
        );
    }

    #[test]
    fn a_server_answer_wrapped_in_retries_is_not_a_transport_failure() {
        // Менеджер повторов yt-dlp дописывает «Giving up after N retries»
        // к любому повторяемому семейству, в том числе к состоявшемуся
        // ответу сервера. Считать эту обёртку признаком обрыва значило бы
        // вернуть `HTTP Error 4xx` в транспортные маркеры, откуда разбор
        // ссылки убрал их сознательно, — и подарить пользователю пять
        // пауз и «соединение потеряно» на ошибке, к сети отношения не
        // имеющей.
        let stderr = "ERROR: \r[download] Got error: HTTP Error 403: Forbidden. \
                      Giving up after 10 retries";

        assert_eq!(
            verdict(&classify_attempt(&AttemptOutcome {
                exit_code: Some(1),
                stderr,
            })),
            Expected::YtDlpFailure(YtDlpFailureReason::Generic),
            "состоявшийся ответ сервера — не обрыв связи, сколько бы раз \
             yt-dlp его ни повторил"
        );
    }

    #[test]
    fn a_real_transport_death_survives_without_the_retry_wrapper() {
        // Обратная сторона: снятая живьём фикстура обрыва остаётся
        // повтором и без маркера «giving up after» — внутри обёртки стоит
        // настоящий транспортный признак, тот же, что у разбора ссылки.
        let outcome = fixtures::outcome("connection-lost-mid-download.json");
        let text = fatal_text(&outcome.stderr);

        assert!(
            is_transport_failure(&text),
            "исход обязан опознаваться маркерами разбора, а не обёрткой \
             менеджера повторов"
        );
        assert!(
            !contains_any(&text, &DOWNLOAD_TRANSPORT_MARKERS),
            "и даже без собственных маркеров скачивания: если этот тест \
             покраснел, признак переехал, и вывод «обёртка не нужна» надо \
             перепроверить"
        );
    }

    #[test]
    fn the_progress_bar_leaves_a_carriage_return_inside_the_fatal_line() {
        // Форма отказа на пути скачивания, снятая побайтно: полоса
        // прогресса закрывается **возвратом каретки**, и маркер, пробел,
        // `\r` и текст стоят в одной физической строке. Отбор
        // `probe::fatal_text` режет только по `\n`, поэтому такой отказ
        // забирается целиком, без всяких правил про перенос.
        //
        // Тест сторожит две вещи сразу: что фикстура снята без
        // преобразования байт (ровно в этом месте прошлая съёмка через
        // питоновский слой подменила `\r` на `\n` и родила
        // несуществующий дефект) и что отбор текста от возврата каретки
        // не разваливается.
        let outcome = fixtures::outcome("connection-lost-mid-download.json");
        let fatal = outcome
            .stderr
            .lines()
            .find(|line| line.starts_with("ERROR:"))
            .expect("итоговая строка есть");

        assert!(
            fatal.contains('\r'),
            "в фикстуре пропал возврат каретки — значит она снята с \
             преобразованием переводов строк, а не как есть: переснимите \
             перенаправлением потока в файл (см. README)"
        );
        assert!(
            fatal.contains("Giving up after 10 retries"),
            "текст отказа стоит в той же физической строке, что и маркер"
        );
        assert_eq!(
            outcome.stderr.matches('\r').count(),
            1,
            "возврат каретки в снятом выводе ровно один — тот, что \
             закрывает полосу прогресса"
        );

        assert!(fatal_text(&outcome.stderr).contains("giving up after 10 retries"));
    }

    #[test]
    fn a_process_killed_by_the_watchdog_leaves_nothing_to_read() {
        // С-8 в снятом виде: сторож убил замерший процесс. Ни кода
        // завершения, ни строки ERROR — вердикт держится только на
        // отсутствии кода.
        let outcome = fixtures::outcome("stalled-killed-by-watchdog.json");
        assert_eq!(outcome.exit_code, None, "процесс убит сигналом");
        assert!(
            !outcome.stderr.contains("ERROR:"),
            "убитый процесс не успевает ничего сказать"
        );

        assert!(matches!(
            classify_fixture("stalled-killed-by-watchdog.json"),
            AttemptVerdict::Interrupted { .. }
        ));
    }

    #[test]
    fn a_stalled_stream_still_prints_lines_while_no_byte_arrives() {
        // Замер, из-за которого сторож С-8 обязан быть таймером. В stdout
        // убитой попытки после последнего принятого байта стоят строки
        // «Read timed out … Retrying» — вывод есть, загрузки нет, и
        // счётчик строк счёл бы такую попытку живой.
        //
        // Сколько именно таких строк успеет напечататься, зависит от
        // того, на какой секунде замолчал транспорт (внутренние повторы
        // yt-dlp идут раз в `--socket-timeout`), поэтому закреплено «хотя
        // бы одна», а не число: снятые попытки давали и две, и одну.
        let outcome = fixtures::outcome("stalled-killed-by-watchdog.json");
        let after_last_byte: Vec<&str> = outcome
            .stdout
            .lines()
            .skip_while(|line| !line.contains("Read timed out"))
            .collect();

        assert!(
            !after_last_byte.is_empty(),
            "фикстура снята ради строк, напечатанных замершим потоком"
        );
        assert!(
            !after_last_byte
                .iter()
                .any(|line| line.starts_with("@tl-progress")),
            "после последнего принятого байта строк прогресса быть не должно"
        );
    }

    #[test]
    fn the_reused_classes_are_recognised_by_the_markers_of_the_probe() {
        // Смысл переиспользования: те же два ролика, что сняты для E2
        // разбором (`-J`), сняты ещё раз скачиванием и опознаются теми же
        // маркерами. Проверяется не буква, а именно это: класс приходит
        // от общей функции разбора, а не от таблицы этого модуля.
        //
        // Формулировок у класса бывает несколько, и это наблюдение, а не
        // допущение: E2 уже ловил приватный ролик, отвечавший на два
        // запуска подряд разными текстами, а этот удалённый ролик в
        // первой съёмке TL-43 ответил «Video unavailable», а в
        // пересъёмке — «This video is unavailable». Оба текста есть в
        // маркерах разбора, класс от перестановки не меняется — а если
        // апстрим уйдёт из них совсем, покраснеет этот тест.
        for (name, expected) in [
            (
                "video-unavailable.json",
                SharedFailureClass::VideoUnavailable,
            ),
            ("sign-in-required.json", SharedFailureClass::SignInRequired),
        ] {
            let stderr = fixtures::outcome(name).stderr;

            assert_eq!(
                shared_failure_class(&fatal_text(&stderr)),
                Some(expected),
                "{name}: класс перестал опознаваться маркерами probe — \
                 переснимите фикстуру и сверьтесь с probe::classify"
            );
        }
    }

    #[test]
    fn the_typographic_apostrophe_of_youtube_is_a_sign_in_on_this_path_too() {
        // Дефект TL-122 (#129) владелец поймал на разборе ссылки, но
        // маркеры у обоих классификаторов общие, и починка обязана
        // доехать сюда же. Вывод читается из той единственной фикстуры,
        // в которой он снят: второй копии этого текста в проекте нет —
        // копиям было бы нечем помешать разойтись.
        let envelope = fixtures::probe_metadata("outcomes/sign-in-not-a-bot.json");
        let stderr = envelope["stderr"]
            .as_str()
            .expect("в конверте фикстуры есть stderr");

        assert!(
            stderr.contains('\u{2019}') && !stderr.contains('\''),
            "фикстура перестала быть тем случаем, ради которого взята: \
             апостроф в ней обязан быть только типографским"
        );

        assert!(
            matches!(
                classify_attempt(&AttemptOutcome {
                    exit_code: Some(1),
                    stderr,
                }),
                AttemptVerdict::Failed(DownloadFailure::SignInRequired { .. })
            ),
            "класс «требуется вход» опознаётся независимо от формы апострофа"
        );
    }

    #[test]
    fn an_outdated_yt_dlp_does_not_look_like_a_stale_card() {
        // Ловушка, которой на этом пине не оказалось, — и именно поэтому
        // закреплена. yt-dlp, не сумевший разобрать ответ, форматов не
        // находит вовсе; соблазнительно было бы ждать от него «Requested
        // format is not available» (мы ведь просили формат), и тогда
        // пользователь получил бы «обновите карточку» вместо честного
        // «yt-dlp не понимает ответ YouTube».
        let outcome = fixtures::outcome("ytdlp-failure-outdated.json");
        assert!(outcome.stderr.contains("No video formats found!"));
        assert!(
            !outcome
                .stderr
                .to_lowercase()
                .contains("requested format is not available"),
            "апстрим сменил формулировку — порядок проверок 5 и 8 надо \
             пересмотреть осознанно"
        );

        assert_eq!(
            verdict(&classify_fixture("ytdlp-failure-outdated.json")),
            Expected::YtDlpFailure(YtDlpFailureReason::Outdated)
        );
    }

    #[test]
    fn a_full_disk_never_looks_like_a_lost_connection() {
        // Цена ошибки в порядке проверок: пять повторов с растущей
        // паузой закончились бы сообщением «соединение потеряно» —
        // прямой неправдой о причине, после которой пользователь пошёл
        // бы чинить Wi-Fi вместо того, чтобы освободить место.
        let outcome = fixtures::outcome("disk-full.json");
        assert!(outcome
            .stderr
            .contains("[Errno 28] No space left on device"));
        assert!(
            outcome
                .stdout
                .lines()
                .filter(|line| line.starts_with("@tl-progress"))
                .count()
                > 10,
            "фикстура снята ради отказа ПО ХОДУ записи, а не до старта"
        );

        assert_eq!(
            verdict(&classify_fixture("disk-full.json")),
            Expected::Failed(DownloadErrorKind::DiskFull)
        );
    }

    #[test]
    fn an_unwritable_destination_names_the_reason_the_contract_expects() {
        let AttemptVerdict::Failed(DownloadFailure::DestinationUnavailable { reason, .. }) =
            classify_fixture("destination-read-only.json")
        else {
            panic!("каталог без права записи — это недоступная папка назначения");
        };

        assert_eq!(reason, "нет прав на запись");
    }

    #[test]
    fn a_missing_destination_is_told_apart_from_a_forbidden_one() {
        // ЖИВЬЁМ НЕ ПРОВЕРЕНО: удалить папку посреди загрузки на macOS
        // недостаточно — открытый дескриптор переживает удаление
        // каталога. Строка собрана из настоящего обрамления снятой
        // ошибки и второй штатной формулировки ОС.
        let stderr = "ERROR: unable to open for writing: [Errno 2] \
                      No such file or directory: '/Users/u/Downloads/gone/v.mp4.part'";

        let AttemptVerdict::Failed(DownloadFailure::DestinationUnavailable { reason, .. }) =
            classify_attempt(&AttemptOutcome {
                exit_code: Some(1),
                stderr,
            })
        else {
            panic!("исчезнувшая папка — тоже недоступная папка назначения");
        };

        assert_eq!(reason, "папка не существует");
    }

    #[test]
    fn a_missing_file_without_a_write_context_is_not_a_broken_destination() {
        // Половина признака — не признак: «no such file or directory»
        // без контекста записи означает что угодно, и класс «Папка
        // «Загрузки» недоступна» показывался бы наугад.
        let stderr = "ERROR: [youtube] abc: Unable to download webpage: \
                      [Errno 2] No such file or directory: 'cookies.txt'";

        assert_eq!(
            verdict(&classify_attempt(&AttemptOutcome {
                exit_code: Some(1),
                stderr,
            })),
            Expected::YtDlpFailure(YtDlpFailureReason::Generic)
        );
    }

    #[test]
    fn the_order_of_the_groups_decides_when_two_of_them_match_at_once() {
        // Пары, за которыми есть смысл, но которых нет в снятом выводе.
        // Каждая строка собрана из настоящей итоговой ошибки фикстуры и
        // дописанной формулировки второй группы — тот же приём, что в
        // `probe::classify`, и единственный способ закрепить
        // очерёдность, пока yt-dlp не выдал такой текст сам.
        let cases: [(&str, String, Expected); 4] = [
            (
                "нет места важнее обрыва: повторять запись некуда, а «соединение \
                 потеряно» после пяти пауз — прямая неправда о причине",
                format!(
                    "{} Giving up after 10 retries",
                    fatal_line("disk-full.json")
                ),
                Expected::Failed(DownloadErrorKind::DiskFull),
            ),
            (
                "недоступная папка важнее обрыва по той же причине: правами на \
                 запись повторы не заведуют",
                format!(
                    "{} Giving up after 10 retries",
                    fatal_line("destination-read-only.json")
                ),
                Expected::Failed(DownloadErrorKind::DestinationUnavailable),
            ),
            (
                "обрыв важнее содержательного класса: при мёртвом транспорте \
                 ответа от YouTube не было вовсе, и «формат недоступен» сказано \
                 не про этот запуск",
                format!("{} Read timed out", fatal_line("stale-format.json")),
                Expected::Interrupted,
            ),
            (
                "известный класс важнее признака устаревания: под-причина \
                 уточняет только «сбой yt-dlp», а не перебивает диагноз",
                format!(
                    "{}; unable to extract player response",
                    fatal_line("video-unavailable.json")
                ),
                Expected::Failed(DownloadErrorKind::VideoUnavailable),
            ),
        ];

        for (why, stderr, expected) in cases {
            assert_eq!(
                verdict(&classify_attempt(&AttemptOutcome {
                    exit_code: Some(1),
                    stderr: &stderr,
                })),
                expected,
                "{why}"
            );
        }
    }

    #[test]
    fn region_wording_of_youtube_is_recognised_on_the_download_path_too() {
        // Тот же единственный не-снятый класс, что в E2: ролика,
        // заблокированного для страны съёмки, найти не удалось. Строка
        // собрана из формулировки YouTube и обрамления настоящей ошибки
        // скачивания.
        let stderr = "ERROR: [youtube] dQw4w9WgXcQ: Video unavailable. \
                      The uploader has not made this video available in your country";

        assert_eq!(
            verdict(&classify_attempt(&AttemptOutcome {
                exit_code: Some(1),
                stderr,
            })),
            Expected::Failed(DownloadErrorKind::RegionBlocked)
        );
    }

    #[test]
    fn a_silent_process_with_a_code_is_a_yt_dlp_failure_and_not_a_retry() {
        // Граница между шагами 7 и 8: код завершения есть, сказать
        // процессу нечего. Повторять такое вслепую значило бы пять раз
        // ждать паузу ради того же молчания.
        assert_eq!(
            verdict(&classify_attempt(&AttemptOutcome {
                exit_code: Some(1),
                stderr: "   \n  ",
            })),
            Expected::YtDlpFailure(YtDlpFailureReason::Generic)
        );
    }

    #[test]
    fn details_carry_the_whole_stderr_of_a_short_failure() {
        // Н-4 на снятых данных: длиннее предела не оказалась ни одна из
        // девяти фикстур (самая многословная — 774 символа), поэтому
        // живой набор проверяет только вторую половину правила — что до
        // «Подробнее» доезжает весь текст, а не его пересказ. Обрезку
        // проверяет соседний тест на собранном входе.
        let outcome = fixtures::outcome("sign-in-required.json");
        assert!(
            outcome.stderr.trim().chars().count() < STDERR_TAIL_MAX_CHARS,
            "фикстура переросла предел — перенесите проверку обрезки сюда, \
             на живой текст, и уберите собранный вход из соседнего теста"
        );

        let AttemptVerdict::Failed(failure) = classify_fixture("sign-in-required.json") else {
            panic!("это отказ");
        };
        let details = failure
            .to_contract(PartialData::Removed)
            .details
            .expect("детали есть");

        assert_eq!(details.exit_code, Some(1));
        assert_eq!(
            details.stderr_tail.as_deref(),
            Some(outcome.stderr.trim()),
            "короткий stderr доезжает целиком"
        );
    }

    #[test]
    fn a_long_stderr_reaches_the_contract_only_by_its_tail() {
        // Н-4: наружу уходит хвост, а не поток целиком. Вход собран из
        // настоящего stderr, повторённого до превышения предела: снятого
        // вывода такой длины у скачивания не бывает (yt-dlp говорит о
        // причине одной строкой), а правило проверять надо.
        let captured = fixtures::outcome("connection-lost-mid-download.json").stderr;
        let stderr = captured.repeat(3);
        assert!(stderr.trim().chars().count() > STDERR_TAIL_MAX_CHARS);

        let AttemptVerdict::Interrupted { details } = classify_attempt(&AttemptOutcome {
            exit_code: Some(1),
            stderr: &stderr,
        }) else {
            panic!("класс от длины текста не меняется");
        };
        let tail = details.stderr_tail.expect("хвост есть");

        assert_eq!(tail.chars().count(), STDERR_TAIL_MAX_CHARS);
        assert!(
            stderr.trim().ends_with(&tail),
            "хвост обязан быть концом настоящего stderr, а не пересказом"
        );
    }

    #[test]
    fn no_class_leaks_a_rust_identifier_into_the_message() {
        // Сторож контракта на настоящих исходах: `message` виден в
        // «Подробнее» (Н-4).
        for (name, _) in OUTCOMES {
            let AttemptVerdict::Failed(failure) = classify_fixture(name) else {
                continue;
            };

            let message = failure.to_contract(PartialData::Kept).message;
            for identifier in [
                "DownloadFailure",
                "AttemptVerdict",
                "Generic",
                "Outdated",
                "YtDlpFailureReason",
            ] {
                assert!(
                    !message.contains(identifier),
                    "{name}: «{message}» содержит Rust-идентификатор {identifier}"
                );
            }
            assert!(
                !message.contains("ERROR:"),
                "{name}: в «{message}» протёк stderr — он живёт в деталях (Н-4)"
            );
        }
    }

    /// Итоговая строка отказа снятой фикстуры — как есть.
    fn fatal_line(name: &str) -> String {
        fixtures::outcome(name)
            .stderr
            .lines()
            .find(|line| line.starts_with("ERROR:"))
            .expect("в фикстуре отказа есть итоговая ошибка")
            .to_string()
    }
}
