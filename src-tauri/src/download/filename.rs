//! Имя готового файла: санитизация названия ролика (Ф-6) и суффикс
//! коллизии (Ф-7) — TL-40.
//!
//! Название ролика приходит от YouTube и в нём бывает что угодно:
//! двоеточия и вопросительные знаки, переводы строк, `../`, точка в конце,
//! эмодзи, четыреста символов подряд. Инвариант `CLAUDE.md` — «ни один
//! компонент пути не строится из непроверенного ввода» — держится тем, что
//! между названием и файловой системой стоит ровно этот модуль, и другого
//! пути у названия нет.
//!
//! # Что здесь чистое, а что нет
//!
//! Вычисление имени — чистая функция строки в строку
//! ([`sanitized_stem`], [`candidate_name`]). Перебор кандидатов при
//! коллизии ([`claim_file_name`]) тоже чистый: чем именно занимается имя,
//! решает переданный вызывающим «претендент». Файловую систему трогает
//! одна функция — [`reserve_in_dir`], и трогает она её ровно тем
//! способом, который закрывает гонку (см. ниже).
//!
//! # Правила санитизации и откуда они взяты
//!
//! Общие на три ОС, а не «под текущую»: файл, скачанный на macOS, обязан
//! пережить копирование на Windows (Ф-6). Windows строже всех, поэтому
//! правила — её:
//!
//! | Что | Как | Источник |
//! |---|---|---|
//! | `< > : " / \ | ? *` | заменяются или выбрасываются | Microsoft, «Naming Files, Paths, and Namespaces» |
//! | символы 0–31, DEL, C1 (`char::is_control`) | выбрасываются | там же |
//! | `CON`, `PRN`, `AUX`, `NUL`, `COM0`…`COM9`, `LPT0`…`LPT9`, `CONIN$`, `CONOUT$`, `CLOCK$` | получают префикс `_` | там же |
//! | точка или пробел в конце имени | срезаются | там же |
//! | длина | не больше [`MAX_FILE_NAME_BYTES`] байт | POSIX `NAME_MAX` = 255 (ext4), APFS 255, NTFS 255 UTF-16 |
//!
//! Сверх документации Microsoft добавлено три правила, и ни одно — не
//! косметика:
//!
//! - **Управляющие символы двунаправленного письма** (U+202E и соседи,
//!   [`BIDI_CONTROLS`]) выбрасываются. Это классическая подмена имени:
//!   `отчёт\u{202e}gpj.exe` показывается пользователю как
//!   `отчётexe.jpg`. Ни одна из трёх ОС такое имя не запрещает — значит
//!   запрещаем мы.
//! - **Точка в начале** срезается вместе с точкой в конце: имя с ведущей
//!   точкой прячет готовый файл из Finder и из файловых менеджеров Linux,
//!   и пользователь получает «скачалось, но где».
//! - **Череда точек схлопывается в одну.** Требование Ф-6 про `..`
//!   выполняется буквально — в результате их не бывает вовсе, а не
//!   «не бывает целым компонентом пути». Цена — многоточие в названии
//!   («Продолжение следует...») превращается в точку; почему всё-таки так,
//!   написано у [`push_once`].
//!
//! # Что проверено живьём, а что — нет
//!
//! Живьём (macOS 15, APFS, тесты этого модуля) проверено: создание файлов
//! с получившимися именами, атомарность заявки на имя, суффикс коллизии,
//! неприкосновенность существующего файла.
//!
//! **Не проверено живьём** — вся строгость Windows: запрет `< > : " \ | ? *`,
//! зарезервированные имена устройств, срез точки и пробела в конце,
//! предел NTFS в 255 UTF-16-единиц и `MAX_PATH`. Машины с Windows у
//! проекта нет (К-1 E1 не закрыт по Р-6), поэтому правила взяты из
//! документации Microsoft и закреплены тестами на **обеих** сторонах: и
//! что запрещённое не проходит, и что безобидное название проходит
//! **без единого изменения** (`ordinary_titles_survive_untouched`).
//! Тест ловит не «сломается ли на Windows» — этого он не умеет, — а то,
//! ради чего правила и пишутся: что от них не пострадали обычные имена.
//!
//! # Чего этот модуль не делает
//!
//! Он не защищает **аргументы дочерних процессов**. Имя, начинающееся с
//! дефиса, здесь остаётся как есть (в названиях роликов дефис в начале —
//! обычное дело), и от разбора его как опции защищается тот, кто зовёт
//! процесс: склейка приписывает путям префикс `file:` и снимает этим сразу
//! и разбор пути как протокола, и разбор имени как опции (фикстура
//! `success-awkward-filename`). Правило «имя файла безопасно во всех
//! контекстах сразу» здесь не действует и не может: контексты разные, а
//! калечить название ради чужого разбора аргументов — плата не тем.
//!
//! Отдельно не проверено и не обойдено: предел пути Windows `MAX_PATH`
//! (260 символов на путь целиком). Здесь ограничивается **компонент**, а
//! не путь: длину папки назначения чистая функция не знает. Для
//! `C:\Users\<имя>\Downloads\` запаса хватает, для глубокой ручной папки
//! в E5 — может не хватить; std Rust обращается к длинным абсолютным
//! путям через `\\?\`-форму и сам такой файл создаст, а вот сторонний
//! проводник или плеер может его не открыть.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

/// Предел длины имени файла в байтах.
///
/// Байты, а не символы: и `NAME_MAX` ext4, и предел APFS считают в
/// байтах UTF-8, а кириллическое название даёт два байта на символ,
/// эмодзи — четыре. NTFS считает в UTF-16-единицах, но их всегда не
/// больше, чем байт UTF-8 (ASCII 1↔1, BMP 3↔1, за пределами BMP 4↔2),
/// поэтому предел в байтах покрывает и её.
pub const MAX_FILE_NAME_BYTES: usize = 255;

/// Сколько байт имени зарезервировано под расширение с точкой.
///
/// `.webm` — самое длинное расширение, которое может выбрать склейка
/// ([`crate::download::merge::MergeContainer`]).
const EXTENSION_TAIL_BYTES: usize = 5;

/// Сколько байт зарезервировано под суффикс коллизии.
///
/// ` (999)` — предельный суффикс, дальше [`LAST_COLLISION_INDEX`].
const COLLISION_TAIL_BYTES: usize = 6;

/// Сколько байт зарезервировано под метку рабочего файла склейки.
///
/// `.tl-merging` — вставка, которую делает
/// [`crate::download::merge::working_file_name`] из **той же** основы
/// имени. Если её не учесть здесь, предельно длинное название дало бы
/// корректное финальное имя и невозможное рабочее — то есть отказ склейки
/// на ровном месте, причём только у длинных названий. Что число совпадает
/// с реальной меткой, проверяет тест
/// `the_working_name_of_the_longest_stem_still_fits`.
const WORKING_TAIL_BYTES: usize = 11;

/// Предел длины основы имени (то, что до расширения) в байтах.
///
/// Из предела имени вычтено всё, что к основе могут дописать **позже**:
/// расширение, суффикс коллизии и метка рабочего файла склейки. Бюджет
/// один на все случаи, а не свой на каждый: основу выбирает эта задача,
/// а дописывают её три разных места в разное время, и «здесь влезет, а
/// там уже нет» — ровно тот класс дефекта, который проявляется только на
/// длинных названиях и только иногда.
pub const MAX_STEM_BYTES: usize =
    MAX_FILE_NAME_BYTES - EXTENSION_TAIL_BYTES - COLLISION_TAIL_BYTES - WORKING_TAIL_BYTES;

/// Предел длины расширения в байтах.
///
/// Расширение приходит либо от склейки (константа), либо от фактического
/// имени файла, который скачал yt-dlp, — то есть в одном из путей это
/// тоже не наш ввод, и предел ему нужен свой.
const MAX_EXTENSION_BYTES: usize = 16;

/// Последний номер, до которого доходит суффикс коллизии.
///
/// Перебор обязан быть конечным: претендент, который на любое имя
/// отвечает «занято» (сломанные права, экзотическая ФС), иначе повесил бы
/// задачу навсегда. Тысяча копий одного ролика в одной папке — уже не
/// коллизия, а состояние папки, о котором пользователю честнее сказать.
pub const LAST_COLLISION_INDEX: u32 = 999;

/// Основа запасного имени, когда от названия не осталось ничего.
const FALLBACK_STEM: &str = "video";

/// Предел длины идентификатора ролика в запасном имени.
///
/// У YouTube идентификатор — 11 символов, но приезжает он сюда из ссылки,
/// то есть тоже непроверенным.
const MAX_FALLBACK_ID_BYTES: usize = 64;

/// Символы управления двунаправленным письмом.
///
/// Выбрасываются целиком: единственное, ради чего они бывают в названии
/// ролика, — показать пользователю не то расширение, которое у файла на
/// самом деле. Соединители эмодзи (U+200D и U+200C) в список **не
/// входят**: они несут смысл (семья из одного эмодзи вместо трёх) и
/// подменить ничего не могут.
const BIDI_CONTROLS: [char; 12] = [
    '\u{061c}', '\u{200e}', '\u{200f}', '\u{202a}', '\u{202b}', '\u{202c}', '\u{202d}', '\u{202e}',
    '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
];

/// Имена устройств DOS, недопустимые как имена файлов в Windows.
///
/// Запрет действует и с расширением (`CON.mp4` — то же устройство), и на
/// сегменте до первой точки (`CON.обзор.mp4` — тоже), поэтому сравнение
/// идёт с сегментом, а не со всей основой. Надстрочные варианты
/// (`COM¹`) — из той же документации Microsoft: Windows приводит
/// надстрочные цифры к обычным.
const RESERVED_DEVICE_NAMES: [&str; 33] = [
    "CON", "PRN", "AUX", "NUL", "COM0", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
    "COM8", "COM9", "LPT0", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    "CONIN$", "CONOUT$", "CLOCK$", "COM¹", "COM²", "COM³", "LPT¹", "LPT²", "LPT³",
];

/// Основа имени файла (без расширения) из названия ролика.
///
/// `video_id` — идентификатор ролика для запасного имени: название вроде
/// `???` целиком состоит из выбрасываемых символов, и после очистки от
/// него не остаётся ничего. Пустое имя файла невозможно, поэтому у
/// функции есть запасной вариант, а не `Option`: вызывающему нечего было
/// бы с ним делать, кроме как придумать то же самое.
///
/// Результат годится и как основа финального имени, и как
/// [`crate::download::merge::MergeRequest::stem`] — бюджет длины уже
/// учитывает метку рабочего файла склейки.
pub fn sanitized_stem(title: &str, video_id: &str) -> String {
    let fitted = fit_stem(&cleaned(title), MAX_STEM_BYTES);
    if fitted.is_empty() {
        fallback_stem(video_id)
    } else {
        fitted
    }
}

/// Запасное имя, когда от названия не осталось ничего (Ф-6).
///
/// `video-<id>`: идентификатор — единственное, что в этот момент известно
/// про ролик и что отличает его от соседнего. Доверия ему при этом не
/// больше, чем названию: он приезжает из ссылки, а не из справочника.
fn fallback_stem(video_id: &str) -> String {
    // Не очистка, а белый список: идентификатор YouTube — одиннадцать
    // символов из `A-Za-z0-9_-`, то есть формат известен целиком, и
    // всё, что в него не укладывается, — не идентификатор. Это строже
    // очистки названия и намеренно: имя, целиком собранное из
    // непроверенного ввода, чинить правилами «на что заменить» незачем,
    // когда можно просто не пропустить ничего лишнего.
    let id: String = video_id
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
        .collect();
    let id = truncated(&id, MAX_FALLBACK_ID_BYTES);

    if id.is_empty() {
        // Ролик без опознавательных знаков вообще: имя всё равно обязано
        // быть, а коллизию с соседним таким же разведёт суффикс Ф-7.
        FALLBACK_STEM.to_string()
    } else {
        format!("{FALLBACK_STEM}-{id}")
    }
}

/// Имя-кандидат: основа, суффикс коллизии при `index` больше единицы и
/// расширение.
///
/// `index` — порядковый номер попытки, а не «номер копии»: первая попытка
/// (`1`) идёт без суффикса, вторая даёт ` (2)` — нумерация из требования
/// Ф-7 и из привычки браузеров, где второй файл называется `… (2)`.
///
/// Функция не доверяет своим аргументам: `stem` укорачивается под
/// фактический хвост, расширение чистится и обрезается, пустая основа
/// получает запасную. Дублирование с [`sanitized_stem`] намеренное — имя,
/// собранное этой функцией, попадает в файловую систему, и её
/// безопасность не должна зависеть от того, что вызывающий не забыл
/// сходить в санитизацию.
pub fn candidate_name(stem: &str, index: u32, extension: &str) -> String {
    let extension = fitted_extension(extension);
    let suffix = if index > 1 {
        format!(" ({index})")
    } else {
        String::new()
    };

    let tail = suffix.len()
        + if extension.is_empty() {
            0
        } else {
            1 + extension.len()
        };
    let stem = fit_stem(&cleaned(stem), MAX_FILE_NAME_BYTES.saturating_sub(tail));
    let stem = if stem.is_empty() {
        FALLBACK_STEM
    } else {
        stem.as_str()
    };

    if extension.is_empty() {
        format!("{stem}{suffix}")
    } else {
        format!("{stem}{suffix}.{extension}")
    }
}

/// Чем кончилась попытка занять имя-кандидат.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameClaim {
    /// Имя теперь наше — перебор окончен.
    Taken,
    /// Имя уже занято, нужен следующий кандидат.
    Occupied,
}

/// Подобрать имя, свободное в папке назначения (Ф-7).
///
/// Перебирает `stem.ext`, `stem (2).ext`, `stem (3).ext` … и отдаёт
/// первое, которое претендент сумел занять.
///
/// # Почему претендент, а не предикат «файл существует»
///
/// Потому что предикат не закрывает гонку, а претендент закрывает.
/// «Проверить, что файла нет» и «создать файл» — два разных момента; между
/// ними файл успевает появиться (второй экземпляр приложения, любой другой
/// процесс, сам пользователь), и проверка окажется правдой про прошлое.
/// Требование С-12 — «молчаливой перезаписи не бывает ни в каком
/// сценарии» — предикатом не выполняется в принципе, только сужается окно.
///
/// Претендент же **сам** и есть занятие имени: в продакшене это
/// `File::create_new` (см. [`reserve_in_dir`]), то есть один атомарный
/// системный вызов `open(O_CREAT|O_EXCL)`, который либо создал файл, либо
/// достоверно сообщил, что кто-то другой успел раньше. Проверять после
/// него нечего — результат и есть ответ.
///
/// Чистой при этом функция быть не перестала: сама она файловую систему
/// не трогает, а в тестах претендент — замыкание над множеством имён
/// (никакого диска и никаких флаки-гонок ФС).
///
/// # Ошибки
///
/// Ошибка претендента (нет прав, папка исчезла) не проглатывается и не
/// превращается в «занято»: перебор прекращается, ошибка уходит наверх —
/// для оркестрации это `destinationUnavailable`, а не повод дописать
/// сто одинаковых суффиксов. Исчерпание [`LAST_COLLISION_INDEX`] — тоже
/// ошибка ([`io::ErrorKind::AlreadyExists`]).
pub fn claim_file_name(
    stem: &str,
    extension: &str,
    mut claim: impl FnMut(&str) -> io::Result<NameClaim>,
) -> io::Result<String> {
    for index in 1..=LAST_COLLISION_INDEX {
        let candidate = candidate_name(stem, index, extension);
        match claim(&candidate)? {
            NameClaim::Taken => return Ok(candidate),
            NameClaim::Occupied => {}
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!(
            "в папке назначения заняты все имена от «{}» до «{}»",
            candidate_name(stem, 1, extension),
            candidate_name(stem, LAST_COLLISION_INDEX, extension)
        ),
    ))
}

/// Занятое имя в папке назначения: файл под ним уже создан и пуст.
///
/// Живёт от заявки до финализации, то есть доли секунды. Что с ним делать
/// дальше — в doc [`reserve_in_dir`].
#[derive(Debug)]
pub struct ReservedName {
    path: PathBuf,
    file_name: String,
}

impl ReservedName {
    /// Полный путь к заявке — цель переименования готового файла.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Имя без пути — то самое, что уезжает во фронтенд в
    /// [`crate::types::DownloadProgress::Done`].
    pub fn file_name(&self) -> &str {
        &self.file_name
    }

    /// Забрать имя, когда файл под ним уже лежит готовый.
    pub fn into_file_name(self) -> String {
        self.file_name
    }

    /// Снять заявку: удалить пустой файл, если финализация не состоялась
    /// (отмена, отказ склейки, ошибка переименования).
    ///
    /// Отсутствие файла ошибкой не считается — снимать заявку дважды
    /// безопасно.
    pub fn release(self) -> io::Result<()> {
        match std::fs::remove_file(&self.path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }
}

/// Занять свободное имя в папке `dir`, создав под ним пустой файл.
///
/// Единственная функция модуля, которая трогает диск, и трогает ровно
/// затем, чтобы имя перестало быть предположением: `File::create_new` —
/// это `O_EXCL`, то есть «создай, если никто не успел раньше» одним
/// атомарным вызовом.
///
/// # Как этим пользуется финализация (TL-44)
///
/// 1. Склейка закончилась, готовый файл лежит под рабочим именем
///    ([`crate::download::merge::working_file_name`]).
/// 2. `reserve_in_dir` — заявка на финальное имя.
/// 3. `std::fs::rename` рабочего файла на [`ReservedName::path`]:
///    переименование поверх существующего файла атомарно на всех трёх ОС
///    (POSIX `rename`, `MoveFileEx` с `MOVEFILE_REPLACE_EXISTING` — его
///    std и использует), и поверх затирается **наша собственная**
///    нулевая заявка, а не чужие данные.
/// 4. [`ReservedName::into_file_name`] — в событие `done`.
///
/// Если между 2 и 3 что-то пошло не так, заявку снимает
/// [`ReservedName::release`]. Имя должно вычисляться прямо здесь, в конце
/// фазы Merging, а не на старте задачи: за час загрузки одноимённый файл
/// успевает появиться (решение дизайна «Момент проверки коллизии имени»).
///
/// # Цена решения, названная прямо
///
/// Между шагами 2 и 3 в папке назначения доли секунды лежит файл
/// финального имени и нулевой длины. Формально это шов в Ф-8
/// («под финальным именем не бывает недоделанного»): если ровно в этот
/// момент выключить машину, останется пустышка. Альтернатива —
/// «проверить и переименовать» — вместо пустышки оставляет **чужой файл,
/// затёртый молча**, что С-12 запрещает прямым текстом; цена выбрана
/// осознанно и в пользу данных пользователя.
///
/// Идеальный вариант — атомарное переименование с запретом замены
/// (`renameat2(RENAME_NOREPLACE)` в Linux, `renamex_np(RENAME_EXCL)` в
/// macOS, `MoveFileEx` без флага замены в Windows) — не взят: в std его
/// нет ни в каком виде, а ради него пришлось бы завести `libc` и
/// `windows-sys` и написать три `unsafe`-ветки под три ОС, из которых
/// вживую проверялась бы одна.
pub fn reserve_in_dir(dir: &Path, stem: &str, extension: &str) -> io::Result<ReservedName> {
    let mut path = PathBuf::new();

    let file_name = claim_file_name(stem, extension, |candidate| {
        // Кандидат не содержит разделителей пути по построению
        // (`no_candidate_can_ever_escape_its_directory`), поэтому `join`
        // не может вывести за пределы папки назначения.
        let target = dir.join(candidate);
        match File::create_new(&target) {
            Ok(_) => {
                path = target;
                Ok(NameClaim::Taken)
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(NameClaim::Occupied),
            Err(error) => Err(error),
        }
    })?;

    Ok(ReservedName { path, file_name })
}

/// Посимвольная очистка названия.
///
/// Замена, а не выбрасывание, там, где выбрасывание съедает смысл:
/// `Обзор: часть 2` без двоеточия склеился бы в `Обзор часть 2`, а с
/// заменой читается как `Обзор - часть 2`. Соответствия те же, что у
/// самого yt-dlp: наши файлы лежат в одной папке с теми, что он
/// сохраняет сам, и расходиться в написании одного и того же названия
/// им незачем.
fn cleaned(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());

    for ch in raw.chars() {
        match ch {
            // Пробельные — раньше управляющих намеренно: перевод строки и
            // табуляция принадлежат обеим категориям, и выброси их как
            // управляющие — «две\nстроки» склеилось бы в «двестроки».
            // Всё пробельное (включая U+00A0, U+0085 и U+2028) приводится
            // к обычному пробелу: иначе «пробел в конце», который
            // запрещает Windows, пришлось бы ловить в десятке видов.
            c if c.is_whitespace() => push_once(&mut out, ' '),
            // Управляющие (C0 кроме пробельных, DEL, C1) — молча
            // выбрасываются: в имени файла они бессмысленны, а на части
            // ОС ещё и запрещены.
            c if c.is_control() => {}
            c if BIDI_CONTROLS.contains(&c) => {}
            // Разделители пути. Единственное место, где уход вверх по
            // дереву прекращает существовать как возможность.
            '/' | '\\' | '<' | '>' | '|' | '*' => out.push('_'),
            ':' => {
                push_once(&mut out, ' ');
                out.push('-');
            }
            '"' => out.push('\''),
            '?' => {}
            // Точка одна на любую их череду — так `..` не появляется в
            // результате ни при каком вводе (Ф-6 «`..` невозможны в
            // принципе»), а не только «не появляется целым компонентом».
            '.' => push_once(&mut out, '.'),
            c => out.push(c),
        }
    }

    out
}

/// Добавить символ, если предыдущий не такой же.
///
/// Схлопывание нужно двум символам, и по разным причинам. Пробелы:
/// выброшенный управляющий символ между двумя словами иначе оставлял бы
/// дыру, по которой видно, что имя чинили. Точки: череда точек — это и
/// есть `..`.
///
/// Цена схлопывания точек названа прямо: многоточие в названии
/// («Продолжение следует...») превращается в одну точку, а она потом
/// срезается как краевая. Это заметная потеря на обычном названии, и
/// взята она сознательно — ради инварианта, который проверяется одним
/// взглядом («результат не содержит `..`») и переживает любую будущую
/// правку правил, вместо рассуждения «`..` внутри имени безвредно, пока
/// разделителей нет».
fn push_once(out: &mut String, ch: char) {
    if !out.ends_with(ch) {
        out.push(ch);
    }
}

/// Уложить основу в бюджет байт и снять всё, что запрещено по краям.
fn fit_stem(stem: &str, budget: usize) -> String {
    let mut fitted = truncated(stem.trim_matches(is_edge_noise), budget)
        .trim_matches(is_edge_noise)
        .to_string();

    if is_reserved_device_name(&fitted) {
        // Префикс, а не суффикс: запрет висит на сегменте до первой точки,
        // и `CON.обзор` + `_` в конце остался бы тем же устройством.
        fitted.insert(0, '_');
        fitted = truncated(&fitted, budget)
            .trim_matches(is_edge_noise)
            .to_string();
    }

    fitted
}

/// Точка и пробел по краям имени: в конце их запрещает Windows (молча
/// срезая, отчего имя перестаёт совпадать с тем, что мы сообщили
/// пользователю), в начале точка прячет файл на macOS и Linux.
fn is_edge_noise(ch: char) -> bool {
    ch == '.' || ch.is_whitespace()
}

/// Обрезка по границе символа.
///
/// Предел — в байтах (см. [`MAX_FILE_NAME_BYTES`]), а резать посреди
/// символа нельзя: `&str` этого просто не переживёт, а если бы пережил —
/// получилось бы имя с половиной кириллической буквы. Сдвигаемся влево до
/// ближайшей границы; на четырёхбайтовом эмодзи это до трёх байт.
///
/// Кластеры графем при этом не защищены: составное эмодзи (флаг, семья)
/// может распасться на части. Это косметика, а не порча — распавшиеся
/// части остаются корректными символами, — и цена ей ноль зависимостей
/// вместо `unicode-segmentation` ради последнего символа обрезанного
/// названия.
fn truncated(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }

    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }

    &value[..end]
}

/// Зарезервировано ли имя под устройство Windows.
///
/// Сравнивается сегмент до первой точки и без краевых пробелов: и
/// `CON.mp4`, и `CON.обзор.mp4`, и `CON .mp4` для Windows — то же
/// устройство, что и `CON`.
fn is_reserved_device_name(stem: &str) -> bool {
    let head = stem
        .split('.')
        .next()
        .unwrap_or(stem)
        .trim_matches(char::is_whitespace);

    RESERVED_DEVICE_NAMES
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(head))
}

/// Расширение, годное в имя файла.
///
/// В одном из путей оно приходит от склейки константой, в другом — из
/// фактического имени файла, который скачал yt-dlp; второй путь так же
/// непроверен, как и название. Точки внутри выбрасываются (иначе
/// расширение само себе дописало бы суффикс), длина ограничена.
fn fitted_extension(extension: &str) -> String {
    let cleaned: String = cleaned(extension)
        .chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '.')
        .collect();

    truncated(&cleaned, MAX_EXTENSION_BYTES).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Названия, в которых нет ничего запрещённого. Санитизация обязана
    /// оставить их ровно такими, какие они есть: правило, портящее
    /// обычное имя, хуже отсутствия правила.
    const ORDINARY_TITLES: [&str; 10] = [
        "Как приручить дракона (2010)",
        "Rust 1.98 — что нового",
        "S01E02. Начало",
        "C++ vs Rust, часть 2",
        "8 бит & 16 бит",
        "#shorts",
        "100% гайд по ffmpeg",
        "日本語のタイトル",
        "🎬 премьера завтра",
        "Tom's diner [official]",
    ];

    /// Названия, каждое из которых ломает наивную склейку пути.
    const HOSTILE_TITLES: [&str; 14] = [
        "../../../etc/passwd",
        "..\\..\\Windows\\System32",
        "C:\\Users\\me\\file",
        "..",
        ".",
        "...",
        "   ",
        "?<>|*:\"/\\",
        "имя\u{0}с\u{0}нулями",
        "две\nстроки\tи\tтабы",
        "отчёт\u{202e}gpj.exe",
        ".скрытый",
        "точка в конце.",
        "CON",
    ];

    fn stem_of(title: &str) -> String {
        sanitized_stem(title, "dQw4w9WgXcQ")
    }

    #[test]
    fn ordinary_titles_survive_untouched() {
        for title in ORDINARY_TITLES {
            assert_eq!(stem_of(title), title, "безобидное название испорчено");
        }
    }

    #[test]
    fn windows_forbidden_characters_never_reach_the_name() {
        // Таблица Microsoft целиком, по символу на строку: слева — что
        // прислали, справа — во что это обязано превратиться.
        let cases = [
            ("Обзор: часть 2", "Обзор - часть 2"),
            ("Что<это>такое", "Что_это_такое"),
            ("труба|качалка", "труба_качалка"),
            ("звёзды*в*имени", "звёзды_в_имени"),
            ("он сказал \"нет\"", "он сказал 'нет'"),
            ("правда?", "правда"),
            ("AC/DC — Back in Black", "AC_DC — Back in Black"),
            ("папка\\файл", "папка_файл"),
        ];

        for (title, expected) in cases {
            assert_eq!(stem_of(title), expected, "название: {title}");
        }
    }

    #[test]
    fn control_characters_are_dropped_and_whitespace_collapses() {
        let cases = [
            ("имя\u{0}с\u{0}нулями", "имяснулями"),
            ("две\nстроки", "две строки"),
            ("таб\tвнутри", "таб внутри"),
            ("escape\u{1b}[0m", "escape[0m"),
            ("delete\u{7f}тут", "deleteтут"),
            ("c1\u{9b}тут", "c1тут"),
            ("много     пробелов", "много пробелов"),
            ("неразрывный\u{a0}пробел", "неразрывный пробел"),
            ("разделитель\u{2028}строк", "разделитель строк"),
        ];

        for (title, expected) in cases {
            assert_eq!(stem_of(title), expected, "название: {title}");
        }
    }

    #[test]
    fn bidi_overrides_are_dropped_so_the_extension_cannot_be_faked() {
        // Подмена: пользователь видит «отчётexe.jpg», а файл — .exe.
        let stem = stem_of("отчёт\u{202e}gpj.exe");

        assert_eq!(stem, "отчётgpj.exe");
        for control in BIDI_CONTROLS {
            assert!(!stem.contains(control));
        }
    }

    #[test]
    fn every_reserved_windows_device_name_gets_out_of_the_way() {
        // По кейсу на каждое имя из документации Microsoft — и отдельно
        // в трёх видах, в которых Windows видит то же устройство.
        for reserved in RESERVED_DEVICE_NAMES {
            for title in [
                reserved.to_string(),
                reserved.to_lowercase(),
                format!("{reserved}.обзор"),
                format!("{reserved} "),
            ] {
                let stem = stem_of(&title);

                assert!(
                    !is_reserved_device_name(&stem),
                    "«{title}» осталось именем устройства: {stem}"
                );
                assert!(
                    stem.to_uppercase().contains(&reserved.to_uppercase()),
                    "«{title}»: от названия должно остаться читаемое, а не заглушка"
                );
            }
        }
    }

    #[test]
    fn a_reserved_name_survives_as_a_readable_file_name() {
        assert_eq!(stem_of("CON"), "_CON");
        assert_eq!(stem_of("con"), "_con");
        assert_eq!(stem_of("CON.обзор"), "_CON.обзор");
        assert_eq!(stem_of("Console"), "Console", "не устройство, а слово");
        assert_eq!(stem_of("COM10"), "COM10", "устройств больше COM9 нет");
    }

    #[test]
    fn dots_and_spaces_never_stay_at_the_edges() {
        let cases = [
            ("точка в конце.", "точка в конце"),
            ("много точек...", "много точек"),
            ("пробел в конце   ", "пробел в конце"),
            (" пробел в начале", "пробел в начале"),
            (".скрытый", "скрытый"),
            ("..двойная", "двойная"),
            (" . смесь . ", "смесь"),
        ];

        for (title, expected) in cases {
            assert_eq!(stem_of(title), expected, "название: {title}");
        }
    }

    #[test]
    fn nothing_left_after_cleanup_falls_back_to_the_video_id() {
        for title in ["", "   ", "..", ".", "...", "?", "???", "\u{0}\n\t"] {
            assert_eq!(
                sanitized_stem(title, "dQw4w9WgXcQ"),
                "video-dQw4w9WgXcQ",
                "название: {title:?}"
            );
        }
    }

    #[test]
    fn a_video_without_a_usable_id_still_gets_a_name() {
        // Ни названия, ни идентификатора — имя всё равно обязано быть:
        // пустое имя файла не существует, а разводить одинаковые будет
        // суффикс Ф-7.
        assert_eq!(sanitized_stem("", ""), "video");
        assert_eq!(sanitized_stem("???", "../.."), "video");
        assert_eq!(sanitized_stem("", "id/со/слешами"), "video-id");
        assert_eq!(sanitized_stem("", "dQw4w9WgXcQ"), "video-dQw4w9WgXcQ");
        assert!(!sanitized_stem("", &"x".repeat(500)).is_empty());
        assert!(sanitized_stem("", &"x".repeat(500)).len() <= MAX_STEM_BYTES);
    }

    #[test]
    fn no_result_can_ever_contain_a_path_separator() {
        // Инвариант CLAUDE.md: ни один компонент пути не строится из
        // непроверенного ввода. Проверяется на итоговом имени, а не на
        // отдельном правиле, — правил несколько, а инвариант один.
        for title in HOSTILE_TITLES.iter().chain(ORDINARY_TITLES.iter()) {
            let stem = stem_of(title);

            assert!(!stem.contains('/'), "{title}: {stem}");
            assert!(!stem.contains('\\'), "{title}: {stem}");
            assert!(!stem.contains(".."), "{title}: {stem}");
            assert!(!stem.is_empty(), "{title}");
            assert!(!stem.starts_with('.'), "{title}: {stem}");
            assert!(!stem.ends_with('.'), "{title}: {stem}");
            assert!(!stem.ends_with(' '), "{title}: {stem}");
            assert!(!stem.chars().any(char::is_control), "{title}: {stem}");
            assert_eq!(Path::new(&stem).components().count(), 1, "{title}: {stem}");
        }
    }

    #[test]
    fn no_candidate_can_ever_escape_its_directory() {
        // То же, но уже про готовое имя: суффикс коллизии и расширение
        // ничего не портят и сами ничем не приезжают.
        let extensions = ["mp4", "webm", "mkv", "", "../../evil", "mp4/../..", "  "];

        for title in HOSTILE_TITLES {
            for extension in extensions {
                for index in [1, 2, 999] {
                    let name = candidate_name(&stem_of(title), index, extension);
                    let path = Path::new(&name);

                    assert!(!name.contains('/'), "{title}/{extension}: {name}");
                    assert!(!name.contains('\\'), "{title}/{extension}: {name}");
                    assert_eq!(path.components().count(), 1, "{title}/{extension}: {name}");
                    assert!(path.file_name().is_some(), "{title}/{extension}: {name}");
                    assert!(name.len() <= MAX_FILE_NAME_BYTES, "{name}");
                }
            }
        }
    }

    #[test]
    fn length_is_capped_in_bytes_without_splitting_a_character() {
        // Кириллица — два байта на символ, эмодзи — четыре: предел
        // файловой системы измеряется в байтах, а не в символах, и
        // название на 200 символов кириллицей его превышает.
        let long_cyrillic = "я".repeat(400);
        let long_emoji = "🎬".repeat(200);
        let long_ascii = "a".repeat(400);

        for title in [&long_cyrillic, &long_emoji, &long_ascii] {
            let stem = sanitized_stem(title, "dQw4w9WgXcQ");

            assert!(stem.len() <= MAX_STEM_BYTES, "{}", stem.len());
            // Символ не разрублен: строка построена, а её содержимое —
            // начало исходного названия.
            assert!(title.starts_with(&stem), "обрезка исказила начало имени");
            // И запас на предельный хвост действительно остался.
            assert!(candidate_name(&stem, 999, "webm").len() <= MAX_FILE_NAME_BYTES);
        }

        // Четырёхбайтовый эмодзи на границе бюджета не разрубается, а
        // выбрасывается целиком — иначе строки бы не существовало.
        let stem = sanitized_stem(&long_emoji, "id");
        assert_eq!(stem.chars().count(), MAX_STEM_BYTES / 4);
    }

    #[test]
    fn the_extension_survives_the_length_cap() {
        // «Ограничение длины с сохранением расширения» (Ф-6): режется
        // основа, расширение остаётся целым и остаётся последним — по
        // нему ОС выбирает приложение, а ffmpeg — мультиплексор.
        let name = candidate_name(&stem_of(&"я".repeat(400)), 1, "webm");

        assert!(name.ends_with(".webm"), "{name}");
        assert!(name.len() <= MAX_FILE_NAME_BYTES, "{}", name.len());
    }

    #[test]
    fn an_extension_is_optional_and_never_leaves_a_bare_dot() {
        assert_eq!(candidate_name("Ролик", 1, ""), "Ролик");
        assert_eq!(candidate_name("Ролик", 1, "   "), "Ролик");
        assert_eq!(candidate_name("Ролик", 2, ""), "Ролик (2)");
        assert_eq!(candidate_name("Ролик", 1, ".mp4"), "Ролик.mp4");
        assert_eq!(candidate_name("Ролик", 1, "tar.gz"), "Ролик.targz");
        assert!(candidate_name("Ролик", 1, &"z".repeat(100)).len() <= MAX_FILE_NAME_BYTES);
    }

    #[test]
    fn candidate_name_defends_itself_against_an_unsanitized_stem() {
        // Вызывающий может не сходить в санитизацию; имя всё равно не
        // должно оказаться путём.
        assert_eq!(
            candidate_name("../../etc/passwd", 1, "mp4"),
            "_._etc_passwd.mp4"
        );
        assert_eq!(candidate_name("..", 1, "mp4"), "video.mp4");
        assert_eq!(candidate_name("CON", 1, "mp4"), "_CON.mp4");
        assert_eq!(candidate_name("", 1, "mp4"), "video.mp4");
        assert_eq!(candidate_name("  ...  ", 1, "mp4"), "video.mp4");
    }

    #[test]
    fn an_ellipsis_is_the_price_of_the_rule_about_two_dots() {
        // Правило со схлопыванием точек портит обычное название — это
        // видно здесь, а не выясняется на приёмке. Если однажды решат,
        // что цена не окупает инварианта, менять придётся ровно этот тест
        // и `push_once`.
        assert_eq!(stem_of("Продолжение следует..."), "Продолжение следует");
        assert_eq!(stem_of("Часть 1... и часть 2"), "Часть 1. и часть 2");
        assert_eq!(stem_of("S01E02. Начало"), "S01E02. Начало", "одна — цела");
    }

    #[test]
    fn a_sanitized_stem_passes_through_the_composer_unchanged() {
        // Основа уезжает в два места: в склейку (рабочее имя) и сюда
        // (финальное). Если сборка имени ещё раз что-то в ней поменяет,
        // рабочий файл и готовый окажутся названы по-разному, а
        // пользователю мы сообщим третье. Заодно это проверка того, что
        // очистка идемпотентна.
        for title in HOSTILE_TITLES.iter().chain(ORDINARY_TITLES.iter()) {
            let stem = stem_of(title);

            assert_eq!(
                candidate_name(&stem, 1, "mp4"),
                format!("{stem}.mp4"),
                "название: {title}"
            );
            assert_eq!(
                candidate_name(&stem, 2, "mp4"),
                format!("{stem} (2).mp4"),
                "название: {title}"
            );
        }
    }

    #[test]
    fn the_working_name_of_the_longest_stem_still_fits() {
        // Сторож на связь с TL-42: рабочее имя склейки строится из той же
        // основы, и если бюджет её длины перестанет учитывать метку
        // `.tl-merging`, предельно длинное название начнёт валить склейку.
        use crate::download::merge::{working_file_name, MergeContainer};

        let stem = sanitized_stem(&"я".repeat(400), "id");

        for container in [
            MergeContainer::Mp4,
            MergeContainer::Webm,
            MergeContainer::Mkv,
        ] {
            let working = working_file_name(&stem, container);
            assert!(
                working.len() <= MAX_FILE_NAME_BYTES,
                "рабочее имя длиной {} байт: {working}",
                working.len()
            );
        }
    }

    /// Претендент для юнит-тестов: множество занятых имён вместо диска.
    /// Ни файловой системы, ни гонок — ровно то, ради чего перебор
    /// параметризован.
    fn occupied(taken: &mut HashSet<String>) -> impl FnMut(&str) -> io::Result<NameClaim> + '_ {
        move |candidate| {
            if taken.insert(candidate.to_string()) {
                Ok(NameClaim::Taken)
            } else {
                Ok(NameClaim::Occupied)
            }
        }
    }

    #[test]
    fn the_second_and_third_copy_get_browser_style_suffixes() {
        let mut taken = HashSet::new();

        let first = claim_file_name("Ролик", "mp4", occupied(&mut taken)).unwrap();
        let second = claim_file_name("Ролик", "mp4", occupied(&mut taken)).unwrap();
        let third = claim_file_name("Ролик", "mp4", occupied(&mut taken)).unwrap();

        assert_eq!(first, "Ролик.mp4");
        assert_eq!(second, "Ролик (2).mp4");
        assert_eq!(third, "Ролик (3).mp4");
    }

    #[test]
    fn a_free_name_is_taken_on_the_first_try() {
        let mut taken = HashSet::from(["Другой.mp4".to_string()]);

        assert_eq!(
            claim_file_name("Ролик", "mp4", occupied(&mut taken)).unwrap(),
            "Ролик.mp4"
        );
    }

    #[test]
    fn a_claim_failure_stops_the_search_instead_of_counting_up() {
        // Нет прав на папку — это не «имя занято»: дописывать суффиксы
        // тысячу раз значило бы превратить отказ папки в зависание.
        let mut seen = 0;
        let error = claim_file_name("Ролик", "mp4", |_| {
            seen += 1;
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        })
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(seen, 1, "перебор обязан прекратиться на первой же ошибке");
    }

    #[test]
    fn an_exhausted_search_fails_instead_of_looping_forever() {
        let mut seen = 0;
        let error = claim_file_name("Ролик", "mp4", |_| {
            seen += 1;
            Ok(NameClaim::Occupied)
        })
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(seen, LAST_COLLISION_INDEX as usize);
    }

    #[test]
    fn a_reservation_creates_the_file_it_promises() {
        let dir = tempfile::tempdir().unwrap();

        let reserved = reserve_in_dir(dir.path(), "Ролик", "mp4").unwrap();

        assert_eq!(reserved.file_name(), "Ролик.mp4");
        assert_eq!(reserved.path(), dir.path().join("Ролик.mp4"));
        assert!(reserved.path().exists(), "заявка обязана быть материальной");
        assert_eq!(std::fs::metadata(reserved.path()).unwrap().len(), 0);
    }

    #[test]
    fn a_second_download_never_touches_the_first_file() {
        // С-12 и К-4 буквально: второй файл получает суффикс, первый не
        // изменён — ни размером, ни содержимым.
        let dir = tempfile::tempdir().unwrap();
        let first = reserve_in_dir(dir.path(), "Ролик", "mp4").unwrap();
        std::fs::write(first.path(), "готовый файл").unwrap();

        let second = reserve_in_dir(dir.path(), "Ролик", "mp4").unwrap();
        let third = reserve_in_dir(dir.path(), "Ролик", "mp4").unwrap();

        assert_eq!(second.file_name(), "Ролик (2).mp4");
        assert_eq!(third.file_name(), "Ролик (3).mp4");
        assert_eq!(
            std::fs::read_to_string(first.path()).unwrap(),
            "готовый файл"
        );
    }

    #[test]
    fn a_reservation_loses_to_whoever_created_the_file_first() {
        // Смысл O_EXCL: имя, появившееся между «проверить» и «создать»,
        // мы не перезаписываем, а обходим.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Ролик.mp4"), "чужой файл").unwrap();

        let reserved = reserve_in_dir(dir.path(), "Ролик", "mp4").unwrap();

        assert_eq!(reserved.file_name(), "Ролик (2).mp4");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("Ролик.mp4")).unwrap(),
            "чужой файл"
        );
    }

    #[test]
    fn a_directory_with_the_same_name_is_an_occupied_name_too() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("Ролик.mp4")).unwrap();

        let reserved = reserve_in_dir(dir.path(), "Ролик", "mp4").unwrap();

        assert_eq!(reserved.file_name(), "Ролик (2).mp4");
        assert!(dir.path().join("Ролик.mp4").is_dir());
    }

    #[test]
    fn releasing_a_reservation_leaves_the_folder_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let reserved = reserve_in_dir(dir.path(), "Ролик", "mp4").unwrap();
        let path = reserved.path().to_path_buf();

        reserved.release().unwrap();

        assert!(!path.exists());
        // И имя снова свободно — отменённая задача не занимает его навсегда.
        assert_eq!(
            reserve_in_dir(dir.path(), "Ролик", "mp4")
                .unwrap()
                .into_file_name(),
            "Ролик.mp4"
        );
    }

    #[test]
    fn releasing_a_reservation_that_is_already_gone_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let reserved = reserve_in_dir(dir.path(), "Ролик", "mp4").unwrap();
        std::fs::remove_file(reserved.path()).unwrap();

        assert!(reserved.release().is_ok());
    }

    #[test]
    fn a_missing_destination_is_an_error_and_not_a_collision() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("исчезла");

        let error = reserve_in_dir(&missing, "Ролик", "mp4").unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn a_hostile_title_still_produces_a_file_that_can_be_created() {
        // Последняя проверка правил — не текстом, а файловой системой:
        // под каждым получившимся именем файл действительно создаётся.
        // Живьём это macOS/APFS; поведение Windows тем же способом
        // проверить негде (см. шапку модуля).
        let dir = tempfile::tempdir().unwrap();

        for (index, title) in HOSTILE_TITLES.iter().enumerate() {
            let stem = sanitized_stem(title, &format!("id{index}"));
            let reserved = reserve_in_dir(dir.path(), &stem, "mp4").unwrap();

            assert!(reserved.path().is_file(), "название: {title}");
            assert_eq!(
                reserved.path().parent(),
                Some(dir.path()),
                "название {title} увело файл из папки назначения"
            );
        }
    }
}
