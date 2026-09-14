//! Команды очереди загрузок и её боевое окружение (контракт TL-70,
//! планировщик TL-73).
//!
//! Тонкий слой над доменом, как и остальные команды: ни одного решения о
//! том, что и когда выполнять, здесь нет — их принимает
//! [`crate::queue::scheduler`]. Здесь собирается то, чего у него быть не
//! может: `AppHandle`, реестр процессов, пути к бинарникам и состояние
//! контура обновления.
//!
//! # Одно окружение на три шва
//!
//! [`AppQueue`] реализует [`QueueEnv`] целиком: событие `queue://changed`
//! в окно, запуск воркера задачи и граница обновления yt-dlp. Собирается
//! оно на каждый вызов команды и уезжает воркеру — тем же объектом он
//! возвращает управление очереди, закончив задачу.
//!
//! Разрешение канала `queue://changed` выдано контрактной задачей (TL-70)
//! и раньше первого подписчика (урок TL-24); сторож —
//! `tests/frontend_acl.rs`.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager, State};

use super::sidecar::resolve_ytdlp_path;
use super::update::trigger_broken_extraction_check;
use super::{HistoryState, SettingsState};
use crate::download::{self, AppSink, DownloadTask, SidecarDownloader, SidecarFfmpeg, TaskEnv};
use crate::queue::scheduler::{self, QueueEnv, QueueScheduler};
use crate::queue::CHANGED_EVENT;
use crate::sidecar::{resolve_sidecar_path, ChildRegistry, YtDlpJsRuntime};
use crate::types::{DownloadCommandError, DownloadErrorKind, DownloadProgress, QueueSnapshot};
use crate::ytdlp::UpdateController;

/// Боевое окружение очереди.
pub struct AppQueue {
    app: AppHandle,
}

/// Окружение для одного вызова команды.
///
/// Собирается заново, а не хранится состоянием приложения: внутри одна
/// ссылка на `AppHandle`, клонирование которой стоит меньше, чем поиск в
/// таблице состояний, — а хранение потребовало бы завести его после
/// `build`, то есть в третьем месте.
pub fn app_queue(app: &AppHandle) -> Arc<dyn QueueEnv> {
    Arc::new(AppQueue { app: app.clone() })
}

impl QueueEnv for AppQueue {
    fn emit(&self, snapshot: QueueSnapshot) {
        // Неотправленное событие — не повод бросать очередь: работа
        // важнее индикатора, а снимок фронтенд заберёт командой
        // `queue_state` при следующем открытии экрана (то же решение,
        // что в подготовке yt-dlp и в скачивании ролика).
        if let Err(err) = self.app.emit(CHANGED_EVENT, snapshot) {
            eprintln!("queue: событие состава не отправлено: {err}");
        }
    }

    fn spawn(self: Arc<Self>, scheduler: Arc<QueueScheduler>, task: Arc<DownloadTask>) {
        let app = self.app.clone();

        tauri::async_runtime::spawn(async move {
            // Воркер задачи — отдельная вложенная задача рантайма, и это
            // не украшение. Слот освобождает `task_finished`, и позвать
            // его обязан **любой** исход, включая панику внутри задачи:
            // иначе очередь осталась бы в состоянии «слот занят
            // навсегда», которое Ф-3 запрещает прямым текстом. Паника в
            // воркере роняет только его задачу рантайма, а `JoinHandle`
            // отдаёт её сюда ошибкой.
            let worker = tauri::async_runtime::spawn(run(app.clone(), Arc::clone(&task)));
            if worker.await.is_err() {
                eprintln!(
                    "queue: воркер задачи {} завершился аварийно — слот всё равно освобождается",
                    task.id
                );
            }

            // Реакция на С-13 эпика E6: сломанное извлечение — штатный
            // симптом «YouTube поменялся, нужна свежая версия», и контур
            // отвечает на него внеплановой проверкой. Спрашивается
            // терминальное состояние задачи, а не класс, угаданный по
            // пути: второе место, решающее «сломался ли yt-dlp»,
            // разошлось бы с первым.
            if broke_extraction(&task) {
                trigger_broken_extraction_check(&app);
            }

            let env: Arc<dyn QueueEnv> = self;
            scheduler::task_finished(&scheduler, &task.id, &env).await;
        });
    }

    fn update_holds_boundary(&self) -> bool {
        self.app
            .state::<Arc<UpdateController>>()
            .holds_task_boundary()
    }

    fn wait_for_update(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        let controller = self.app.state::<Arc<UpdateController>>().inner().clone();
        Box::pin(async move { controller.wait_until_idle().await })
    }
}

/// Ведёт одну задачу до терминальной фазы (E3, механизм не меняется).
///
/// Настроек здесь не читает (TL-89): воркер отдаёт задаче **хранилища** —
/// managed-состояние настроек и истории, — а значения из них снимает сам
/// `download::run_task` в своей первой строке (Р-4 эпика E5).
async fn run(app: AppHandle, task: Arc<DownloadTask>) {
    // Пути резолвятся здесь, а не в команде, по двум причинам: они нужны
    // воркеру, а не вызывающему, и их неудача обязана стать отказом
    // **задачи** (событие `failed` в панели), а не отказом команды —
    // иначе один и тот же класс ошибки рисовался бы то панелью, то
    // реджектом промиса.
    //
    // Системная «Загрузки» резолвится и при своей папке в настройках: по ней
    // `folderDisplay` решает, «Загрузки» это или свой путь, — тем же
    // `download_dir()`, что у команд истории и настроек.
    let system_downloads = match app.path().download_dir() {
        Ok(dir) => Some(dir),
        Err(err) => {
            // Практически недостижимо: системная папка «Загрузки» есть на
            // всех трёх целевых ОС. При системной папке в настройках задача
            // честно доедет до класса `destinationUnavailable` проверкой в
            // домене — своей ветки отказа заводить незачем.
            eprintln!("download: папка «Загрузки» не определяется: {err}");
            None
        }
    };
    // `try_state`: состояние кладёт `setup` до первой команды, но если его
    // нет, задача всё равно идёт — на умолчаниях и без записи в историю
    // (Н-4), с причиной в логе.
    let env = TaskEnv {
        settings: app
            .try_state::<Arc<SettingsState>>()
            .map(|state| Arc::clone(&state)),
        history: app
            .try_state::<Arc<HistoryState>>()
            .map(|state| Arc::clone(&state)),
        system_downloads,
        today: download::today_utc_date,
    };

    // `None` — готовой установки нет; домен отдаст задаче честный «сбой
    // yt-dlp», не запуская ничего постороннего.
    //
    // Страж занятости живёт столько же, сколько задача, — до конца
    // `run_task`, включая все её повторы. Ровно это и обещает Р-2 эпика
    // E6: задача доходит до конца на той версии, на которой началась, а
    // уборка контура обновления её дерево не трогает, даже если запись
    // Ф-5 за это время переключилась на новую установку.
    let (ytdlp, _in_use) = match resolve_ytdlp_path(&app) {
        Ok((path, guard)) => (Some(path), Some(guard)),
        Err(_) => (None, None),
    };
    // Пустой путь вместо имени `ffmpeg` по той же причине, что и `None`
    // выше: относительное имя ОС искала бы в `PATH`, а Ф-9 требует ffmpeg
    // из дистрибутива, а не системный.
    let ffmpeg = resolve_sidecar_path("ffmpeg").unwrap_or_default();

    // deno — путь и окружение одним значением (TL-109), тем же построителем,
    // что у разбора; без deno yt-dlp получает `--no-js-runtimes`, а не
    // ищет чужой deno в `PATH`.
    let js_runtime = YtDlpJsRuntime::for_app(&app);

    let registry = app.state::<ChildRegistry>();
    let launcher = SidecarDownloader::new(ytdlp, js_runtime, &registry);
    let merger = SidecarFfmpeg::new(ffmpeg, &registry);
    let sink = AppSink(app.clone());

    download::run_task(&task, &launcher, &merger, &sink, &env).await;
}

/// Кончилась ли задача сбоем yt-dlp (С-13 эпика E6).
///
/// Отдельная функция с именем, а не условие по месту: предмет здесь —
/// **один** класс из девяти, и читателю важно видеть, что остальные
/// восемь внеплановую проверку не запускают. «Нет сети» и «место на
/// диске» к свежести yt-dlp отношения не имеют, а проверка по каждому
/// отказу превратила бы С-13 в долбёжку источника.
fn broke_extraction(task: &Arc<DownloadTask>) -> bool {
    matches!(
        task.snapshot(),
        DownloadProgress::Failed { error } if error.kind == DownloadErrorKind::YtDlpFailure
    )
}

/// Снимок очереди — то, что экран запрашивает при монтировании и после
/// перезагрузки webview (С-9).
///
/// Не отказывает ничем: пустая очередь — это `tasks: []`, а не ошибка.
/// `Result` здесь по той же причине, что у `check_sidecar` и
/// `ytdlp_update_state`: async-команда, принимающая `State<'_, …>`,
/// обязана возвращать `Result`, иначе её future не `'static`.
#[tauri::command]
pub async fn queue_state(scheduler: State<'_, Arc<QueueScheduler>>) -> Result<QueueSnapshot, ()> {
    Ok(scheduler::queue_snapshot(&scheduler))
}

/// «Продолжить очередь» — снимает приостановку после перезапуска (Р-3).
///
/// Без параметров: слот один и порядок строгий, поэтому продолжается
/// очередь целиком, а не выбранная задача. Вызов при уже идущей очереди —
/// не ошибка, а ничего.
#[tauri::command]
pub async fn resume_queue(
    app: AppHandle,
    scheduler: State<'_, Arc<QueueScheduler>>,
) -> Result<(), ()> {
    scheduler::resume_queue(&scheduler, &app_queue(&app));
    Ok(())
}

/// «Скрыть» — убирает завершённую задачу из списка.
///
/// Команда ядра, а не локальное состояние стора: иначе перезагрузка
/// webview воскрешала бы скрытое (С-9).
#[tauri::command]
pub async fn dismiss_queue_task(
    app: AppHandle,
    task_id: String,
    scheduler: State<'_, Arc<QueueScheduler>>,
) -> Result<(), DownloadCommandError> {
    scheduler::dismiss_queue_task(&scheduler, &task_id, &app_queue(&app)).map_err(|rejection| {
        eprintln!("queue: скрытие отклонено: {rejection}");
        rejection.to_contract()
    })
}
