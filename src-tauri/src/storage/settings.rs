//! Настройки пользователя на диске: формат, чтение со сбросом по полю,
//! атомарная запись одного поля (TL-87; Ф-9, Ф-10, Ф-11, Ф-13 эпика E5).
//!
//! Команд Tauri здесь нет — тонкий слой над модулем `crate::commands::settings`
//! (TL-91). Чтение настроек при старте задачи тоже не здесь, его делает
//! оркестрация (TL-89) через [`SettingsStore::current`]. Обе задачи
//! обращаются только к API этого модуля.
//!
//! Типы здесь свои, и сведение к контракту делают потребители — прецедент
//! истории (TL-85). Из `crate::types` приходят как есть только значения, у
//! которых вторая форма была бы дублем без смысла: [`SettingsPatch`] (вход
//! `settings_set` и так ровно одно поле), [`SettingsField`],
//! [`FolderProblem`] и [`TemplateProblem`].
//!
//! # Файл
//!
//! `settings.json` лежит в корне каталога данных приложения, рядом с
//! `queue.json` и `history.sqlite`, **вне** `yt-dlp/`, чью уборку ведёт
//! контур обновления. Форма версии 1:
//!
//! ```json
//! {
//!   "destinationFolder": { "kind": "custom", "path": "/Users/me/Movies" },
//!   "maxAttempts": 8,
//!   "nameTemplate": "{title}",
//!   "version": 1
//! }
//! ```
//!
//! `destinationFolder` — `{"kind":"system"}` либо `{"kind":"custom","path":…}`.
//! Ключи и значения выписаны в этом модуле руками, а не сериализацией
//! контрактных типов: иначе переименование варианта в контракте молча
//! сменило бы форму файла, уже лежащего у пользователей (тот же довод, что
//! у `quality_kind` в истории).
//!
//! Версия формата растёт только при несовместимом изменении (Ф-10). Новое
//! поле добавляется без неё: неизвестные ключи читатель игнорирует и
//! **сохраняет при записи**, поэтому старая сборка не стирает поле, которое
//! завела новая.
//!
//! # Таблица чтения ([`SettingsStore::open`])
//!
//! Чтение не отказывает никогда: любой исход — действующие настройки на
//! сеанс (Н-4: настройки не мешают скачивать).
//!
//! | Что на диске | Настройки | Пометка | Файл |
//! |---|---|---|---|
//! | файла нет (и каталога нет) | умолчания | — | не создаётся |
//! | пустой, не JSON, не объект, нет `version` или она не целое ≥ 1 | умолчания | `whole_file_reset` | откладывается под `settings.json.broken-<метка>` |
//! | `version` больше [`SETTINGS_FORMAT_VERSION`] | умолчания | `whole_file_reset` | **не тронут**; копия байт в байт откладывается при первом сохранении |
//! | файл не читается: отказ в доступе или каталог на месте файла | умолчания | `whole_file_reset` | откладывается под `settings.json.broken-<метка>` |
//! | файл не читается по иной причине (`EIO` на сетевом томе, таймаут) | умолчания | `whole_file_reset` | **не тронут**; копия откладывается при первом сохранении |
//! | версия 1, поле отсутствует | умолчание поля | — | — |
//! | версия 1, поле вне правил | умолчание поля, остальные как в файле | имя поля в `reset_fields` | — |
//! | версия 1, неизвестный ключ | игнорируется | — | ключ переживает запись |
//!
//! **Правила полей.** `maxAttempts` — целое JSON-число от 1 до 20 (`8.0`,
//! `"8"`, `null` — вне правил). `nameTemplate` — строка не длиннее
//! [`NAME_TEMPLATE_MAX_CHARS`] символов Unicode, которую принимает
//! [`NameTemplate::parse`]. `destinationFolder` — `system` либо `custom` с
//! абсолютным путём без компонентов `..`. Существование папки при чтении
//! **не проверяется** (Ф-11: писатели сами сообщат отказом, а отключённый
//! диск не повод забыть выбор) — это [`destination_exists`] на момент
//! запроса.
//!
//! **Будущая версия не откладывается при чтении** — доктрина
//! `ForeignVersion` TL-71: не понимать, но и не портить. Файл, написанный
//! более новой сборкой, остаётся на месте, и откат обратно на неё
//! возвращает пользователю его настройки. Пометка при этом правдива на
//! каждом запуске: файл по-прежнему не читается этой сборкой. Первое
//! сохранение неизбежно занимает имя `settings.json`, поэтому перед ним
//! прежние байты копируются под `.broken-<метка>`, и данные не теряются ни
//! в одном из двух исходов.
//!
//! **Испорченный и нечитаемый файл откладываются при чтении**, а не при
//! сохранении: они уезжают под `.broken-<метка>` переименованием, и
//! следующий запуск видит «файла нет», а не тот же мусор с той же пометкой.
//! Для нечитаемого это единственный способ не потерять настройки: `rename`
//! права чтения файла не требует, и без откладывания первое же сохранение
//! молча заняло бы его имя. Если отложить не удалось (например, каталог
//! только на чтение), файл остаётся на месте и копируется перед первым
//! сохранением, как чужая версия; у нечитаемого копия не удаётся, и
//! сохранение отказывает `WriteFailed`, не тронув файл.
//!
//! **Нечитаемый откладывается только по белому списку отказов** (TL-102):
//! отказ в доступе и каталог на месте файла — состояния диска, которые сами
//! не пройдут. Прочий отказ чтения (`EIO` на сетевом томе, таймаут, устаревший
//! дескриптор NFS) может быть временным: отложенный по нему файл следующий
//! запуск не нашёл бы, взял бы умолчания уже без пометки, и настройки,
//! целые на диске, пропали бы для пользователя молча. Такой файл остаётся на
//! месте, как чужая версия: пометка правдива на каждом запуске, пока отказ
//! держится, а первое сохранение сначала копирует байты под `.broken-` и при
//! отказе копии отказывает само.
//!
//! **Отложенная копия не затирает прежнюю**: занятая метка получает суффикс
//! `-1`, `-2`, … (приём истории). Метка — Unix-секунды.
//!
//! # Пометки «сброшено»
//!
//! `reset_fields` и `whole_file_reset` выставляет чтение файла один раз за
//! процесс и снимает первое **успешное** сохранение любого поля — обещание
//! контракта (`SettingsView`) и дизайна (пункт 3): сохранение переписывает
//! файл целиком, после него сбрасывать уже нечего. Чтение пометок их не
//! гасит: перезагрузка webview (К-13) обязана показать баннер снова.
//! Отказавшее сохранение их тоже не гасит — файл остался прежним.
//!
//! # Запись ([`SettingsStore::set`])
//!
//! Одно поле за раз: остальные два берутся из памяти, то есть из того, что
//! уже действует. Проверка значения — до диска; отказ проверки файл не
//! трогает. Запись — через `settings.json.tmp` в том же каталоге,
//! `sync_all`, `rename` (приём `queue/store.rs`): в любой момент на диске
//! либо прежний файл целиком, либо новый целиком. Отказ записи не оставляет
//! временного файла и не меняет значения в памяти. Каталог после `rename`
//! не синхронизируется — потеря записи каталога при отказе питания даёт
//! прежний файл, то есть тот же исход, что отказ записи.
//!
//! Двух писателей не бывает: приложение одно на пользователя (TL-20), а
//! хранилище открывается один раз за процесс и живёт в состоянии Tauri
//! (`manage`, TL-91). «Один раз» держит процессный флаг в
//! [`SettingsStore::open`], а не договорённость: второе открытие отклоняется
//! до диска. Два открытых хранилища — это две копии настроек в памяти, и
//! сохранение через одно молча затёрло бы в файле сохранённое через другое;
//! а второе открытие испорченного файла не нашло бы его на месте, потому что
//! первое уже отложило его под `.broken-`.
//!
//! # Замки
//!
//! Замков два, берутся всегда в одном порядке — запись, затем состояние:
//!
//! - **проверка значения** (`check_folder` с `canonicalize`, разбор шаблона,
//!   диапазон попыток) идёт до любого замка: от состояния она не зависит, а
//!   подвисший том не должен останавливать никого, кроме самого `set`;
//! - **замок записи** держится на всё сохранение — от чтения значений, из
//!   которых собирается файл, до подмены их в памяти. Сохранения поля не
//!   теряют: второе видит в памяти результат первого;
//! - **замок состояния** держится только на чтение значений для сборки
//!   файла и на финальную подмену. Диск под ним не трогается, поэтому
//!   [`SettingsStore::current`], который оркестрация зовёт на старте каждой
//!   задачи (Р-4), не ждёт ни `canonicalize`, ни копии, ни `sync_all`, ни
//!   `rename`.
//!
//! Обещание закреплено тестами на швах `canonicalize` и `rename` (указатели
//! на функции в [`SettingsStore`], в продакшене — вызовы `std::fs`):
//! `current()` из другого потока отвечает, пока шов стоит.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use serde_json::{Map, Value};

use crate::download::name_template::NameTemplate;
use crate::download::retry::MAX_ATTEMPTS;
use crate::types::{
    DestinationFolder, FolderProblem, Settings as ContractSettings, SettingsField, SettingsPatch,
    TemplateProblem,
};

// Реэкспорт для оркестрации (TL-89): контекст подстановки ей нужен, а модуль
// шаблона — нет.
pub use crate::download::name_template::{TemplateContext, TemplateDate};

/// Имя файла настроек в корне каталога данных приложения.
pub const SETTINGS_FILE_NAME: &str = "settings.json";

/// Имя временного файла записи. Постоянное: писатель один (TL-20), а
/// огрызок убитого процесса убирается при чтении одной строкой.
pub const SETTINGS_TEMP_FILE_NAME: &str = "settings.json.tmp";

/// Версия формата файла настроек.
pub const SETTINGS_FORMAT_VERSION: u64 = 1;

/// Наименьшее допустимое число попыток (Ф-13, Р-5).
pub const MIN_ATTEMPTS: u32 = 1;

/// Наибольшее допустимое число попыток (Ф-13, Р-5).
pub const MAX_ATTEMPTS_LIMIT: u32 = 20;

/// Предел длины шаблона имени в символах Unicode (добавка ведущего к TL-87
/// по ревью TL-86): разбор шаблона длину не ограничивает, а порог совпадает
/// с порогом длины пути Ф-11.
pub const NAME_TEMPLATE_MAX_CHARS: usize = 200;

/// Метка отложенной копии: `settings.json.broken-<метка>`.
const BROKEN_MARKER: &str = ".broken-";

const KEY_VERSION: &str = "version";
const KEY_DESTINATION: &str = "destinationFolder";
const KEY_TEMPLATE: &str = "nameTemplate";
const KEY_ATTEMPTS: &str = "maxAttempts";
const KEY_KIND: &str = "kind";
const KEY_PATH: &str = "path";
const KIND_SYSTEM: &str = "system";
const KIND_CUSTOM: &str = "custom";

/// Абсолютный путь своей папки назначения.
///
/// Поле приватное: значение появляется только из правила чтения (абсолютный,
/// без `..`) или из проверки [`check_folder`] (канонический, существующая
/// папка на момент сохранения). Строка, а не `PathBuf`: путь хранится в
/// JSON и уходит в контракт строкой, поэтому не-UTF-8 путь сюда не
/// попадает вовсе и потерь при преобразовании нет.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderPath(String);

impl FolderPath {
    /// Путь для файловых операций.
    pub fn as_path(&self) -> &Path {
        Path::new(&self.0)
    }

    /// Путь строкой — для контракта и файла.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Правило чтения: абсолютный путь без компонентов `..`. Диск не
    /// трогается.
    fn from_file(text: &str) -> Option<Self> {
        let path = Path::new(text);
        let lexically_clean = !path
            .components()
            .any(|component| matches!(component, Component::ParentDir));
        (path.is_absolute() && lexically_clean).then(|| Self(text.to_owned()))
    }
}

/// Папка назначения (Ф-10).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Destination {
    /// Системная «Загрузки». Её путь резолвит вызывающий
    /// (`app.path().download_dir()`), в файл он не пишется.
    #[default]
    System,
    /// Своя папка.
    Custom(FolderPath),
}

/// Действующие настройки. Каждое значение проверено: конструкторов в обход
/// правил нет.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    destination: Destination,
    name_template: NameTemplate,
    max_attempts: u32,
}

impl Default for Settings {
    /// Ф-10: системная папка, `{title}`, 8 попыток (калибровка E3).
    fn default() -> Self {
        Self {
            destination: Destination::System,
            name_template: NameTemplate::default(),
            max_attempts: MAX_ATTEMPTS,
        }
    }
}

impl Settings {
    /// Папка назначения.
    pub fn destination(&self) -> &Destination {
        &self.destination
    }

    /// Текст шаблона имени, как его сохранил пользователь.
    pub fn name_template(&self) -> &str {
        self.name_template.as_str()
    }

    /// Предел попыток на задачу, всегда в [`MIN_ATTEMPTS`]…[`MAX_ATTEMPTS_LIMIT`].
    pub fn max_attempts(&self) -> u32 {
        self.max_attempts
    }

    /// Основа имени файла по сохранённому шаблону (Ф-12) — для оркестрации
    /// (TL-89), чтобы ей не импортировать модуль шаблона. Отказов нет:
    /// шаблон проверен при чтении или сохранении.
    pub fn file_stem(&self, ctx: &TemplateContext<'_>) -> String {
        self.name_template.file_stem(ctx)
    }

    /// Форма контракта — для `settings_get`/`settings_set` (TL-91).
    pub fn to_contract(&self) -> ContractSettings {
        ContractSettings {
            destination_folder: match &self.destination {
                Destination::System => DestinationFolder::System,
                Destination::Custom(path) => DestinationFolder::Custom {
                    path: path.as_str().to_owned(),
                },
            },
            name_template: self.name_template().to_owned(),
            max_attempts: self.max_attempts,
        }
    }
}

/// Существует ли папка назначения прямо сейчас (`destinationFolderExists`
/// контракта).
///
/// Функция домена, а не поле чтения: одна дешёвая проверка на момент
/// запроса (С-6, Н-3). Зовёт её TL-91 на каждый `settings_get` и
/// `settings_set`; путь системной «Загрузок» резолвит он же, тем же
/// `download_dir()`, что воркер очереди, — `None`, если ОС его не дала.
pub fn destination_exists(destination: &Destination, system_downloads: Option<&Path>) -> bool {
    match destination {
        Destination::System => system_downloads.is_some_and(Path::is_dir),
        Destination::Custom(path) => path.as_path().is_dir(),
    }
}

/// Что показать на экране настроек: действующие значения и пометки сброса.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsReadout {
    pub settings: Settings,
    /// Поля, заменённые умолчанием при чтении. Порядок — порядок полей
    /// [`Settings`], повторов нет.
    pub reset_fields: Vec<SettingsField>,
    /// Файл не прочитался целиком.
    pub whole_file_reset: bool,
}

/// Почему файл не прочитался целиком — для лога (Н-4 E2); экран рисуется по
/// `whole_file_reset`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WholeFileProblem {
    /// Пустой, не JSON, не объект, без версии.
    #[error("настройки {path} не разбираются ({reason}); {}", aside_detail(.set_aside))]
    Malformed {
        path: PathBuf,
        reason: String,
        /// Куда отложен файл, либо почему отложить не удалось.
        set_aside: Result<PathBuf, String>,
    },
    /// Файл более новой версии формата. Не тронут.
    #[error(
        "настройки {path} версии формата {found}, эта сборка читает {supported} — файл не тронут"
    )]
    NewerVersion {
        path: PathBuf,
        found: u64,
        supported: u64,
    },
    /// Файл не читается (права, каталог на месте файла).
    #[error("настройки {path} не читаются ({reason}); {}", aside_detail(.set_aside))]
    Unreadable {
        path: PathBuf,
        reason: String,
        /// Куда отложен файл, либо почему он на месте: отложить не удалось
        /// или отказ чтения не из белого списка (шапка модуля).
        set_aside: Result<PathBuf, String>,
    },
}

fn aside_detail(set_aside: &Result<PathBuf, String>) -> String {
    match set_aside {
        Ok(path) => format!("отложен под {}", path.display()),
        Err(reason) => format!("файл на месте: {reason}"),
    }
}

/// Почему `set` не сохранил поле. Файл и значения в памяти не изменены.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SettingsSetError {
    /// Папка не принята (Ф-11). `reason` — подробности ОС для лога.
    #[error("папка назначения не принята ({problem:?}): {reason}")]
    Folder {
        problem: FolderProblem,
        reason: String,
    },
    /// Шаблон не прошёл белый список (Ф-12) — та же проблема, что даёт
    /// `name_template::validate_for_save` предпросмотру.
    #[error("шаблон имени не принят: {0:?}")]
    InvalidTemplate(TemplateProblem),
    /// Шаблон длиннее [`NAME_TEMPLATE_MAX_CHARS`]. Проверяется до разбора.
    ///
    /// В контракте — `invalidTemplate { problem: tooLong { max } }`, перевод
    /// делает `commands::settings`.
    #[error("шаблон имени длиной {found} символов, предел {max}")]
    TemplateTooLong { found: usize, max: usize },
    /// Число попыток вне пределов (Ф-13).
    #[error("число попыток {value} вне {min}…{max}")]
    InvalidAttempts { value: i64, min: u32, max: u32 },
    /// Файл не записан. Прежний файл цел, временного нет.
    #[error("настройки {path} не записаны: {reason}")]
    WriteFailed { path: PathBuf, reason: String },
}

/// Отказ проверки папки: причина контракта и текст ОС для лога.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderCheckError {
    pub problem: FolderProblem,
    pub reason: String,
}

/// Проверка своей папки из патча (правило пути `custom` в контракте,
/// Ф-11): абсолютный путь → канонизация → папка. Возвращает канонический
/// путь — его и сохраняет хранилище. Проверки на запись нет.
///
/// Канонический путь, не представимый в UTF-8, — `noAccess`: сохранить его
/// в JSON и показать без потерь нельзя, а папкой, которую нельзя назвать,
/// пользоваться нельзя (класс «прочие отказы» контракта).
///
/// На Windows `canonicalize` даёт путь с префиксом `\\?\`. Он снимается
/// здесь, до сохранения ([`without_verbatim_prefix`]): такой путь нельзя
/// показывать пользователю и сравнивать с системной «Загрузками». Хранит
/// значение домен, поэтому и снимает он, а не команда.
///
/// `canonicalize` — шов: хранилище передаёт свой (в продакшене
/// [`std::fs::canonicalize`]), тест — медленный, чтобы проверить, что
/// читатели настроек проверку не ждут.
///
/// # Errors
///
/// [`FolderProblem`] по порядку проверок; `reason` — текст для лога.
pub fn check_folder(
    path: &str,
    canonicalize: CanonicalizeFn,
) -> Result<FolderPath, FolderCheckError> {
    let fail = |problem, reason: String| FolderCheckError { problem, reason };

    if !Path::new(path).is_absolute() {
        return Err(fail(
            FolderProblem::NotAbsolute,
            format!("путь {path:?} не абсолютный"),
        ));
    }

    let canonical = canonicalize(Path::new(path)).map_err(|err| {
        let problem = match err.kind() {
            io::ErrorKind::NotFound | io::ErrorKind::NotADirectory => FolderProblem::NotFound,
            _ => FolderProblem::NoAccess,
        };
        fail(problem, format!("{path}: {err}"))
    })?;

    let metadata = fs::metadata(&canonical).map_err(|err| {
        fail(
            FolderProblem::NoAccess,
            format!("{}: {err}", canonical.display()),
        )
    })?;
    if !metadata.is_dir() {
        return Err(fail(
            FolderProblem::NotADirectory,
            format!("{} — не папка", canonical.display()),
        ));
    }

    canonical
        .into_os_string()
        .into_string()
        .map(|text| FolderPath(without_verbatim_prefix(text, cfg!(windows))))
        .map_err(|raw| {
            fail(
                FolderProblem::NoAccess,
                format!("путь {} не в UTF-8", PathBuf::from(raw).display()),
            )
        })
}

/// Путь после `canonicalize` в той форме, в какой его пишет пользователь
/// Windows: `\\?\C:\…` → `C:\…`, `\\?\UNC\сервер\шара\…` → `\\сервер\шара\…`.
///
/// Функция строковая, а ОС — параметром (`windows`): так обе ветки
/// проверяются тестом на любой машине. Вне Windows путь не меняется.
///
/// Префикс снимается, только когда путь без него значит **то же самое**.
/// `\\?\` отключает разбор пути Win32, и без префикса Windows отрезает у
/// компонента точки и пробелы на конце и превращает имя устройства (`CON`,
/// `CON.txt`) в устройство. Папка с таким компонентом могла появиться
/// только через `\\?\`, и её путь остаётся с префиксом: показать его
/// некрасиво, но подменить им другую папку нельзя. С префиксом остаются и
/// формы, у которых нет Win32-записи (`\\?\Volume{…}\`, `\\?\GLOBALROOT\`).
///
/// **Обещание «то же самое» дано только для входов, которые выдаёт
/// `canonicalize`**, — других здесь в продакшене нет (`check_folder`).
/// Строка, которой `canonicalize` не выдаёт, при снятии префикса может
/// сменить смысл, и функция её не отклоняет. Закреплено тестом-таблицей
/// `verbatim_inputs_canonicalize_never_produces_may_change_meaning`:
///
/// - `\\?\C:\a/b` → `C:\a/b`: под `\\?\` `/` — знак имени, без префикса —
///   разделитель;
/// - `\\?\UNC\server` (без share) → `\\server`;
/// - `\\?\UNC\` → `\\`.
///
/// Длина не проверяется: файловые функции std сами возвращают префикс
/// длинному абсолютному пути перед вызовом ОС.
fn without_verbatim_prefix(path: String, windows: bool) -> String {
    const VERBATIM: &str = r"\\?\";
    const VERBATIM_UNC: &str = r"\\?\UNC\";

    if !windows {
        return path;
    }
    let (plain, rest) = if let Some(rest) = path.strip_prefix(VERBATIM_UNC) {
        (format!(r"\\{rest}"), rest)
    } else if let Some(rest) = path.strip_prefix(VERBATIM) {
        let mut head = rest.chars();
        let drive = matches!(
            (head.next(), head.next(), head.next()),
            (Some(letter), Some(':'), Some('\\')) if letter.is_ascii_alphabetic()
        );
        if !drive {
            return path;
        }
        (rest.to_owned(), rest)
    } else {
        return path;
    };

    let same_meaning = rest
        .split('\\')
        .filter(|component| !component.is_empty())
        .all(|component| {
            !component.ends_with(['.', ' '])
                && !crate::download::filename::is_reserved_device_name(component)
        });
    if same_meaning {
        plain
    } else {
        path
    }
}

/// Чтение байт `settings.json` — при открытии и перед копией под `.broken-`.
/// Шов для теста «временный отказ чтения не откладывает файл»: у настоящей
/// файловой системы `EIO` по заказу не получить.
type ReadFn = fn(&Path) -> io::Result<Vec<u8>>;

/// Канонизация пути своей папки в проверке значения ([`check_folder`]). Шов
/// для теста «`current()` не ждёт `canonicalize`».
type CanonicalizeFn = fn(&Path) -> io::Result<PathBuf>;

/// Переименование на последнем шаге записи. Шов для теста атомарности:
/// отказ `rename` подменяется, а всё до него — настоящее.
type RenameFn = fn(&Path, &Path) -> io::Result<()>;

/// Метка отложенной копии. Шов для теста «прежняя копия не затёрта».
type LabelFn = fn() -> String;

/// Швы файловой системы хранилища. В продакшене — [`FsSeams::REAL`], то есть
/// ровно те вызовы `std::fs`, что стояли бы на их месте без шва; тесты
/// подменяют отдельные поля.
#[derive(Debug, Clone, Copy)]
struct FsSeams {
    read: ReadFn,
    canonicalize: CanonicalizeFn,
    rename: RenameFn,
    label: LabelFn,
}

impl FsSeams {
    const REAL: Self = Self {
        read: real_read,
        canonicalize: real_canonicalize,
        rename: real_rename,
        label: unix_secs_label,
    };
}

fn real_read(path: &Path) -> io::Result<Vec<u8>> {
    fs::read(path)
}

fn real_canonicalize(path: &Path) -> io::Result<PathBuf> {
    fs::canonicalize(path)
}

fn real_rename(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)
}

fn unix_secs_label() -> String {
    crate::clock::now_unix_secs().to_string()
}

/// Открывалось ли хранилище настроек в этом процессе ([`SettingsStore::open`]).
static OPENED_IN_PROCESS: AtomicBool = AtomicBool::new(false);

/// Почему хранилище настроек не открыто.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SettingsOpenError {
    /// Хранилище в этом процессе уже открывалось. Повторное открытие
    /// отклонено до любого обращения к диску.
    #[error(
        "настройки: хранилище уже открыто в этом процессе — повторное открытие отклонено, \
         диск не тронут"
    )]
    AlreadyOpen,
}

/// Хранилище настроек. Открывается один раз за процесс.
#[derive(Debug)]
pub struct SettingsStore {
    path: PathBuf,
    temp_path: PathBuf,
    /// Замок записи: одно сохранение за раз, от сборки файла до подмены
    /// значений в памяти. Берётся раньше `state`.
    writer: Mutex<()>,
    /// Замок состояния: только короткие чтения и подмена, диск под ним не
    /// трогается.
    state: Mutex<State>,
    open_problem: Option<WholeFileProblem>,
    seams: FsSeams,
}

#[derive(Debug)]
struct State {
    settings: Settings,
    /// Неизвестные ключи файла — пишутся обратно при сохранении.
    extras: Map<String, Value>,
    reset_fields: Vec<SettingsField>,
    whole_file_reset: bool,
    /// Под именем файла лежат байты, которые эта сборка не поняла и не
    /// отложила: перед первым сохранением их копия уходит под `.broken-`.
    preserve_before_save: bool,
}

impl SettingsStore {
    /// Читает `settings.json` из каталога данных приложения. Чтение не
    /// отказывает: таблица исходов — в шапке модуля. Файлов не создаёт;
    /// отложить испорченный файл и убрать огрызок временного — может.
    ///
    /// **Ровно один раз за процесс** (шапка модуля, «Запись»). Первый вызов
    /// забирает процессный флаг; второй и последующие отвечают
    /// [`SettingsOpenError::AlreadyOpen`], не читая, не откладывая и не
    /// удаляя ничего. Приём и цена — те же, что у `HistoryStore::open`:
    /// лишний вызов в продакшене забрал бы флаг первым, и настройки на сеанс
    /// стали бы недоступны у настоящего владельца. Место единственного вызова
    /// пинает сторож по исходникам в `commands::settings`.
    ///
    /// # Errors
    ///
    /// [`SettingsOpenError::AlreadyOpen`] — хранилище в процессе уже
    /// открывалось.
    pub fn open(data_dir: &Path) -> Result<Self, SettingsOpenError> {
        if OPENED_IN_PROCESS.swap(true, Ordering::SeqCst) {
            return Err(SettingsOpenError::AlreadyOpen);
        }
        Ok(Self::open_with(data_dir, FsSeams::REAL))
    }

    /// [`Self::open`] без процессного флага — только для тестов: открытий за
    /// тестовый процесс много, и порядок их не задан.
    #[cfg(test)]
    pub(crate) fn open_isolated(data_dir: &Path) -> Self {
        Self::open_with(data_dir, FsSeams::REAL)
    }

    fn open_with(data_dir: &Path, seams: FsSeams) -> Self {
        let path = data_dir.join(SETTINGS_FILE_NAME);
        let temp_path = data_dir.join(SETTINGS_TEMP_FILE_NAME);
        let (state, open_problem) = load(&path, &temp_path, seams);
        Self {
            path,
            temp_path,
            writer: Mutex::new(()),
            state: Mutex::new(state),
            open_problem,
            seams,
        }
    }

    /// Путь к временному файлу записи — только для тестов хранилища.
    #[cfg(test)]
    pub fn temp_path(&self) -> &Path {
        &self.temp_path
    }

    /// Текущие настройки — дешёвая копия из памяти, диск не трогается. Её
    /// снимает оркестрация при старте каждой задачи (Р-4, Ф-14).
    pub fn current(&self) -> Settings {
        self.lock().settings.clone()
    }

    /// Настройки с пометками сброса — для `settings_get`.
    pub fn readout(&self) -> SettingsReadout {
        let state = self.lock();
        SettingsReadout {
            settings: state.settings.clone(),
            reset_fields: state.reset_fields.clone(),
            whole_file_reset: state.whole_file_reset,
        }
    }

    /// Почему файл не прочитался целиком при открытии — для лога. Не
    /// гасится сохранением: это факт открытия, а не пометка экрана.
    pub fn open_problem(&self) -> Option<&WholeFileProblem> {
        self.open_problem.as_ref()
    }

    /// Сохраняет одно поле и возвращает действующие настройки.
    ///
    /// # Errors
    ///
    /// Отказ проверки значения или записи; в обоих случаях файл и память
    /// прежние, пометки сброса не сняты.
    pub fn set(&self, patch: &SettingsPatch) -> Result<Settings, SettingsSetError> {
        // Проверка — до замков: `canonicalize` на подвисшем томе не должен
        // держать ни читателей, ни очередь сохранений.
        let accepted = accept_patch(patch, self.seams.canonicalize)?;

        let _writer = self.write_lock();

        // Под замком состояния — только снимок значений для сборки файла.
        let (next, bytes, preserve_before_save) = {
            let state = self.lock();
            let next = accepted.apply(state.settings.clone());
            let bytes = file_bytes(&next, &state.extras).map_err(|err| self.write_failed(&err))?;
            (next, bytes, state.preserve_before_save)
        };

        if preserve_before_save {
            preserve_copy(&self.path, self.seams).map_err(|err| self.write_failed(&err))?;
            // Копия есть: повторная попытка после отказа ниже не плодит
            // вторую такую же.
            self.lock().preserve_before_save = false;
        }

        write_atomic(&self.path, &self.temp_path, &bytes, self.seams.rename).map_err(|err| {
            let _ = fs::remove_file(&self.temp_path);
            self.write_failed(&err)
        })?;

        let mut state = self.lock();
        state.settings = next;
        state.reset_fields.clear();
        state.whole_file_reset = false;
        Ok(state.settings.clone())
    }

    fn write_failed(&self, err: &dyn std::fmt::Display) -> SettingsSetError {
        SettingsSetError::WriteFailed {
            path: self.path.clone(),
            reason: err.to_string(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // Под мьютексом только значения, каждое из которых целиком заменяется
        // одним присваиванием; паника соседа не может оставить их наполовину
        // изменёнными.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn write_lock(&self) -> MutexGuard<'_, ()> {
        // Под замком записи данных нет: отравление ничего не говорит о
        // состоянии, файл на диске цел целиком (атомарная запись).
        self.writer.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Проверенное значение одного поля — результат проверки патча до замков.
enum Accepted {
    Destination(Destination),
    Template(NameTemplate),
    Attempts(u32),
}

impl Accepted {
    /// Настройки с этим полем; два других — из `base`.
    fn apply(self, mut base: Settings) -> Settings {
        match self {
            Self::Destination(destination) => base.destination = destination,
            Self::Template(template) => base.name_template = template,
            Self::Attempts(attempts) => base.max_attempts = attempts,
        }
        base
    }
}

/// Проверка патча. От состояния хранилища не зависит и зовётся без замков.
fn accept_patch(
    patch: &SettingsPatch,
    canonicalize: CanonicalizeFn,
) -> Result<Accepted, SettingsSetError> {
    Ok(match patch {
        SettingsPatch::DestinationFolder(DestinationFolder::System) => {
            Accepted::Destination(Destination::System)
        }
        SettingsPatch::DestinationFolder(DestinationFolder::Custom { path }) => {
            let folder =
                check_folder(path, canonicalize).map_err(|err| SettingsSetError::Folder {
                    problem: err.problem,
                    reason: err.reason,
                })?;
            Accepted::Destination(Destination::Custom(folder))
        }
        SettingsPatch::NameTemplate(text) => Accepted::Template(accept_template(text)?),
        SettingsPatch::MaxAttempts(value) => Accepted::Attempts(accept_attempts(*value)?),
    })
}

/// Шаблон из патча: сначала длина (работа на непроверенном вводе
/// ограничена), затем белый список.
fn accept_template(text: &str) -> Result<NameTemplate, SettingsSetError> {
    check_template_length(text)?;
    NameTemplate::parse(text).map_err(SettingsSetError::InvalidTemplate)
}

/// Предел длины шаблона на непроверенном вводе — одна проверка для
/// сохранения и для предпросмотра (`commands::settings`): иначе предпросмотр
/// показал бы пример по шаблону, который сохранение отклонит.
///
/// # Errors
///
/// [`SettingsSetError::TemplateTooLong`], если символов Unicode больше
/// [`NAME_TEMPLATE_MAX_CHARS`].
pub fn check_template_length(text: &str) -> Result<(), SettingsSetError> {
    let found = text.chars().count();
    if template_length_ok(found) {
        Ok(())
    } else {
        Err(SettingsSetError::TemplateTooLong {
            found,
            max: NAME_TEMPLATE_MAX_CHARS,
        })
    }
}

/// Предел длины шаблона — одно место для чтения и сохранения.
fn template_length_ok(chars: usize) -> bool {
    chars <= NAME_TEMPLATE_MAX_CHARS
}

fn attempts_ok(value: u64) -> bool {
    (u64::from(MIN_ATTEMPTS)..=u64::from(MAX_ATTEMPTS_LIMIT)).contains(&value)
}

fn accept_attempts(value: i64) -> Result<u32, SettingsSetError> {
    u64::try_from(value)
        .ok()
        .filter(|&v| attempts_ok(v))
        .and_then(|v| u32::try_from(v).ok())
        .ok_or(SettingsSetError::InvalidAttempts {
            value,
            min: MIN_ATTEMPTS,
            max: MAX_ATTEMPTS_LIMIT,
        })
}

fn load(path: &Path, temp_path: &Path, seams: FsSeams) -> (State, Option<WholeFileProblem>) {
    // Огрызок временного файла пережить запись не может: это след убитого
    // процесса, а не чужое незаконченное дело (писатель один, TL-20).
    let _ = fs::remove_file(temp_path);

    let whole_reset = |preserve_before_save| State {
        settings: Settings::default(),
        extras: Map::new(),
        reset_fields: Vec::new(),
        whole_file_reset: true,
        preserve_before_save,
    };

    let raw = match (seams.read)(path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return (
                State {
                    settings: Settings::default(),
                    extras: Map::new(),
                    reset_fields: Vec::new(),
                    whole_file_reset: false,
                    preserve_before_save: false,
                },
                None,
            );
        }
        Err(err) => {
            // Не прочитали — не значит, что байт нет. Отказ, который сам не
            // пройдёт, — убрать с дороги, как мусор, иначе первое сохранение
            // займёт имя поверх настроек. Прочий оставить на месте: копия
            // уйдёт под `.broken-` перед первым сохранением.
            let set_aside = if set_aside_on_read_error(err.kind()) {
                set_aside(path, seams.label).map_err(|err| format!("отложить не удалось: {err}"))
            } else {
                Err(
                    "отказ чтения не из белого списка (права, каталог на месте файла) \
                     и может быть временным — файл не откладывается"
                        .to_owned(),
                )
            };
            return (
                whole_reset(set_aside.is_err()),
                Some(WholeFileProblem::Unreadable {
                    path: path.to_path_buf(),
                    reason: err.to_string(),
                    set_aside,
                }),
            );
        }
    };

    match classify(&raw) {
        Classified::Current(object) => (parse_fields(object), None),
        Classified::Newer(found) => (
            whole_reset(true),
            Some(WholeFileProblem::NewerVersion {
                path: path.to_path_buf(),
                found,
                supported: SETTINGS_FORMAT_VERSION,
            }),
        ),
        Classified::Malformed(reason) => {
            let set_aside =
                set_aside(path, seams.label).map_err(|err| format!("отложить не удалось: {err}"));
            (
                whole_reset(set_aside.is_err()),
                Some(WholeFileProblem::Malformed {
                    path: path.to_path_buf(),
                    reason,
                    set_aside,
                }),
            )
        }
    }
}

/// Отказ чтения, после которого файл откладывается при открытии. Белый
/// список (шапка модуля): отказ в доступе и каталог на месте файла — на
/// Unix это `EISDIR`, на Windows открытие каталога даёт отказ в доступе.
/// Всё прочее может пройти само, и такой файл остаётся на месте.
fn set_aside_on_read_error(kind: io::ErrorKind) -> bool {
    matches!(
        kind,
        io::ErrorKind::PermissionDenied | io::ErrorKind::IsADirectory
    )
}

enum Classified {
    Current(Map<String, Value>),
    Newer(u64),
    Malformed(String),
}

fn classify(raw: &[u8]) -> Classified {
    let value: Value = match serde_json::from_slice(raw) {
        Ok(value) => value,
        Err(err) => return Classified::Malformed(format!("не JSON: {err}")),
    };
    let Value::Object(object) = value else {
        return Classified::Malformed("не JSON-объект".to_owned());
    };
    match object.get(KEY_VERSION).and_then(Value::as_u64) {
        Some(SETTINGS_FORMAT_VERSION) => Classified::Current(object),
        Some(found) if found > SETTINGS_FORMAT_VERSION => Classified::Newer(found),
        // Версии 0 не было никогда, а версия не целым числом — чужая
        // структура, а не будущий формат.
        Some(_) | None => Classified::Malformed("нет целой версии формата ≥ 1".to_owned()),
    }
}

fn parse_fields(mut object: Map<String, Value>) -> State {
    let defaults = Settings::default();
    let mut reset_fields = Vec::new();

    // Порядок разбора — порядок полей `Settings`: он же порядок пометок.
    let destination = match object.remove(KEY_DESTINATION) {
        None => defaults.destination.clone(),
        Some(value) => destination_from_file(&value).unwrap_or_else(|| {
            reset_fields.push(SettingsField::DestinationFolder);
            defaults.destination.clone()
        }),
    };
    let name_template = match object.remove(KEY_TEMPLATE) {
        None => defaults.name_template.clone(),
        Some(value) => template_from_file(&value).unwrap_or_else(|| {
            reset_fields.push(SettingsField::NameTemplate);
            defaults.name_template.clone()
        }),
    };
    let max_attempts = match object.remove(KEY_ATTEMPTS) {
        None => defaults.max_attempts,
        Some(value) => attempts_from_file(&value).unwrap_or_else(|| {
            reset_fields.push(SettingsField::MaxAttempts);
            defaults.max_attempts
        }),
    };
    object.remove(KEY_VERSION);

    State {
        settings: Settings {
            destination,
            name_template,
            max_attempts,
        },
        extras: object,
        reset_fields,
        whole_file_reset: false,
        preserve_before_save: false,
    }
}

fn destination_from_file(value: &Value) -> Option<Destination> {
    let object = value.as_object()?;
    match object.get(KEY_KIND)?.as_str()? {
        KIND_SYSTEM => Some(Destination::System),
        KIND_CUSTOM => {
            FolderPath::from_file(object.get(KEY_PATH)?.as_str()?).map(Destination::Custom)
        }
        _ => None,
    }
}

fn template_from_file(value: &Value) -> Option<NameTemplate> {
    let text = value.as_str()?;
    if !template_length_ok(text.chars().count()) {
        return None;
    }
    NameTemplate::parse(text).ok()
}

fn attempts_from_file(value: &Value) -> Option<u32> {
    value
        .as_u64()
        .filter(|&v| attempts_ok(v))
        .and_then(|v| u32::try_from(v).ok())
}

fn file_bytes(settings: &Settings, extras: &Map<String, Value>) -> serde_json::Result<Vec<u8>> {
    let mut object = extras.clone();
    object.insert(KEY_VERSION.to_owned(), Value::from(SETTINGS_FORMAT_VERSION));
    let destination = match &settings.destination {
        Destination::System => serde_json::json!({ KEY_KIND: KIND_SYSTEM }),
        Destination::Custom(path) => {
            serde_json::json!({ KEY_KIND: KIND_CUSTOM, KEY_PATH: path.as_str() })
        }
    };
    object.insert(KEY_DESTINATION.to_owned(), destination);
    object.insert(
        KEY_TEMPLATE.to_owned(),
        Value::from(settings.name_template()),
    );
    object.insert(KEY_ATTEMPTS.to_owned(), Value::from(settings.max_attempts));

    let mut bytes = serde_json::to_vec_pretty(&Value::Object(object))?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Кладёт байты под именем `path` целиком или не кладёт вовсе. Уборка
/// временного файла при отказе — на вызывающем.
fn write_atomic(path: &Path, temp_path: &Path, bytes: &[u8], rename: RenameFn) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = File::create(temp_path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);

    rename(temp_path, path)
}

/// Переименовывает испорченный файл под свободное имя
/// `settings.json.broken-<метка>[-N]`.
fn set_aside(path: &Path, label: LabelFn) -> io::Result<PathBuf> {
    let target = free_broken_name(path, &label())?;
    fs::rename(path, &target)?;
    Ok(target)
}

/// Копирует байты, лежащие под именем файла, под свободное имя
/// `.broken-<метка>[-N]` — перед первым сохранением поверх файла, который
/// эта сборка не поняла или не смогла прочитать. Файла уже нет — копировать
/// нечего. Байты читаются тем же швом, что при открытии.
fn preserve_copy(path: &Path, seams: FsSeams) -> io::Result<()> {
    let bytes = match (seams.read)(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    let target = free_broken_name(path, &(seams.label)())?;
    let written = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target)
        .and_then(|mut file| {
            file.write_all(&bytes)?;
            file.sync_all()
        });
    if written.is_err() {
        let _ = fs::remove_file(&target);
    }
    written
}

/// Имя, не занятое ничем. `rename` на Unix молча заменяет существующий
/// файл, поэтому занятость проверяется явно: прежняя отложенная копия не
/// затирается. Гонки нет — писатель один.
fn free_broken_name(path: &Path, label: &str) -> io::Result<PathBuf> {
    let base = with_suffix(path, &format!("{BROKEN_MARKER}{label}"));
    for attempt in 0..1000_u32 {
        let candidate = if attempt == 0 {
            base.clone()
        } else {
            with_suffix(&base, &format!("-{attempt}"))
        };
        match fs::symlink_metadata(&candidate) {
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(candidate),
            Err(err) => return Err(err),
            Ok(_) => {}
        }
    }
    Err(io::Error::other(
        "свободное имя для отложенных настроек не нашлось за 1000 попыток",
    ))
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;
