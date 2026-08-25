//! Форматирование текущего времени в RFC 3339 без внешних зависимостей.
//!
//! Вынесено из `crate::commands::sidecar` в TL-12: тем же штампом
//! помечается манифест распакованного дерева yt-dlp
//! (`crate::ytdlp::layout::Manifest`), а держать две копии алгоритма
//! перевода дней в календарную дату — гарантированное расхождение в
//! будущем.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Текущее время в UTC, отформатированное как RFC 3339
/// (`2026-08-25T09:15:30.123Z`), без внешних зависимостей — только
/// `std::time` плюс алгоритм перевода дней с эпохи Unix в календарную дату
/// Говарда Хайнанта (`civil_from_days`, общественное достояние, см.
/// http://howardhinnant.github.io/date_algorithms.html). Добавлять `chrono`
/// или `time` ради одного поля лога — решение о новой зависимости, не
/// принимается на уровне этой задачи (см. CLAUDE.md, «owner-level
/// questions»).
pub fn now_iso8601() -> String {
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format_unix_timestamp(since_epoch)
}

/// Текущее время в секундах с эпохи Unix.
///
/// Отдельно от [`now_iso8601`], потому что по нему считают, а не читают:
/// «сколько прошло с прошлой попытки» (`crate::ytdlp::layout::RepairLog`)
/// из отформатированной строки не вывести, не разбирая её обратно.
pub fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Текущее время в наносекундах с эпохи Unix, усечённое до `u64`
/// (переполнение — 2554 год).
///
/// Нужно там, где от времени требуется не точка на календаре, а различие
/// между двумя соседними вызовами: суффикс каталога распаковки
/// (`crate::ytdlp::layout`).
pub fn now_unix_nanos() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
    )
    .unwrap_or(u64::MAX)
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
/// для всего диапазона дат, поддерживаемых `i64`, включая годы до эпохи.
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

    #[test]
    fn unix_seconds_agree_with_the_formatted_timestamp() {
        // Обе функции обязаны читать одни и те же часы: по одной считают
        // остывание счётчика починки, по другой его читают в логе.
        let secs = now_unix_secs();
        assert!(secs > 1_700_000_000, "часы явно не идут от эпохи Unix");
        assert_eq!(
            format_unix_timestamp(Duration::from_secs(secs))[..10],
            now_iso8601()[..10]
        );
    }

    #[test]
    fn unix_nanoseconds_move_between_calls() {
        // От суффикса каталога распаковки требуется различие соседних
        // значений, а не точность.
        assert_ne!(now_unix_nanos(), 0);
        assert!(now_unix_nanos() <= now_unix_nanos());
    }

    #[test]
    fn produces_a_plausible_timestamp_for_the_current_moment() {
        // Не сверяем с эталоном (часы идут), но форма и порядок величины
        // ловят подмену эпохи и «съеденные» разряды года.
        let now = now_iso8601();
        assert_eq!(now.len(), "1970-01-01T00:00:00.000Z".len());
        assert!(now.ends_with('Z'));
        assert!(now.as_str() > "2020-01-01T00:00:00.000Z");
    }
}
