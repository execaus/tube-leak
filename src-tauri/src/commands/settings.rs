//! Три команды настроек (контракт TL-83, тела TL-91, эпик E5).
//!
//! Тонкий слой над доменами: хранилище — [`crate::storage::settings`] (TL-87),
//! шаблон имени — [`crate::download::name_template`] (TL-86). Типы ответов и
//! отказов — секция E5 в [`crate::types`]. Решений о данных здесь нет: только
//! перевод домена в контракт, вынос блокирующей работы с рантайма и лог.
//!
//! # Хранилище открывается ровно один раз за процесс
//!
//! Приём истории (TL-90, doc `commands::history`). Открытие забирает
//! процессный флаг в `SettingsStore::open`, и второй вызов за процесс — через
//! [`SettingsState::open`] или мимо него — отклоняется, не трогая диск.
//! `main.rs` зовёт [`SettingsState::open`] один раз в `setup` и кладёт
//! результат в состояние приложения (`Arc`); команды получают уже открытое
//! хранилище и каталога данных не видят.
//!
//! Без флага второе открытие было бы не безобидным: две копии настроек в
//! памяти, из которых сохранение через одну молча затирает в файле
//! сохранённое через другую, — и испорченный файл, который первое открытие
//! уже отложило под `.broken-`, второе не нашло бы вовсе.
//!
//! Сторожей два (`settings_tests.rs`): флаг — второе открытие не трогает
//! испорченный файл и отвечает отказом; по исходникам — место единственного
//! обращения к `SettingsStore::open` (тот же сканер, что у истории, и те же
//! границы того, что он видит).
//!
//! Хранилища нет — каталог данных не определился, флаг уже взят или `setup`
//! не положил состояние. Тогда `settings_get` отвечает умолчаниями без
//! пометок сброса (они и действуют: Н-4) и на каждый такой ответ пишет в лог
//! строку с причиной: по самому ответу откат от свежего каталога данных не
//! отличить. `settings_set` отвечает `writeFailed`: файл не записан.
//!
//! # Блокирующая работа — вне асинхронного рантайма (Н-3)
//!
//! Сохранение канонизирует путь папки (`canonicalize` на подвисшем сетевом
//! томе ждёт без потолка), пишет временный файл с `sync_all` и
//! переименовывает его. Проверка `destinationFolderExists` — `stat` папки, а
//! резолв системной «Загрузок» — обращение к ОС (doc `commands::history`).
//! Поэтому сохранение целиком и проверка папки у обеих команд идут через
//! `spawn_blocking`. Чтение значений из памяти (`readout`) и предпросмотр
//! шаблона диска не трогают и выполняются на месте.
//!
//! Неблокирование доказывается как у истории: тело сохранения ([`set_in`])
//! принимает параметрами доменное сохранение и резолвер «Загрузок», тело
//! чтения ([`get_in`]) — резолвер. Команды передают настоящие
//! (`SettingsStore::set`, `download_dir()`), тесты — с рандеву на
//! `current_thread`-рантайме.
//!
//! **Граница доказанного.** Доказано одно: весь вызов `SettingsStore::set`
//! (и резолвер «Загрузок») идёт вне потока рантайма, а значит, и всё, что
//! делает `set` внутри, — `canonicalize` и запись файла отдельно не
//! подменяются, шва файловой системы в домене нет.
//!
//! **Не видит ни один тест** то, что тело [`set_in`] делает **до**
//! `off_runtime`, на потоке рантайма: мутация ревью, вставившая туда
//! `check_folder` (с `canonicalize`), осталась зелёной — рандеву стоит внутри
//! подменённого `set` и дожидается соседа уже после неё. Та же зона у
//! [`get_in`] до `off_runtime`. Держит её договорённость, а не тест: до
//! `off_runtime` обе функции диска не касаются — у `set_in` там только
//! `field_of` (разбор варианта патча), у `get_in` — чтение значений из памяти
//! (`readout`) и строка лога.
//!
//! # Перевод ошибок домена в контракт
//!
//! | домен (`SettingsSetError`) | контракт (`SettingsCommandErrorKind`) |
//! |---|---|
//! | `Folder { problem }` | `notADirectory { problem }` |
//! | `InvalidTemplate(problem)` | `invalidTemplate { problem }` |
//! | `TemplateTooLong { max }` | `invalidTemplate { problem: tooLong { max: 200 } }` |
//! | `InvalidAttempts { min, max }` | `invalidValue { min: 1, max: 20 }` |
//! | `WriteFailed` | `writeFailed` |
//! | хранилища нет, паника в блокирующем пуле | `writeFailed` |
//!
//! Перевод один ([`set_error_to_contract`]) и у сохранения, и у
//! предпросмотра: один и тот же шаблон даёт в обеих командах один и тот же
//! отказ, включая `tooLong`.
//!
//! **Паника не доходит до `invoke`.** Паника внутри асинхронной команды
//! Tauri оставила бы промис висеть навсегда, поэтому `JoinError` переводится
//! в типизированный ответ, а не в `unwrap`.
//!
//! # Лог
//!
//! Отказ сохранения печатается с полем и классом, а текст домена — в форме
//! `{:?}`: путь папки и шаблон приходят от фронтенда непроверенными, и
//! перевод строки в них не должен начинать в логе новую строку.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tauri::{AppHandle, Manager, State};

use crate::clock;
use crate::download::name_template::{validate_for_save, TemplateContext, TemplateDate};
use crate::storage::settings::{
    check_template_length, destination_exists, Settings as StoredSettings, SettingsOpenError,
    SettingsReadout, SettingsSetError, SettingsStore,
};
use crate::types::{
    QualityKind, SelectedQuality, SettingsCommandError, SettingsCommandErrorKind, SettingsField,
    SettingsPatch, SettingsView, TemplatePreview, TemplateProblem,
};

/// Образец данных предпросмотра шаблона: название ролика.
///
/// Решение неясности дизайна (сноска ¹ в пункте 3): образец один и тот же
/// везде, где строится пример имени. Полный состав и правило для даты — в
/// doc [`TemplatePreview`].
pub const PREVIEW_SAMPLE_TITLE: &str = "Как приручить дракона";

/// Образец данных предпросмотра шаблона: канонический id ролика (11 знаков,
/// форма TL-72).
pub const PREVIEW_SAMPLE_VIDEO_ID: &str = "dQw4w9WgXcQ";

/// Образец данных предпросмотра шаблона: пункт качества, `{quality}` даёт
/// `1080p`.
pub const PREVIEW_SAMPLE_QUALITY: SelectedQuality = SelectedQuality {
    kind: QualityKind::Standard,
    height_px: Some(1080),
};

/// Настройки этого процесса: открытое хранилище либо причина, по которой его
/// на сеанс нет.
///
/// Открывается один раз ([`SettingsState::open`], doc модуля). Поле
/// приватное: другого способа получить хранилище, кроме этого конструктора,
/// нет.
#[derive(Debug)]
pub struct SettingsState {
    store: Result<SettingsStore, String>,
}

/// Способ открыть хранилище: в продакшене `SettingsStore::open` с процессным
/// флагом, в тестах — без него.
type OpenStore = fn(&Path) -> Result<SettingsStore, SettingsOpenError>;

impl SettingsState {
    /// Открывает `settings.json` в каталоге данных приложения.
    ///
    /// `data_dir` — `Err` с диагностикой, если каталог данных не определился
    /// (`app_data_dir()`): тогда хранилища на сеанс нет. Приложение при любом
    /// исходе запускается (Н-4). Второй вызов за процесс диска не трогает
    /// (doc модуля).
    pub fn open(data_dir: Result<PathBuf, String>) -> Self {
        Self::open_by(data_dir, SettingsStore::open)
    }

    /// [`Self::open`] без процессного флага — только для тестов.
    #[cfg(test)]
    pub(crate) fn open_isolated(data_dir: Result<PathBuf, String>) -> Self {
        Self::open_by(data_dir, |dir| Ok(SettingsStore::open_isolated(dir)))
    }

    fn open_by(data_dir: Result<PathBuf, String>, open: OpenStore) -> Self {
        let store = match data_dir {
            Ok(dir) => open(&dir).map_err(|err| err.to_string()),
            Err(reason) => Err(format!(
                "настройки: каталог данных не определяется — {reason}"
            )),
        };
        match &store {
            Ok(store) => {
                if let Some(problem) = store.open_problem() {
                    eprintln!("settings: файл настроек прочитан не целиком: {problem}");
                }
            }
            Err(message) => {
                eprintln!("settings: хранилища настроек на этот сеанс нет, действуют умолчания: {message}");
            }
        }
        Self { store }
    }

    /// Открытое хранилище или причина, по которой его нет. Для команд здесь
    /// и для чтения настроек оркестрацией на старте задачи (TL-89).
    pub fn store(&self) -> Result<&SettingsStore, &str> {
        self.store.as_ref().map_err(String::as_str)
    }
}

/// Действующие настройки с пометками сброса (Ф-9).
///
/// Без отказов: нечитаемый файл — это умолчания с `wholeFileReset`, а не
/// ошибка команды. `destinationFolderExists` проверяется на каждый вызов.
#[tauri::command]
pub async fn settings_get(app: AppHandle) -> SettingsView {
    // `try_state`, а не `State`: асинхронная команда с заимствованным
    // аргументом обязана возвращать `Result`, а у `settings_get` отказа в
    // контракте нет. Состояние кладёт `setup` до первой команды; если его
    // всё же нет, ответ — тот же, что без хранилища.
    let state = app
        .try_state::<Arc<SettingsState>>()
        .map_or_else(unconnected, |state| Arc::clone(&state));
    // Тот же резолв системной «Загрузки», что у воркера очереди. Резолв —
    // обращение к ОС (doc модуля), поэтому передаётся замыканием.
    get_in(state, move || app.path().download_dir().ok(), log_to_stderr).await
}

/// Состояние на случай, когда `setup` его не положил: хранилища нет, причина
/// уходит в лог каждого `settings_get` (doc модуля).
fn unconnected() -> Arc<SettingsState> {
    Arc::new(SettingsState {
        store: Err("настройки: состояние не подключено".to_owned()),
    })
}

/// Строка лога в stderr — сток лога `settings_get` в продакшене.
fn log_to_stderr(line: &str) {
    eprintln!("{line}");
}

/// Сохранить **одно** поле настроек (Ф-9, Ф-11…Ф-13).
///
/// Успешный ответ — действующие настройки после сохранения, пометки сброса
/// в нём всегда пусты. Отказ не меняет ни файл, ни значения в памяти.
/// Перевод отказов — таблица в doc модуля.
#[tauri::command]
pub async fn settings_set(
    app: AppHandle,
    patch: SettingsPatch,
    settings: State<'_, Arc<SettingsState>>,
) -> Result<SettingsView, SettingsCommandError> {
    set_in(
        Arc::clone(&settings),
        patch,
        move || app.path().download_dir().ok(),
        SettingsStore::set,
    )
    .await
}

/// Пример основы имени по черновому шаблону на фиксированном образце
/// ([`TemplatePreview`]). Файл не пишет, хранилища и папки назначения не
/// требует.
///
/// Отказ — тот же `invalidTemplate`, что у `settings_set` на том же шаблоне:
/// сначала предел длины (`tooLong`), затем белый список.
///
/// Дата образца — сегодняшняя по UTC (doc [`TemplatePreview`],
/// `clock::today_utc`). `writeFailed` — только если часы ОС показывают год
/// за 9999 и образец не строится.
///
/// **Одиночный суррогат** (`"\ud800"`) в JSON-аргументе `template` сюда не
/// доходит: такую строку отклоняет разбор аргументов Tauri, и промис
/// получает строку ошибки разбора, а не `invalidTemplate`. Строка Rust не
/// может содержать суррогат, класса у такого отказа нет.
#[tauri::command]
pub async fn preview_name_template(
    template: String,
) -> Result<TemplatePreview, SettingsCommandError> {
    let Some(date) = today() else {
        return Err(SettingsCommandError {
            kind: SettingsCommandErrorKind::WriteFailed,
            message: format!(
                "preview_name_template: дата образца не строится из часов ОС ({:?})",
                clock::today_utc()
            ),
        });
    };
    preview_on(&template, date)
}

/// Сегодняшняя дата образца (doc [`preview_name_template`]).
fn today() -> Option<TemplateDate> {
    TemplateDate::from_civil(clock::today_utc())
}

/// Тело предпросмотра с датой параметром.
fn preview_on(template: &str, date: TemplateDate) -> Result<TemplatePreview, SettingsCommandError> {
    check_template_length(template).map_err(set_error_to_contract)?;
    let sample = TemplateContext {
        title: PREVIEW_SAMPLE_TITLE,
        video_id: PREVIEW_SAMPLE_VIDEO_ID,
        quality: PREVIEW_SAMPLE_QUALITY,
        date,
    };
    validate_for_save(template, &sample)
        .map(|result| TemplatePreview { result })
        .map_err(|problem| set_error_to_contract(SettingsSetError::InvalidTemplate(problem)))
}

/// Выполняет блокирующую работу в пуле `spawn_blocking`, освобождая поток
/// асинхронного рантайма (Н-3). `Err` — работа запаниковала.
async fn off_runtime<T, W>(work: W) -> Result<T, tokio::task::JoinError>
where
    T: Send + 'static,
    W: FnOnce() -> T + Send + 'static,
{
    tokio::task::spawn_blocking(work).await
}

/// Тело `settings_get` (doc модуля): значения из памяти, проверка папки с
/// резолвером «Загрузок» — вне потока рантайма. Сток лога — параметром,
/// как у `commands::history` (`page_in`).
async fn get_in<D, L>(state: Arc<SettingsState>, system_downloads: D, log: L) -> SettingsView
where
    D: FnOnce() -> Option<PathBuf> + Send + 'static,
    L: Fn(&str) + Send,
{
    let readout = match state.store() {
        Ok(store) => store.readout(),
        Err(reason) => {
            // Ответ тот же, что у свежего каталога данных, — по нему откат не
            // отличить, поэтому причина остаётся хотя бы в логе.
            log(&format!(
                "settings_get: хранилища настроек нет, ответ — умолчания: {reason}"
            ));
            SettingsReadout {
                settings: StoredSettings::default(),
                reset_fields: Vec::new(),
                whole_file_reset: false,
            }
        }
    };
    let destination = readout.settings.destination().clone();
    let exists =
        off_runtime(move || destination_exists(&destination, system_downloads().as_deref()))
            .await
            .unwrap_or_else(|join| {
                // Непроверенное отсутствие не утверждается: пометка «папки нет»
                // зовёт выбрать другую папку, а причина — сбой проверки, не диск.
                log(&format!(
                    "settings_get: проверка папки назначения прервалась: {join}"
                ));
                true
            });
    view(
        &readout.settings,
        readout.reset_fields,
        readout.whole_file_reset,
        exists,
    )
}

/// Тело `settings_set` (doc модуля): доменное сохранение и резолвер
/// «Загрузок» — параметрами, оба зовутся только вне потока рантайма.
async fn set_in<D, S>(
    state: Arc<SettingsState>,
    patch: SettingsPatch,
    system_downloads: D,
    set: S,
) -> Result<SettingsView, SettingsCommandError>
where
    D: FnOnce() -> Option<PathBuf> + Send + 'static,
    S: FnOnce(&SettingsStore, &SettingsPatch) -> Result<StoredSettings, SettingsSetError>
        + Send
        + 'static,
{
    let field = field_of(&patch);
    off_runtime(move || {
        let store = state.store().map_err(|message| SettingsCommandError {
            kind: SettingsCommandErrorKind::WriteFailed,
            message: message.to_owned(),
        })?;
        let saved = set(store, &patch).map_err(set_error_to_contract)?;
        let exists = destination_exists(saved.destination(), system_downloads().as_deref());
        // Пометки сброса после успешного сохранения всегда пусты (контракт
        // `SettingsView`, TL-87): файл переписан целиком.
        Ok(view(&saved, Vec::new(), false, exists))
    })
    .await
    .unwrap_or_else(|join| {
        Err(SettingsCommandError {
            kind: SettingsCommandErrorKind::WriteFailed,
            message: format!("настройки: сохранение прервалось — {join}"),
        })
    })
    .inspect_err(|err| {
        eprintln!(
            "settings_set: поле {field:?} не сохранено ({:?}): {:?}",
            err.kind, err.message
        );
    })
}

/// Исчерпывающий перевод отказа сохранения (таблица — в doc модуля).
fn set_error_to_contract(err: SettingsSetError) -> SettingsCommandError {
    let message = err.to_string();
    let kind = match err {
        SettingsSetError::Folder { problem, .. } => {
            SettingsCommandErrorKind::NotADirectory { problem }
        }
        SettingsSetError::InvalidTemplate(problem) => {
            SettingsCommandErrorKind::InvalidTemplate { problem }
        }
        SettingsSetError::TemplateTooLong { max, .. } => {
            SettingsCommandErrorKind::InvalidTemplate {
                problem: TemplateProblem::TooLong {
                    max: u32::try_from(max).unwrap_or(u32::MAX),
                },
            }
        }
        SettingsSetError::InvalidAttempts { min, max, .. } => {
            SettingsCommandErrorKind::InvalidValue { min, max }
        }
        SettingsSetError::WriteFailed { .. } => SettingsCommandErrorKind::WriteFailed,
    };
    SettingsCommandError { kind, message }
}

/// Какое поле сохраняется — для строки лога.
fn field_of(patch: &SettingsPatch) -> SettingsField {
    match patch {
        SettingsPatch::DestinationFolder(_) => SettingsField::DestinationFolder,
        SettingsPatch::NameTemplate(_) => SettingsField::NameTemplate,
        SettingsPatch::MaxAttempts(_) => SettingsField::MaxAttempts,
    }
}

fn view(
    settings: &StoredSettings,
    reset_fields: Vec<SettingsField>,
    whole_file_reset: bool,
    destination_folder_exists: bool,
) -> SettingsView {
    SettingsView {
        settings: settings.to_contract(),
        defaults: StoredSettings::default().to_contract(),
        reset_fields,
        whole_file_reset,
        destination_folder_exists,
    }
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;
