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
//! - **`standard` ⇒ ступень ровно из четырёх.** Контракт
//!   ([`QualityKind`]) допускает `{kind: standard, heightPx: 360}`, а Р-1
//!   это запрещает: всё ниже 720p не показывается вовсе, а если 720p —
//!   выше максимума ролика, вместо всей лестницы остаётся одна строка
//!   [`QualityKind::MaxAvailable`]. Ступени перебираются по
//!   [`LADDER_STEPS`], поэтому другого числа у `standard` появиться
//!   неоткуда; закреплено тестом
//!   `standard_items_only_ever_carry_ladder_heights`.
//! - **Ступень — это метка качества, а не высота кадра** (Р-4, см.
//!   [`step_of`]).
//! - **Порядок фиксирован** — от большего разрешения к меньшему, «только
//!   аудио» последней. Фронтенд список не пересортировывает.
//! - **Один аудиопоток на все строки.** Аудиодорожка не зависит от
//!   выбранного разрешения (дизайн E2), поэтому выбирается один раз.
//!
//! # Что попадает в `heightPx`
//!
//! Ступень (то есть метку качества), а не физическую высоту кадра.
//!
//! Решение осознанное, а не побочный эффект Р-4. Поле контракта прямо
//! описано как «идёт в подпись строки, а не в решение о её виде»: подпись
//! обязана показывать то же число, что YouTube показывает в плеере, иначе
//! у вертикального ролика (реальная фикстура `vertical-video.json`)
//! пользователь увидит «1920p» там, где ждёт «1080p». Альтернатива —
//! класть физическую высоту и заставлять фронтенд выводить из неё метку —
//! это ровно то дублирование логики агрегации на двух сторонах границы,
//! которое контракт запрещает своим же doc-комментарием к
//! [`QualityKind`].
//!
//! Цена решения: у не-16:9 роликов имя поля перестаёт быть буквальным —
//! `heightPx` несёт число метки, а не пиксели кадра. Для подавляющего
//! большинства роликов (16:9, горизонтальные) оба числа совпадают.
//! Переименовать поле нельзя: контракт смержен (TL-27) и по нему уже
//! написано TS-зеркало — вместо переименования уточнён doc-комментарий
//! самого поля [`QualityItem::height_px`].

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
    let audio = best_audio(streams.iter().filter(|stream| stream.step.is_none()));

    let mut items = Vec::with_capacity(LADDER_STEPS.len() + 1);

    let standard: Vec<(u32, &Stream<'_>)> = LADDER_STEPS
        .iter()
        .filter_map(|&step| {
            best_video(streams.iter().filter(|stream| stream.step == Some(step)))
                .map(|stream| (step, stream))
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
        let max_step = streams.iter().filter_map(|stream| stream.step).max();

        if let Some(step) = max_step {
            if let Some(stream) = best_video(streams.iter().filter(|s| s.step == Some(step))) {
                items.push(video_item(QualityKind::MaxAvailable, step, stream, audio));
            }
        }
    } else {
        for (step, stream) in standard {
            items.push(video_item(QualityKind::Standard, step, stream, audio));
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
    step: u32,
    video: &Stream<'_>,
    audio: Option<&Stream<'_>>,
) -> QualityItem {
    let audio = if video.carries_audio { None } else { audio };

    QualityItem {
        kind,
        // В `heightPx` кладётся ступень (Р-4), а не физическая высота
        // кадра — см. раздел «Что попадает в `heightPx`» в шапке модуля.
        height_px: Some(step),
        // Сумма размеров агрегированных потоков (Ф-4).
        size: size_of(video_row_size(
            video.size,
            audio.and_then(|audio| audio.size),
        )),
        streams: QualityStreams {
            video_format_id: Some(video.format_id.to_owned()),
            audio_format_id: audio.map(|audio| audio.format_id.to_owned()),
        },
    }
}

/// Оценка размера видеостроки: размер видеопотока плюс размер дорожки,
/// если он известен.
///
/// Два несимметричных случая, и несимметричны они по существу:
///
/// - **размер видео известен, размер дорожки нет** — оценка выдаётся:
///   дорожка весит проценты от строки, и «размер неизвестен» вместо
///   честных «≈ 1,36 ГБ» был бы потерей информации на ровном месте;
/// - **размер видео неизвестен** — оценки нет, чем бы ни был известен
///   размер дорожки. Ступень из одних HLS-вариантов (размера не сообщает
///   ни один) плюс прямое аудио дала бы строку «2160p ≈ 10 МБ»: промах не
///   на проценты, а на два порядка. Это уже не оценка, а дезинформация,
///   и «размер неизвестен» здесь — единственный честный ответ (Ф-4
///   прямо разрешает пункт без оценки, он остаётся выбираемым).
fn video_row_size(video: Option<u64>, audio: Option<u64>) -> Option<u64> {
    Some(video?.saturating_add(audio.unwrap_or(0)))
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

/// Лучший видеопоток ступени.
///
/// Язык в сравнении не участвует — см. [`Rank::compare_as_audio`].
fn best_video<'a, 'f>(streams: impl Iterator<Item = &'a Stream<'f>>) -> Option<&'a Stream<'f>>
where
    'f: 'a,
{
    best_by(streams, Rank::compare)
}

/// Лучшая аудиодорожка ролика — единственное место, где сравнивается язык.
fn best_audio<'a, 'f>(streams: impl Iterator<Item = &'a Stream<'f>>) -> Option<&'a Stream<'f>>
where
    'f: 'a,
{
    best_by(streams, Rank::compare_as_audio)
}

/// Лучший поток из перечисленных или `None`, если перечислять нечего.
///
/// При полном равенстве всех признаков выигрывает тот, кто идёт в выводе
/// yt-dlp раньше (замена — только по строгому «лучше»): порядок форматов
/// у yt-dlp стабилен, и разбор одного и того же ролика обязан давать один
/// и тот же результат.
fn best_by<'a, 'f>(
    streams: impl Iterator<Item = &'a Stream<'f>>,
    compare: fn(&Rank, &Rank) -> Ordering,
) -> Option<&'a Stream<'f>>
where
    'f: 'a,
{
    streams.fold(None, |best, candidate| match best {
        Some(best) if compare(&candidate.rank, &best.rank) != Ordering::Greater => Some(best),
        _ => Some(candidate),
    })
}

/// Один поток yt-dlp, приведённый к тому, что нужно лестнице.
struct Stream<'a> {
    format_id: &'a str,
    /// Ступень, к которой поток относится (Р-4, см. [`step_of`]);
    /// `None` — поток без видео, то есть кандидат в аудио.
    step: Option<u32>,
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
    /// оба `none`, но у них есть размеры кадра — без этой проверки
    /// раскадровка 180 px попала бы в лестницу как «максимальное
    /// доступное») и любые будущие служебные форматы того же вида.
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

        // Видеопоток, ступень которого определить нечем (ни метки, ни
        // размеров кадра), выбрать некуда, а как аудио он не годится;
        // у аудиопотоков ступени нет по определению.
        let step = match (has_video, step_of(format)) {
            (true, Some(step)) => Some(step),
            (true, None) => return None,
            (false, _) if has_audio => None,
            (false, _) => return None,
        };

        Some(Self {
            format_id,
            step,
            carries_audio: has_video && matches!(acodec, Some(codec) if codec != "none"),
            size: read_size(format),
            rank: Rank::from_format(format),
        })
    }
}

/// Ступень, к которой относится видеопоток (решение владельца Р-4).
///
/// Источник истины — метка качества, которую даёт сам yt-dlp
/// (`format_note`: `2160p60`, `1080p`, `480p`); если метки нет или в ней
/// нет числа — короткая сторона кадра `min(width, height)`.
///
/// Почему не просто высота кадра, как было до Р-4: Р-1 и вся лестница
/// говорят на языке меток YouTube, а не пикселей, и пользователь обязан
/// увидеть в списке то же число, что видит в плеере. У вертикального
/// ролика (реальная фикстура `vertical-video.json`) «1080p» — это кадр
/// 1080×1920: по высоте не совпадала ни одна ступень, и вместо лестницы
/// получалась одна строка «максимальное доступное (3840p)».
///
/// Почему не просто короткая сторона: на кашетированном широкоэкранном
/// кадре она врёт (1920×804 дала бы 804), а метка — нет. Поэтому метка
/// первая, короткая сторона — запасной путь, и он реально нужен: метку
/// несут только прямые потоки, у манифестных (`m3u8_native`)
/// `format_note` отсутствует вовсе — проверено на всех семи фикстурах.
fn step_of(format: &Value) -> Option<u32> {
    label_step(format).or_else(|| short_side(format))
}

/// Число из метки качества yt-dlp: `2160p60` → 2160, `1080p` → 1080.
///
/// `None`, если метки нет или числа в ней нет: у премиального потока
/// (формат `616`, фикстура `label-differs-from-frame.json`) вместо цифры
/// стоит слово `Premium`. Такой формат не отбрасывается — он уходит на
/// короткую сторону кадра, иначе ролик потерял бы поток на ровном месте.
///
/// Проверка «за цифрами идёт `p`» обязательна: без неё меткой стала бы
/// любая строка, начинающаяся с числа, — например `60fps`.
fn label_step(format: &Value) -> Option<u32> {
    let note = format.get("format_note").and_then(Value::as_str)?;
    let digits_len = note
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(note.len());
    let (digits, rest) = note.split_at(digits_len);

    if digits.is_empty() || !rest.starts_with('p') {
        return None;
    }

    digits.parse().ok()
}

/// Короткая сторона кадра — `min(width, height)`.
///
/// Ширины может не быть вовсе; тогда остаётся высота — для
/// горизонтального ролика это то же самое, что было до Р-4.
fn short_side(format: &Value) -> Option<u32> {
    let height = read_u32(format.get("height"));
    let width = read_u32(format.get("width"));

    match (width, height) {
        (Some(width), Some(height)) => Some(width.min(height)),
        (width, height) => width.or(height),
    }
}

/// Признаки, по которым потоки сравниваются между собой, в порядке
/// убывания важности.
///
/// Каждое поле — отдельное правило выбора; порядок полей и есть
/// приоритет правил (см. [`Rank::compare`]).
struct Rank {
    /// `language_preference` yt-dlp: 10 у оригинальной дорожки, −1 у
    /// автодубляжа.
    ///
    /// Участвует только в выборе аудио ([`Rank::compare_as_audio`]) —
    /// объяснение там же.
    language_preference: i64,
    /// Поток отдаётся напрямую по http(s), а не через манифест
    /// (HLS/DASH/mhtml).
    direct: bool,
    /// `tbr`, при отсутствии — `vbr`/`abr`; `None` — битрейт не объявлен.
    bitrate: Option<f64>,
    /// yt-dlp сообщает оценку размера этого потока.
    has_size: bool,
    /// `quality` yt-dlp — его собственная оценка «лучше/хуже»; у аудио
    /// разводит обычную дорожку и её DRC-копию.
    quality: f64,
    /// `source_preference` yt-dlp — его же предпочтение источника.
    source_preference: i64,
}

/// Чем становится необъявленный признак ранга.
///
/// Ровно то же значение, что подставляет сам yt-dlp в своей сортировке
/// форматов (`FormatSorter`, поля `lang`, `quality`, `source`), и это не
/// косметика: одни и те же признаки у части форматов объявлены, у части
/// нет (у HLS-вариантов нет ни `language_preference`, ни размера, а у
/// прямых потоков `language_preference` = −1). «Нейтральный» ноль на
/// месте отсутствующего значения поставил бы формат без признака выше
/// формата с объявленным −1 — первая же прогонка на фикстуре
/// `4k-full-ladder.json` из-за этого выбрала HLS-вариант `628` вместо
/// прямого `315`.
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
    /// 1. **Прямая раздача важнее манифеста.** Это уточнение правила
    ///    дизайна «наибольший битрейт», без которого правило не работает
    ///    на живых данных: у HLS-вариантов (`m3u8_native`) `tbr` — это
    ///    объявленная в манифесте пиковая полоса, а у прямых потоков —
    ///    средний битрейт, посчитанный из размера (у формата `315`
    ///    фикстуры `4k-full-ladder.json` `filesize · 8 / duration` = 17 162
    ///    против объявленных `tbr` = 17 174, у HLS-варианта той же
    ///    ступени — 27 987). Сравнивать их между собой некорректно, HLS
    ///    всегда «выигрывает» и при этом никогда не сообщает размер — все
    ///    видеостроки лестницы стали бы «размер неизвестен» вопреки К-1.
    ///    (Строка «только аудио» уцелела бы и по буквальному правилу:
    ///    HLS-аудио `233`/`234` не объявляет `tbr` вовсе и проигрывает
    ///    прямой дорожке по битрейту.) Отбором это не сделано: если у
    ///    ступени есть только HLS-варианты, строка всё равно нужна (Р-1 —
    ///    строка существует, если качество доступно), просто с
    ///    неизвестным размером.
    /// 2. **Собственная оценка yt-dlp (`quality`).** Второе уточнение
    ///    правила «наибольший битрейт», и по той же причине, что языковой
    ///    ключ: на живых данных буква дизайна выбирает не то. У ролика
    ///    `vertical-video.json` DRC-копия `251-drc` (дорожка со сжатой
    ///    динамикой) объявляет 138,459 kbps против 137,847 у обычной
    ///    `251` — полпроцента разницы, и по одному битрейту пользователь
    ///    получал бы пожатый звук вместо оригинала. Сам yt-dlp ставит DRC
    ///    ниже именно через `quality` (3.0 против 2.5), и в его
    ///    сортировке по умолчанию `quality` стоит выше `br` — так что
    ///    здесь мы не изобретаем правило, а перестаём расходиться с
    ///    апстримом. У видеопотоков внутри одной ступени `quality`
    ///    одинаков и ни на что не влияет: проверено — подъём ключа не
    ///    поменял ни одной видеостроки ни в одной из семи фикстур.
    /// 3. **Битрейт** — правило дизайна: на глаз он соответствует
    ///    «лучше» вернее, чем кодек или контейнер (которые в выборе
    ///    не участвуют вовсе и наружу не выходят).
    /// 4. **Наличие оценки размера** — тай-брейк дизайна: пункт не должен
    ///    становиться «размер неизвестен» из-за порядка перебора при
    ///    прочих равных.
    /// 5. **`source_preference` yt-dlp** — разводит потоки, равные по
    ///    всему вышеперечисленному (HLS-аудио `233`/`234`: и `quality`
    ///    у обоих −1, и битрейт не объявлен, и размера нет; предпочтение
    ///    источника 0 и 1).
    fn compare(&self, other: &Self) -> Ordering {
        self.direct
            .cmp(&other.direct)
            .then(self.quality.total_cmp(&other.quality))
            .then(bitrate_key(self.bitrate).total_cmp(&bitrate_key(other.bitrate)))
            .then(self.has_size.cmp(&other.has_size))
            .then(self.source_preference.cmp(&other.source_preference))
    }

    /// То же сравнение, но с языком дорожки первым ключом — только для
    /// выбора аудио.
    ///
    /// Оригинал важнее автодубляжа: у ролика с дубляжами (реальная
    /// фикстура `multi-language-audio.json`) самый жирный аудиопоток —
    /// малаялам (132,2 kbps) против английского оригинала (129,5), и
    /// выбор «просто по наибольшему битрейту» молча подменил бы звук.
    /// В самом yt-dlp порядок ключей по умолчанию тот же: `… lang,
    /// quality, res, …, br, …` — язык выше битрейта.
    ///
    /// Ключ сужен до аудио намеренно, и это не оптимизация. yt-dlp
    /// проставляет `language_preference` любому формату, у которого есть
    /// звук, — то есть и прогрессивному видео тоже. На ролике с
    /// дубляжами прогрессивный `22` (720p, ~1 Мбит/с) получил бы
    /// `language_preference` оригинальной дорожки и обошёл бы раздельный
    /// `298` (720p60, 1,9 Мбит/с) ещё до сравнения по битрейту —
    /// пользователь получил бы худшую картинку. Язык — свойство дорожки,
    /// а не картинки, и в выборе видеопотока ему делать нечего.
    fn compare_as_audio(&self, other: &Self) -> Ordering {
        self.language_preference
            .cmp(&other.language_preference)
            .then(self.compare(other))
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
    const FIXTURES: [&str; 7] = [
        "4k-full-ladder.json",
        "max-1080p.json",
        "max-240p.json",
        "multi-language-audio.json",
        "sizes-unknown.json",
        "vertical-video.json",
        "label-differs-from-frame.json",
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
    fn a_vertical_video_gets_the_whole_ladder_not_one_row() {
        // Ради этого случая и принято Р-4. У вертикального ролика «1080p»
        // — это кадр 1080×1920: по высоте не совпадает ни одна ступень, и
        // прежнее правило схлопывало лестницу в одну строку
        // «максимальное доступное (3840p)».
        let items = ladder("vertical-video.json");

        assert_eq!(
            items
                .iter()
                .map(|item| (item.kind, item.height_px))
                .collect::<Vec<_>>(),
            vec![
                (QualityKind::Standard, Some(2160)),
                (QualityKind::Standard, Some(1440)),
                (QualityKind::Standard, Some(1080)),
                (QualityKind::Standard, Some(720)),
                (QualityKind::AudioOnly, None),
            ]
        );

        // И подпись строки берётся из метки, а не из кадра: у выбранного
        // на верхней ступени формата `313` кадр 2160×3840.
        let top = &items[0];
        assert_eq!(top.streams.video_format_id.as_deref(), Some("313"));
        assert_eq!(top.height_px, Some(2160), "в подписи — метка, а не 3840");
    }

    #[test]
    fn the_quality_label_wins_when_it_disagrees_with_the_frame() {
        // Живое расхождение: у «Despacito» есть рендиция 1080×608 —
        // короткая сторона 608, а метка YouTube «480p». Побеждает метка:
        // пользователь в плеере видит именно 480p, и отдельной ступени
        // «608» в лестнице не возникает.
        let odd = fixture("label-differs-from-frame.json")
            .get("formats")
            .and_then(Value::as_array)
            .expect("в фикстуре есть форматы")
            .iter()
            .find(|format| format.get("format_id").and_then(Value::as_str) == Some("779"))
            .cloned()
            .expect("формат 779 — та самая рендиция 1080×608");

        assert_eq!(odd.get("height").and_then(Value::as_u64), Some(608));
        assert_eq!(odd.get("width").and_then(Value::as_u64), Some(1080));
        assert_eq!(short_side(&odd), Some(608));
        assert_eq!(label_step(&odd), Some(480), "метка формата — «480p»");
        assert_eq!(step_of(&odd), Some(480));

        // В лестнице ролика это никак не проявляется: 480p ниже 720p и не
        // показывается вовсе (Р-1).
        assert_eq!(
            ladder("label-differs-from-frame.json")
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
    fn a_label_without_a_number_falls_back_to_the_short_side() {
        // Премиальный поток `616` помечен словом `Premium` — числа в метке
        // нет. Формат не отбрасывается: ступень считается по кадру.
        let premium = fixture("label-differs-from-frame.json")
            .get("formats")
            .and_then(Value::as_array)
            .expect("в фикстуре есть форматы")
            .iter()
            .find(|format| format.get("format_id").and_then(Value::as_str) == Some("616"))
            .cloned()
            .expect("формат 616 — премиальный поток");

        assert_eq!(
            premium.get("format_note").and_then(Value::as_str),
            Some("Premium")
        );
        assert_eq!(label_step(&premium), None);
        assert_eq!(step_of(&premium), Some(1080), "1920×1080 — короткая 1080");
    }

    #[test]
    fn quality_labels_are_read_the_way_yt_dlp_writes_them() {
        // Метка несёт частоту кадров (`2160p60`) и иногда приписки; число
        // — до `p`. Всё, что на метку не похоже, ступенью не становится:
        // «60fps» начинается с цифр, но это не метка качества, а
        // `medium`/`Default, high` — подписи аудиодорожек.
        for (note, expected) in [
            ("2160p60", Some(2160)),
            ("1080p", Some(1080)),
            ("144p", Some(144)),
            ("1080p60 HDR", Some(1080)),
            ("Premium", None),
            ("60fps", None),
            ("medium", None),
            ("Default, high", None),
            ("", None),
            ("p", None),
        ] {
            let format = json!({ "format_note": note });
            assert_eq!(label_step(&format), expected, "метка {note:?}");
        }

        // Метки нет вовсе — так yt-dlp отдаёт манифестные потоки.
        assert_eq!(label_step(&json!({})), None);
    }

    #[test]
    fn manifest_streams_carry_no_label_and_fall_back_to_the_frame() {
        // Утверждение из doc [`step_of`], проверенное на всех фикстурах:
        // метку несут только прямые потоки, у манифестных её нет, и
        // ступень им даёт короткая сторона кадра.
        let mut manifest_video = 0_u32;

        for name in FIXTURES {
            let metadata = fixture(name);
            let formats = metadata
                .get("formats")
                .and_then(Value::as_array)
                .expect("в фикстуре есть форматы");

            for format in formats {
                let is_video = matches!(
                    format.get("vcodec").and_then(Value::as_str),
                    Some(codec) if codec != "none"
                );
                let is_manifest = !matches!(
                    format.get("protocol").and_then(Value::as_str),
                    Some("https" | "http")
                );
                if !is_video || !is_manifest {
                    continue;
                }

                manifest_video += 1;
                assert_eq!(
                    label_step(format),
                    None,
                    "{name}: у манифестного {:?} внезапно есть метка качества",
                    format.get("format_id")
                );
                assert!(
                    step_of(format).is_some(),
                    "{name}: манифестный поток остался без ступени"
                );
            }
        }

        assert!(
            manifest_video > 0,
            "в наборе фикстур не осталось манифестных видеопотоков —              проверять стало нечего"
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
                // Прогрессивная строка (видеопоток уже со звуком) отдельной
                // дорожки не несёт по контракту — сравнивать там нечего.
                // Живых фикстур с такими форматами сейчас нет, но набор
                // фикстур пополняется, и ложное падение тут было бы
                // неприятным сюрпризом для того, кто их добавит.
                if item.streams.video_format_id.is_some() && item.streams.audio_format_id.is_none()
                {
                    continue;
                }

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
    fn the_language_of_a_progressive_stream_never_outranks_a_better_picture() {
        // yt-dlp проставляет `language_preference` любому формату со
        // звуком, включая прогрессивный. На ролике с дубляжами
        // прогрессивный `22` несёт оригинальную дорожку (10), а
        // раздельный `298` — видео без языка (−1). Если сравнивать
        // видеопотоки по языку, `22` (1,0 Мбит/с) обойдёт `298`
        // (1,9 Мбит/с) ещё до битрейта, и пользователь получит худшую
        // картинку при том же звуке. Язык — свойство дорожки, не картинки.
        let metadata = json!({
            "formats": [
                {"format_id": "22", "vcodec": "avc1.64001F", "acodec": "mp4a.40.2",
                 "height": 720, "tbr": 1000.0, "filesize": 30_000_000,
                 "language": "en", "language_preference": 10, "protocol": "https"},
                {"format_id": "298", "vcodec": "avc1.4d4020", "acodec": "none",
                 "height": 720, "tbr": 1897.673, "filesize": 150_524_867,
                 "language_preference": -1, "protocol": "https"},
                {"format_id": "140-en", "vcodec": "none", "acodec": "mp4a.40.2",
                 "tbr": 129.476, "filesize": 19_880_859,
                 "language": "en", "language_preference": 10, "protocol": "https"},
                {"format_id": "140-ml", "vcodec": "none", "acodec": "mp4a.40.2",
                 "tbr": 132.243, "filesize": 20_304_957,
                 "language": "ml", "language_preference": -1, "protocol": "https"},
            ]
        });

        let items = build_quality_ladder(&metadata);

        assert_eq!(
            items.iter().map(shape).collect::<Vec<_>>(),
            vec![
                (
                    QualityKind::Standard,
                    Some(720),
                    Some("298"),
                    Some("140-en")
                ),
                (QualityKind::AudioOnly, None, None, Some("140-en")),
            ],
            "видео выбирается по битрейту, дорожка — по языку"
        );
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
    fn prefers_the_plain_audio_track_over_its_drc_copy() {
        // DRC-копия — та же дорожка со сжатой динамикой. yt-dlp ставит её
        // ниже через `quality` (2.5 против 3.0), и мы делаем так же, а не
        // сравниваем битрейты: по битрейту DRC то равна оригиналу, то
        // чуть «жирнее» — и во втором случае забирала бы строку.

        // Равенство до последнего знака: `140` и `140-drc` совпадают и по
        // битрейту (129,481), и по размеру (10 271 496 Б).
        let items = ladder("4k-full-ladder.json");
        let audio = items.last().expect("у ролика есть аудио");
        assert_eq!(audio.streams.audio_format_id.as_deref(), Some("140"));
        assert_eq!(size_bytes(audio), Some(10_271_496));

        // А здесь DRC объявляет битрейт ВЫШЕ обычной дорожки — 138,459
        // против 137,847. Правило «наибольший битрейт» в чистом виде
        // выбрало бы `251-drc`; `quality` этого не даёт.
        let items = ladder("vertical-video.json");
        let audio = items.last().expect("у ролика есть аудио");
        assert_eq!(audio.streams.audio_format_id.as_deref(), Some("251"));
        assert_eq!(size_bytes(audio), Some(1_500_492));

        // Дорожка едет во все строки лестницы, значит подмена звука
        // испортила бы и их тоже.
        for item in &items {
            assert_eq!(item.streams.audio_format_id.as_deref(), Some("251"));
        }
    }

    #[test]
    fn raising_the_quality_key_left_every_video_row_untouched() {
        // Подъём `quality` выше битрейта — уточнение ради аудио, и у
        // видеопотоков внутри ступени этот ключ одинаков. Строки лестницы
        // на всех фикстурах остались прежними; здесь это зафиксировано
        // явно, чтобы будущая правка ключей показала цену на видео, а не
        // только на звуке.
        let expected: [(&str, &[(u32, &str)]); 7] = [
            (
                "4k-full-ladder.json",
                &[(2160, "315"), (1440, "308"), (1080, "299"), (720, "298")],
            ),
            ("max-1080p.json", &[(1080, "137"), (720, "247")]),
            ("max-240p.json", &[(240, "133")]),
            (
                "multi-language-audio.json",
                &[(2160, "313"), (1440, "271"), (1080, "137"), (720, "136")],
            ),
            ("sizes-unknown.json", &[(240, "230")]),
            (
                "vertical-video.json",
                &[(2160, "313"), (1440, "400"), (1080, "137"), (720, "136")],
            ),
            (
                "label-differs-from-frame.json",
                &[(1080, "137"), (720, "247")],
            ),
        ];

        for (name, rows) in expected {
            let items = ladder(name);
            let actual: Vec<(u32, &str)> = items
                .iter()
                .filter(|item| item.kind != QualityKind::AudioOnly)
                .map(|item| {
                    (
                        item.height_px.expect("у видеостроки есть ступень"),
                        item.streams
                            .video_format_id
                            .as_deref()
                            .expect("у видеостроки есть видеопоток"),
                    )
                })
                .collect();

            assert_eq!(actual, rows, "{name}: видеостроки изменились");
        }
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
    fn a_row_keeps_its_estimate_when_only_the_audio_size_is_missing() {
        // Размер известен только у видеопотока: аудиодорожка весит
        // проценты от строки, и «неизвестно» при наличии данных было бы
        // потерей информации на ровном месте. Обратный случай (неизвестен
        // видеопоток) разобран отдельным тестом и ведёт себя иначе.
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
    fn a_row_whose_video_stream_has_no_size_reports_no_estimate() {
        // Обратный случай к предыдущему, и ведёт он себя иначе. Ступень
        // собрана из одних HLS-вариантов (размера не сообщает ни один), а
        // дорожка — прямая и с размером. Сложение известного дало бы
        // «2160p ≈ 10 МБ»: промах на два порядка, потому что видеопоток —
        // доминирующая часть строки. Такое «≈» дезинформирует сильнее,
        // чем честное «размер неизвестен» (Ф-4 оставляет пункт
        // выбираемым).
        let metadata = json!({
            "formats": [
                {"format_id": "628", "vcodec": "vp09", "acodec": "none", "height": 2160,
                 "tbr": 27987.109, "protocol": "m3u8_native"},
                {"format_id": "140", "vcodec": "none", "acodec": "mp4a.40.2",
                 "tbr": 129.481, "filesize": 10_271_496, "protocol": "https"},
            ]
        });

        let items = build_quality_ladder(&metadata);

        assert_eq!(
            shape(&items[0]),
            (QualityKind::Standard, Some(2160), Some("628"), Some("140")),
            "строка остаётся: качество доступно, скачать его есть чем (Р-1)"
        );
        assert_eq!(
            items[0].size,
            QualitySize::Unknown,
            "размер дорожки не выдаётся за размер строки"
        );
        assert_eq!(size_bytes(&items[1]), Some(10_271_496));
    }

    #[test]
    fn a_gap_in_the_ladder_removes_only_the_missing_step() {
        // У ролика есть 2160p и 720p, но нет 1440p и 1080p. Живые фикстуры
        // такого не дают (YouTube выкладывает высоты подряд), а код этот
        // случай обрабатывает — значит он должен быть зафиксирован: строк
        // ровно две, порядок сверху вниз сохраняется, дыра не превращает
        // лестницу в «максимальное доступное».
        let metadata = json!({
            "formats": [
                {"format_id": "top", "vcodec": "vp9", "acodec": "none", "height": 2160,
                 "tbr": 17174.188, "filesize": 1_362_269_481, "protocol": "https"},
                {"format_id": "bottom", "vcodec": "avc1", "acodec": "none", "height": 720,
                 "tbr": 1897.673, "filesize": 150_524_867, "protocol": "https"},
                {"format_id": "audio", "vcodec": "none", "acodec": "mp4a.40.2",
                 "tbr": 129.481, "filesize": 10_271_496, "protocol": "https"},
            ]
        });

        let items = build_quality_ladder(&metadata);

        assert_eq!(
            items.iter().map(shape).collect::<Vec<_>>(),
            vec![
                (
                    QualityKind::Standard,
                    Some(2160),
                    Some("top"),
                    Some("audio")
                ),
                (
                    QualityKind::Standard,
                    Some(720),
                    Some("bottom"),
                    Some("audio")
                ),
                (QualityKind::AudioOnly, None, None, Some("audio")),
            ]
        );
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
    fn a_step_outside_the_ladder_never_makes_its_own_row() {
        // Ступень, которой нет в лестнице, своей строки не получает, если
        // хоть одна ступень лестницы у ролика есть.
        //
        // Живых данных под этот случай нет и после Р-4 они маловероятны:
        // метки YouTube — фиксированный набор (144p…2160p), а запасной
        // путь (короткая сторона) срабатывает только у манифестных
        // потоков, у которых рядом всегда есть прямые аналоги с метками.
        // Единственная живая рендиция «мимо сетки» — 1080×608 у
        // «Despacito» — размечена самим YouTube как «480p» и после Р-4
        // просто сливается со ступенью 480p (см.
        // `the_quality_label_wins_when_it_disagrees_with_the_frame`).
        // Поэтому здесь синтетика: 900 — не метка YouTube.
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

    /// Единственная фикстура, у которой состав форматов изменён: из вывода
    /// оставлены только потоки через манифест (см. README рядом с ней).
    const DERIVED_FIXTURE: &str = "sizes-unknown.json";

    #[test]
    fn fixtures_are_real_output_of_the_pinned_yt_dlp() {
        // Фикстуры заморожены, а yt-dlp — нет: набор останется зелёным и
        // после того, как апстрим сменит форму вывода. Единственная защита
        // от «тесты проходят, приложение получает другое» — чтобы смена
        // пина в binaries.lock.json громко ломала этот тест и заставляла
        // переснять фикстуры. До сих пор эта связь держалась только
        // словами README.
        let pinned = pinned_yt_dlp_version();

        for name in FIXTURES {
            let metadata = fixture(name);

            assert_eq!(
                metadata
                    .get("_version")
                    .and_then(|version| version.get("version"))
                    .and_then(Value::as_str),
                Some(pinned.as_str()),
                "{name}: фикстура снята не тем yt-dlp, который вложен в                  приложение ({pinned} по binaries.lock.json). Пин сменили —                  переснимите фикстуры по README, а не правьте эту строку"
            );
            assert_eq!(
                metadata.get("extractor").and_then(Value::as_str),
                Some("youtube"),
                "{name}: фикстура должна быть выводом yt-dlp по ролику YouTube"
            );

            let formats = metadata
                .get("formats")
                .and_then(Value::as_array)
                .unwrap_or_else(|| panic!("{name}: в фикстуре нет массива форматов"));
            assert!(!formats.is_empty(), "{name}: в фикстуре нет форматов");

            for format in formats {
                for key in ["format_id", "protocol", "ext"] {
                    assert!(
                        format.get(key).is_some(),
                        "{name}: у формата нет поля {key} — так yt-dlp не отдаёт"
                    );
                }
            }

            // Живой вывод содержит прямые потоки; их отсутствие — признак
            // производной фикстуры, и она у нас ровно одна.
            let has_direct = formats.iter().any(|format| {
                matches!(
                    format.get("protocol").and_then(Value::as_str),
                    Some("https" | "http")
                )
            });
            assert_eq!(
                has_direct,
                name != DERIVED_FIXTURE,
                "{name}: состав форматов не соответствует README — производной                  объявлена только {DERIVED_FIXTURE}"
            );
        }
    }

    /// Версия yt-dlp из пина `binaries.lock.json` — та, что реально
    /// вкладывается в приложение.
    fn pinned_yt_dlp_version() -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("binaries.lock.json");
        let raw = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("пин {} не читается: {err}", path.display()));
        let pin: Value = serde_json::from_str(&raw)
            .unwrap_or_else(|err| panic!("пин {} — не JSON: {err}", path.display()));

        pin.get("ytDlp")
            .and_then(|yt_dlp| yt_dlp.get("version"))
            .and_then(Value::as_str)
            .expect("в пине объявлена версия yt-dlp")
            .to_owned()
    }
}
