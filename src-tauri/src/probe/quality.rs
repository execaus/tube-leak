//! Схлопывание форматов yt-dlp в лестницу качеств (Ф-3) и оценка размера
//! пункта (Ф-4) — TL-30.
//!
//! Функция [`build_quality_ladder`] чистая: на входе — уже распарсенные
//! метаданные ролика (`yt-dlp -J`), на выходе — готовый к отправке
//! фронтенду список [`QualityItem`]. Процессов здесь не запускается
//! (это TL-32), классификация ошибок тоже не здесь (TL-31).
//!
//! # Что именно решает этот модуль
//!
//! yt-dlp отдаёт десятки форматов на один ролик: одно и то же разрешение
//! приходит в трёх-четырёх кодеках, плюс отдельные аудиодорожки, плюс
//! HLS-варианты тех же дорожек, плюс раскадровки превью. Пользователю
//! показываются четыре ступени (2160p/1440p/1080p/720p) и «только аудио»
//! (Р-1), кодеки и контейнеры скрыты, у каждой строки — оценка размера.
//! Значит кто-то обязан выбрать по одному потоку на строку, и выбор
//! обязан быть один и тот же и для показанной оценки размера, и для
//! будущего скачивания в E3 (Ф-3: «результат разбора вместе с выбранным
//! пунктом однозначно определяет, что скачивать, без повторного разбора»).
//!
//! # Инварианты, которые держит только код
//!
//! - **`standard` ⇒ высота ровно из четырёх ступеней.** Контракт
//!   ([`QualityKind`]) допускает `{kind: standard, heightPx: 360}`, а Р-1
//!   это запрещает: всё ниже 720p не показывается вовсе, а если 720p —
//!   выше максимума ролика, вместо всей лестницы остаётся одна строка
//!   [`QualityKind::MaxAvailable`]. Ступени перебираются по
//!   [`LADDER_STEPS`], поэтому другой высоты у `standard` появиться неоткуда;
//!   закреплено тестом `standard_items_only_ever_carry_ladder_heights`.
//! - **Порядок фиксирован** — от большего разрешения к меньшему, «только
//!   аудио» последней. Фронтенд список не пересортировывает.
//! - **Один аудиопоток на все строки.** Аудиодорожка не зависит от
//!   выбранного разрешения (дизайн E2), поэтому выбирается один раз.

// Вызывающего у лестницы пока нет: её зовёт оркестрация разбора (TL-32),
// а до неё код модуля живёт только под тестами. Тот же приём и по той же
// причине, что у типов контракта в `crate::types`, — снимается задачей,
// которая начнёт лестницу использовать.
#![allow(dead_code)]

use serde_json::Value;
use std::cmp::Ordering;

use crate::types::{QualityItem, QualityKind, QualitySize, QualityStreams};

/// Ступени лестницы в порядке показа — сверху вниз (Ф-3, Р-1).
///
/// Массив — единственный источник и состава `standard`-строк, и их
/// порядка в списке. Появление 4320p будет правкой ровно этой строки.
const LADDER_STEPS: [u32; 4] = [2160, 1440, 1080, 720];

/// Собрать лестницу качеств из метаданных ролика (`yt-dlp -J`).
///
/// Принимается весь объект метаданных, а не массив `formats`: TL-32
/// разбирает JSON один раз и передаёт его сюда как есть. Отсутствие или
/// непригодность `formats` — не паника и не ошибка домена, а пустой
/// список: решать, является ли ролик без единого пункта отказом разбора,
/// будет оркестрация (TL-32), которая одна знает контекст запуска.
pub fn build_quality_ladder(metadata: &Value) -> Vec<QualityItem> {
    let formats = metadata
        .get("formats")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);

    let streams: Vec<Stream<'_>> = formats.iter().filter_map(Stream::from_format).collect();

    // Аудио выбирается один раз на весь ролик: и для «только аудио», и для
    // каждой видеостроки (дизайн E2 — дорожка не зависит от разрешения).
    let audio = best(streams.iter().filter(|stream| stream.height.is_none()));

    let mut items = Vec::with_capacity(LADDER_STEPS.len() + 1);

    let standard: Vec<(u32, &Stream<'_>)> = LADDER_STEPS
        .iter()
        .filter_map(|&height| {
            best(
                streams
                    .iter()
                    .filter(|stream| stream.height == Some(height)),
            )
            .map(|stream| (height, stream))
        })
        .collect();

    if standard.is_empty() {
        // Ни одной ступени лестницы у ролика нет — либо максимум ниже 720p
        // (обычный случай Р-1: старая или намеренно низкокачественная
        // запись), либо высоты вообще не совпали со ступенями. Обе ситуации
        // дают ровно одну видеостроку «максимальное доступное (NNNp)»
        // вместо лестницы, а не четыре выключенных пункта.
        //
        // Максимум берётся по высоте, и лучший поток ищется уже внутри
        // неё: «лучший из всех» мог бы оказаться 360p-потоком с битрейтом
        // выше, чем у 480p, и строка «максимальное доступное (480p)»
        // указывала бы на 360p.
        let max_height = streams.iter().filter_map(|stream| stream.height).max();

        if let Some(height) = max_height {
            if let Some(stream) = best(streams.iter().filter(|s| s.height == Some(height))) {
                items.push(video_item(QualityKind::MaxAvailable, height, stream, audio));
            }
        }
    } else {
        for (height, stream) in standard {
            items.push(video_item(QualityKind::Standard, height, stream, audio));
        }
    }

    // «Только аудио» — всегда последней строкой (Р-1, дизайн E2).
    if let Some(audio) = audio {
        items.push(QualityItem {
            kind: QualityKind::AudioOnly,
            height_px: None,
            size: size_of(audio.size),
            streams: QualityStreams {
                video_format_id: None,
                audio_format_id: Some(audio.format_id.to_owned()),
            },
        });
    }

    items
}

/// Собрать видеостроку: выбранный видеопоток плюс общая аудиодорожка.
///
/// Аудио не приклеивается, если лучший видеопоток ступени уже со звуком
/// (прогрессивный формат): скачивать вторую дорожку в этом случае незачем,
/// и контракт [`QualityStreams`] прямо описывает такой пункт как «только
/// `videoFormatId`».
fn video_item(
    kind: QualityKind,
    height: u32,
    video: &Stream<'_>,
    audio: Option<&Stream<'_>>,
) -> QualityItem {
    let audio = if video.carries_audio { None } else { audio };

    QualityItem {
        kind,
        height_px: Some(height),
        // Сумма размеров агрегированных потоков (Ф-4). Если размер известен
        // только у одного из двух — суммируется он один: оценка «видео без
        // звуковой дорожки» промахивается на проценты, а «неизвестно» при
        // наличии данных было бы потерей информации на ровном месте.
        size: size_of(sum_sizes(video.size, audio.and_then(|audio| audio.size))),
        streams: QualityStreams {
            video_format_id: Some(video.format_id.to_owned()),
            audio_format_id: audio.map(|audio| audio.format_id.to_owned()),
        },
    }
}

/// Сумма известных размеров; `None`, только если не известен ни один.
fn sum_sizes(video: Option<u64>, audio: Option<u64>) -> Option<u64> {
    match (video, audio) {
        (None, None) => None,
        (video, audio) => Some(video.unwrap_or(0).saturating_add(audio.unwrap_or(0))),
    }
}

/// Оценка размера в терминах контракта.
///
/// «Нет данных» и «ноль байт» — разные вещи, и первое не должно приезжать
/// на экран как второе (Ф-4). Обычный `From` для этого не заводится:
/// контрактный тип не должен обрастать поверхностью из-за задачи, которая
/// его наполняет.
fn size_of(bytes: Option<u64>) -> QualitySize {
    bytes.map_or(QualitySize::Unknown, |bytes| QualitySize::Known { bytes })
}

/// Лучший поток из перечисленных или `None`, если перечислять нечего.
///
/// При полном равенстве всех признаков выигрывает тот, кто идёт в выводе
/// yt-dlp раньше (замена — только по строгому «лучше»): порядок форматов
/// у yt-dlp стабилен, и разбор одного и того же ролика обязан давать один
/// и тот же результат.
fn best<'a, 'f>(streams: impl Iterator<Item = &'a Stream<'f>>) -> Option<&'a Stream<'f>>
where
    'f: 'a,
{
    streams.fold(None, |best, candidate| match best {
        Some(best) if candidate.rank.compare(&best.rank) != Ordering::Greater => Some(best),
        _ => Some(candidate),
    })
}

/// Один поток yt-dlp, приведённый к тому, что нужно лестнице.
struct Stream<'a> {
    format_id: &'a str,
    /// Высота кадра; `None` — поток без видео, то есть кандидат в аудио.
    height: Option<u32>,
    /// Видеопоток уже содержит звук (прогрессивный формат).
    carries_audio: bool,
    size: Option<u64>,
    rank: Rank,
}

impl<'a> Stream<'a> {
    /// Разобрать элемент `formats`, отбросив всё, что не является ни
    /// видео-, ни аудиопотоком.
    ///
    /// Отбрасываются раскадровки превью (`sb0`…`sb3`: `vcodec` и `acodec`
    /// оба `none`, но у них есть `height` — без этой проверки раскадровка
    /// 180 px попала бы в лестницу как «максимальное доступное») и любые
    /// будущие служебные форматы того же вида.
    fn from_format(format: &'a Value) -> Option<Self> {
        let format_id = format.get("format_id").and_then(Value::as_str)?;
        let vcodec = format.get("vcodec").and_then(Value::as_str);
        let acodec = format.get("acodec").and_then(Value::as_str);

        let has_video = matches!(vcodec, Some(codec) if codec != "none");
        // `acodec` может отсутствовать вовсе — так yt-dlp отдаёт HLS-аудио
        // (форматы 233/234: `vcodec: "none"`, `resolution: "audio only"`,
        // кодек не объявлен). Отсутствие кодека — «неизвестно», а не «нет»:
        // отбрасывается только явное `none`.
        let has_audio = acodec != Some("none");

        let height = read_u32(format.get("height"));

        // Видео без высоты выбрать в ступень нельзя, а как аудио оно не
        // годится; у аудиопотоков высоты нет по определению.
        let height = match (has_video, height) {
            (true, Some(height)) => Some(height),
            (true, None) => return None,
            (false, _) if has_audio => None,
            (false, _) => return None,
        };

        Some(Self {
            format_id,
            height,
            carries_audio: has_video && matches!(acodec, Some(codec) if codec != "none"),
            size: read_size(format),
            rank: Rank::from_format(format),
        })
    }
}

/// Признаки, по которым потоки сравниваются между собой, в порядке
/// убывания важности.
///
/// Каждое поле — отдельное правило выбора; порядок полей и есть
/// приоритет правил (см. [`Rank::compare`]).
struct Rank {
    /// `language_preference` yt-dlp: 10 у оригинальной дорожки, −1 у
    /// автодубляжа и у всего, где язык не при чём (видеопотоки).
    language_preference: i64,
    /// Поток отдаётся напрямую по http(s), а не через манифест
    /// (HLS/DASH/mhtml).
    direct: bool,
    /// `tbr`, при отсутствии — `vbr`/`abr`; `None` — битрейт не объявлен.
    bitrate: Option<f64>,
    /// yt-dlp сообщает оценку размера этого потока.
    has_size: bool,
    /// `quality` yt-dlp — его собственная оценка «лучше/хуже» внутри
    /// разрешения.
    quality: f64,
    /// `source_preference` yt-dlp — его же предпочтение источника.
    source_preference: i64,
}

/// Чем становится необъявленный признак ранга.
///
/// Ровно то же значение, что подставляет сам yt-dlp в своей сортировке
/// форматов (`FormatSorter`, поля `lang`, `quality`, `source`), и это не
/// косметика: у HLS-вариантов `language_preference` не объявлен вовсе, а у
/// прямых потоков он равен −1. Любой «нейтральный» ноль на месте
/// отсутствующего значения поставил бы HLS выше прямого потока ещё до
/// сравнения по протоколу и битрейту — ровно тот дефект, который поймала
/// первая же прогонка на фикстуре `4k-full-ladder.json` (выбирался `628`
/// вместо `315`).
const UNDECLARED: i64 = -1;

/// [`UNDECLARED`] для дробного `quality`.
const UNDECLARED_F64: f64 = -1.0;

impl Rank {
    fn from_format(format: &Value) -> Self {
        let bitrate = ["tbr", "vbr", "abr"]
            .iter()
            .find_map(|key| read_positive_f64(format.get(key)));

        Self {
            language_preference: read_i64(format.get("language_preference")).unwrap_or(UNDECLARED),
            direct: matches!(
                format.get("protocol").and_then(Value::as_str),
                // Отсутствие поля трактуется как «прямая раздача»: так
                // отдают потоки экстракторы попроще, и наказывать их
                // за необъявленный протокол не за что.
                None | Some("https" | "http")
            ),
            bitrate,
            has_size: read_size(format).is_some(),
            quality: read_f64(format.get("quality")).unwrap_or(UNDECLARED_F64),
            source_preference: read_i64(format.get("source_preference")).unwrap_or(UNDECLARED),
        }
    }

    /// Кто из двух потоков лучше. Правила по убыванию важности:
    ///
    /// 1. **Язык дорожки.** Оригинал важнее автодубляжа: у ролика с
    ///    дубляжами (реальная фикстура `multi-language-audio.json`)
    ///    самый жирный аудиопоток — малаялам (132 kbps) против
    ///    английского оригинала (129 kbps), и выбор «просто по битрейту»
    ///    молча подменил бы звук. Для видеопотоков признак одинаков и
    ///    ни на что не влияет.
    /// 2. **Прямая раздача важнее манифеста.** Это уточнение правила
    ///    дизайна «наибольший битрейт», без которого правило не работает
    ///    на живых данных: у HLS-вариантов (`m3u8_native`) `tbr` — это
    ///    объявленная в манифесте пиковая полоса, а у прямых потоков —
    ///    средний битрейт, посчитанный из размера. Сравнивать их между
    ///    собой некорректно, HLS всегда «выигрывает» и при этом никогда
    ///    не сообщает размер — лестница целиком стала бы «размер
    ///    неизвестен» вопреки К-1. Отбором это не сделано: если у ступени
    ///    есть только HLS-варианты, строка всё равно нужна (Р-1 — строка
    ///    существует, если качество доступно), просто с неизвестным
    ///    размером.
    /// 3. **Битрейт** — правило дизайна: на глаз он соответствует
    ///    «лучше» вернее, чем кодек или контейнер (которые в выборе
    ///    не участвуют вовсе и наружу не выходят).
    /// 4. **Наличие оценки размера** — тай-брейк дизайна: пункт не должен
    ///    становиться «размер неизвестен» из-за порядка перебора при
    ///    прочих равных.
    /// 5. **`quality` yt-dlp** — разводит потоки, равные по всему
    ///    вышеперечисленному. Реальный случай: `140` и `140-drc`
    ///    (сжатая по громкости копия) с точностью до бита одинаковы по
    ///    битрейту и размеру, и yt-dlp сам ставит DRC ниже (3.0 против
    ///    2.5).
    /// 6. **`source_preference` yt-dlp** — то же для случая, когда и
    ///    `quality` совпал (HLS-аудио `233`/`234`: качество −1 у обоих,
    ///    предпочтение 0 и 1).
    fn compare(&self, other: &Self) -> Ordering {
        self.language_preference
            .cmp(&other.language_preference)
            .then(self.direct.cmp(&other.direct))
            .then(bitrate_key(self.bitrate).total_cmp(&bitrate_key(other.bitrate)))
            .then(self.has_size.cmp(&other.has_size))
            .then(self.quality.total_cmp(&other.quality))
            .then(self.source_preference.cmp(&other.source_preference))
    }
}

/// Необъявленный битрейт хуже любого объявленного, но сравним с ним:
/// битрейты строго положительны, так что «минус бесконечность» здесь —
/// то же самое, что подставляемая yt-dlp `-1`, только без риска
/// столкнуться с отрицательным значением из данных.
fn bitrate_key(value: Option<f64>) -> f64 {
    value.unwrap_or(f64::NEG_INFINITY)
}

/// Оценка размера потока: точная, при отсутствии — приблизительная (Ф-4).
fn read_size(format: &Value) -> Option<u64> {
    ["filesize", "filesize_approx"]
        .iter()
        .find_map(|key| read_u64(format.get(key)))
}

fn read_u64(value: Option<&Value>) -> Option<u64> {
    let number = value?.as_f64()?;
    // Размер приезжает целым числом, но yt-dlp местами считает его
    // умножением и мог бы отдать дробное: неотрицательное конечное число
    // округляется, всё остальное — «данных нет».
    (number.is_finite() && number >= 0.0).then(|| number.round() as u64)
}

fn read_u32(value: Option<&Value>) -> Option<u32> {
    u32::try_from(read_u64(value)?).ok()
}

fn read_i64(value: Option<&Value>) -> Option<i64> {
    value?.as_i64()
}

fn read_f64(value: Option<&Value>) -> Option<f64> {
    let number = value?.as_f64()?;
    number.is_finite().then_some(number)
}

/// Битрейт: нулевой считается необъявленным.
///
/// yt-dlp заполняет `vbr: 0` у аудиопотоков и `abr: 0` у видеопотоков —
/// это «неприменимо», а не «нулевой битрейт».
fn read_positive_f64(value: Option<&Value>) -> Option<f64> {
    read_f64(value).filter(|number| *number > 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::path::{Path, PathBuf};

    /// Фикстуры — настоящий вывод `yt-dlp -J` (2026.08.19), см. README
    /// рядом с ними: сетевых вызовов в тестах нет (Ф-7), а числа ниже —
    /// не выдумка, а то, что YouTube отдал на конкретный ролик.
    const FIXTURES: [&str; 5] = [
        "4k-full-ladder.json",
        "max-1080p.json",
        "max-240p.json",
        "multi-language-audio.json",
        "sizes-unknown.json",
    ];

    fn fixture_path(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/ytdlp-probe")
            .join(name)
    }

    fn fixture(name: &str) -> Value {
        let path = fixture_path(name);
        let raw = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("фикстура {} не читается: {err}", path.display()));
        serde_json::from_str(&raw)
            .unwrap_or_else(|err| panic!("фикстура {} — не JSON: {err}", path.display()))
    }

    fn ladder(name: &str) -> Vec<QualityItem> {
        build_quality_ladder(&fixture(name))
    }

    /// Компактная форма пункта для сравнения: вид, высота, потоки.
    fn shape(item: &QualityItem) -> (QualityKind, Option<u32>, Option<&str>, Option<&str>) {
        (
            item.kind,
            item.height_px,
            item.streams.video_format_id.as_deref(),
            item.streams.audio_format_id.as_deref(),
        )
    }

    fn size_bytes(item: &QualityItem) -> Option<u64> {
        match item.size {
            QualitySize::Known { bytes } => Some(bytes),
            QualitySize::Unknown => None,
        }
    }

    #[test]
    fn full_ladder_collapses_to_four_steps_and_one_audio_row() {
        let items = ladder("4k-full-ladder.json");

        // Big Buck Bunny 4K: у ролика восемь высот (144…2160) и по три-пять
        // форматов на каждую — на экран выходят четыре ступени и аудио.
        assert_eq!(
            items.iter().map(shape).collect::<Vec<_>>(),
            vec![
                (QualityKind::Standard, Some(2160), Some("315"), Some("140")),
                (QualityKind::Standard, Some(1440), Some("308"), Some("140")),
                (QualityKind::Standard, Some(1080), Some("299"), Some("140")),
                (QualityKind::Standard, Some(720), Some("298"), Some("140")),
                (QualityKind::AudioOnly, None, None, Some("140")),
            ],
            "лестница строится сверху вниз, «только аудио» — последней"
        );
    }

    /// Инвариант Р-1, который контракт выразить не может: `standard`
    /// существует только для четырёх ступеней.
    ///
    /// Тип допускает `{kind: standard, heightPx: 360}` — запрет держится
    /// исключительно этим кодом и этим тестом (решение ревью TL-27).
    #[test]
    fn standard_items_only_ever_carry_ladder_heights() {
        for name in FIXTURES {
            let items = ladder(name);

            for item in &items {
                match item.kind {
                    QualityKind::Standard => {
                        let height = item
                            .height_px
                            .unwrap_or_else(|| panic!("{name}: у ступени обязана быть высота"));
                        assert!(
                            LADDER_STEPS.contains(&height),
                            "{name}: ступень {height}p вне лестницы {LADDER_STEPS:?} — Р-1 \
                             запрещает показывать её как обычную строку"
                        );
                    }
                    QualityKind::MaxAvailable => assert!(
                        item.height_px.is_some(),
                        "{name}: строке «максимальное доступное» нужна высота для подписи"
                    ),
                    QualityKind::AudioOnly => assert_eq!(
                        item.height_px, None,
                        "{name}: у «только аудио» высоты кадра нет"
                    ),
                }
            }

            let standard = items
                .iter()
                .filter(|item| item.kind == QualityKind::Standard)
                .count();
            let max_available = items
                .iter()
                .filter(|item| item.kind == QualityKind::MaxAvailable)
                .count();

            assert!(
                max_available <= 1,
                "{name}: «максимальное доступное» — ровно одна строка вместо лестницы"
            );
            assert!(
                standard == 0 || max_available == 0,
                "{name}: строка «максимальное доступное» заменяет лестницу, а не дополняет её"
            );
        }
    }

    #[test]
    fn video_below_720p_collapses_to_a_single_max_available_row() {
        // «Me at the zoo» — первый ролик YouTube, максимум 240p: ступеней
        // лестницы у него нет ни одной, и по Р-1 остаётся одна строка.
        let items = ladder("max-240p.json");

        assert_eq!(
            items.iter().map(shape).collect::<Vec<_>>(),
            vec![
                (
                    QualityKind::MaxAvailable,
                    Some(240),
                    Some("133"),
                    Some("140")
                ),
                (QualityKind::AudioOnly, None, None, Some("140")),
            ],
            "строк 144p и «выключенных» ступеней в списке быть не должно"
        );
    }

    #[test]
    fn ladder_stops_at_the_highest_resolution_the_video_has() {
        // Gangnam Style: максимум 1080p — строк 2160p и 1440p нет вовсе
        // (Р-1: не выключенные, а отсутствующие), 608p и ниже отброшены.
        let items = ladder("max-1080p.json");

        assert_eq!(
            items
                .iter()
                .map(|item| (item.kind, item.height_px))
                .collect::<Vec<_>>(),
            vec![
                (QualityKind::Standard, Some(1080)),
                (QualityKind::Standard, Some(720)),
                (QualityKind::AudioOnly, None),
            ]
        );
    }

    #[test]
    fn one_audio_stream_is_shared_by_every_row() {
        for name in FIXTURES {
            let items = ladder(name);
            let Some(audio) = items
                .last()
                .filter(|item| item.kind == QualityKind::AudioOnly)
            else {
                continue;
            };
            let expected = audio.streams.audio_format_id.as_deref();

            for item in &items {
                assert_eq!(
                    item.streams.audio_format_id.as_deref(),
                    expected,
                    "{name}: аудиодорожка не зависит от выбранного разрешения (дизайн E2)"
                );
            }
        }
    }

    #[test]
    fn prefers_the_original_audio_track_over_a_louder_dub() {
        // Ролик с 45 дорожками автодубляжа. Самая жирная — малаялам
        // (132.243 kbps), английский оригинал скромнее (129.476), так что
        // выбор «просто по наибольшему битрейту» молча подменил бы звук.
        let items = ladder("multi-language-audio.json");
        let audio = items.last().expect("у ролика есть аудио");

        assert_eq!(audio.kind, QualityKind::AudioOnly);
        assert_eq!(
            audio.streams.audio_format_id.as_deref(),
            Some("140-21"),
            "у оригинальной дорожки language_preference = 10, у дубляжей −1"
        );
        assert_eq!(size_bytes(audio), Some(19_880_859));
    }

    #[test]
    fn prefers_a_direct_stream_over_a_louder_manifest_variant() {
        // У 240p «Me at the zoo» самый высокий tbr — у HLS-варианта 230
        // (294.965 против 182.995 у прямого 133), но у HLS это объявленная
        // в манифесте пиковая полоса, а не средний битрейт, и размера он
        // не сообщает вовсе.
        let items = ladder("max-240p.json");
        let video = &items[0];

        assert_eq!(video.streams.video_format_id.as_deref(), Some("133"));
        assert_eq!(
            size_bytes(video),
            Some(433_081 + 309_288),
            "оценка строки — сумма размеров видеопотока и аудиодорожки (Ф-4)"
        );
    }

    #[test]
    fn breaks_a_complete_tie_the_way_yt_dlp_itself_does() {
        // 140 и 140-drc совпадают по битрейту (129.481) и размеру
        // (10 271 496 Б) до последнего знака: различает их только
        // собственная оценка yt-dlp (quality 3.0 против 2.5 у DRC-копии).
        let items = ladder("4k-full-ladder.json");
        let audio = items.last().expect("у ролика есть аудио");

        assert_eq!(audio.streams.audio_format_id.as_deref(), Some("140"));
        assert_eq!(size_bytes(audio), Some(10_271_496));
    }

    #[test]
    fn size_of_a_row_is_the_sum_of_its_streams() {
        let items = ladder("4k-full-ladder.json");

        assert_eq!(
            items.iter().map(size_bytes).collect::<Vec<_>>(),
            vec![
                Some(1_362_269_481 + 10_271_496),
                Some(473_363_704 + 10_271_496),
                Some(257_619_653 + 10_271_496),
                Some(150_524_867 + 10_271_496),
                Some(10_271_496),
            ]
        );

        // К-1 требует правдоподобия: оценки убывают сверху вниз, а «только
        // аудио» меньше любой видеостроки.
        let sizes: Vec<u64> = items.iter().filter_map(size_bytes).collect();
        assert!(
            sizes.windows(2).all(|pair| pair[0] > pair[1]),
            "оценки размера обязаны убывать: {sizes:?}"
        );
    }

    #[test]
    fn a_row_survives_when_no_stream_of_it_reports_a_size() {
        // Тот же ролик, но в выводе остались только потоки через манифест:
        // yt-dlp не сообщает размер ни для одного из них. Пункт обязан
        // остаться в списке с явным «неизвестно» (Ф-4), а не исчезнуть и
        // не приехать нулём.
        let items = ladder("sizes-unknown.json");

        assert_eq!(
            items.iter().map(shape).collect::<Vec<_>>(),
            vec![
                (
                    QualityKind::MaxAvailable,
                    Some(240),
                    Some("230"),
                    Some("234")
                ),
                (QualityKind::AudioOnly, None, None, Some("234")),
            ]
        );
        for item in &items {
            assert_eq!(item.size, QualitySize::Unknown);
        }
    }

    #[test]
    fn a_partially_known_size_is_still_an_estimate() {
        // Размер известен только у видеопотока: аудиодорожка весит
        // проценты от строки, и «неизвестно» при наличии данных было бы
        // потерей информации на ровном месте (Ф-4 требует «неизвестно»
        // только когда не известно ничего).
        let metadata = json!({
            "formats": [
                {"format_id": "v", "vcodec": "avc1", "acodec": "none", "height": 1080,
                 "tbr": 3000.0, "filesize": 100, "protocol": "https"},
                {"format_id": "a", "vcodec": "none", "acodec": "mp4a.40.2",
                 "tbr": 129.0, "protocol": "https"},
            ]
        });

        let items = build_quality_ladder(&metadata);

        assert_eq!(size_bytes(&items[0]), Some(100));
        assert_eq!(items[1].size, QualitySize::Unknown);
    }

    #[test]
    fn prefers_a_sized_stream_when_the_bitrate_ties() {
        // Тай-брейк дизайна: пункт не должен становиться «размер
        // неизвестен» только из-за порядка перебора. Поток без размера
        // идёт первым — и всё равно проигрывает.
        let metadata = json!({
            "formats": [
                {"format_id": "no-size", "vcodec": "avc1", "acodec": "none", "height": 720,
                 "tbr": 1500.0, "protocol": "https"},
                {"format_id": "sized", "vcodec": "vp9", "acodec": "none", "height": 720,
                 "tbr": 1500.0, "filesize": 42, "protocol": "https"},
            ]
        });

        let items = build_quality_ladder(&metadata);

        assert_eq!(
            shape(&items[0]),
            (QualityKind::Standard, Some(720), Some("sized"), None)
        );
        assert_eq!(size_bytes(&items[0]), Some(42));
    }

    #[test]
    fn a_progressive_stream_needs_no_separate_audio() {
        // Смёржанный формат (в терминах YouTube — 18, «360p mp4 со
        // звуком») несёт звук сам, и второй поток ему не нужен: контракт
        // описывает такой пункт как «только videoFormatId».
        //
        // Синтетический вход, и это отмечено сознательно: ни в одной из
        // пяти живых фикстур прогрессивных форматов не оказалось вовсе —
        // YouTube отдаёт раздельные DASH-потоки, — поэтому ветка
        // проверяется минимальным собранным вручную JSON, а не фикстурой.
        let metadata = json!({
            "formats": [
                {"format_id": "18", "vcodec": "avc1.42001E", "acodec": "mp4a.40.2",
                 "height": 360, "tbr": 700.0, "filesize": 5_000_000, "protocol": "https"},
            ]
        });

        let items = build_quality_ladder(&metadata);

        assert_eq!(
            items.iter().map(shape).collect::<Vec<_>>(),
            vec![(QualityKind::MaxAvailable, Some(360), Some("18"), None)],
            "аудиопотока у ролика нет — строки «только аудио» тоже нет"
        );
        assert_eq!(size_bytes(&items[0]), Some(5_000_000));
    }

    #[test]
    fn storyboards_never_become_a_quality_row() {
        // Раскадровки превью (`sb0`…`sb3`) приезжают в том же массиве
        // `formats` и у них есть `height` (до 180 px) — без явного отсева
        // ролик без видеопотоков получил бы строку «максимальное
        // доступное (180p)», которую нечем скачать.
        for name in FIXTURES {
            for item in ladder(name) {
                let referenced = [
                    item.streams.video_format_id.clone(),
                    item.streams.audio_format_id.clone(),
                ];
                for format_id in referenced.into_iter().flatten() {
                    assert!(
                        !format_id.starts_with("sb"),
                        "{name}: раскадровка {format_id} попала в лестницу"
                    );
                }
            }
        }

        let only_storyboards = json!({
            "formats": [
                {"format_id": "sb0", "vcodec": "none", "acodec": "none", "height": 180,
                 "ext": "mhtml", "protocol": "mhtml"},
            ]
        });
        assert!(build_quality_ladder(&only_storyboards).is_empty());
    }

    #[test]
    fn every_row_points_at_something_downloadable() {
        // Инвариант контракта: пункт без единого потока в список не
        // попадает (`QualityStreams::has_any`).
        for name in FIXTURES {
            for item in ladder(name) {
                assert!(
                    item.streams.has_any(),
                    "{name}: пункт {:?} без потоков",
                    item.kind
                );
                if item.kind == QualityKind::AudioOnly {
                    assert_eq!(item.streams.video_format_id, None);
                    assert!(item.streams.audio_format_id.is_some());
                } else {
                    assert!(item.streams.video_format_id.is_some());
                }
            }
        }
    }

    #[test]
    fn metadata_without_usable_formats_yields_an_empty_ladder() {
        // Пустая лестница — не паника и не ошибка: чем становится ролик
        // без единого пункта, решает оркестрация (TL-32).
        for metadata in [
            json!({}),
            json!({"formats": []}),
            json!({"formats": "не массив"}),
            json!({"formats": [{"vcodec": "avc1", "height": 720}]}),
            json!({"formats": [{"format_id": "no-height", "vcodec": "avc1", "acodec": "none"}]}),
        ] {
            assert!(
                build_quality_ladder(&metadata).is_empty(),
                "{metadata} должен давать пустую лестницу"
            );
        }
    }

    #[test]
    fn a_height_that_is_not_a_ladder_step_never_makes_its_own_row() {
        // Реальный случай: у «Despacito» есть высота 608 px. Она ниже
        // 720p, значит по Р-1 не показывается вовсе, а лестница строится
        // из тех ступеней, что есть. Здесь то же самое на синтетике:
        // 900p не ступень и своей строки не получает.
        let metadata = json!({
            "formats": [
                {"format_id": "odd", "vcodec": "avc1", "acodec": "none", "height": 900,
                 "tbr": 4000.0, "filesize": 900, "protocol": "https"},
                {"format_id": "step", "vcodec": "avc1", "acodec": "none", "height": 720,
                 "tbr": 1500.0, "filesize": 720, "protocol": "https"},
            ]
        });

        let items = build_quality_ladder(&metadata);

        assert_eq!(
            items.iter().map(shape).collect::<Vec<_>>(),
            vec![(QualityKind::Standard, Some(720), Some("step"), None)]
        );
    }

    #[test]
    fn fixtures_are_real_yt_dlp_output() {
        // Сторож происхождения: фикстуры — вывод настоящего yt-dlp, а не
        // собранный руками JSON (Ф-7). Если кто-то однажды «поправит»
        // фикстуру, потеряв эти поля, тест скажет об этом раньше ревью.
        for name in FIXTURES {
            let metadata = fixture(name);

            assert_eq!(
                metadata.get("extractor").and_then(Value::as_str),
                Some("youtube"),
                "{name}: фикстура должна быть выводом yt-dlp по ролику YouTube"
            );
            assert!(
                metadata.get("_version").is_some(),
                "{name}: в фикстуре нет блока версии yt-dlp"
            );
            assert!(
                metadata
                    .get("formats")
                    .and_then(Value::as_array)
                    .is_some_and(|formats| !formats.is_empty()),
                "{name}: в фикстуре нет форматов"
            );
        }
    }
}
