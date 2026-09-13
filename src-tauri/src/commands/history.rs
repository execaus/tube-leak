//! Четыре команды истории загрузок (контракт TL-83, эпик E5).
//!
//! Тонкий слой над доменом, как и остальные команды. Типы ответов и отказов
//! — секция E5 в [`crate::types`].
//!
//! # Что здесь настоящее, а что ждёт реализации
//!
//! Контрактная задача фиксирует имена, аргументы и формы ответа. На них
//! ui-задача TL-93 опирается раньше, чем появится хранилище (прецедент TL-70
//! в E4). Тела подставит TL-90 поверх хранилища TL-85 и показа в папке
//! TL-88. Пока в историю не пишет никто (запись в момент Done — TL-89), и
//! заглушки отвечают тем, что сейчас **правда**, а не выдумкой:
//!
//! - `history_page` — пустая страница без курсора: записей действительно нет;
//! - `delete_history_record` и `show_in_folder` — `unknownRecord`: ни одной
//!   записи с каким бы то ни было `id` не существует;
//! - `clear_history` — успех: очищать нечего, и повторная очистка пустой
//!   истории по контракту тоже успех.

use crate::types::{
    HistoryCommandError, HistoryCommandErrorKind, HistoryCursor, HistoryPage,
    HistoryUnavailableError, ShowInFolderError, ShowInFolderErrorKind,
};

/// Порция истории, новые сверху (Ф-4).
///
/// `cursor` отсутствует у первой порции, у следующих это
/// [`HistoryPage::next_cursor`] предыдущего ответа. Размер порции —
/// константа ядра [`crate::types::HISTORY_PAGE_SIZE`], а не аргумент.
#[tauri::command]
pub async fn history_page(
    cursor: Option<HistoryCursor>,
) -> Result<HistoryPage, HistoryUnavailableError> {
    // Курсор пустой истории листать некуда, чем бы он ни был.
    let _ = cursor;
    Ok(HistoryPage {
        entries: Vec::new(),
        next_cursor: None,
        // Пометкам неоткуда взяться: базы ещё нет, пересоздавать и не
        // сохранять в неё нечего.
        notices: Vec::new(),
    })
}

/// Удалить одну запись (Ф-6). Файлы на диске не трогает никогда.
#[tauri::command]
pub async fn delete_history_record(id: String) -> Result<(), HistoryCommandError> {
    Err(unknown_record(&id))
}

/// Очистить историю целиком одной транзакцией (Ф-6). Файлы не трогает.
#[tauri::command]
pub async fn clear_history() -> Result<(), HistoryCommandError> {
    Ok(())
}

/// «Показать в папке» по `id` записи (Ф-8).
///
/// Путь ядро строит из собственной копии записи, от фронтенда он не
/// приходит. Про побочный эффект у `fileMissing` — см.
/// [`ShowInFolderErrorKind`].
#[tauri::command]
pub async fn show_in_folder(id: String) -> Result<(), ShowInFolderError> {
    Err(ShowInFolderError {
        kind: ShowInFolderErrorKind::UnknownRecord,
        message: format!("history: записи {id} нет — история пока не ведётся"),
    })
}

fn unknown_record(id: &str) -> HistoryCommandError {
    HistoryCommandError {
        kind: HistoryCommandErrorKind::UnknownRecord,
        message: format!("history: записи {id} нет — история пока не ведётся"),
    }
}
