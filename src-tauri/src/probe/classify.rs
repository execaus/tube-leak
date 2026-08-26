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
//! will begin in 3 days.»), то есть класс «прямая трансляция» живёт на
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

use serde_json::Value;

use crate::probe::error::ProbeFailure;
// Хвост stderr для «Подробнее» (Н-4) берётся общей функцией домена
// `sidecar`, а не своей копией: предел и способ обрезки — одна конвенция
// приложения на служебный экран E1 и на разбор ссылки, и две копии этой
// конвенции неизбежно разошлись бы.
use crate::sidecar::stderr_tail;
use crate::types::{ProbeErrorDetails, YtDlpFailureReason};

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
    if outcome.exit_code == Some(0) {
        if let Some(metadata) = parse_metadata(outcome.stdout) {
            if is_playlist(&metadata) {
                return Err(ProbeFailure::PlaylistUnsupported {
                    details: details(outcome),
                });
            }
            if is_live(&metadata) {
                return Err(ProbeFailure::LiveUnsupported {
                    details: details(outcome),
                });
            }

            return Ok(metadata);
        }
    }

    Err(failure_from_stderr(outcome.stderr, details(outcome)))
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
/// Порядок проверок нагружен: группы маркеров пересекаются словами, и
/// первая совпавшая решает исход. Обоснование каждого шага — в
/// комментариях внутри.
///
/// # Чем порядок проверен, а чем — нет
///
/// Из девятнадцати снятых исходов **ровно один** попадает больше чем в
/// одну группу: `playlist-without-network` (ссылка на плейлист при
/// выключенной сети — метка `[youtube:tab]` и транспортный сбой сразу).
/// Он и держит единственную пару шагов, проверенную живым выводом, —
/// шаги 1 и 2, тест
/// `a_playlist_link_stays_a_playlist_even_when_the_network_is_down`.
///
/// Остальные пары порядка на снятом выводе **не проверяются ничем**:
/// каждая живая фикстура совпадает ровно с одной группой, и перестановка
/// шагов 3–6 между собой (включая порядок внутри
/// [`shared_failure_class`]) не изменила бы на наборе ни одного исхода.
/// Тесты вроде `a_gone_recording_of_a_broadcast_is_unavailable_and_not_live`
/// проверяют **состав** маркеров (что общее слово не попало в чужую
/// группу), а не их очерёдность, и прошли бы при любой перестановке.
/// Пары, за которыми есть смысл, но нет живого вывода, закреплены
/// собранными строками в
/// `the_order_of_the_groups_decides_when_two_of_them_match_at_once`; там
/// же сказано, из чего собрана каждая. Если менять порядок — смотреть
/// надо туда, живой набор перестановку не заметит.
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

    // 4–6. Регион → вход → «ролик недоступен». Три класса, общие с
    //    эпиком скачивания, и порядок между ними тоже общий — он живёт в
    //    [`shared_failure_class`], а не здесь.
    if let Some(shared) = shared_failure_class(&text) {
        return match shared {
            SharedFailureClass::RegionBlocked => ProbeFailure::RegionBlocked { details },
            SharedFailureClass::SignInRequired => ProbeFailure::SignInRequired { details },
            SharedFailureClass::VideoUnavailable => ProbeFailure::VideoUnavailable { details },
        };
    }

    // 7. Всё остальное — сбой yt-dlp; под-причина меняет только текст
    //    пояснения на экране, но не класс.
    ProbeFailure::YtDlpFailure {
        reason: yt_dlp_failure_reason(&text),
        details,
    }
}

/// Класс отказа, который эпик скачивания (E3) переиспользует у разбора
/// ссылки (E2) без изменений.
///
/// Три значения, а не четыре: «сбой yt-dlp» тоже общий, но у него нет
/// собственных маркеров — он остаток после всех проверок, и остаток у
/// каждого вызывающего свой (у скачивания перед ним стоят ещё «нет места»
/// и «формат недоступен»). Общее у этого класса — только под-причина, и
/// её отдаёт [`yt_dlp_failure_reason`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SharedFailureClass {
    RegionBlocked,
    SignInRequired,
    VideoUnavailable,
}

/// Класс по тексту отказа — тот, который на проводе одинаков у разбора
/// ссылки и у скачивания.
///
/// `text` обязан быть результатом [`fatal_text`]: маркеры записаны в
/// нижнем регистре и сравниваются подстрокой.
///
/// Порядок внутри нагружен и переезжает к вызывающим целиком:
///
/// 1. **Регион — строго перед «ролик недоступен»**: YouTube формулирует
///    региональную блокировку как частный случай недоступности («Video
///    unavailable. This video is not available in your country»), и общий
///    маркер съел бы частный.
/// 2. **Вход — тоже перед «ролик недоступен»** и по тому же правилу:
///    «video unavailable» — такая же общая обёртка, и формулировка вида
///    «This video is unavailable. Sign in…» уехала бы в класс, у которого
///    действия нет вовсе, вместо класса, у которого действие появится в
///    E8. Живого текста с обоими признаками сразу в снятом выводе нет —
///    порядок стоит на будущее, а не на наблюдении (см. тест
///    `the_order_of_the_groups_decides_when_two_of_them_match_at_once`).
/// 3. **Ролик недоступен**: удалён, снят, не существует.
pub(crate) fn shared_failure_class(text: &str) -> Option<SharedFailureClass> {
    if contains_any(text, &REGION_MARKERS) {
        return Some(SharedFailureClass::RegionBlocked);
    }
    if contains_any(text, &SIGN_IN_MARKERS) {
        return Some(SharedFailureClass::SignInRequired);
    }
    if contains_any(text, &UNAVAILABLE_MARKERS) {
        return Some(SharedFailureClass::VideoUnavailable);
    }
    None
}

/// Транспортный сбой: до YouTube не доехал запрос.
///
/// У разбора ссылки это класс «нет сети», у скачивания — не класс вовсе,
/// а повод уйти в цикл повторов (С-6): маркеры одни, решение по ним
/// разное, поэтому функция отвечает фактом, а не классом.
pub(crate) fn is_transport_failure(text: &str) -> bool {
    contains_any(text, &NETWORK_MARKERS) || contains_any(text, &NETWORK_MARKERS_OTHER_OS)
}

/// Под-причина класса «сбой yt-dlp»: опознан ли в тексте признак того,
/// что yt-dlp не понимает ответ YouTube.
///
/// Общая для обоих эпиков ровно потому, что признак один и тот же:
/// [`crate::types::YtDlpFailureReason::Outdated`] меняет только текст
/// пояснения на экране, но не класс.
pub(crate) fn yt_dlp_failure_reason(text: &str) -> YtDlpFailureReason {
    if contains_any(text, &OUTDATED_MARKERS) {
        YtDlpFailureReason::Outdated
    } else {
        YtDlpFailureReason::Generic
    }
}

/// Текст, по которому принимается решение: строки `ERROR:` в нижнем
/// регистре.
///
/// Если ни одной строки `ERROR:` нет (процесс убит, упал до вывода,
/// сказал всё предупреждениями) — берётся весь stderr: лучше решать по
/// шумному тексту, чем не решать вовсе.
///
/// # Перенос текста на следующую строку
///
/// Пустая `ERROR:` забирает себе следующую строку. Правило появилось не
/// из осторожности, а из снятого вывода **скачивания** (E3, фикстура
/// `connection-lost-mid-download`): когда полоса прогресса уже что-то
/// напечатала, yt-dlp закрывает её переводом строки, и отказ выезжает
/// двумя строками — пустой `ERROR:` и текст под ней:
///
/// ```text
/// ERROR:
/// [download] Got error: ('Unable to connect to proxy', …). Giving up after 10 retries
/// ```
///
/// Без переноса от такого отказа остаётся строка `error:`, в которой нет
/// ни одного маркера, и транспортный сбой уезжает в «сбой yt-dlp» — то
/// есть в класс, у которого нет ни повторов, ни объяснения. На выводе
/// разбора ссылки (`-J`, полосы нет) правило не меняет ничего: во всех
/// девятнадцати снятых исходах E2 текст стоит на той же строке.
pub(crate) fn fatal_text(stderr: &str) -> String {
    let lines: Vec<&str> = stderr.lines().collect();
    let mut errors: Vec<&str> = Vec::new();

    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("ERROR:") else {
            continue;
        };

        errors.push(line);
        if rest.trim().is_empty() {
            if let Some(continuation) = lines.get(index + 1) {
                errors.push(continuation);
            }
        }
    }

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
const LIVE_MARKERS: [&str; 2] = [
    // Снято живьём: «This live event will begin in 3 days.» Взято общее
    // начало: дальше идёт срок, а у эфира, который вот-вот начнётся, —
    // «in a few moments» вместо срока.
    "live event will begin",
    // ЖИВЬЁМ НЕ ПРОВЕРЕНО: премьера теми же словами, но другим глаголом.
    // Подходящего ролика на момент съёмки фикстур не нашлось.
    "premieres in",
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
    use crate::sidecar::STDERR_TAIL_MAX_CHARS;
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
    const OUTCOMES: [(&str, Expected); 19] = [
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
        (
            "playlist-without-network",
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

    /// Итоговая строка `ERROR:` снятой фикстуры — как есть.
    fn first_error_line(name: &str) -> String {
        capture(name)
            .stderr
            .lines()
            .find(|line| line.starts_with("ERROR:"))
            .expect("в фикстуре отказа есть итоговая ошибка")
            .to_string()
    }

    /// Приписка «please report this issue … yt-dlp -U», снятая с живого
    /// вывода: yt-dlp дописывает её к любой неожиданной ошибке.
    fn report_issue_footer() -> String {
        let line = first_error_line("network-proxy-refused");
        let at = line
            .find("please report this issue")
            .expect("в сетевой фикстуре есть приписка про отчёт об ошибке");

        line[at..].to_string()
    }

    /// Группы маркеров, совпавшие с выводом, в порядке проверки.
    fn matching_groups(stderr: &str) -> Vec<&'static str> {
        let text = fatal_text(stderr);
        let mut groups = Vec::new();

        if text.contains("[youtube:tab]") {
            groups.push("плейлист");
        }
        if contains_any(&text, &NETWORK_MARKERS) || contains_any(&text, &NETWORK_MARKERS_OTHER_OS) {
            groups.push("сеть");
        }
        if contains_any(&text, &LIVE_MARKERS) {
            groups.push("трансляция");
        }
        if contains_any(&text, &REGION_MARKERS) {
            groups.push("регион");
        }
        if contains_any(&text, &SIGN_IN_MARKERS) {
            groups.push("вход");
        }
        if contains_any(&text, &UNAVAILABLE_MARKERS) {
            groups.push("недоступен");
        }

        groups
    }

    #[test]
    fn only_one_captured_outcome_matches_two_groups_at_once() {
        // Честная мера того, что живой набор проверяет в порядке шагов, а
        // что нет: если вывод совпал ровно с одной группой, перестановка
        // шагов на нём ничего не изменит. Такова вся выборка, кроме
        // единственного исхода — ссылки на плейлист при выключенной сети.
        for (name, _) in OUTCOMES {
            let groups = matching_groups(&capture(name).stderr);

            if name == "playlist-without-network" {
                assert_eq!(
                    groups,
                    vec!["плейлист", "сеть"],
                    "{name}: фикстура снята ради двух признаков сразу"
                );
                continue;
            }

            assert!(
                groups.len() <= 1,
                "{name}: вывод совпал с группами {groups:?}. Появилась вторая \
                 живая пара — закрепите её порядок отдельным тестом и \
                 поправьте doc `failure_from_stderr`, который сейчас честно \
                 говорит, что такая пара одна"
            );
        }
    }

    #[test]
    fn a_playlist_link_stays_a_playlist_even_when_the_network_is_down() {
        // Единственная пара шагов, проверенная снятым выводом: в одной
        // строке ERROR сразу метка экстрактора `[youtube:tab]` и отказ
        // резолвера. Выбран плейлист, а не сеть: метка проставлена по
        // адресу ещё до обращения к сети, и даже с восстановленной сетью
        // эта ссылка карточку не даст — «вставьте ссылку на отдельный
        // ролик» остаётся единственным действием, которое что-то меняет.
        let capture = capture("playlist-without-network");
        let error = first_error_line("playlist-without-network");

        assert!(
            error.contains("[youtube:tab]"),
            "признак плейлиста на месте"
        );
        assert!(
            error.to_lowercase().contains("failed to resolve"),
            "признак мёртвой сети на месте"
        );

        assert!(matches!(
            classify(&capture.outcome()),
            Err(ProbeFailure::PlaylistUnsupported { .. })
        ));
    }

    #[test]
    fn the_order_of_the_groups_decides_when_two_of_them_match_at_once() {
        // Пары, за которыми есть смысл, но которых нет в снятом выводе.
        // Каждая строка собрана из настоящей итоговой ошибки фикстуры и
        // дописанной формулировки второй группы — это единственный способ
        // закрепить очерёдность, пока YouTube не выдал такой текст сам.
        let cases: [(&str, String, ProbeErrorKind); 3] = [
            (
                "региональная блокировка сформулирована как частный случай \
                 недоступности — общая обёртка не должна её съедать",
                format!(
                    "{} The uploader has not made this video available in your country",
                    first_error_line("video-unavailable-missing-id")
                ),
                ProbeErrorKind::RegionBlocked,
            ),
            (
                "просьба войти важнее общей недоступности: у класса «вход» \
                 действие появится в E8, у «недоступен» действия нет вовсе",
                format!(
                    "{}. Sign in to confirm your age",
                    first_error_line("video-unavailable-deleted")
                ),
                ProbeErrorKind::SignInRequired,
            ),
            (
                "известный класс важнее признака устаревания: под-причина \
                 уточняет только «сбой yt-dlp», а не перебивает диагноз",
                format!(
                    "{}; unable to extract player response",
                    first_error_line("video-unavailable-missing-id")
                ),
                ProbeErrorKind::VideoUnavailable,
            ),
        ];

        for (why, stderr, expected) in cases {
            assert!(
                matching_groups(&stderr).len() >= 2 || expected == ProbeErrorKind::VideoUnavailable,
                "{why}: вход обязан совпасть с двумя группами, иначе тест \
                 порядка ничего не проверяет"
            );

            let result = classify(&YtDlpOutcome {
                exit_code: Some(1),
                stdout: "",
                stderr: &stderr,
            });

            let Err(failure) = result else {
                panic!("{why}: это отказ");
            };
            assert_eq!(failure.kind(), expected, "{why}");
        }
    }

    #[test]
    fn warnings_alone_decide_the_class_when_there_is_no_error_line() {
        // Фолбэк на весь stderr: процесс упал, не сказав ничего строкой
        // ERROR. Взяты настоящие предупреждения о повторах из сетевой
        // фикстуры — без итоговой строки они и есть всё, что известно.
        let captured = capture("network-proxy-refused");
        let stderr = captured
            .stderr
            .lines()
            .filter(|line| line.starts_with("WARNING:"))
            .collect::<Vec<&str>>()
            .join("\n");

        assert!(
            !stderr.contains("ERROR:"),
            "во входе не должно остаться итоговой строки"
        );

        let result = classify(&YtDlpOutcome {
            exit_code: Some(1),
            stdout: "",
            stderr: &stderr,
        });

        assert!(
            matches!(result, Err(ProbeFailure::NetworkUnavailable { .. })),
            "решать по шумному тексту лучше, чем не решать вовсе"
        );
    }

    #[test]
    fn the_report_this_issue_footer_never_makes_a_failure_look_outdated() {
        // Ловушка сама по себе: приписка про «последнюю версию» стоит в
        // сетевой фикстуре, но та отсекается шагом 2 и до сигнатуры
        // устаревания не доходит никогда — то есть на ней утверждение
        // «приписка не считается устареванием» непроверяемо. Здесь вход
        // собран так, чтобы дойти до шага 7: настоящая ошибка, не
        // совпадающая ни с одной группой классов, плюс настоящая приписка.
        let stderr = format!(
            "{} {}",
            first_error_line("ytdlp-failure-generic"),
            report_issue_footer()
        );

        assert!(
            stderr
                .to_lowercase()
                .contains("confirm you are on the latest version"),
            "приписка на месте — иначе тест ничего не проверяет"
        );
        assert!(
            matching_groups(&stderr).is_empty(),
            "вход обязан дойти до шага 7, а не отсечься раньше"
        );

        let result = classify(&YtDlpOutcome {
            exit_code: Some(1),
            stdout: "",
            stderr: &stderr,
        });

        assert_eq!(
            verdict(&result),
            Expected::YtDlpFailure(YtDlpFailureReason::Generic),
            "«Confirm you are on the latest version» — приписка к любой \
             неожиданной ошибке, а не признак устаревшего yt-dlp"
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
