//! `#[tauri::command]` управления загрузкой (Ф-1, Ф-4) — TL-44.
//!
//! Тонкий слой над [`crate::download`]: резолвит папку назначения и пути к
//! бинарникам, собирает продакшен-запускатели и приёмник событий,
//! конвертирует доменный отказ [`DownloadCommandRejection`] в контрактный
//! [`DownloadCommandError`]. Автомат фаз, отмена с подчисткой и
//! финализация имени живут в домене.
//!
//! Экран зовёт три команды и все три — отсюда. Новых подписок задача не
//! добавляет: событие `download://progress` едет тем же `listen()`, на
//! который разрешение уже выдано (E1), а сторож `tests/frontend_acl.rs`
//! (урок TL-24) проверяет это сам.
//!
//! # Папка назначения — системные «Загрузки», без вопросов (Р-1)
//!
//! Не параметр команды и не настройка: решение владельца прямое, а
//! настройкой папка станет в E5, не меняя форму команды. Резолв — через
//! `tauri::path`, а не через переменные окружения: на macOS и Windows
//! папка «Загрузки» переименовывается вместе с языком системы, и знать её
//! настоящее имя умеет только ОС.

use std::sync::Arc;

use tauri::{AppHandle, Manager, State};

use super::sidecar::resolve_ytdlp_path;
use crate::download::SidecarFfmpeg;
use crate::download::{
    self, AppSink, DownloadSession, DownloadTask, SidecarDownloader, WorkerSpawn,
};
use crate::sidecar::{resolve_sidecar_path, ChildRegistry};
use crate::types::{DownloadCommandError, DownloadStarted, StartDownloadRequest};

/// Начинает загрузку выбранного пункта карточки.
///
/// Возвращается быстро — идентификатором задачи и планом, а не
/// результатом: ждать в промисе часовую работу нечего, всё идёт
/// событиями. Слот проверяется в ядре, а не только неактивной кнопкой
/// (С-13).
#[tauri::command]
pub async fn start_download(
    app: AppHandle,
    request: StartDownloadRequest,
    session: State<'_, Arc<DownloadSession>>,
) -> Result<DownloadStarted, DownloadCommandError> {
    let session = Arc::clone(&session);
    download::start_download(&session, request, TauriWorker { app })
        .await
        .map_err(|rejection| {
            eprintln!("download: старт отклонён: {rejection}");
            rejection.to_contract()
        })
}

/// Отменяет задачу в любой нетерминальной фазе (Ф-4).
#[tauri::command]
pub async fn cancel_download(
    task_id: String,
    session: State<'_, Arc<DownloadSession>>,
) -> Result<(), DownloadCommandError> {
    download::cancel_download(&session, &task_id)
        .await
        .map_err(|rejection| {
            eprintln!("download: отмена отклонена: {rejection}");
            rejection.to_contract()
        })
}

/// Продолжает ту же задачу после отказа (тот же `taskId`).
#[tauri::command]
pub async fn retry_download(
    app: AppHandle,
    task_id: String,
    session: State<'_, Arc<DownloadSession>>,
) -> Result<(), DownloadCommandError> {
    let session = Arc::clone(&session);
    download::retry_download(&session, &task_id, TauriWorker { app })
        .await
        .map_err(|rejection| {
            eprintln!("download: повтор отклонён: {rejection}");
            rejection.to_contract()
        })
}

/// Боевой запуск воркера задачи: отдельная задача рантайма Tauri.
///
/// Команда не ждёт его ни секунды — она уже вернула идентификатор
/// задачи, а всё остальное едет событиями.
struct TauriWorker {
    app: AppHandle,
}

impl WorkerSpawn for TauriWorker {
    fn spawn(
        self,
        session: Arc<DownloadSession>,
        task: Arc<DownloadTask>,
        previous: Option<Arc<DownloadTask>>,
    ) {
        let app = self.app;

        tauri::async_runtime::spawn(async move {
            // Пути резолвятся здесь, а не в команде, по двум причинам:
            // они нужны воркеру, а не вызывающему, и их неудача обязана
            // стать отказом **задачи** (событие `failed` в панели), а не
            // отказом команды — иначе один и тот же класс ошибки рисовался
            // бы то панелью, то реджектом промиса.
            let destination = match app.path().download_dir() {
                Ok(dir) => dir,
                Err(err) => {
                    // Практически недостижимо: системная папка «Загрузки»
                    // есть на всех трёх целевых ОС. Путь, которого не
                    // существует, честно доедет до класса
                    // `destinationUnavailable` проверкой в домене — своей
                    // ветки отказа заводить незачем.
                    eprintln!("download: папка «Загрузки» не определяется: {err}");
                    std::path::PathBuf::new()
                }
            };

            // `None` — готовой установки нет; домен отдаст задаче честный
            // «сбой yt-dlp», не запуская ничего постороннего.
            let ytdlp = resolve_ytdlp_path(&app).ok();
            // Пустой путь вместо имени `ffmpeg` по той же причине, что и
            // `None` выше: относительное имя ОС искала бы в `PATH`, а
            // Ф-9 требует ffmpeg из дистрибутива, а не системный.
            let ffmpeg = resolve_sidecar_path("ffmpeg").unwrap_or_default();

            let registry = app.state::<ChildRegistry>();
            let launcher = SidecarDownloader::new(ytdlp, &registry);
            let merger = SidecarFfmpeg::new(ffmpeg, &registry);
            let sink = AppSink(app.clone());

            download::run_task(
                &session,
                &task,
                &launcher,
                &merger,
                &sink,
                &destination,
                previous,
            )
            .await;
        });
    }
}
