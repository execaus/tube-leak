//! Четыре команды истории загрузок (контракт TL-83, тела TL-90, эпик E5).
//!
//! Тонкий слой над доменами: хранилище — [`crate::storage::history`] (TL-85),
//! показ в папке — [`crate::os_reveal`] (TL-88). Типы ответов и отказов —
//! секция E5 в [`crate::types`]. Решений о данных здесь нет: только перевод
//! домена в контракт, вынос блокирующей работы с рантайма и лог.
//!
//! # Хранилище открывается ровно один раз за процесс
//!
//! [`HistoryState`] — единственный владелец [`HistoryStore`], и его
//! единственный конструктор [`HistoryState::open`] — единственное место
//! продакшен-кода, где зовётся `HistoryStore::open`. `main.rs` зовёт его один
//! раз в `setup` и кладёт результат в состояние приложения (`Arc`), а
//! команды получают уже открытое хранилище через `State` и каталога данных
//! не видят вовсе — открыть второе им нечем. Оркестрация (TL-89) пишет Done
//! через это же состояние, а не открывает своё.
//!
//! Почему это важно, а не аккуратность: замок единственности приложения
//! работает fail-open (`Claim::Undecided`), а два `open` на испорченном
//! файле могли бы оставить соединение на уже отложенной копии (ревью TL-85).
//! Отказ открытия тоже хранится в состоянии: история на сеанс недоступна, и
//! каждая команда отвечает этой причиной, не пытаясь открыть заново.
//!
//! Сторожей два (`history_tests.rs`): поведенческий — после удаления файла
//! базы из-под открытого хранилища ни одна команда не создаёт его заново; и
//! по исходникам — вызов `HistoryStore::open(` в продакшен-коде ровно один.
//!
//! # Блокирующая работа — вне асинхронного рантайма (Н-3)
//!
//! Чтение страницы проверяет статус файла каждой записи через `metadata`
//! без потолка времени, и отключённый сетевой том задержал бы поток
//! рантайма. «Показать в папке» делает `stat` и ждёт утилиту показа до
//! потолка. Удаление и очистка — транзакции SQLite с `fsync`. Поэтому тела
//! всех четырёх команд идут через [`off_runtime`] (`spawn_blocking`).
//!
//! Шва файловой системы в домене нет, поэтому неблокирование доказывается
//! через параметры: чтение страницы ([`page_in`]) и показ ([`show_in`])
//! принимают доменную операцию аргументом. Команда передаёт настоящую
//! (`HistoryStore::page`, `os_reveal::reveal`), тест — ту же, но с рандеву
//! на `current_thread`-рантайме.
//!
//! # Перевод ошибок домена в контракт
//!
//! | домен | контракт |
//! |---|---|
//! | отказ открытия (`HistoryOpenError::reason()`) | `HistoryUnavailableError { reason }` у `history_page`, `unavailable { reason }` у остальных |
//! | каталог данных не определяется | то же, `reason: noAccess` |
//! | отказ чтения страницы или записи после открытия | `HistoryUnavailableError` / `unavailable` с `reason: noAccess` — см. ниже |
//! | `HistoryDeleteError::UnknownRecord` | `unknownRecord` |
//! | `HistoryDeleteError::Storage`, отказ `clear` | `writeFailed` |
//! | записи с таким `id` нет (`get` → `None`) | `unknownRecord` у `show_in_folder` |
//! | `HistoryRecord::file_path()` → `None` (запись правили вне приложения) | `launcherFailed` без деталей |
//! | `RevealError::FileMissing` | `fileMissing` — папка уже открыта |
//! | `RevealError::FolderMissing` | `folderMissing` |
//! | `RevealError::LauncherFailed` | `launcherFailed { exitCode, stderrTail }` |
//! | `RevealError::Rejected` | `launcherFailed` без деталей, причина в `message` |
//! | паника в блокирующем пуле | `HistoryUnavailableError` (`noAccess`) / `writeFailed` / `launcherFailed` без деталей |
//!
//! **Отказ чтения после открытия.** У `history_page` и `show_in_folder` нет
//! класса «база отказала на чтении»: причины `HistoryUnavailableReason` —
//! только про открытие. Ближайший честный класс — «история стала
//! недоступна», причина `noAccess`; подробности уходят в `message` и лог.
//! Текст экрана для `noAccess` в этом редком случае неточен — названо в
//! отчёте TL-90, контракт не менялся.
//!
//! **Паника не доходит до `invoke`.** Паника внутри асинхронной команды
//! Tauri оставила бы промис висеть навсегда (doc `commands::settings`),
//! поэтому `JoinError` переводится в типизированный отказ, а не в `unwrap`.
//!
//! # Лог (Н-1)
//!
//! Печатаются только `id` записи, класс отказа и тексты доменных ошибок —
//! ни ссылок, ни названий. Путь к файлу пользователя в лог не попадает.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tauri::{AppHandle, Manager, State};

use crate::os_reveal::{self, RevealError};
use crate::storage::history::{
    self, HistoryDeleteError, HistoryRecord, HistoryRecordsPage, HistoryStorageError, HistoryStore,
    StorageFailure,
};
use crate::types::{
    FolderDisplay, HistoryCommandError, HistoryCommandErrorKind, HistoryCursor, HistoryEntry,
    HistoryPage, HistoryUnavailableError, HistoryUnavailableReason, LauncherFailureDetails,
    ShowInFolderError, ShowInFolderErrorKind,
};

/// История загрузок этого процесса: открытое хранилище либо причина, по
/// которой оно на сеанс недоступно.
///
/// Открывается один раз ([`HistoryState::open`], doc модуля). Поле
/// приватное: другого способа получить хранилище, кроме этого конструктора,
/// нет.
#[derive(Debug)]
pub struct HistoryState {
    store: Result<HistoryStore, HistoryUnavailableError>,
}

impl HistoryState {
    /// Открывает `history.sqlite` в каталоге данных приложения.
    ///
    /// `data_dir` — `Err` с диагностикой, если каталог данных не определился
    /// (`app_data_dir()`): тогда история на сеанс недоступна с причиной
    /// `noAccess`. Приложение при любом отказе запускается (Н-4).
    pub fn open(data_dir: Result<PathBuf, String>) -> Self {
        let store = match data_dir {
            Ok(dir) => HistoryStore::open(&dir).map_err(|err| HistoryUnavailableError {
                reason: err.reason(),
                message: err.to_string(),
            }),
            Err(reason) => Err(HistoryUnavailableError {
                reason: HistoryUnavailableReason::NoAccess,
                message: format!("история: каталог данных не определяется — {reason}"),
            }),
        };
        if let Err(err) = &store {
            eprintln!(
                "history: история на этот сеанс недоступна ({:?}): {}",
                err.reason, err.message
            );
        }
        Self { store }
    }

    /// Открытое хранилище или причина, по которой его нет. Для команд здесь
    /// и для записи Done оркестрацией (TL-89).
    pub fn store(&self) -> Result<&HistoryStore, &HistoryUnavailableError> {
        self.store.as_ref()
    }
}

/// Порция истории, новые сверху (Ф-4).
///
/// `cursor` отсутствует у первой порции, у следующих это
/// [`HistoryPage::next_cursor`] предыдущего ответа. Размер порции —
/// константа ядра [`crate::types::HISTORY_PAGE_SIZE`], а не аргумент.
/// Пометки приходят только в ответе без курсора.
#[tauri::command]
pub async fn history_page(
    app: AppHandle,
    cursor: Option<HistoryCursor>,
    history: State<'_, Arc<HistoryState>>,
) -> Result<HistoryPage, HistoryUnavailableError> {
    // Тот же резолв системной «Загрузки», что у воркера очереди: по нему
    // решается `folderDisplay` записи. Путь строится без обращения к диску.
    let system_downloads = app.path().download_dir().ok();
    page_in(
        Arc::clone(&history),
        cursor,
        system_downloads,
        HistoryStore::page,
    )
    .await
}

/// Удалить одну запись (Ф-6). Файлы на диске не трогает никогда.
#[tauri::command]
pub async fn delete_history_record(
    id: String,
    history: State<'_, Arc<HistoryState>>,
) -> Result<(), HistoryCommandError> {
    delete_in(Arc::clone(&history), id).await
}

/// Очистить историю целиком одной транзакцией (Ф-6). Файлы не трогает.
#[tauri::command]
pub async fn clear_history(
    history: State<'_, Arc<HistoryState>>,
) -> Result<(), HistoryCommandError> {
    clear_in(Arc::clone(&history)).await
}

/// «Показать в папке» по `id` записи (Ф-8).
///
/// Путь ядро строит из собственной копии записи
/// ([`HistoryRecord::file_path`]), от фронтенда он не приходит. Про
/// побочный эффект у `fileMissing` — см. [`ShowInFolderErrorKind`].
#[tauri::command]
pub async fn show_in_folder(
    id: String,
    history: State<'_, Arc<HistoryState>>,
) -> Result<(), ShowInFolderError> {
    show_in(Arc::clone(&history), id, os_reveal::reveal).await
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

/// Тело `history_page` с доменным чтением параметром (doc модуля).
async fn page_in<R>(
    state: Arc<HistoryState>,
    cursor: Option<HistoryCursor>,
    system_downloads: Option<PathBuf>,
    read: R,
) -> Result<HistoryPage, HistoryUnavailableError>
where
    R: FnOnce(
            &HistoryStore,
            Option<&HistoryCursor>,
        ) -> Result<HistoryRecordsPage, HistoryStorageError>
        + Send
        + 'static,
{
    off_runtime(move || {
        let store = state.store().map_err(Clone::clone)?;
        if let Some(line) = cursor.as_ref().and_then(foreign_cursor_log) {
            eprintln!("{line}");
        }
        match read(store, cursor.as_ref()) {
            Ok(page) => Ok(page_to_contract(page, system_downloads.as_deref())),
            Err(err) => {
                eprintln!("history_page: чтение отказало: {err}");
                Err(HistoryUnavailableError {
                    reason: read_failure_reason(err.failure),
                    message: err.to_string(),
                })
            }
        }
    })
    .await
    .unwrap_or_else(|join| {
        eprintln!("history_page: чтение прервалось: {join}");
        Err(HistoryUnavailableError {
            reason: HistoryUnavailableReason::NoAccess,
            message: format!("история: чтение прервалось — {join}"),
        })
    })
}

async fn delete_in(state: Arc<HistoryState>, id: String) -> Result<(), HistoryCommandError> {
    off_runtime(move || {
        let store = state.store().map_err(unavailable_command_error)?;
        store.delete(&id).map_err(|err| {
            let kind = match &err {
                HistoryDeleteError::UnknownRecord { .. } => HistoryCommandErrorKind::UnknownRecord,
                HistoryDeleteError::Storage(_) => HistoryCommandErrorKind::WriteFailed,
            };
            eprintln!("delete_history_record: запись {id} не удалена ({kind:?}): {err}");
            HistoryCommandError {
                kind,
                message: err.to_string(),
            }
        })
    })
    .await
    .unwrap_or_else(|join| Err(interrupted_write("delete_history_record", &join)))
}

async fn clear_in(state: Arc<HistoryState>) -> Result<(), HistoryCommandError> {
    off_runtime(move || {
        let store = state.store().map_err(unavailable_command_error)?;
        store.clear().map_err(|err| {
            eprintln!("clear_history: история не очищена: {err}");
            HistoryCommandError {
                kind: HistoryCommandErrorKind::WriteFailed,
                message: err.to_string(),
            }
        })
    })
    .await
    .unwrap_or_else(|join| Err(interrupted_write("clear_history", &join)))
}

/// Тело `show_in_folder` с показом параметром (doc модуля).
async fn show_in<V>(
    state: Arc<HistoryState>,
    id: String,
    reveal: V,
) -> Result<(), ShowInFolderError>
where
    V: FnOnce(&Path) -> Result<(), RevealError> + Send + 'static,
{
    let logged_id = id.clone();
    off_runtime(move || {
        let result = show_blocking(&state, &id, reveal);
        if let Err(err) = &result {
            eprintln!(
                "show_in_folder: запись {id}: {:?} — {}",
                err.kind, err.message
            );
        }
        result
    })
    .await
    .unwrap_or_else(|join| {
        eprintln!("show_in_folder: запись {logged_id}: показ прервался: {join}");
        Err(ShowInFolderError {
            kind: launcher_failed_without_details(),
            message: format!("показ в папке прервался — {join}"),
        })
    })
}

fn show_blocking<V>(state: &HistoryState, id: &str, reveal: V) -> Result<(), ShowInFolderError>
where
    V: FnOnce(&Path) -> Result<(), RevealError>,
{
    let store = state.store().map_err(|err| ShowInFolderError {
        kind: ShowInFolderErrorKind::Unavailable { reason: err.reason },
        message: err.message.clone(),
    })?;
    let record = store
        .get(id)
        .map_err(|err| ShowInFolderError {
            kind: ShowInFolderErrorKind::Unavailable {
                reason: read_failure_reason(err.failure),
            },
            message: err.to_string(),
        })?
        .ok_or_else(|| ShowInFolderError {
            kind: ShowInFolderErrorKind::UnknownRecord,
            message: format!("история: записи {id} нет"),
        })?;
    // Путь — только через `file_path()`: запись из базы — непроверенный ввод.
    let Some(file) = record.file_path() else {
        return Err(ShowInFolderError {
            kind: launcher_failed_without_details(),
            message: format!(
                "история: путь записи {id} не строится — папка не абсолютная или имя файла \
                 не одно имя (запись изменена вне приложения)"
            ),
        });
    };
    reveal(&file).map_err(reveal_to_contract)
}

/// Исчерпывающий перевод отказа показа (таблица — у [`RevealError`]).
fn reveal_to_contract(err: RevealError) -> ShowInFolderError {
    let message = err.to_string();
    let kind = match err {
        RevealError::FileMissing => ShowInFolderErrorKind::FileMissing,
        RevealError::FolderMissing => ShowInFolderErrorKind::FolderMissing,
        RevealError::LauncherFailed(failure) => ShowInFolderErrorKind::LauncherFailed {
            details: LauncherFailureDetails {
                exit_code: failure.exit_code,
                stderr_tail: failure.stderr_tail,
            },
        },
        RevealError::Rejected(_) => launcher_failed_without_details(),
    };
    ShowInFolderError { kind, message }
}

fn launcher_failed_without_details() -> ShowInFolderErrorKind {
    ShowInFolderErrorKind::LauncherFailed {
        details: LauncherFailureDetails {
            exit_code: None,
            stderr_tail: None,
        },
    }
}

/// Класс отказа базы на чтении после открытия — в причину недоступности.
/// Отдельной причины в контракте нет (doc модуля); матч исчерпывающий,
/// чтобы новый класс хранилища не проехал сюда молча.
fn read_failure_reason(failure: StorageFailure) -> HistoryUnavailableReason {
    match failure {
        StorageFailure::NoAccess | StorageFailure::DiskFull | StorageFailure::Other => {
            HistoryUnavailableReason::NoAccess
        }
    }
}

fn unavailable_command_error(err: &HistoryUnavailableError) -> HistoryCommandError {
    HistoryCommandError {
        kind: HistoryCommandErrorKind::Unavailable { reason: err.reason },
        message: err.message.clone(),
    }
}

fn interrupted_write(command: &str, join: &tokio::task::JoinError) -> HistoryCommandError {
    eprintln!("{command}: операция прервалась: {join}");
    HistoryCommandError {
        kind: HistoryCommandErrorKind::WriteFailed,
        message: format!("история: операция прервалась — {join}"),
    }
}

/// Строка лога для курсора, которого ядро не выдавало, или `None`.
///
/// Такой курсор даёт пустую страницу (контракт), и без записи в лог
/// испорченный на стороне UI курсор терялся бы молча. Класс назван явно —
/// `foreignCursor`, чтобы его можно было искать. `id` печатается обрезанным:
/// это непроверенный ввод произвольной длины.
fn foreign_cursor_log(cursor: &HistoryCursor) -> Option<String> {
    if history::is_issued_cursor(cursor) {
        return None;
    }
    const SHOWN_CHARS: usize = 40;
    let shown: String = cursor.id.chars().take(SHOWN_CHARS).collect();
    let cut = if cursor.id.chars().count() > SHOWN_CHARS {
        "…"
    } else {
        ""
    };
    Some(format!(
        "history_page: курсор отклонён (класс foreignCursor): id {shown:?}{cut}, \
         finishedAtUnixSecs {} — ядро такой не выдавало, отдана пустая страница",
        cursor.finished_at_unix_secs
    ))
}

fn page_to_contract(page: HistoryRecordsPage, system_downloads: Option<&Path>) -> HistoryPage {
    HistoryPage {
        entries: page
            .records
            .into_iter()
            .map(|record| entry_to_contract(record, system_downloads))
            .collect(),
        next_cursor: page.next_cursor,
        notices: page.notices,
    }
}

fn entry_to_contract(record: HistoryRecord, system_downloads: Option<&Path>) -> HistoryEntry {
    HistoryEntry {
        id: record.id.to_string(),
        folder_display: folder_display(&record.folder, system_downloads),
        video_id: record.video_id,
        url: record.url,
        title: record.title,
        quality: record.quality,
        file_name: record.file_name,
        size_bytes: record.size_bytes,
        finished_at_unix_secs: record.finished_at_unix_secs,
        file_status: record.file_status,
    }
}

/// Как назвать папку записи: системная «Загрузки» или своя.
///
/// Сравнение — по компонентам пути (`Path::eq`: хвостовой разделитель не
/// важен), без обращения к диску. Символьные ссылки и регистр на
/// нечувствительных к нему томах не разрешаются: такая папка покажется
/// своим путём, что не ложь, а только менее короткая подпись. Правило для
/// `Done` выбирает TL-89 и должно совпасть с этим — иначе одна и та же
/// папка называлась бы по-разному на панели и в истории.
pub(crate) fn folder_display(folder: &Path, system_downloads: Option<&Path>) -> FolderDisplay {
    if system_downloads == Some(folder) {
        FolderDisplay::SystemDownloads
    } else {
        FolderDisplay::Custom {
            path: folder.to_string_lossy().into_owned(),
        }
    }
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;
