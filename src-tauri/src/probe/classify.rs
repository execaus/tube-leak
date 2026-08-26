//! Классификация исхода запуска yt-dlp (Ф-6, Ф-7) — TL-31.
//!
//! Функция [`classify`] чистая: на входе — то, что осталось от одного
//! запуска процесса (код завершения, stdout, stderr), на выходе — либо
//! метаданные ролика, либо один из семи классов Ф-6, различимых по выводу.
//! Процессов здесь не запускается и таймаутов не отсчитывается — это TL-32;
//! два оставшихся класса Ф-6 ([`ProbeFailure::NotAUrl`] и
//! [`ProbeFailure::Timeout`]) порождает она же.
//!
//! # Почему код завершения 0 ещё не успех
//!
//! Проверено живьём на пине 2026.08.19 (см. фикстуры и README рядом с
//! ними): у идущего эфира yt-dlp завершается **кодом 0** и печатает
//! валидный JSON с `is_live: true` — «ошибкой процесса» это не является
//! вовсе. Плейлист ведёт себя так же: код 0 и JSON с `_type: "playlist"`.
//! Поэтому успешный по коду выхода запуск сначала проверяется по
//! метаданным и только потом становится успехом карточки.
//!
//! Обратное тоже верно и тоже проверено: запланированная трансляция
//! (премьера) приходит **кодом 1 и текстом в stderr** («This live event
//! will begin in 4 days.»), то есть класс «прямая трансляция» живёт на
//! обеих ветках сразу, а не на одной из них.
//!
//! # Классификация идёт по строкам `ERROR:`, а не по всему stderr
//!
//! yt-dlp пишет в stderr три разных вещи: предупреждения окружения
//! (в нашем случае — «No supported JavaScript runtime could be found»,
//! оно есть в **каждом** запуске по ролику YouTube), предупреждения о
//! повторах (`Retrying (1/3)…`) и одну итоговую строку `ERROR:`.
//! Классификация по всему потоку означала бы, что сетевая рябь, после
//! которой запрос всё-таки прошёл, перебивает настоящую причину отказа.
//! Поэтому решение принимается по строкам `ERROR:`; весь stderr
//! используется только если ни одной такой строки нет.
//!
//! # Что классификация предполагает об аргументах запуска (для TL-32)
//!
//! - **`--no-playlist` обязателен.** С ним `watch?v=…&list=…` разбирается
//!   как одиночный ролик (extractor `youtube`, код 0, `_type: "video"`) —
//!   проверено живьём, это же зафиксировано анализом эпика. Без него тот
//!   же адрес уехал бы в extractor `youtube:tab` и стал бы классом
//!   «плейлист», прямо вопреки решению анализа.
//! - **Ссылку на плейлист без `--flat-playlist` классифицировать не
//!   получится.** Замерено: `-J --no-playlist` на плейлисте из 19 роликов
//!   не завершился за 60 с — yt-dlp обходит каждый ролик целиком, и
//!   таймаут разбора (30 с по дизайну) сработает раньше, чем появится
//!   `_type: "playlist"`. Класс «плейлист» останется недостижимым, а
//!   пользователь получит «таймаут» вместо «плейлисты не поддерживаются».
//!   Это ограничение оркестрации, а не классификации, и снимается выбором
//!   аргументов в TL-32.

// Вызывающего у классификации пока нет: её зовёт оркестрация разбора
// (TL-32). Тот же приём и по той же причине, что у лестницы качеств в
// `crate::probe::quality` и у типов контракта в `crate::types`, —
// снимается задачей, которая начнёт классификацию использовать.
#![allow(dead_code)]

use serde_json::Value;

use crate::probe::error::ProbeFailure;
use crate::types::{ProbeErrorDetails, YtDlpFailureReason};

/// Сколько символов stderr уходит в «Подробнее» (Н-4).
///
/// Значение и способ обрезки — те же, что у служебного экрана E1
/// (`crate::commands::sidecar`): пользователю нужен хвост, которого хватает
/// на опознание причины, а не поток целиком. Полный stderr пишет в лог
/// вызывающий.
const STDERR_TAIL_MAX_CHARS: usize = 1000;

/// Всё, что осталось от одного запуска yt-dlp.
///
/// Структура сырая намеренно: разбор JSON — часть классификации (успех
/// определяется в том числе по метаданным), поэтому stdout приходит сюда
/// текстом, а не разобранным значением, и разбирается ровно один раз.
#[derive(Debug, Clone, Copy)]
pub struct YtDlpOutcome<'a> {
    /// Код завершения процесса; `None` — процесс убит сигналом и своего
    /// кода не оставил.
    pub exit_code: Option<i32>,
    /// stdout процесса целиком: при `-J` это одна строка JSON (или `null`,
    /// когда извлечь ничего не удалось).
    pub stdout: &'a str,
    /// stderr процесса целиком. Наружу уходит только хвост (Н-4).
    pub stderr: &'a str,
}

/// Разобрать исход запуска: метаданные ролика либо класс отказа.
///
/// Успехом считается только код завершения 0 **вместе** с JSON-объектом
/// метаданных, который не оказался ни плейлистом, ни идущим/запланированным
/// эфиром. Всё остальное — отказ; какой именно, решает stderr.
pub fn classify(outcome: &YtDlpOutcome<'_>) -> Result<Value, ProbeFailure> {
    let details = details(outcome);

    if outcome.exit_code == Some(0) {
        if let Some(metadata) = parse_metadata(outcome.stdout) {
            if is_playlist(&metadata) {
                return Err(ProbeFailure::PlaylistUnsupported { details });
            }
            if is_live(&metadata) {
                return Err(ProbeFailure::LiveUnsupported { details });
            }

            return Ok(metadata);
        }
    }

    Err(failure_from_stderr(outcome.stderr, details))
}

/// Метаданные ролика из stdout, если они там есть.
///
/// `-J` на неудачном извлечении печатает в stdout `null`, а не пустоту, —
/// это валидный JSON, но не объект метаданных, поэтому пригодным считается
/// только объект.
fn parse_metadata(stdout: &str) -> Option<Value> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return None;
    }

    let value: Value = serde_json::from_str(trimmed).ok()?;
    value.is_object().then_some(value)
}

/// Ссылка привела к плейлисту или каналу, а не к ролику.
///
/// `_type` у одиночного ролика — `"video"`, у плейлиста и у вкладки
/// канала — `"playlist"` (проверено живьём на обоих). `multi_video` —
/// та же ветка по смыслу (несколько роликов под одним адресом).
fn is_playlist(metadata: &Value) -> bool {
    matches!(
        metadata.get("_type").and_then(Value::as_str),
        Some("playlist" | "multi_video")
    )
}

/// Ролик — идущий эфир или запланированная трансляция/премьера (С-10).
///
/// Смотрим два поля, а не одно: `live_status` точнее (различает
/// `is_live`, `is_upcoming`, `was_live`, `post_live`, `not_live`), а
/// `is_live` — исторически более старое поле и страховка на случай, если
/// апстрим переименует первое.
///
/// Завершённая трансляция (`was_live`) — обычный успех: у неё есть
/// готовая запись, длительность и полный список форматов (проверено
/// живьём). `post_live` (эфир только что кончился, запись ещё
/// обрабатывается) сознательно оставлен на ветке успеха: дизайн относит к
/// классу «трансляция» ровно две ситуации — идущий эфир и запланированную
/// премьеру, — и расширять класс без живых данных мы не стали.
fn is_live(metadata: &Value) -> bool {
    if metadata.get("is_live").and_then(Value::as_bool) == Some(true) {
        return true;
    }

    matches!(
        metadata.get("live_status").and_then(Value::as_str),
        Some("is_live" | "is_upcoming")
    )
}

/// Класс отказа по выводу процесса.
///
/// Порядок проверок нагружен: сообщения yt-dlp пересекаются словами, и
/// первая совпавшая группа решает исход. Обоснование каждого шага — в
/// комментариях внутри; каждую развилку держит отдельный тест на снятом
/// выводе — `a_gone_recording_of_a_broadcast_is_unavailable_and_not_live`,
/// `a_failed_playlist_is_a_playlist_and_not_a_yt_dlp_failure`,
/// `an_http_error_from_the_site_is_not_a_dead_network`,
/// `a_dead_network_is_not_mistaken_for_an_outdated_yt_dlp`,
/// `a_private_video_lands_in_one_class_on_both_of_its_wordings`. Порядок
/// «регион раньше недоступности» — единственный, под который живого
/// вывода не нашлось (см. README фикстур).
fn failure_from_stderr(stderr: &str, details: ProbeErrorDetails) -> ProbeFailure {
    let text = fatal_text(stderr);

    // 1. Плейлист/канал — по метке экстрактора, а не по тексту ошибки.
    //    Метку `[youtube:tab]` yt-dlp ставит по совпадению адреса,
    //    ещё до всякого обращения к сети, поэтому она не может оказаться
    //    ложной из-за сетевого сбоя, — а вот наоборот бывает: у плейлиста,
    //    которого нет, ошибка выглядит как «Unable to download API page:
    //    HTTP Error 400». По классу это всё равно «вставьте ссылку на
    //    отдельный ролик», и других веток у пользователя нет.
    if text.contains("[youtube:tab]") {
        return ProbeFailure::PlaylistUnsupported { details };
    }

    // 2. Сеть. Проверяется до всего остального содержательного: при
    //    транспортном сбое ответа от YouTube не было вовсе, и любые
    //    выводы о самом ролике сделаны не будут.
    if contains_any(&text, &NETWORK_MARKERS) || contains_any(&text, &NETWORK_MARKERS_OTHER_OS) {
        return ProbeFailure::NetworkUnavailable { details };
    }

    // 3. Трансляция, о которой yt-dlp сообщил ошибкой (запланированная).
    if contains_any(&text, &LIVE_MARKERS) {
        return ProbeFailure::LiveUnsupported { details };
    }

    // 4. Регион — строго перед «ролик недоступен»: YouTube формулирует
    //    региональную блокировку как частный случай недоступности
    //    («Video unavailable. This video is not available in your
    //    country»), и общий маркер съел бы частный.
    if contains_any(&text, &REGION_MARKERS) {
        return ProbeFailure::RegionBlocked { details };
    }

    // 5. Ролик недоступен: удалён, снят, не существует.
    if contains_any(&text, &UNAVAILABLE_MARKERS) {
        return ProbeFailure::VideoUnavailable { details };
    }

    // 6. Нужен вход: возрастное ограничение, приватный ролик, ролик для
    //    подписчиков канала.
    if contains_any(&text, &SIGN_IN_MARKERS) {
        return ProbeFailure::SignInRequired { details };
    }

    // 7. Всё остальное — сбой yt-dlp; под-причина меняет только текст
    //    пояснения на экране, но не класс.
    ProbeFailure::YtDlpFailure {
        reason: if contains_any(&text, &OUTDATED_MARKERS) {
            YtDlpFailureReason::Outdated
        } else {
            YtDlpFailureReason::Generic
        },
        details,
    }
}

/// Текст, по которому принимается решение: строки `ERROR:` в нижнем
/// регистре.
///
/// Если ни одной строки `ERROR:` нет (процесс убит, упал до вывода,
/// сказал всё предупреждениями) — берётся весь stderr: лучше решать по
/// шумному тексту, чем не решать вовсе.
fn fatal_text(stderr: &str) -> String {
    let errors: Vec<&str> = stderr
        .lines()
        .filter(|line| line.trim_start().starts_with("ERROR:"))
        .collect();

    if errors.is_empty() {
        stderr.to_lowercase()
    } else {
        errors.join("\n").to_lowercase()
    }
}

fn contains_any(text: &str, markers: &[&str]) -> bool {
    markers.iter().any(|marker| text.contains(marker))
}

/// Технические детали отказа для «Подробнее» (Н-4).
fn details(outcome: &YtDlpOutcome<'_>) -> ProbeErrorDetails {
    ProbeErrorDetails {
        stderr_tail: stderr_tail(outcome.stderr),
        exit_code: outcome.exit_code,
    }
}

/// Последние [`STDERR_TAIL_MAX_CHARS`] символов stderr; `None`, если после
/// `trim()` не осталось ничего.
fn stderr_tail(stderr: &str) -> Option<String> {
    let trimmed = stderr.trim();
    if trimmed.is_empty() {
        return None;
    }

    let char_count = trimmed.chars().count();
    if char_count <= STDERR_TAIL_MAX_CHARS {
        Some(trimmed.to_string())
    } else {
        Some(
            trimmed
                .chars()
                .skip(char_count - STDERR_TAIL_MAX_CHARS)
                .collect(),
        )
    }
}

/// Транспортные сбои, снятые живьём на macOS с пином 2026.08.19.
///
/// Все маркеры — про то, что до YouTube не доехал запрос, а не про то,
/// что YouTube ответил отказом: `HTTP Error 4xx` сюда сознательно не
/// входит, это как раз состоявшийся ответ (у несуществующего плейлиста он
/// и приходит).
const NETWORK_MARKERS: [&str; 4] = [
    // «Failed to resolve 'www.youtube.com' ([Errno 8] nodename nor
    // servname provided, or not known)» — DNS не отвечает; ровно это даёт
    // выключенная сеть на macOS.
    "failed to resolve",
    "nodename nor servname",
    // «Failed to establish a new connection: [Errno 61] Connection
    // refused».
    "connection refused",
    // Системный прокси настроен, но недоступен: «('Unable to connect to
    // proxy', …)». Инвариант проекта — уважать системный прокси, значит
    // его недоступность пользователь увидит как «нет сети».
    "unable to connect to proxy",
];

/// Те же транспортные сбои на других платформах.
///
/// **ЖИВЬЁМ НЕ ПРОВЕРЕНО** (Р-6 E1 — машин нет): формулировки задаёт не
/// yt-dlp, а системный резолвер, и на Linux/Windows он отвечает своими
/// текстами. Маркеры взяты из этих штатных текстов, а не из наблюдения;
/// на macOS ни один из них не встречается — это и проверяет тест
/// `other_os_network_markers_do_not_fire_on_the_macos_fixtures`.
const NETWORK_MARKERS_OTHER_OS: [&str; 4] = [
    // glibc: «[Errno -3] Temporary failure in name resolution».
    "temporary failure in name resolution",
    // Windows: «[Errno 11001] getaddrinfo failed».
    "getaddrinfo failed",
    // «[Errno 101] Network is unreachable», «[Errno 51] Network is down».
    "network is unreachable",
    "network is down",
];

/// Идущий эфир и запланированная трансляция/премьера, сообщённые ошибкой.
const LIVE_MARKERS: [&str; 3] = [
    // Снято живьём: «This live event will begin in 4 days.»
    "live event will begin",
    // ЖИВЬЁМ НЕ ПРОВЕРЕНО: премьера, до которой остались минуты, и эфир,
    // который вот-вот начнётся, — те же ситуации другими словами
    // yt-dlp. Подходящего ролика на момент съёмки фикстур не нашлось.
    "premieres in",
    "live event will begin in a few moments",
];

/// Региональная блокировка.
///
/// **ЖИВЬЁМ НЕ ПРОВЕРЕНО**: ролика, заблокированного для страны съёмки,
/// найти не удалось — поиск YouTube сам не показывает то, что в регионе
/// недоступно. Маркеры — формулировки YouTube, которые yt-dlp передаёт
/// дословно; эпик заранее допускает такую проверку только фикстурой
/// (К-7), но фикстуры без ролика не бывает, поэтому здесь честно
/// «не проверено».
const REGION_MARKERS: [&str; 3] = [
    "in your country",
    "not available from your location",
    "not available in your location",
];

/// Ролик недоступен: удалён, снят с публикации, не существует.
const UNAVAILABLE_MARKERS: [&str; 5] = [
    // Снято живьём: «Video unavailable» (несуществующий id) и «Video
    // unavailable. This video is not available».
    "video unavailable",
    "this video is not available",
    // Снято живьём на удалённом ролике: «This video is unavailable».
    "this video is unavailable",
    // Снято живьём: «This live stream recording is not available.» —
    // эфир кончился, записи не осталось. Не класс «трансляция»: ждать
    // нечего, ролика уже нет.
    "live stream recording is not available",
    // ЖИВЬЁМ НЕ ПРОВЕРЕНО: удаление по жалобе и блокировка канала —
    // штатные формулировки YouTube, подходящего ролика не нашлось.
    "has been terminated",
];

/// Нужен вход в аккаунт (С-6).
///
/// Сюда же попадает приватный ролик, хотя контракт числит «скрыт
/// (private)» за классом «ролик недоступен». Причина — живые данные, а не
/// удобство: пин 2026.08.19 отвечает на **один и тот же** приватный ролик
/// то «Private video. Sign in if you've been granted access to this
/// video.», то обезличенным «Please sign in.» (обе фикстуры сняты подряд с
/// одного адреса). Развести эти два текста по разным классам значило бы
/// показывать на повторную попытку другой экран. К-3(в) эпика допускает
/// для приватного ролика оба исхода, и из двух допустимых выбран тот, что
/// не зависит от того, какой клиент YouTube ответил первым.
const SIGN_IN_MARKERS: [&str; 5] = [
    // Снято живьём на ролике с возрастным ограничением.
    "sign in to confirm your age",
    // Снято живьём на приватном ролике — обе формулировки.
    "sign in if you've been granted access",
    "please sign in",
    // ЖИВЬЁМ НЕ ПРОВЕРЕНО: ролик для спонсоров канала и антибот-проверка
    // YouTube. Обе просят ровно того же — войти в аккаунт.
    "members-only content",
    "confirm you're not a bot",
];

/// Признак того, что yt-dlp не понимает текущий ответ YouTube.
///
/// Это **под-причина** класса «сбой yt-dlp», а не отдельный класс
/// (С-12): на экране меняется только пояснение.
///
/// Чего здесь сознательно нет: строки «Confirm you are on the latest
/// version using yt-dlp -U». Она выглядит идеальным маркером устаревшей
/// версии, но yt-dlp приписывает её к **любой** неожиданной ошибке —
/// снято живьём, она есть и в сетевом отказе тоже. По ней «нет сети»
/// превращалось бы в «yt-dlp устарел».
const OUTDATED_MARKERS: [&str; 5] = [
    // Снято живьём (запуск с принудительным клиентом `web`): «No video
    // formats found!» — YouTube ответил, а разобрать ответ нечем.
    "no video formats found",
    // ЖИВЬЁМ НЕ ПРОВЕРЕНО: семейство ошибок извлечения. Их появление и
    // означает «формат ответа поменялся»; воспроизвести их на рабочем
    // пине по живому ролику, естественно, не вышло — если бы вышло, пин
    // надо было бы менять, а не тестировать.
    "unable to extract",
    "failed to extract",
    "signature extraction failed",
    "nsig extraction failed",
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};

    use crate::types::ProbeErrorKind;

    /// Что классификация обязана сказать про снятый исход.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Expected {
        /// Метаданные ролика — карточка строится.
        Success,
        /// Класс отказа Ф-6.
        Failure(ProbeErrorKind),
        /// Класс «сбой yt-dlp» с конкретной под-причиной.
        YtDlpFailure(YtDlpFailureReason),
    }

    /// Снятые исходы и класс каждого.
    ///
    /// Таблица — источник истины теста, а сами фикстуры хранят только
    /// факты («что напечатал процесс»), не ответ. Разбор, чем какой ролик
    /// интересен и как он найден, — в README рядом с фикстурами.
    const OUTCOMES: [(&str, Expected); 18] = [
        // Успех — контрольные случаи.
        ("success-watch-with-list", Expected::Success),
        ("success-finished-live", Expected::Success),
        // Ролик недоступен.
        (
            "video-unavailable-missing-id",
            Expected::Failure(ProbeErrorKind::VideoUnavailable),
        ),
        (
            "video-unavailable-deleted",
            Expected::Failure(ProbeErrorKind::VideoUnavailable),
        ),
        (
            "video-unavailable-live-recording-gone",
            Expected::Failure(ProbeErrorKind::VideoUnavailable),
        ),
        // Требуется вход.
        (
            "sign-in-age-restricted",
            Expected::Failure(ProbeErrorKind::SignInRequired),
        ),
        (
            "sign-in-private",
            Expected::Failure(ProbeErrorKind::SignInRequired),
        ),
        (
            "sign-in-private-generic",
            Expected::Failure(ProbeErrorKind::SignInRequired),
        ),
        // Нет сети.
        (
            "network-dns-failure",
            Expected::Failure(ProbeErrorKind::NetworkUnavailable),
        ),
        (
            "network-proxy-refused",
            Expected::Failure(ProbeErrorKind::NetworkUnavailable),
        ),
        // Плейлист и канал.
        (
            "playlist-url",
            Expected::Failure(ProbeErrorKind::PlaylistUnsupported),
        ),
        (
            "playlist-missing",
            Expected::Failure(ProbeErrorKind::PlaylistUnsupported),
        ),
        (
            "channel-url",
            Expected::Failure(ProbeErrorKind::PlaylistUnsupported),
        ),
        (
            "channel-missing",
            Expected::Failure(ProbeErrorKind::PlaylistUnsupported),
        ),
        // Прямая трансляция — обе ветки, с кодом 0 и с кодом 1.
        (
            "live-ongoing",
            Expected::Failure(ProbeErrorKind::LiveUnsupported),
        ),
        (
            "live-upcoming",
            Expected::Failure(ProbeErrorKind::LiveUnsupported),
        ),
        // Сбой yt-dlp — обе под-причины.
        (
            "ytdlp-failure-generic",
            Expected::YtDlpFailure(YtDlpFailureReason::Generic),
        ),
        (
            "ytdlp-failure-outdated",
            Expected::YtDlpFailure(YtDlpFailureReason::Outdated),
        ),
    ];

    /// Метаданные обычных роликов, снятые для лестницы качеств (TL-30).
    ///
    /// Переиспользуются как есть: семь настоящих выводов `yt-dlp -J` по
    /// живым роликам — бесплатная проверка того, что ветка успеха не
    /// принимает обычный ролик за эфир или плейлист.
    const LADDER_FIXTURES: [&str; 7] = [
        "4k-full-ladder.json",
        "max-1080p.json",
        "max-240p.json",
        "multi-language-audio.json",
        "sizes-unknown.json",
        "vertical-video.json",
        "label-differs-from-frame.json",
    ];

    fn fixtures_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ytdlp-probe")
    }

    fn read_json(path: &Path) -> Value {
        let raw = fs::read_to_string(path)
            .unwrap_or_else(|err| panic!("фикстура {} не читается: {err}", path.display()));
        serde_json::from_str(&raw)
            .unwrap_or_else(|err| panic!("фикстура {} — не JSON: {err}", path.display()))
    }

    /// Снятый исход запуска: конверт `outcomes/<name>.json`.
    struct Capture {
        exit_code: Option<i32>,
        stdout: String,
        stderr: String,
        envelope: Value,
    }

    impl Capture {
        fn outcome(&self) -> YtDlpOutcome<'_> {
            YtDlpOutcome {
                exit_code: self.exit_code,
                stdout: &self.stdout,
                stderr: &self.stderr,
            }
        }
    }

    fn capture(name: &str) -> Capture {
        let envelope = read_json(&fixtures_dir().join("outcomes").join(format!("{name}.json")));

        let exit_code = envelope
            .get("exitCode")
            .and_then(Value::as_i64)
            .and_then(|code| i32::try_from(code).ok());
        let stdout = match envelope.get("stdout") {
            Some(value) if value.is_object() => value.to_string(),
            // yt-dlp печатает в stdout `null`, когда извлечь нечего;
            // конверт хранит это как отсутствие метаданных.
            _ => String::new(),
        };
        let stderr = envelope
            .get("stderr")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        Capture {
            exit_code,
            stdout,
            stderr,
            envelope,
        }
    }

    fn verdict(result: &Result<Value, ProbeFailure>) -> Expected {
        match result {
            Ok(_) => Expected::Success,
            Err(ProbeFailure::YtDlpFailure { reason, .. }) => Expected::YtDlpFailure(*reason),
            Err(failure) => Expected::Failure(failure.kind()),
        }
    }

    #[test]
    fn classifies_every_captured_outcome_into_its_declared_class() {
        for (name, expected) in OUTCOMES {
            let capture = capture(name);
            let result = classify(&capture.outcome());

            assert_eq!(
                verdict(&result),
                expected,
                "{name}: классификация разошлась с таблицей (см. README фикстур)"
            );
        }
    }

    #[test]
    fn every_class_recognisable_by_output_has_a_live_fixture() {
        // Семь классов Ф-6, которые распознаются по выводу процесса.
        // Оставшиеся два (`notAUrl`, `timeout`) порождает оркестрация
        // TL-32 — фикстуры вывода у них не бывает по построению.
        let expected_classes = [
            ProbeErrorKind::VideoUnavailable,
            ProbeErrorKind::SignInRequired,
            ProbeErrorKind::NetworkUnavailable,
            ProbeErrorKind::PlaylistUnsupported,
            ProbeErrorKind::LiveUnsupported,
            ProbeErrorKind::YtDlpFailure,
        ];

        for class in expected_classes {
            assert!(
                OUTCOMES.iter().any(|(name, _)| {
                    matches!(
                        classify(&capture(name).outcome()),
                        Err(ref failure) if failure.kind() == class
                    )
                }),
                "класс {class:?} не покрыт ни одной фикстурой (Ф-7)"
            );
        }

        // `regionBlocked` в этом списке нет намеренно: ролика,
        // заблокированного для страны съёмки, найти не удалось (поиск
        // YouTube сам не показывает недоступное в регионе). Класс покрыт
        // тестом `region_wording_of_youtube_is_recognised` на собранной
        // строке, и это единственный такой случай в наборе — см. README.
        assert!(
            !OUTCOMES.iter().any(|(name, _)| matches!(
                classify(&capture(name).outcome()),
                Err(ProbeFailure::RegionBlocked { .. })
            )),
            "появилась живая фикстура региональной блокировки — внесите её \
             в набор и уберите оговорку из README и из этого теста"
        );
    }

    #[test]
    fn fixtures_are_real_output_of_the_pinned_yt_dlp() {
        // Та же связь, что у фикстур лестницы (TL-30): фикстуры
        // заморожены, а yt-dlp — нет. Формулировки ошибок задаёт апстрим,
        // и смена пина обязана громко ломать этот тест, а не тихо
        // оставлять набор зелёным.
        let pinned = pinned_yt_dlp_version();

        for (name, _) in OUTCOMES {
            let capture = capture(name);
            let envelope = &capture.envelope;

            assert_eq!(
                envelope
                    .get("_capture")
                    .and_then(|meta| meta.get("ytDlpVersion"))
                    .and_then(Value::as_str),
                Some(pinned.as_str()),
                "{name}: исход снят не тем yt-dlp, который вложен в приложение \
                 ({pinned} по binaries.lock.json). Пин сменили — переснимите \
                 фикстуры по README, а не правьте эту строку"
            );

            // Там, где yt-dlp что-то напечатал в stdout, он сам сообщает
            // свою версию — сверяем и её, чтобы конверт не мог разойтись
            // с содержимым.
            if let Some(stdout) = envelope.get("stdout").filter(|value| value.is_object()) {
                assert_eq!(
                    stdout
                        .get("_version")
                        .and_then(|version| version.get("version"))
                        .and_then(Value::as_str),
                    Some(pinned.as_str()),
                    "{name}: версия внутри вывода не совпадает с объявленной в конверте"
                );
            }

            assert!(
                !capture.stderr.is_empty() || capture.exit_code == Some(0),
                "{name}: неуспешный запуск обязан был что-то сказать в stderr"
            );
        }
    }

    #[test]
    fn a_live_stream_is_recognised_even_when_the_process_succeeded() {
        // Главный вопрос, который декомпозиция оставила исполнителю:
        // приходит ли признак эфира ошибкой процесса или метаданными.
        // Живые данные: идущий эфир — код завершения 0 и валидный JSON.
        let capture = capture("live-ongoing");
        assert_eq!(
            capture.exit_code,
            Some(0),
            "фикстура перестала быть тем случаем, ради которого снята"
        );

        let metadata: Value = serde_json::from_str(&capture.stdout).expect("в фикстуре есть JSON");
        assert_eq!(metadata.get("is_live"), Some(&Value::Bool(true)));
        assert_eq!(
            metadata.get("live_status").and_then(Value::as_str),
            Some("is_live")
        );

        assert!(matches!(
            classify(&capture.outcome()),
            Err(ProbeFailure::LiveUnsupported { .. })
        ));
    }

    #[test]
    fn a_scheduled_broadcast_arrives_as_a_process_error_instead() {
        // Вторая ветка того же класса: запланированная трансляция
        // завершает процесс кодом 1 и говорит текстом.
        let capture = capture("live-upcoming");
        assert_eq!(capture.exit_code, Some(1));
        assert!(capture.stdout.is_empty(), "метаданных у неё не бывает");

        assert!(matches!(
            classify(&capture.outcome()),
            Err(ProbeFailure::LiveUnsupported { .. })
        ));
    }

    #[test]
    fn a_playlist_is_recognised_even_when_the_process_succeeded() {
        for name in ["playlist-url", "channel-url"] {
            let capture = capture(name);
            assert_eq!(capture.exit_code, Some(0), "{name}");

            let metadata: Value = serde_json::from_str(&capture.stdout).expect("JSON");
            assert_eq!(
                metadata.get("_type").and_then(Value::as_str),
                Some("playlist"),
                "{name}: и плейлист, и канал приходят одним и тем же `_type`"
            );

            assert!(
                matches!(
                    classify(&capture.outcome()),
                    Err(ProbeFailure::PlaylistUnsupported { .. })
                ),
                "{name}"
            );
        }
    }

    #[test]
    fn watch_with_a_list_parameter_stays_an_ordinary_video() {
        // Зафиксировано анализом эпика: пользователь копирует ссылку из
        // открытого плейлиста не задумываясь, и это ролик, а не плейлист.
        let capture = capture("success-watch-with-list");
        let metadata = classify(&capture.outcome()).expect("это успех, а не отказ");

        assert_eq!(metadata.get("_type").and_then(Value::as_str), Some("video"));
        assert_eq!(
            metadata.get("id").and_then(Value::as_str),
            Some("jNQXAC9IVRw"),
            "разобран должен быть ролик из v=, а не первый ролик плейлиста"
        );
    }

    #[test]
    fn a_finished_broadcast_is_an_ordinary_success() {
        let metadata = classify(&capture("success-finished-live").outcome())
            .expect("у завершённого эфира есть готовая запись");

        assert_eq!(
            metadata.get("live_status").and_then(Value::as_str),
            Some("was_live")
        );
        assert!(
            metadata.get("duration").and_then(Value::as_u64).is_some(),
            "длительность у записи есть — карточка выразима (обязанность TL-32)"
        );
    }

    #[test]
    fn ordinary_video_metadata_is_never_mistaken_for_a_live_or_a_playlist() {
        // Семь живых выводов, снятых для лестницы качеств (TL-30).
        for name in LADDER_FIXTURES {
            let stdout = fs::read_to_string(fixtures_dir().join(name))
                .unwrap_or_else(|err| panic!("{name}: {err}"));

            let result = classify(&YtDlpOutcome {
                exit_code: Some(0),
                stdout: &stdout,
                // yt-dlp предупреждает про JS-рантайм в каждом запуске по
                // ролику YouTube; на ветке успеха stderr не смотрят вовсе,
                // и этот тест — заодно проверка, что это так.
                stderr: "WARNING: [youtube] No supported JavaScript runtime could be found.",
            });

            assert!(result.is_ok(), "{name}: обычный ролик должен быть успехом");
        }
    }

    #[test]
    fn a_dead_network_is_not_mistaken_for_an_outdated_yt_dlp() {
        // Ловушка, ради которой сигнатура устаревания сужена: yt-dlp
        // приписывает «Confirm you are on the latest version using yt-dlp
        // -U» к любой неожиданной ошибке, в том числе к сетевой. Маркер
        // «по этой строке» превращал бы «нет сети» в «yt-dlp устарел».
        let trapped = capture("network-proxy-refused");
        assert!(
            trapped
                .stderr
                .to_lowercase()
                .contains("confirm you are on the latest version"),
            "фикстура перестала содержать ловушку — проверьте, что апстрим не \
             убрал приписку, и обновите этот тест осознанно"
        );

        for name in ["network-dns-failure", "network-proxy-refused"] {
            assert!(
                matches!(
                    classify(&capture(name).outcome()),
                    Err(ProbeFailure::NetworkUnavailable { .. })
                ),
                "{name}"
            );
        }
    }

    #[test]
    fn a_gone_recording_of_a_broadcast_is_unavailable_and_not_live() {
        // «This live stream recording is not available.» — про эфир, но
        // класс не «трансляция»: ждать нечего, записи уже нет, и совет
        // «дождитесь окончания эфира» был бы враньём.
        let capture = capture("video-unavailable-live-recording-gone");
        assert!(
            capture.stderr.to_lowercase().contains("live stream"),
            "фикстура перестала быть неоднозначной — проверьте текст"
        );

        assert!(matches!(
            classify(&capture.outcome()),
            Err(ProbeFailure::VideoUnavailable { .. })
        ));
    }

    #[test]
    fn a_private_video_lands_in_one_class_on_both_of_its_wordings() {
        // Один и тот же приватный ролик, два запуска подряд, два разных
        // текста. Если бы они попадали в разные классы, повторная попытка
        // меняла бы экран без всякой причины.
        let with_hint = capture("sign-in-private");
        let without_hint = capture("sign-in-private-generic");

        assert!(
            with_hint
                .stderr
                .to_lowercase()
                .contains("sign in if you've been granted access")
                && without_hint
                    .stderr
                    .to_lowercase()
                    .contains("please sign in"),
            "фикстуры перестали быть двумя разными формулировками одного случая"
        );

        assert_eq!(
            verdict(&classify(&with_hint.outcome())),
            verdict(&classify(&without_hint.outcome()))
        );
    }

    #[test]
    fn a_failed_playlist_is_a_playlist_and_not_a_yt_dlp_failure() {
        // У несуществующего плейлиста в тексте нет ни слова про плейлист:
        // «Unable to download API page: HTTP Error 400». Класс держится
        // на метке экстрактора, а она проставлена по адресу.
        let capture = capture("playlist-missing");
        assert!(capture.stderr.contains("[youtube:tab]"));

        assert!(matches!(
            classify(&capture.outcome()),
            Err(ProbeFailure::PlaylistUnsupported { .. })
        ));
    }

    #[test]
    fn an_http_error_from_the_site_is_not_a_dead_network() {
        // «HTTP Error 404» — состоявшийся ответ, а не обрыв связи:
        // сеть работает, просто отвечать нечем.
        let capture = capture("ytdlp-failure-generic");
        assert!(capture.stderr.contains("HTTP Error 404"));

        assert_eq!(
            verdict(&classify(&capture.outcome())),
            Expected::YtDlpFailure(YtDlpFailureReason::Generic)
        );
    }

    #[test]
    fn retry_warnings_do_not_outvote_the_final_error() {
        // Сеть рябила, повторы прошли, а ролика всё равно нет. Сшито из
        // двух настоящих фикстур: предупреждения о повторах взяты из
        // сетевой, итоговая ошибка — из фикстуры недоступного ролика.
        let flaky = capture("network-proxy-refused");
        let unavailable = capture("video-unavailable-missing-id");

        let warnings: Vec<&str> = flaky
            .stderr
            .lines()
            .filter(|line| line.starts_with("WARNING:"))
            .collect();
        assert!(
            !warnings.is_empty(),
            "в сетевой фикстуре были предупреждения"
        );

        let stderr = format!("{}\n{}", warnings.join("\n"), unavailable.stderr);
        let result = classify(&YtDlpOutcome {
            exit_code: Some(1),
            stdout: "",
            stderr: &stderr,
        });

        assert!(
            matches!(result, Err(ProbeFailure::VideoUnavailable { .. })),
            "решает итоговая строка ERROR, а не шум до неё"
        );
    }

    #[test]
    fn region_wording_of_youtube_is_recognised() {
        // ЕДИНСТВЕННЫЙ тест набора не на снятом выводе: ролика,
        // заблокированного для страны съёмки, найти не удалось — YouTube
        // не показывает такие в поиске и в выдаче каналов, а угадывать
        // идентификаторы бессмысленно. Строка собрана из формулировки
        // YouTube, которую yt-dlp передаёт дословно, и обрамления, снятого
        // с настоящей ошибки. Появится ролик — фикстура заменит этот тест.
        let stderr = "ERROR: [youtube] dQw4w9WgXcQ: Video unavailable. \
                      The uploader has not made this video available in your country";

        let result = classify(&YtDlpOutcome {
            exit_code: Some(1),
            stdout: "",
            stderr,
        });

        assert!(
            matches!(result, Err(ProbeFailure::RegionBlocked { .. })),
            "региональная блокировка сформулирована как частный случай \
             недоступности — общий маркер не должен съедать частный"
        );
    }

    #[test]
    fn other_os_network_markers_stay_silent_on_the_captured_output() {
        // Маркеры Linux и Windows живьём не проверены (Р-6): машин нет.
        // Проверить можно хотя бы обратное — что ни один из них не
        // срабатывает на том, что снято на macOS, то есть не может
        // случайно перехватить чужой класс.
        for (name, _) in OUTCOMES {
            let text = capture(name).stderr.to_lowercase();
            for marker in NETWORK_MARKERS_OTHER_OS {
                assert!(
                    !text.contains(marker),
                    "{name}: маркер другой ОС «{marker}» встретился в выводе macOS — \
                     значит он не так специфичен, как считалось"
                );
            }
        }
    }

    #[test]
    fn details_carry_the_tail_and_the_exit_code_but_not_the_whole_stderr() {
        // Н-4: наружу уходит хвост, а не поток целиком.
        let capture = capture("network-proxy-refused");
        let full = capture.stderr.trim().chars().count();
        assert!(
            full > STDERR_TAIL_MAX_CHARS,
            "фикстура должна быть длиннее предела, иначе тест ничего не проверяет"
        );

        let failure = classify(&capture.outcome()).expect_err("это отказ");
        let contract = failure.to_contract();
        let details = contract.details.expect("детали есть");

        let tail = details.stderr_tail.expect("хвост есть");
        assert_eq!(tail.chars().count(), STDERR_TAIL_MAX_CHARS);
        assert!(
            capture.stderr.trim().ends_with(&tail),
            "хвост обязан быть концом настоящего stderr, а не пересказом"
        );
        assert_eq!(details.exit_code, Some(1));
    }

    #[test]
    fn a_short_stderr_reaches_the_contract_whole() {
        let capture = capture("video-unavailable-missing-id");
        let failure = classify(&capture.outcome()).expect_err("это отказ");
        let details = failure.to_contract().details.expect("детали есть");

        assert_eq!(details.stderr_tail.as_deref(), Some(capture.stderr.trim()));
    }

    #[test]
    fn a_silent_process_leaves_no_details_to_show() {
        // Процесс не сказал ничего и не оставил кода: «Подробнее»
        // показывать нечего, и пустой объект границу не пересекает.
        let failure = classify(&YtDlpOutcome {
            exit_code: None,
            stdout: "",
            stderr: "   \n  ",
        })
        .expect_err("молчание — не успех");

        assert_eq!(
            verdict(&Err(failure.clone())),
            Expected::YtDlpFailure(YtDlpFailureReason::Generic)
        );
        assert_eq!(failure.to_contract().details, None);
    }

    #[test]
    fn exit_code_zero_without_metadata_is_not_a_success() {
        // `-J` печатает в stdout `null`, когда извлекать нечего. Валидный
        // JSON — но не карточка.
        let result = classify(&YtDlpOutcome {
            exit_code: Some(0),
            stdout: "null\n",
            stderr: "",
        });

        assert_eq!(
            verdict(&result),
            Expected::YtDlpFailure(YtDlpFailureReason::Generic)
        );
    }

    #[test]
    fn a_truncated_json_is_a_yt_dlp_failure_and_not_a_panic() {
        // Процесс убит на полуслове: разбор обязан пережить обрывок.
        let result = classify(&YtDlpOutcome {
            exit_code: Some(0),
            stdout: "{\"id\": \"abc\", \"formats\": [",
            stderr: "",
        });

        assert_eq!(
            verdict(&result),
            Expected::YtDlpFailure(YtDlpFailureReason::Generic)
        );
    }

    #[test]
    fn no_class_leaks_a_rust_identifier_into_the_message() {
        // Сторож контракта (TL-27) — но уже на настоящих исходах, а не на
        // собранных руками вариантах: `message` виден в «Подробнее».
        for (name, _) in OUTCOMES {
            let Err(failure) = classify(&capture(name).outcome()) else {
                continue;
            };

            let message = failure.to_contract().message;
            for identifier in ["Generic", "Outdated", "ProbeFailure", "YtDlpFailureReason"] {
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

    /// Версия yt-dlp из пина `binaries.lock.json` — та, что реально
    /// вкладывается в приложение.
    fn pinned_yt_dlp_version() -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("binaries.lock.json");
        let pin = read_json(&path);

        pin.get("ytDlp")
            .and_then(|yt_dlp| yt_dlp.get("version"))
            .and_then(Value::as_str)
            .expect("в пине объявлена версия yt-dlp")
            .to_owned()
    }
}
