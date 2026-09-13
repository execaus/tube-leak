//! Разбор stdout yt-dlp во время скачивания (Ф-2, TL-41).
//!
//! Модуль чистый: ни процессов, ни файлов, ни событий Tauri. Вход —
//! одна строка stdout, выход — [`StdoutLine`]. Кто эти строки читает и что
//! с ними делает, решает оркестрация (TL-44); агрегация двух потоков в один
//! процент — соседний модуль [`crate::download::aggregate`].
//!
//! # Почему не разбирается человекочитаемая полоса
//!
//! По умолчанию yt-dlp печатает прогресс так:
//!
//! ```text
//! [download]  57.0% of   17.45MiB at  596.88KiB/s ETA 00:12
//! ```
//!
//! Это витрина, а не данные: процент округлён до десятой, размер — до
//! сотой мегабайта, скорость несёт единицу измерения, ETA — `мм:сс`.
//! Считать по такой строке агрегированный процент двух потоков нельзя:
//! «17.45MiB» — это любое число байт из диапазона шириной ~5 КБ, а
//! знаменатель агрегации обязан быть точным (иначе получается ровно тот
//! дефект, о котором предупреждает doc [`crate::types::DownloadPercent`]:
//! «99, 100, 100, 100 %» на заниженной оценке).
//!
//! Поэтому запуск обязан нести [`PROGRESS_TEMPLATE`], и разбирается именно
//! его вывод — с точными байтами. Строки человекочитаемой полосы при этом
//! не печатаются вовсе: шаблон её заменяет, а не дополняет.
//!
//! # Ловушка шаблона: `NA`, а не `null`
//!
//! Соблазнительно собрать шаблоном JSON — у yt-dlp есть конверсия `j`,
//! которая пишет значения как литералы JSON. Проверено живьём: **на
//! отсутствующем значении `%(…)j` печатает `NA`, а не `null`**, то есть
//! строка перестаёт быть валидным JSON ровно там, где данных нет, — а нет
//! их почти всегда (`total_bytes` у потоков через манифест, `eta` у
//! завершившегося потока). Чинить это заменой `NA` на `null` перед
//! разбором значило бы портить и те поля, где `NA` могло бы быть
//! настоящим значением. Отсюда простая форма: разделитель `|`, ровно
//! девять значений после маркера, `NA` — единственный признак отсутствия.
//!
//! # Что ещё приезжает в stdout
//!
//! Кроме строк прогресса, в том же потоке идут служебные строки yt-dlp.
//! Разбираются четыре — те, что меняют смысл чисел вокруг:
//!
//! - `[info] <id>: Downloading N format(s): a, b` — какие форматы yt-dlp
//!   **выбрал** к скачиванию (TL-48). Формат, которого нет в метаданных,
//!   из этого перечня молча выпадает, а процесс выходит с кодом 0;
//! - `[download] Destination: …` — с какого места начинается поток
//!   очередного формата (и куда он пишется);
//! - `[download] Resuming download at byte N` — попытка продолжает
//!   частичный файл, и первый же `downloaded_bytes` будет не с нуля;
//! - `[download] … has already been downloaded` — поток качать не
//!   пришлось, **строк `downloading` по нему не будет ни одной**. Будет
//!   ли следом `finished`, зависит от аргументов (замер TL-48 на вложенном
//!   бинарнике): на argv приложения — да, с `downloaded_bytes` = `NA`; с
//!   отдельным временным каталогом (`-P temp:…`) — нет.
//!
//! Всё остальное — [`StdoutLine::Other`]: `[youtube] …`, прочие `[info] …`,
//! `[hlsnative] …`, вывод постпроцессоров. Модуль их не классифицирует —
//! классификация отказов это TL-43, и она смотрит на stderr и код выхода.
//!
//! # Как часто эти строки приходят (замер, а не таблица)
//!
//! Дизайн назвал 300 мс частотой эмита событий и 5 с порогом «мягкого»
//! индикатора зависания стартовыми точками, подлежащими калибровке (урок
//! TL-12). Замерено на живой загрузке того же ролика, что и фикстуры,
//! macOS 15, 2026-08-26, домашний канал ~0,8 МБ/с:
//!
//! | Поток | `--progress-delta` | Строк | Медианный разрыв | p90 | Максимум |
//! |---|---|---|---|---|---|
//! | прямой, 18 МБ (`134`) | нет | 36 | 0,18 с | 1,12 с | **1,68 с** |
//! | прямой, 18 МБ (`134`) | 0,3 | 32 | 0,46 с | 0,98 с | 1,00 с |
//! | манифест, 2 МБ (`602`) | нет | 683 за 43,6 с (**15,7 строк/с**) | — | — | — |
//! | манифест, 2 МБ (`602`) | 0,3 | 133 | 0,31 с | 0,32 с | 0,32 с |
//!
//! Три следствия для тех, кто будет ставить числа:
//!
//! 1. **Заваливает webview не прямой поток, а манифестный.** Прямой даёт
//!    полторы строки в секунду и в порог 300 мс укладывается сам;
//!    манифестный печатает по строке на чанк — пятнадцать в секунду.
//!    Дешевле всего это режется у источника: `--progress-delta 0.3` даёт
//!    ровно тот же порог 300 мс и сокращает поток впятеро **до** того,
//!    как строки дойдут до разбора. Собственный троттлинг в TL-44 это не
//!    отменяет — правило «переход в терминальную фазу не может быть
//!    проглочен» касается событий, которых в stdout вообще нет.
//! 2. **Порог 5 с на здоровой загрузке не ложный.** Наибольший разрыв
//!    между строками — 1,68 с без `--progress-delta` и 1,00 с с ним, то
//!    есть запас втрое-впятеро. Сузить порог до секунды нельзя: медиана и
//!    максимум различаются вдесятеро, разрывы неравномерны по устройству
//!    (строка печатается на границе чанка, а чанки растут).
//! 3. **Сторож «ни байта за 20 с» (С-8) обязан быть таймером, а не
//!    счётчиком строк.** Замерший поток не печатает ничего: строка
//!    выходит на принятых данных, и при их отсутствии не выходит ни одна.
//!    Ждать «строку с тем же `downloaded_bytes`» бесполезно — её не
//!    будет.

use std::str::FromStr;

/// Значение аргумента `--progress-template` для запуска скачивания.
///
/// Константа — часть контракта между запуском (TL-44) и разбором: строки,
/// снятой другим шаблоном, [`parse_line`] не поймёт. Именно поэтому её
/// текст записан внутрь каждой фикстуры (`_capture.progressTemplate`), а
/// тест `fixtures_are_the_output_of_this_template_and_of_the_pinned_yt_dlp`
/// сверяет их посимвольно: правка шаблона обязана краснеть до тех пор,
/// пока фикстуры не пересняты, а не превращать набор в проверку
/// замороженного прошлого.
///
/// Префикс `download:` — тип шаблона; без него yt-dlp применил бы его и к
/// постпроцессорам. Полей девять, порядок фиксирован; `info.format_id` в
/// конце — ключ, по которому агрегация относит строку к видео или к
/// звуку.
pub const PROGRESS_TEMPLATE: &str = "download:@tl-progress|%(progress.status)s|%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.total_bytes_estimate)s|%(progress.speed)s|%(progress.eta)s|%(progress.fragment_index)s|%(progress.fragment_count)s|%(info.format_id)s";

/// Маркер начала строки прогресса.
///
/// Собственный, а не `[download]`: yt-dlp печатает под этим префиксом ещё
/// с десяток разных строк, и отличать «полосу» от них по форме значило бы
/// повторять внутри себя чужую эвристику. Маркер выдаётся шаблоном, то
/// есть его наличие — наше решение, а не совпадение.
const MARKER: &str = "@tl-progress";

const SEPARATOR: char = '|';

/// Чем yt-dlp обозначает отсутствующее значение в шаблоне (`%(…)s` и
/// `%(…)j` одинаково).
const MISSING: &str = "NA";

/// Сколько значений стоит в шаблоне после маркера.
const VALUE_COUNT: usize = 9;

/// Конец строки `[download] <путь> has already been downloaded`.
///
/// Якорь — конец строки, а не первое вхождение: путь несёт название
/// ролика, и фраза может стоять в нём самом (фикстура
/// `single-launch/phrase-in-title.json`). Продолжения после фразы на пине
/// 2026.08.19 не бывает ни в одной съёмке, поэтому строка с хвостом —
/// не эта строка.
const ALREADY_DOWNLOADED: &str = " has already been downloaded";

/// Что за строка пришла в stdout.
///
/// Заимствует у входной строки: разбор — чистая функция от неё, а
/// владение нужно только тем полям, которые агрегация хранит между
/// вызовами (см. [`ProgressSample::format_id`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StdoutLine<'a> {
    /// Строка прогресса, разобранная целиком.
    Progress(ProgressSample),
    /// Строка с нашим маркером, которую разобрать не вышло.
    ///
    /// Отдельный вариант, а не [`StdoutLine::Other`], намеренно: маркер
    /// ставим мы, и если строка с ним не читается — значит yt-dlp сменил
    /// форму вывода полей шаблона. Проглотить такое молча означало бы
    /// показывать пользователю замерший прогресс вместо честного отказа,
    /// поэтому решение (залогировать, засчитать отсутствие продвижения)
    /// принимает оркестрация, а не этот модуль.
    MalformedProgress(&'a str),
    /// `[info] <id>: Downloading N format(s): a, b` — перечень форматов,
    /// которые yt-dlp выбрал к скачиванию, в его порядке.
    ///
    /// Нужен ради одного вывода, которого иначе не сделать (TL-48): формат,
    /// заказанный через запятую, но отсутствующий в метаданных, yt-dlp не
    /// считает ошибкой — он пропадает из перечня, процесс выходит с кодом 0
    /// (фикстура `single-launch/one-format-missing.json`).
    SelectedFormats { format_ids: Vec<&'a str> },
    /// `[download] Destination: <путь>` — куда пишется очередной поток.
    Destination { path: &'a str },
    /// `[download] Resuming download at byte N` — попытка продолжает
    /// частичный файл (Ф-5, докачка штатным механизмом yt-dlp).
    Resuming { byte_offset: u64 },
    /// `[download] <путь> has already been downloaded` — файл на месте,
    /// строк `downloading` по этому потоку не будет.
    AlreadyDownloaded { path: &'a str },
    /// Всё остальное.
    Other,
}

/// Состояние потока в строке прогресса.
///
/// Значений ровно два, и это не упрощение: `report_progress` в yt-dlp
/// доходит до шаблона только со `status` равным `downloading` или
/// `finished`, остальные (`error`) он отбрасывает раньше. Третье значение
/// в выводе означало бы смену поведения апстрима — и попадёт в
/// [`StdoutLine::MalformedProgress`], а не тихо сойдёт за «качается».
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleStatus {
    Downloading,
    Finished,
}

/// Разобранная строка прогресса одного потока.
///
/// Все числа — как их дал yt-dlp, без сглаживания и без домыслов:
/// отсутствующее поле это `None`, а не ноль (правило Ф-2 «отсутствующие
/// данные опускаются, а не выдумываются»).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressSample {
    pub status: SampleStatus,
    /// Идентификатор формата yt-dlp (`133`, `140-drc`, `602`) — ключ, по
    /// которому агрегация относит строку к видео или к звуку.
    pub format_id: String,
    /// Сколько байт этого потока уже на диске.
    ///
    /// **Величина абсолютная, а не «за эту попытку».** Проверено живьём
    /// (фикстура `resume-continued.json`): после обрыва на 995 883 байтах
    /// следующая попытка первой же строкой сообщает 996 907, а не 1024.
    /// На этом стоит требование К-6 «процент не падает к нулю»: считать
    /// его заново по каждой попытке не нужно, достаточно не терять
    /// накопленное.
    ///
    /// `None` бывает только у `finished`, и ровно в одном снятом случае:
    /// файл уже лежал на диске, yt-dlp его не качал и шлёт `NA`
    /// (фикстура `already-downloaded.json`). У `downloading` число
    /// обязательно — строка без него отвергается целиком.
    pub downloaded_bytes: Option<u64>,
    /// Точный полный размер потока, если yt-dlp его знает.
    ///
    /// У прямых потоков (`https`) известен с первой строки; у потоков
    /// через манифест — только в строке `finished`.
    pub total_bytes: Option<u64>,
    /// Прикидка полного размера, когда точного нет.
    ///
    /// Разбирается, но знаменателем агрегации **не служит** — см.
    /// [`crate::download::aggregate`]: на живом выводе она пересчитывается
    /// на каждом чанке и скачет на порядок.
    pub total_bytes_estimate: Option<u64>,
    /// Мгновенная скорость, байт в секунду (в шаблоне — дробная,
    /// округляется здесь).
    pub speed_bytes_per_sec: Option<u64>,
    /// Оценка оставшегося времени в секундах.
    ///
    /// У прямых потоков приезжает целой, у потоков через манифест —
    /// дробной (`41.750707256883096`); округляется здесь, чтобы разница
    /// протоколов не доехала до контракта.
    pub eta_secs: Option<u64>,
    /// Номер текущего фрагмента и их общее число — только у потоков через
    /// манифест.
    ///
    /// На последней строке индекс живьём доходит до `124` при `count`
    /// равном 123 (yt-dlp считает начатые фрагменты, а не законченные) —
    /// доля по ним обрезается сверху там, где считается.
    pub fragment_index: Option<u64>,
    pub fragment_count: Option<u64>,
}

impl ProgressSample {
    /// Лучший известный полный размер потока: точный, при его отсутствии —
    /// никакой.
    ///
    /// Прикидка сюда сознательно не попадает: см. doc
    /// [`ProgressSample::total_bytes_estimate`].
    pub fn exact_total_bytes(&self) -> Option<u64> {
        self.total_bytes
    }
}

/// Разобрать одну строку stdout.
///
/// Перевод строки и `\r` на конце снимаются: с `--newline` yt-dlp пишет
/// `\n`, но строка может приехать и из построчного чтения буфера, где
/// возврат каретки остался от прошлой полосы.
pub fn parse_line(line: &str) -> StdoutLine<'_> {
    let line = line.trim_end_matches(['\r', '\n']);

    if let Some(fields) = line.strip_prefix(MARKER) {
        return match parse_progress(fields) {
            Some(sample) => StdoutLine::Progress(sample),
            None => StdoutLine::MalformedProgress(line),
        };
    }

    if let Some(rest) = line.strip_prefix("[info] ") {
        return match selected_formats(rest) {
            Some(format_ids) => StdoutLine::SelectedFormats { format_ids },
            None => StdoutLine::Other,
        };
    }

    let Some(rest) = line.strip_prefix("[download] ") else {
        return StdoutLine::Other;
    };

    if let Some(path) = rest.strip_prefix("Destination: ") {
        return StdoutLine::Destination { path };
    }
    if let Some(offset) = rest.strip_prefix("Resuming download at byte ") {
        return match offset.trim().parse::<u64>() {
            Ok(byte_offset) => StdoutLine::Resuming { byte_offset },
            Err(_) => StdoutLine::Other,
        };
    }
    if let Some(path) = rest.strip_suffix(ALREADY_DOWNLOADED) {
        return StdoutLine::AlreadyDownloaded { path };
    }

    StdoutLine::Other
}

/// Перечень из `<id>: Downloading N format(s): a, b`, если строка именно
/// такая.
///
/// Форма сверяется целиком: число перед `format(s)` обязано совпасть с
/// длиной перечня, а идентификатор — быть непустым и без пробелов.
/// Прочие строки `[info]` (`Downloading subtitles`, запись метаданных)
/// этой проверки не проходят и остаются [`StdoutLine::Other`].
fn selected_formats(rest: &str) -> Option<Vec<&str>> {
    let (_video_id, tail) = rest.split_once(": Downloading ")?;
    let (count, list) = tail.split_once(" format(s): ")?;
    let count: usize = count.parse().ok()?;
    let format_ids: Vec<&str> = list.split(", ").collect();
    let well_formed = format_ids.len() == count
        && format_ids
            .iter()
            .all(|id| !id.is_empty() && !id.contains(char::is_whitespace));
    well_formed.then_some(format_ids)
}

/// Разобрать хвост строки прогресса — всё, что после маркера.
///
/// `None` означает «строка с маркером, но не той формы»: количество полей
/// не то, статус незнакомый, обязательное число не читается. Порядок
/// полей задан [`PROGRESS_TEMPLATE`] и здесь только читается.
fn parse_progress(values: &str) -> Option<ProgressSample> {
    // `values` начинается с разделителя — маркер уже отрезан. Число
    // значений сверяется точно: лишний разделитель внутри значения обязан
    // ломать разбор, а не сдвигать поля на одно.
    let mut parts = [""; VALUE_COUNT];
    let mut seen = 0usize;
    for part in values.split(SEPARATOR).skip(1) {
        // skip(1) — пустая часть перед первым разделителем.
        if seen == VALUE_COUNT {
            return None;
        }
        parts[seen] = part;
        seen += 1;
    }
    if seen != VALUE_COUNT {
        return None;
    }

    let [status, downloaded, total, estimate, speed, eta, fragment_index, fragment_count, format_id] =
        parts;

    let status = match status {
        "downloading" => SampleStatus::Downloading,
        "finished" => SampleStatus::Finished,
        _ => return None,
    };
    let format_id = optional_field(format_id)?;
    // `NA` у принятых байт — не порча, а факт: так yt-dlp закрывает поток,
    // который не качал (doc `downloaded_bytes`). Но только в `finished`:
    // строка хода загрузки без числа принятого ничего не сообщает.
    let downloaded_bytes = match status {
        SampleStatus::Downloading => Some(read_number(downloaded)?),
        SampleStatus::Finished => optional_number(downloaded)?,
    };

    Some(ProgressSample {
        status,
        format_id: format_id.to_owned(),
        downloaded_bytes,
        total_bytes: optional_number(total)?,
        total_bytes_estimate: optional_number(estimate)?,
        speed_bytes_per_sec: optional_number(speed)?,
        eta_secs: optional_number(eta)?,
        fragment_index: optional_number(fragment_index)?,
        fragment_count: optional_number(fragment_count)?,
    })
}

/// Значение поля, если оно есть; `None` для `NA` и для пустого поля.
fn optional_field(field: &str) -> Option<&str> {
    (field != MISSING && !field.is_empty()).then_some(field)
}

/// `Ok(None)` для `NA`, `Ok(Some(_))` для читаемого числа, `Err` для
/// нечитаемого — но всё это в форме `Option<Option<_>>`, чтобы `?` в
/// [`parse_progress`] отбрасывал строку целиком: половина разобранной
/// строки прогресса хуже, чем честное «не разобрал».
fn optional_number(field: &str) -> Option<Option<u64>> {
    match optional_field(field) {
        None => Some(None),
        Some(value) => read_number(value).map(Some),
    }
}

/// Неотрицательное целое из поля шаблона.
///
/// Числа приезжают в двух видах сразу: `downloaded_bytes` целым,
/// `speed`/`eta`/`total_bytes_estimate` — дробными (`41.7507…`,
/// `2000642.0`), причём у одного и того же поля вид зависит от протокола
/// потока. Поэтому сначала пробуется целое, потом дробное с округлением;
/// отрицательное, бесконечность и `NaN` — не число.
fn read_number(field: &str) -> Option<u64> {
    if let Ok(value) = u64::from_str(field) {
        return Some(value);
    }
    let value = f64::from_str(field).ok()?;
    (value.is_finite() && value >= 0.0).then(|| value.round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::download::fixtures::{self, PROGRESS_FIXTURES, SINGLE_LAUNCH_FIXTURES};

    /// Разобрать весь снятый stdout фикстуры построчно.
    fn lines(name: &str) -> Vec<StdoutLine<'static>> {
        parse_all(fixtures::stdout(name))
    }

    /// То же для фикстуры одного запуска (TL-48).
    fn launch_lines(name: &str) -> Vec<StdoutLine<'static>> {
        parse_all(fixtures::single_launch(name).stdout)
    }

    fn parse_all(stdout: String) -> Vec<StdoutLine<'static>> {
        // Фикстура живёт до конца теста: содержимое утекает намеренно,
        // иначе заимствующий `StdoutLine` не пережил бы возврат.
        let stdout: &'static str = Box::leak(stdout.into_boxed_str());
        stdout.lines().map(parse_line).collect()
    }

    fn samples(name: &str) -> Vec<ProgressSample> {
        lines(name)
            .into_iter()
            .filter_map(|line| match line {
                StdoutLine::Progress(sample) => Some(sample),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn fixtures_are_the_output_of_this_template_and_of_the_pinned_yt_dlp() {
        // Две привязки в одном тесте, потому что порознь они бесполезны.
        //
        // Версия: фикстуры заморожены, yt-dlp — нет. Набор остался бы
        // зелёным и после того, как апстрим сменит форму вывода, поэтому
        // смена пина обязана ломать этот тест (тот же приём, что в
        // `probe::quality` и `probe::classify` — TL-30/31).
        //
        // Шаблон: наш собственный, и правка константы делает фикстуры
        // выводом чего-то другого. Сверка посимвольная — подогнать её
        // вместо пересъёмки нельзя, не заметив.
        let pinned = fixtures::pinned_yt_dlp_version();

        for name in PROGRESS_FIXTURES {
            let capture = fixtures::capture(name);

            assert_eq!(
                capture.yt_dlp_version, pinned,
                "{name}: фикстура снята не тем yt-dlp, который вложен в \
                 приложение ({pinned} по binaries.lock.json). Пин сменили — \
                 переснимите фикстуры по README, а не правьте эту строку"
            );
            assert_eq!(
                capture.progress_template, PROGRESS_TEMPLATE,
                "{name}: фикстура снята другим --progress-template. Шаблон \
                 сменили — переснимите фикстуры по README"
            );
            assert!(
                capture.argv.iter().any(|arg| arg == "<progressTemplate>"),
                "{name}: в argv фикстуры должен стоять плейсхолдер шаблона — \
                 команда пересъёмки собирается подстановкой поля \
                 progressTemplate"
            );
        }

        // Список фикстур объявлен в коде, а каталог живёт своей жизнью:
        // забытый в нём файл не проверялся бы ни одним тестом и выглядел
        // бы покрытым случаем.
        assert_eq!(
            fixtures::files_on_disk(),
            {
                let mut declared: Vec<String> = PROGRESS_FIXTURES
                    .iter()
                    .map(|name| (*name).to_owned())
                    .collect();
                declared.sort();
                declared
            },
            "каталог фикстур и список PROGRESS_FIXTURES разошлись"
        );

        // Набор одного запуска (TL-48) — те же две привязки: он снят тем
        // же вложенным бинарником и тем же шаблоном.
        for name in SINGLE_LAUNCH_FIXTURES {
            let capture = fixtures::single_launch(name).capture;
            assert_eq!(
                capture.yt_dlp_version, pinned,
                "single-launch/{name}: пин сменили — переснимите по README"
            );
            assert_eq!(
                capture.progress_template, PROGRESS_TEMPLATE,
                "single-launch/{name}: шаблон сменили — переснимите по README"
            );
            assert!(
                capture.argv.iter().any(|arg| arg == "<progressTemplate>"),
                "single-launch/{name}: в argv нет плейсхолдера шаблона"
            );
        }
        assert_eq!(
            fixtures::single_launch_files_on_disk(),
            {
                let mut declared: Vec<String> = SINGLE_LAUNCH_FIXTURES
                    .iter()
                    .map(|name| (*name).to_owned())
                    .collect();
                declared.sort();
                declared
            },
            "каталог single-launch и список SINGLE_LAUNCH_FIXTURES разошлись"
        );
    }

    #[test]
    fn every_marked_line_of_every_fixture_parses() {
        // Сторож обратной стороны предыдущего теста: версия сошлась, а
        // строка не разобралась — значит разбор разошёлся с выводом.
        let every = PROGRESS_FIXTURES
            .iter()
            .map(|name| (*name, lines(name)))
            .chain(
                SINGLE_LAUNCH_FIXTURES
                    .iter()
                    .map(|name| (*name, launch_lines(name))),
            );
        for (name, parsed) in every {
            let malformed: Vec<&str> = parsed
                .into_iter()
                .filter_map(|line| match line {
                    StdoutLine::MalformedProgress(raw) => Some(raw),
                    _ => None,
                })
                .collect();

            assert!(
                malformed.is_empty(),
                "{name}: строки с маркером, которые не разобрались: {malformed:?}"
            );
        }
    }

    #[test]
    fn a_direct_stream_reports_exact_totals_from_the_first_line() {
        let samples = samples("video-and-audio.json");

        let first = &samples[0];
        assert_eq!(first.status, SampleStatus::Downloading);
        assert_eq!(first.format_id, "133");
        assert_eq!(first.downloaded_bytes, Some(1024));
        assert_eq!(first.total_bytes, Some(9_323_483));
        assert_eq!(first.total_bytes_estimate, None);
        assert_eq!(first.speed_bytes_per_sec, Some(288_932));
        assert_eq!(first.eta_secs, Some(32));
        assert_eq!(first.fragment_index, None);
        assert_eq!(first.fragment_count, None);

        // Оба потока доходят до `finished`, и у обоих в этот момент
        // известен точный размер — на этом стоит правило «100 % только
        // когда закрыты все потоки» в агрегации.
        let finished: Vec<(&str, u64)> = samples
            .iter()
            .filter(|sample| sample.status == SampleStatus::Finished)
            .map(|sample| {
                (
                    sample.format_id.as_str(),
                    sample.total_bytes.expect("у finished известен размер"),
                )
            })
            .collect();
        assert_eq!(finished, vec![("133", 9_323_483), ("139", 3_871_021)]);
    }

    #[test]
    fn a_manifest_stream_has_no_exact_total_until_it_finishes() {
        let samples = samples("hls-fragmented.json");

        let downloading: Vec<&ProgressSample> = samples
            .iter()
            .filter(|sample| sample.status == SampleStatus::Downloading)
            .collect();
        assert!(
            downloading
                .iter()
                .all(|sample| sample.total_bytes.is_none()),
            "у потока через манифест точного размера по ходу приёма не бывает"
        );
        assert!(
            downloading
                .iter()
                .all(|sample| sample.fragment_count == Some(123)),
            "число фрагментов известно с первой строки"
        );

        let last = samples.last().expect("фикстура не пуста");
        assert_eq!(last.status, SampleStatus::Finished);
        assert_eq!(last.total_bytes, Some(2_002_089));
        assert_eq!(last.fragment_index, None, "у finished фрагментов нет");
    }

    #[test]
    fn the_estimate_of_a_manifest_stream_swings_by_an_order_of_magnitude() {
        // Не любопытный факт, а обоснование решения агрегации: этой
        // величиной нельзя делить. Если апстрим когда-нибудь начнёт
        // отдавать её сглаженной, тест покраснеет — и решение можно будет
        // пересмотреть осознанно, а не обнаружить, что оно устарело.
        let samples = samples("hls-fragmented.json");
        let estimates: Vec<u64> = samples
            .iter()
            .filter_map(|sample| sample.total_bytes_estimate)
            .collect();

        let smallest = *estimates.iter().min().expect("прикидки есть");
        let largest = *estimates.iter().max().expect("прикидки есть");
        assert!(
            largest > smallest * 10,
            "прикидка размера у потока через манифест обязана прыгать \
             (снято: от {smallest} до {largest}); ровное значение означало \
             бы, что решение «не делить на прикидку» пора пересмотреть"
        );

        // И она не монотонна: соседние строки одного и того же потока
        // дают то больше, то меньше.
        let falls = estimates
            .windows(2)
            .filter(|pair| pair[1] < pair[0])
            .count();
        assert!(
            falls > 10,
            "прикидка обязана и падать между соседними строками, иначе она \
             годилась бы в знаменатель (падений: {falls})"
        );
    }

    #[test]
    fn the_eta_of_a_manifest_stream_is_fractional() {
        // У прямого потока eta целая, у манифестного — дробная. Разница
        // протоколов не должна доезжать до контракта, поэтому округление
        // живёт здесь, а тест закрепляет обе формы сразу.
        let direct = samples("video-and-audio.json");
        assert_eq!(direct[0].eta_secs, Some(32));

        let manifest = samples("hls-fragmented.json");
        let with_eta = manifest
            .iter()
            .find(|sample| sample.eta_secs.is_some())
            .expect("eta появляется не с первой строки, но появляется");
        assert_eq!(with_eta.eta_secs, Some(42), "41.7507… округляется до 42");
    }

    #[test]
    fn a_resumed_attempt_continues_the_byte_count_instead_of_restarting_it() {
        // Ровно то, на чём стоит К-6: после обрыва счёт не начинается с
        // нуля, и агрегации не нужно ничего складывать между попытками.
        let interrupted = samples("resume-interrupted.json");
        let stopped_at = interrupted
            .last()
            .expect("оборванная попытка что-то успела")
            .downloaded_bytes
            .expect("у downloading счётчик есть всегда");
        assert_eq!(stopped_at, 995_883);

        let continued = lines("resume-continued.json");
        assert!(
            continued
                .iter()
                .any(|line| matches!(line, StdoutLine::Resuming { byte_offset } if *byte_offset == stopped_at)),
            "вторая попытка объявляет, с какого байта продолжает"
        );

        let first = samples("resume-continued.json")[0]
            .downloaded_bytes
            .expect("у downloading счётчик есть всегда");
        assert!(
            first > stopped_at,
            "первая же строка продолженной попытки считает накопленное \
             ({first} против {stopped_at})"
        );
    }

    #[test]
    fn a_stream_that_was_already_on_disk_reports_a_finish_without_bytes() {
        // Поток может быть уже готов, и тогда о ходе его загрузки не
        // приедет ни одной строки. На argv приложения yt-dlp всё же
        // закрывает его строкой `finished` — но с `NA` вместо принятых
        // байт (М-1 ревью TL-48). Прочитать её как порчу значило бы
        // писать в лог «не разобрана» на каждом повторе.
        let lines = lines("already-downloaded.json");

        assert!(
            lines.iter().any(|line| matches!(
                line,
                StdoutLine::AlreadyDownloaded { path }
                    if *path == "<destination>/Big Buck Bunny.f140.m4a"
            )),
            "сообщение о том, что качать нечего, — есть: {lines:?}"
        );
        let progress: Vec<&ProgressSample> = lines
            .iter()
            .filter_map(|line| match line {
                StdoutLine::Progress(sample) => Some(sample),
                _ => None,
            })
            .collect();
        assert_eq!(
            progress,
            [&ProgressSample {
                status: SampleStatus::Finished,
                format_id: "140".to_owned(),
                downloaded_bytes: None,
                total_bytes: Some(323_730),
                total_bytes_estimate: None,
                speed_bytes_per_sec: None,
                eta_secs: None,
                fragment_index: None,
                fragment_count: None,
            }],
            "ни одной строки downloading, одна finished без байт с размером файла"
        );
    }

    #[test]
    fn the_phrase_of_an_already_downloaded_line_inside_the_title_does_not_cut_the_path() {
        // М-3 ревью TL-48: название ролика само несёт фразу, и обрезание
        // по первому вхождению отдало бы путь «…/Big Buck Bunny» — файл,
        // который не принадлежит ни одному потоку.
        let paths: Vec<&str> = launch_lines("phrase-in-title.json")
            .into_iter()
            .filter_map(|line| match line {
                StdoutLine::AlreadyDownloaded { path } => Some(path),
                _ => None,
            })
            .collect();

        assert_eq!(
            paths,
            ["<destination>/Big Buck Bunny has already been downloaded.f139.m4a"]
        );
        assert_eq!(
            parse_line("[download] x.m4a has already been downloaded and merged"),
            StdoutLine::Other,
            "строка с продолжением после фразы — не эта строка"
        );
    }

    #[test]
    fn the_list_of_selected_formats_is_read_exactly_as_yt_dlp_prints_it() {
        // Живые строки из двух съёмок одного запуска и из старой фикстуры
        // с объединением потоков.
        let selected = |lines: Vec<StdoutLine<'static>>| -> Vec<Vec<&'static str>> {
            lines
                .into_iter()
                .filter_map(|line| match line {
                    StdoutLine::SelectedFormats { format_ids } => Some(format_ids),
                    _ => None,
                })
                .collect()
        };

        assert_eq!(
            selected(launch_lines("video-and-audio.json")),
            [vec!["133", "139"]]
        );
        assert_eq!(
            selected(launch_lines("one-format-missing.json")),
            [vec!["133"]],
            "формат, которого нет в метаданных, из перечня пропадает"
        );
        assert_eq!(selected(lines("video-and-audio.json")), [vec!["133+139"]]);

        for other in [
            // число не совпадает с перечнем
            "[info] aqz-KE-bpKQ: Downloading 2 format(s): 133",
            // не число
            "[info] aqz-KE-bpKQ: Downloading two format(s): 133, 139",
            // пустой идентификатор
            "[info] aqz-KE-bpKQ: Downloading 2 format(s): 133, ",
            // другая строка [info]
            "[info] aqz-KE-bpKQ: Downloading subtitles: en",
        ] {
            assert_eq!(parse_line(other), StdoutLine::Other, "строка: {other:?}");
        }
    }

    #[test]
    fn destination_lines_carry_the_file_of_each_stream() {
        let destinations: Vec<&str> = lines("video-and-audio.json")
            .into_iter()
            .filter_map(|line| match line {
                StdoutLine::Destination { path } => Some(path),
                _ => None,
            })
            .collect();

        assert_eq!(destinations, vec!["./a.f133.mp4", "./a.f139.m4a"]);
    }

    #[test]
    fn service_lines_of_yt_dlp_are_not_mistaken_for_anything() {
        // Всё, что модуль не разбирает, обязано оставаться `Other`, а не
        // проваливаться в разбор по совпадению префикса.
        for line in [
            "[youtube] aqz-KE-bpKQ: Downloading webpage",
            "[hlsnative] Total fragments: 123",
            "[Merger] Merging formats into \"./a.mp4\"",
            "[FixupM4a] Correcting container of \"./e.m4a\"",
            "Deleting original file ./a.f139.m4a (pass -k to keep)",
            "",
            // Человекочитаемая полоса: под шаблоном её не бывает, но
            // если она когда-нибудь появится, спутать её со строкой
            // прогресса нельзя — числа в ней округлённые.
            "[download]  57.0% of   17.45MiB at  596.88KiB/s ETA 00:12",
        ] {
            assert_eq!(parse_line(line), StdoutLine::Other, "строка: {line:?}");
        }
    }

    #[test]
    fn a_marked_line_of_the_wrong_shape_is_reported_and_not_guessed() {
        // Маркер ставим мы; строка с ним, но не той формы, означает, что
        // yt-dlp сменил вывод. Тихо разобрать её наполовину — это дать
        // пользователю замерший процент вместо отказа.
        let live = "@tl-progress|downloading|1024|9323483|NA|288931.53689875547|32|NA|NA|133";
        assert!(matches!(parse_line(live), StdoutLine::Progress(_)));
        // `NA` вместо принятых байт у `finished` — снятый факт, а не порча.
        let finished_without_bytes = "@tl-progress|finished|NA|323730|NA|NA|NA|NA|NA|140";
        assert!(matches!(
            parse_line(finished_without_bytes),
            StdoutLine::Progress(ProgressSample {
                downloaded_bytes: None,
                ..
            })
        ));

        for broken in [
            // полем меньше
            "@tl-progress|downloading|1024|9323483|NA|288931.5|32|NA|133",
            // полем больше
            "@tl-progress|downloading|1024|9323483|NA|288931.5|32|NA|NA|133|extra",
            // незнакомый статус
            "@tl-progress|paused|1024|9323483|NA|288931.5|32|NA|NA|133",
            // у downloading принятые байты обязательны
            "@tl-progress|downloading|NA|9323483|NA|288931.5|32|NA|NA|133",
            // а у finished `NA` можно, нечитаемое число — нельзя
            "@tl-progress|finished|12x|9323483|NA|NA|NA|NA|NA|133",
            // идентификатора формата нет
            "@tl-progress|downloading|1024|9323483|NA|288931.5|32|NA|NA|NA",
            // отрицательная скорость
            "@tl-progress|downloading|1024|9323483|NA|-1|32|NA|NA|133",
        ] {
            assert!(
                matches!(parse_line(broken), StdoutLine::MalformedProgress(_)),
                "строка обязана быть отвергнута целиком: {broken:?}"
            );
        }
    }

    #[test]
    fn a_trailing_carriage_return_does_not_break_the_last_field() {
        // Без этого `133\r` не совпал бы с идентификатором формата из
        // карточки, и вся строка ушла бы в «поток не опознан».
        let sample = match parse_line(
            "@tl-progress|downloading|1024|9323483|NA|288931.5|32|NA|NA|133\r\n",
        ) {
            StdoutLine::Progress(sample) => sample,
            other => panic!("ожидалась строка прогресса, пришло {other:?}"),
        };
        assert_eq!(sample.format_id, "133");
    }
}
