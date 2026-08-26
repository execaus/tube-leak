//! Типы, пересекающие границу Rust↔TS: результат проверки sidecar-бинарников
//! (yt-dlp, ffmpeg), ход подготовки yt-dlp при первом запуске (TL-12) и
//! результат разбора ссылки на ролик (TL-27, эпик E2).
//! Объявлены здесь один раз; TS-зеркало в `src/types/` поддерживает точное
//! соответствие полей и значений enum-строк — расхождение с этим файлом
//! дорого чинить постфактум (см. TL-1/TL-2 в эпике E1).

use serde::{Deserialize, Serialize};

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

// ────────────────────── разбор ссылки на ролик (TL-27) ──────────────────────
//
// Контракт эпика E2: что команда `probe_url` возвращает фронтенду при успехе
// и чем отвечает при отказе. Реализация — TL-30 (схлопывание форматов),
// TL-31 (классификация ошибок), TL-32 (оркестрация и сами команды); здесь
// только объявление типов, которые зеркалятся в `src/types/` (TL-29).
//
// Отсюда и `#[allow(dead_code)]` на каждом типе секции: конструирует их код,
// которого ещё нет, а зеркало (TL-29) и экран (TL-33) пишутся по контракту
// уже сейчас. Атрибуты снимаются задачами, которые начнут эти типы
// заполнять, — по той же причине, что и у типов E1 выше: контракт не должен
// исчезать из-за того, что реализация отстаёт на задачу.

/// Вид пункта лестницы качеств (решение владельца Р-1).
///
/// Признак вида объявлен явно, а не выводится фронтендом из числа пикселей:
/// иначе правило «2160p» против «максимальное доступное (NNNp)» жило бы
/// двумя копиями — в схлопывании форматов (TL-30) и в подписи строки, и
/// копии разошлись бы при первом же уточнении правила.
///
/// Лестница показывает **только доступные** строки: пункта, которого у
/// ролика нет, в списке нет вовсе — ни выключенного, ни с пометкой (Р-1).
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum QualityKind {
    /// Обычная ступень лестницы: 2160p / 1440p / 1080p / 720p. Подпись
    /// строится из `heightPx` («1080p»).
    Standard,
    /// Единственная видеострока для ролика, максимум которого ниже 720p:
    /// «Максимальное доступное (NNNp)», число берётся из `heightPx` (Р-1).
    /// Вместо всей видео-лестницы, а не в дополнение к ней.
    MaxAvailable,
    /// «Только аудио» — присутствует всегда (Р-1) и идёт последней строкой.
    /// `heightPx` у такого пункта нет.
    AudioOnly,
}

/// Оценка размера пункта: либо число байт, либо явное «неизвестно» (Ф-4).
///
/// Отдельный вариант, а не `0`/`null`: «нет данных» и «нулевой размер» —
/// разные вещи, и строка «размер неизвестен» остаётся кликабельной, то есть
/// отсутствие оценки не делает пункт неполноценным. Приблизительность
/// оценки в контракт не выносится: yt-dlp даёт оценку и в поле точного
/// размера тоже, гарантий совпадения с итоговым файлом эпик не даёт, и UI
/// показывает любую оценку одинаково — со знаком «≈».
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum QualitySize {
    /// Сумма размеров агрегированных потоков пункта.
    Known { bytes: u64 },
    /// Ни у одного потока пункта нет данных о размере.
    Unknown,
}

/// Непрозрачные для фронтенда идентификаторы потоков, которые скачает E3.
///
/// UI не интерпретирует эти строки и ничего по ним не решает — он
/// возвращает объект обратно вместе с выбранным пунктом, поэтому тип
/// десериализуемый, а не только сериализуемый: это единственное поле
/// контракта, которое ходит в обе стороны границы. Требование Ф-3 —
/// «результат разбора вместе с выбранным пунктом однозначно определяет,
/// что скачивать, без повторного разбора» — держится именно на нём.
///
/// Присутствие полей задаётся видом пункта и устройством форматов ролика:
///
/// - `audioOnly` — только `audioFormatId`;
/// - обычная ступень из раздельных потоков — оба поля;
/// - обычная ступень, у которой лучший видеопоток уже содержит звук
///   (прогрессивный формат) — только `videoFormatId`.
///
/// Инвариант: хотя бы одно из полей заполнено всегда — пункт без единого
/// потока не имеет смысла и в список не попадает. Проверяется
/// [`QualityStreams::has_any`]; `Default` тип сознательно не выводит —
/// дефолт конструировал бы ровно то состояние, которое инвариант
/// запрещает, а тип десериализуемый, то есть пустой объект может приехать
/// и снаружи (`{}` из UI в E3).
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QualityStreams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_format_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_format_id: Option<String>,
}

impl QualityStreams {
    /// Есть ли хоть один поток, то есть выполняется ли инвариант типа.
    ///
    /// Одно место на всех, кто его проверяет: TL-30 (не выпускать пункт
    /// без потоков в лестницу), TL-32 и E3 (не принимать такой объект
    /// обратно от UI). Три копии условия разошлись бы.
    // Объявлено раньше своих вызывающих — как и типы этой секции.
    #[allow(dead_code)]
    pub fn has_any(&self) -> bool {
        self.video_format_id.is_some() || self.audio_format_id.is_some()
    }
}

/// Строка лестницы качеств: что это за пункт, сколько примерно весит и что
/// скачивать, если пользователь выберет именно его.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualityItem {
    pub kind: QualityKind,
    /// Ступень качества в том виде, в каком её показывает YouTube: есть у
    /// `standard` и `maxAvailable`, нет у `audioOnly`. Идёт в подпись
    /// строки, а не в решение о её виде.
    ///
    /// У обычного горизонтального ролика 16:9 совпадает с пиксельной
    /// высотой кадра — отсюда имя поля; у вертикальных и кашетированных
    /// не совпадает: у вертикального «1080p» кадр 1080×1920, и подпись
    /// «1920p» разошлась бы с тем, что пользователь видит в плеере.
    /// Источник числа — метка качества yt-dlp, при её отсутствии короткая
    /// сторона кадра (решение владельца Р-4; правило целиком —
    /// в [`crate::probe`], модуль лестницы качеств).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height_px: Option<u32>,
    pub size: QualitySize,
    pub streams: QualityStreams,
}

/// Успешный разбор ролика — всё, из чего рисуется карточка (Ф-5, Р-2).
///
/// Порядок `qualities` задаёт ядро и он фиксирован: от большего разрешения
/// к меньшему, «только аудио» последней. UI рисует список как пришёл и не
/// сортирует его сам — иначе правило порядка окажется в двух местах.
///
/// `channel` и `thumbnailUrl` опциональны не «на всякий случай»: карточка
/// обязана оставаться полезной, если yt-dlp не отдал имя канала или у
/// ролика нет превью — соответствующий элемент просто не рисуется
/// (превью — со статичным плейсхолдером, дизайн E2).
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    /// Название как есть, без обрезания и нормализации: К-1 требует
    /// посимвольного совпадения с YouTube. Санитизация для имён файлов —
    /// забота E3, не этого типа.
    pub title: String,
    /// Длительность в секундах; форматирование в `м:сс`/`ч:мм:сс` — на UI.
    pub duration_secs: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    /// URL превью, который webview грузит напрямую с CDN YouTube (Р-2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
    pub qualities: Vec<QualityItem>,
}

/// Девять классов отказа разбора (Ф-6). Ровно по ним фронтенд выбирает
/// заголовок, пояснение и наличие кнопки «Повторить» (таблица в дизайне
/// E2) — тексты живут на стороне UI, в контракте только классификация.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProbeErrorKind {
    /// Ввод не является http(s)-ссылкой (С-4). Единственный класс, который
    /// возникает **до** запуска процесса: yt-dlp не запускается вовсе,
    /// поэтому у него нет ни `details`, ни «Подробнее» на экране.
    NotAUrl,
    /// Ролик удалён, скрыт или никогда не существовал (С-5).
    VideoUnavailable,
    /// Нужен вход в аккаунт YouTube — возрастное ограничение, подписка и
    /// прочее (С-6). Возрастное ограничение отдельным классом не выделено
    /// сознательно: без входа (E8) действие пользователя одно и то же.
    SignInRequired,
    /// Ролик не показывается в стране пользователя (С-7).
    RegionBlocked,
    /// Нет соединения с интернетом (С-8).
    NetworkUnavailable,
    /// Ссылка ведёт на плейлист или канал (С-9). Ролик с параметром
    /// плейлиста (`watch?v=…&list=…`) этим классом не является — он
    /// разбирается как одиночный ролик.
    PlaylistUnsupported,
    /// Идущий эфир или запланированная премьера (С-10). Завершённая
    /// трансляция с готовой записью — обычный ролик, не этот класс.
    LiveUnsupported,
    /// Сбой yt-dlp, не отнесённый к классам выше (С-12). Под-причина —
    /// в `reason`.
    YtDlpFailure,
    /// Разбор не уложился в отведённое время, процесс убит. Значение
    /// сработавшего порога — в `timeoutSecs`.
    Timeout,
}

/// Под-причина класса `ytDlpFailure` (С-12): меняет только текст пояснения,
/// класс остаётся один — тот же приём, что `launchFailed.reason` в E1.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum YtDlpFailureReason {
    /// Ошибка без узнаваемой сигнатуры: «попробуйте ещё раз».
    Generic,
    /// В выводе опознан признак того, что встроенный yt-dlp не понимает
    /// текущий ответ YouTube. Обновление — E6; здесь только честный текст.
    Outdated,
}

/// Технические детали отказа для свёрнутого «Подробнее».
///
/// Через границу команды идёт именно это — хвост stderr и код завершения,
/// а не поток целиком: полный stderr пишется в лог приложения и на экран
/// не попадает ни в каком состоянии (Н-4).
///
/// Пустая структура границу не пересекает: если сказать нечего, у ошибки
/// нет `details` вовсе (см. [`ProbeErrorDetails::is_empty`] и проекцию
/// [`crate::probe::ProbeFailure::to_contract`]). Иначе фронтенд получил бы
/// повод нарисовать «Подробнее», за которым пусто.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeErrorDetails {
    /// Хвост stderr процесса, обрезанный по длине.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr_tail: Option<String>,
    /// Код завершения процесса yt-dlp, если процесс успел завершиться сам.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

impl ProbeErrorDetails {
    /// Нечего показывать: ни хвоста stderr, ни кода завершения.
    ///
    /// Так бывает у процесса, убитого по таймауту раньше, чем он что-то
    /// написал, — и это не повод отдавать фронтенду пустой объект.
    pub fn is_empty(&self) -> bool {
        self.stderr_tail.is_none() && self.exit_code.is_none()
    }
}

/// Отказ разбора в сериализуемом виде — то, чем реджектится `probe_url`.
///
/// Решение принимается по `kind` (и по `reason` внутри `ytDlpFailure`);
/// `message` — формулировка ядра для «Подробнее» и лога, **не** основной
/// текст на экране: тексты для пользователя задаёт UI по классу (дизайн
/// E2), и stderr в них не попадает никогда (Н-4).
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeError {
    pub kind: ProbeErrorKind,
    pub message: String,
    /// Только при `kind = ytDlpFailure`; для него заполняется всегда.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<YtDlpFailureReason>,
    /// Только при `kind = timeout`: сработавший порог в секундах,
    /// подставляется в текст ошибки («…за отведённое время (N с)»).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    /// Отсутствует, когда деталей нет: `notAUrl` (процесс не запускался) и
    /// вообще любой отказ, о котором нечего сказать технически.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<ProbeErrorDetails>,
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

    // ───────────────── разбор ссылки на ролик (TL-27) ─────────────────
    //
    // Тесты фиксируют не «что код сериализует», а форму JSON, по которой
    // пишется TS-зеркало (TL-29) и экран (TL-33): любое переименование поля
    // или значения enum обязано ронять их, а не всплывать в рантайме.

    fn video_streams() -> QualityStreams {
        QualityStreams {
            video_format_id: Some("137".to_string()),
            audio_format_id: Some("140".to_string()),
        }
    }

    #[test]
    fn serializes_a_standard_ladder_item_with_a_known_size() {
        let item = QualityItem {
            kind: QualityKind::Standard,
            height_px: Some(1080),
            size: QualitySize::Known { bytes: 536_870_912 },
            streams: video_streams(),
        };

        assert_eq!(
            serde_json::to_value(&item).expect("serialization must not fail"),
            json!({
                "kind": "standard",
                "heightPx": 1080,
                "size": { "kind": "known", "bytes": 536_870_912u64 },
                "streams": { "videoFormatId": "137", "audioFormatId": "140" },
            })
        );
    }

    #[test]
    fn serializes_a_max_available_item_for_a_video_below_720p() {
        let item = QualityItem {
            kind: QualityKind::MaxAvailable,
            height_px: Some(480),
            size: QualitySize::Known { bytes: 100_663_296 },
            streams: QualityStreams {
                video_format_id: Some("18".to_string()),
                audio_format_id: None,
            },
        };

        assert_eq!(
            serde_json::to_value(&item).expect("serialization must not fail"),
            json!({
                "kind": "maxAvailable",
                "heightPx": 480,
                "size": { "kind": "known", "bytes": 100_663_296u64 },
                "streams": { "videoFormatId": "18" },
            })
        );
    }

    #[test]
    fn serializes_an_audio_only_item_without_height_and_video_stream() {
        let item = QualityItem {
            kind: QualityKind::AudioOnly,
            height_px: None,
            size: QualitySize::Known { bytes: 14_680_064 },
            streams: QualityStreams {
                video_format_id: None,
                audio_format_id: Some("140".to_string()),
            },
        };

        let value = serde_json::to_value(&item).expect("serialization must not fail");

        assert_eq!(
            value,
            json!({
                "kind": "audioOnly",
                "size": { "kind": "known", "bytes": 14_680_064u64 },
                "streams": { "audioFormatId": "140" },
            })
        );
        // Отсутствующее поле именно отсутствует, а не приходит как null:
        // на TS-стороне это разница между `heightPx?: number` и
        // `heightPx: number | null`.
        let object = value.as_object().expect("item must be an object");
        assert!(!object.contains_key("heightPx"));
    }

    #[test]
    fn serializes_unknown_size_as_a_tagged_variant_without_bytes() {
        let item = QualityItem {
            kind: QualityKind::Standard,
            height_px: Some(1080),
            size: QualitySize::Unknown,
            streams: video_streams(),
        };

        let size =
            serde_json::to_value(&item).expect("serialization must not fail")["size"].clone();

        assert_eq!(size, json!({ "kind": "unknown" }));
        // Ф-4: «неизвестно» не должно читаться как нулевой размер.
        assert!(size.get("bytes").is_none());
    }

    #[test]
    fn quality_streams_survive_a_round_trip_through_json() {
        // Единственное поле контракта, которое ходит в обе стороны: UI
        // возвращает его в E3 вместе с выбранным пунктом (Ф-3).
        let streams = video_streams();
        let json = serde_json::to_string(&streams).expect("serialization must not fail");
        let back: QualityStreams = serde_json::from_str(&json).expect("deserialization must work");

        assert_eq!(back, streams);
    }

    #[test]
    fn omitted_quality_streams_deserialize_as_absent_not_as_an_error() {
        let back: QualityStreams =
            serde_json::from_str(r#"{"audioFormatId":"140"}"#).expect("deserialization must work");

        assert_eq!(
            back,
            QualityStreams {
                video_format_id: None,
                audio_format_id: Some("140".to_string()),
            }
        );
    }

    #[test]
    fn serializes_a_full_probe_result_with_channel_and_thumbnail() {
        let result = ProbeResult {
            title: "Пример ролика — «кавычки» и emoji 🎬".to_string(),
            duration_secs: 754,
            channel: Some("Канал".to_string()),
            thumbnail_url: Some("https://i.ytimg.com/vi/abc/maxresdefault.jpg".to_string()),
            qualities: vec![
                QualityItem {
                    kind: QualityKind::Standard,
                    height_px: Some(720),
                    size: QualitySize::Known { bytes: 303_038_464 },
                    streams: video_streams(),
                },
                QualityItem {
                    kind: QualityKind::AudioOnly,
                    height_px: None,
                    size: QualitySize::Unknown,
                    streams: QualityStreams {
                        video_format_id: None,
                        audio_format_id: Some("140".to_string()),
                    },
                },
            ],
        };

        assert_eq!(
            serde_json::to_value(&result).expect("serialization must not fail"),
            json!({
                "title": "Пример ролика — «кавычки» и emoji 🎬",
                "durationSecs": 754,
                "channel": "Канал",
                "thumbnailUrl": "https://i.ytimg.com/vi/abc/maxresdefault.jpg",
                "qualities": [
                    {
                        "kind": "standard",
                        "heightPx": 720,
                        "size": { "kind": "known", "bytes": 303_038_464u64 },
                        "streams": { "videoFormatId": "137", "audioFormatId": "140" },
                    },
                    {
                        "kind": "audioOnly",
                        "size": { "kind": "unknown" },
                        "streams": { "audioFormatId": "140" },
                    },
                ],
            })
        );
    }

    #[test]
    fn probe_result_without_channel_and_thumbnail_omits_both_keys() {
        let result = ProbeResult {
            title: "Ролик без канала и превью".to_string(),
            duration_secs: 61,
            channel: None,
            thumbnail_url: None,
            qualities: Vec::new(),
        };

        let value = serde_json::to_value(&result).expect("serialization must not fail");
        let object = value.as_object().expect("result must be an object");

        assert!(!object.contains_key("channel"));
        assert!(!object.contains_key("thumbnailUrl"));
        // Пустой список — валидная форма (ролик без единого пригодного
        // формата): карточка остаётся, лестница пуста.
        assert_eq!(object["qualities"], json!([]));
    }

    #[test]
    fn every_probe_error_kind_has_its_own_wire_value() {
        let kinds = [
            (ProbeErrorKind::NotAUrl, "notAUrl"),
            (ProbeErrorKind::VideoUnavailable, "videoUnavailable"),
            (ProbeErrorKind::SignInRequired, "signInRequired"),
            (ProbeErrorKind::RegionBlocked, "regionBlocked"),
            (ProbeErrorKind::NetworkUnavailable, "networkUnavailable"),
            (ProbeErrorKind::PlaylistUnsupported, "playlistUnsupported"),
            (ProbeErrorKind::LiveUnsupported, "liveUnsupported"),
            (ProbeErrorKind::YtDlpFailure, "ytDlpFailure"),
            (ProbeErrorKind::Timeout, "timeout"),
        ];

        // Ф-6 требует ровно девять классов: и лишний, и потерянный класс
        // здесь — расхождение с требованием, а не мелочь.
        assert_eq!(kinds.len(), 9);

        for (kind, expected) in kinds {
            assert_eq!(
                serde_json::to_value(kind).expect("serialization must not fail"),
                json!(expected)
            );
        }
    }

    #[test]
    fn serializes_not_a_url_error_without_reason_timeout_and_details() {
        let error = ProbeError {
            kind: ProbeErrorKind::NotAUrl,
            message: "ввод не похож на http(s)-ссылку".to_string(),
            reason: None,
            timeout_secs: None,
            details: None,
        };

        assert_eq!(
            serde_json::to_value(&error).expect("serialization must not fail"),
            json!({
                "kind": "notAUrl",
                "message": "ввод не похож на http(s)-ссылку",
            })
        );
    }

    #[test]
    fn serializes_yt_dlp_failure_with_outdated_reason_and_details() {
        let error = ProbeError {
            kind: ProbeErrorKind::YtDlpFailure,
            message: "yt-dlp не смог получить данные о ролике".to_string(),
            reason: Some(YtDlpFailureReason::Outdated),
            timeout_secs: None,
            details: Some(ProbeErrorDetails {
                stderr_tail: Some("ERROR: unable to extract player response".to_string()),
                exit_code: Some(1),
            }),
        };

        assert_eq!(
            serde_json::to_value(&error).expect("serialization must not fail"),
            json!({
                "kind": "ytDlpFailure",
                "message": "yt-dlp не смог получить данные о ролике",
                "reason": "outdated",
                "details": {
                    "stderrTail": "ERROR: unable to extract player response",
                    "exitCode": 1,
                },
            })
        );
    }

    #[test]
    fn serializes_generic_yt_dlp_failure_reason() {
        let value =
            serde_json::to_value(YtDlpFailureReason::Generic).expect("serialization must not fail");

        assert_eq!(value, json!("generic"));
    }

    #[test]
    fn serializes_timeout_error_with_the_threshold_that_fired() {
        let error = ProbeError {
            kind: ProbeErrorKind::Timeout,
            message: "разбор не уложился в 30 с".to_string(),
            reason: None,
            timeout_secs: Some(30),
            // Процесс убит по таймауту: своего кода завершения он не
            // оставил, но то, что успел написать в stderr, — оставил
            // (в E1 это ловит `captures_stderr_written_before_the_process_
            // is_killed_by_a_timeout`).
            details: Some(ProbeErrorDetails {
                stderr_tail: Some("[youtube] Downloading player".to_string()),
                exit_code: None,
            }),
        };

        assert_eq!(
            serde_json::to_value(&error).expect("serialization must not fail"),
            json!({
                "kind": "timeout",
                "message": "разбор не уложился в 30 с",
                "timeoutSecs": 30,
                "details": { "stderrTail": "[youtube] Downloading player" },
            })
        );
    }

    #[test]
    fn quality_streams_report_whether_the_invariant_holds() {
        assert!(video_streams().has_any());
        assert!(QualityStreams {
            video_format_id: None,
            audio_format_id: Some("140".to_string()),
        }
        .has_any());
        assert!(QualityStreams {
            video_format_id: Some("18".to_string()),
            audio_format_id: None,
        }
        .has_any());
        // Форма, которая может приехать из UI (`{}`) и которую нельзя
        // принимать: пункт без единого потока скачать нечем (Ф-3).
        assert!(!QualityStreams {
            video_format_id: None,
            audio_format_id: None,
        }
        .has_any());
    }

    #[test]
    fn empty_details_are_recognised_as_having_nothing_to_show() {
        assert!(ProbeErrorDetails {
            stderr_tail: None,
            exit_code: None,
        }
        .is_empty());
        assert!(!ProbeErrorDetails {
            stderr_tail: None,
            exit_code: Some(1),
        }
        .is_empty());
        assert!(!ProbeErrorDetails {
            stderr_tail: Some("ERROR".to_string()),
            exit_code: None,
        }
        .is_empty());
    }

    #[test]
    fn serializes_quality_kind_wire_values() {
        for (kind, expected) in [
            (QualityKind::Standard, "standard"),
            (QualityKind::MaxAvailable, "maxAvailable"),
            (QualityKind::AudioOnly, "audioOnly"),
        ] {
            assert_eq!(
                serde_json::to_value(kind).expect("serialization must not fail"),
                json!(expected)
            );
        }
    }
}
