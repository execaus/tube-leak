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
//! [`prepare_now`], которая берёт один и тот же мьютекс, а удачный итог
//! помнит сеанс ([`crate::ytdlp::Session`], TL-23) — второй вошедший
//! дожидается первого и получает его итог, не запуская yt-dlp ещё раз.
//! До TL-23 он проверял готовую установку запуском заново, и тёплый старт
//! стоил трёх запусков yt-dlp вместо одного.
//!
//! # Фоновый прогрев (TL-21)
//!
//! Если прогрев этой установки уже упирался в таймаут, подготовка его не
//! ждёт: дерево по манифесту цело и считается готовым, а прогрев
//! продолжается задачей рантайма, которую запускает [`prepare_now`]. Экран
//! подготовки при этом не поднимается — события в `ytdlp://prepare` идут
//! только из подготовки переднего плана. Служебный экран проверяет yt-dlp
//! своим запуском, как до TL-23: версии от фонового прогрева на момент
//! проверки ещё нет.
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
use crate::ytdlp::{self, InUse, PrepareError};

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

    /// Занимает право на подготовку до конца жизни возвращённого guard'а.
    ///
    /// Метод, а не прямой доступ к полю: единственное, что разводит две
    /// двери в подготовку (команду фронтенда и автозапуск в `setup`), — этот
    /// мьютекс, и его поведение проверяется тестом
    /// `two_preparations_started_at_once_do_the_work_once`
    /// в `crate::ytdlp::prepare`, который живёт вне этого модуля.
    pub async fn acquire(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.0.lock().await
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
    let _guard = lock.acquire().await;

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
    let session = app.state::<ytdlp::Session>();

    let (prepared, background) = session
        .prepare(&archive_path, &data_dir, &registry, &ytdlp::AppSink(app))
        .await
        .map_err(|error: PrepareError| {
            // В лог — полная формулировка с путями; во фронтенд уходит она же,
            // но её место — «Подробнее», а решение принимается по `kind`.
            eprintln!("yt-dlp: подготовка не удалась: {error}");
            error.to_contract()
        })?;

    if let Some(warmup) = background {
        continue_warm_up_in_background(app, warmup);
    }

    Ok(prepared)
}

/// Продолжает прогрев, который подготовка не стала ждать (TL-21).
///
/// Задача держит отметку занятости установки всё время прогрева: уборка
/// контура обновления (Ф-7) не должна снести дерево из-под работающего
/// процесса. Процесс регистрируется в [`ChildRegistry`], как любой запуск
/// sidecar, поэтому выход из приложения его убивает.
fn continue_warm_up_in_background(app: &AppHandle, warmup: ytdlp::BackgroundWarmup) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let session = app.state::<ytdlp::Session>();
        let registry = app.state::<ChildRegistry>();
        let in_use = app.state::<InUse>();
        let _in_use = in_use.inner().mark(warmup.build_id());

        session.run_background(warmup, &registry).await;
    });
}
