//! `#[tauri::command]` подготовки yt-dlp к работе (TL-12).
//!
//! Тонкий слой над [`crate::ytdlp`]: резолвит путь к вложенному архиву и
//! каталог данных приложения, сериализует одновременные вызовы и
//! конвертирует [`crate::ytdlp::PrepareError`] в контрактный
//! [`crate::types::YtDlpPrepareError`].
//!
//! # Два входа в одну и ту же подготовку
//!
//! Подготовка запускается сама при старте приложения ([`start_ytdlp_preparation`]
//! из `main.rs`) и, независимо от этого, доступна фронтенду командой
//! [`prepare_ytdlp`]. Дублирования работы не происходит: обе двери ведут в
//! [`prepare_now`], которая берёт один и тот же мьютекс и идемпотентна —
//! второй вошедший дожидается первого и застаёт готовую установку.
//!
//! Почему не только команда: приложение без yt-dlp неработоспособно, и
//! готовить его — обязанность ядра, а не экрана. Почему не только
//! автозапуск: фронтенду нужен момент «готово» и итог, а событие можно
//! пропустить, подписавшись позже — команда возвращает результат
//! независимо от того, застал ли фронтенд события.
//!
//! # Что видит фронтенд
//!
//! Ход подготовки приходит событиями `ytdlp://prepare`
//! ([`crate::types::YtDlpPrepareEvent`]). Если работа не понадобилась
//! (обычный запуск), событий нет вовсе, а промис резолвится за доли
//! секунды с `prepared: false` — экран подготовки показывать не нужно.
//! `check_sidecar` до завершения подготовки честно вернёт по yt-dlp
//! `notFound`: дерева ещё нет.

use tauri::{AppHandle, Manager};

use crate::sidecar::ChildRegistry;
use crate::types::{YtDlpPrepareError, YtDlpPrepareErrorKind, YtDlpPrepared};
use crate::ytdlp::{self, PrepareError};

/// Сериализует подготовку: автозапуск при старте и вызов команды с
/// фронтенда не должны распаковывать одно и то же дерево параллельно.
/// Второй вошедший дожидается первого и застаёт готовую установку, то
/// есть возвращается быстро и без событий.
#[derive(Debug, Default)]
pub struct PreparationLock(tokio::sync::Mutex<()>);

impl PreparationLock {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Готовит yt-dlp к работе: при необходимости распаковывает вложенное
/// onedir-дерево в каталог данных приложения и прогревает его.
///
/// Ошибка возвращается реджектом промиса значением
/// [`YtDlpPrepareError`] — типизированным объектом, а не строкой
/// (CLAUDE.md, «Ошибки типизированные, не строки»).
#[tauri::command]
pub async fn prepare_ytdlp(app: AppHandle) -> Result<YtDlpPrepared, YtDlpPrepareError> {
    prepare_now(&app).await
}

/// Запускает подготовку, не дожидаясь фронтенда.
///
/// Вызывается из `setup` в `main.rs`: пока WebView поднимается, а
/// пользователь читает первый экран, распаковка и прогрев уже идут.
/// Итог здесь никого не ждёт — его заберёт [`prepare_ytdlp`], когда
/// фронтенд спросит; в лог пишется только отказ.
pub fn start_ytdlp_preparation(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = prepare_now(&app).await {
            eprintln!(
                "yt-dlp: фоновая подготовка не удалась ({:?}): {}",
                error.kind, error.message
            );
        }
    });
}

/// Общая реализация обоих входов.
async fn prepare_now(app: &AppHandle) -> Result<YtDlpPrepared, YtDlpPrepareError> {
    let lock = app.state::<PreparationLock>();
    let _guard = lock.0.lock().await;

    let archive_path = app
        .path()
        .resolve(
            ytdlp::BUNDLED_ARCHIVE_RESOURCE,
            tauri::path::BaseDirectory::Resource,
        )
        .map_err(|err| YtDlpPrepareError {
            kind: YtDlpPrepareErrorKind::ArchiveMissing,
            message: format!(
                "в дистрибутиве не найден ресурс {}: {err}",
                ytdlp::BUNDLED_ARCHIVE_RESOURCE
            ),
        })?;

    let data_dir = app.path().app_data_dir().map_err(|err| YtDlpPrepareError {
        kind: YtDlpPrepareErrorKind::DataDirUnavailable,
        message: format!("каталог данных приложения не определяется: {err}"),
    })?;

    let registry = app.state::<ChildRegistry>();

    ytdlp::prepare(&archive_path, &data_dir, &registry, &ytdlp::AppSink(app))
        .await
        .map_err(|error: PrepareError| {
            // В лог — полная формулировка с путями; во фронтенд уходит она же,
            // но её место — «Подробнее», а решение принимается по `kind`.
            eprintln!("yt-dlp: подготовка не удалась: {error}");
            error.to_contract()
        })
}
