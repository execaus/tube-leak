//! Три команды контура самообновления yt-dlp и его расписание (TL-58,
//! контракт TL-53).
//!
//! Тонкий слой над [`crate::ytdlp`], как и остальные команды: здесь нет
//! ни одного решения о том, когда обновляться и что считать обновлением
//! — только сборка окружения (каталог данных, ресурс бандла, состояние
//! приложения) и перевод исхода в контрактные типы.
//!
//! # Почему команд ровно три и почему они возвращаются сразу
//!
//! Имена и формы заданы контрактом TL-53 и уже реализованы фронтендом
//! (TL-59, TL-60) на замоканном `invoke`: `ytdlp_update_state` —
//! разовый снимок при открытии служебного экрана, `check_ytdlp_update` —
//! «Проверить сейчас» (С-12), `roll_back_ytdlp` — «Вернуться к …»
//! (Р-3). Ни одна из них не ждёт того, что идёт минуты: проверка и
//! скачивание приезжают событием `ytdlp://update` (Н-3 — ничего не
//! блокирует). Отсюда форма «занять контур, вернуть снимок, работать
//! дальше самим».
//!
//! Откат отличается одним (TL-66): его первый ответ — исход, известный
//! на месте. Если ждать конца загрузки не нужно, команда дожидается
//! переключения (проверка запуска цели и запись) и возвращает строку 13,
//! а не строку 14, которая через долю секунды сменилась бы событием.
//!
//! # Где живёт связь с эпиком E3
//!
//! Здесь и только здесь. Домен `ytdlp` о задачах скачивания не знает
//! ничего: границу задач он видит трейтом [`ytdlp::TaskBoundary`], а
//! боевая реализация ([`QueueBoundary`]) — в этом файле, где очередь
//! загрузки и так под рукой. Тем же способом сюда вынесен и приёмник
//! событий: канал `ytdlp://update` — контракт с фронтендом, и знать о
//! нём должен слой границы.

use std::sync::Arc;

use tauri::{AppHandle, Manager, State};

use crate::queue::scheduler::QueueScheduler;
use crate::sidecar::ChildRegistry;
use crate::types::{YtDlpUpdateCommandError, YtDlpUpdateSnapshot};
use crate::ytdlp::{
    self, CheckTrigger, GithubTransport, InUse, TaskBoundary, UpdateController, UpdateJob,
    UpdateSink, PLANNED_CHECK_INTERVAL, STARTUP_CHECK_DELAY,
};

/// Снимок состояния контура — то, что служебный экран запрашивает при
/// открытии (контракт TL-53, разовый вызов по образцу `check_sidecar`).
///
/// Не отказывает ничем: «ничего ещё не происходило» — это значение
/// `neverChecked`, а не ошибка. `Result` здесь по той же причине, что у
/// `check_sidecar`: async-команда, принимающая `State<'_, …>`, обязана
/// возвращать `Result`, иначе её future не `'static`.
#[tauri::command]
pub async fn ytdlp_update_state(
    app: AppHandle,
    controller: State<'_, Arc<UpdateController>>,
) -> Result<YtDlpUpdateSnapshot, ()> {
    // Цель кнопки «Вернуться к …» перечитывается здесь, а не только
    // после переключений: запись Ф-5 меняет и подготовка первого
    // запуска, и предыдущий сеанс приложения, а конвейер в этом сеансе
    // мог не запускаться ни разу.
    if let Ok(data_dir) = app.path().app_data_dir() {
        controller.refresh_rollback_target_in(&data_dir);
    }

    Ok(controller.snapshot())
}

/// «Проверить сейчас» (С-12): запускает тот же конвейер, что плановая
/// проверка, и возвращает снимок с уже переключённым `checking`, чтобы
/// кнопка погасла, не дожидаясь первого события.
#[tauri::command]
pub async fn check_ytdlp_update(
    app: AppHandle,
    controller: State<'_, Arc<UpdateController>>,
) -> Result<YtDlpUpdateSnapshot, YtDlpUpdateCommandError> {
    let snapshot = controller.begin_manual_check()?;
    spawn(app, Action::Check(CheckTrigger::Manual));
    Ok(snapshot)
}

/// «Вернуться к известно-хорошей» (Р-3). Без параметра-версии: цель одна
/// по построению (Ф-8), и принимать её от фронтенда значило бы принимать
/// выбор, которого он не делает.
///
/// Возвращает исход, который известен на месте отката (TL-66): во время
/// загрузки — строку 14 сразу, без загрузки — строку 13 (или отказ) по
/// окончании. Решает не команда, а конвейер, под своим замком
/// ([`UpdateController::run_rollback`]).
///
/// Сам откат идёт отдельной задачей рантайма, а команда только ждёт
/// первого ответа: брошенный future команды не должен обрывать откат на
/// середине. Ответа нет, только если конвейер не запустился вовсе (каталог
/// данных не определился) — тогда отдаётся то, что стоит в контуре.
#[tauri::command]
pub async fn roll_back_ytdlp(
    app: AppHandle,
    controller: State<'_, Arc<UpdateController>>,
) -> Result<YtDlpUpdateSnapshot, YtDlpUpdateCommandError> {
    controller.begin_rollback()?;
    let (reply, first) = tokio::sync::oneshot::channel();
    spawn(app, Action::Rollback(reply));
    Ok(first.await.unwrap_or_else(|_| controller.snapshot()))
}

/// Запускает расписание контура: проверка при старте, дальше —
/// периодически (Ф-2).
///
/// Зовётся из `setup` в `main.rs`, как и подготовка первого запуска, и
/// по той же причине: обновление yt-dlp — обязанность ядра, а не
/// экрана; открыт ли служебный экран, контуру безразлично (дизайн E6,
/// «Насколько тихо — конкретно»).
///
/// Задача живёт до конца процесса и не имеет выхода из цикла намеренно:
/// выключателя автопроверки в E6 нет (Р-1), он появится вместе с
/// хранилищем настроек в E5.
pub fn start_ytdlp_update_schedule(app: &AppHandle) {
    let app = app.clone();

    tauri::async_runtime::spawn(async move {
        // Задержка перед первым обращением: подготовка первого запуска
        // в этот момент может распаковывать и греть дерево, и спорить с
        // ней за диск незачем (обоснование — у `STARTUP_CHECK_DELAY`).
        tokio::time::sleep(STARTUP_CHECK_DELAY).await;

        // Вторая половина С-10: пин нового релиза приложения новее
        // самообновлённой установки. Стоит до первой сетевой проверки —
        // ставить из бандла дешевле, чем качать, а результат тот же.
        run(app.clone(), Action::BundledPin).await;

        let mut trigger = CheckTrigger::Startup;
        loop {
            let controller = app.state::<Arc<UpdateController>>().inner().clone();
            if controller
                .begin_background_check(trigger, crate::clock::monotonic_now())
                .is_some()
            {
                run(app.clone(), Action::Check(trigger)).await;
            }

            tokio::time::sleep(PLANNED_CHECK_INTERVAL).await;
            trigger = CheckTrigger::Planned;
        }
    });
}

/// Внеплановая проверка по сломанному извлечению (С-13).
///
/// Зовётся с одного места — из воркера задачи скачивания, по факту
/// класса `ytDlpFailure`. Тихая: если контур занят или троттлинг ещё не
/// отпустил, ничего не происходит и никто об этом не узнаёт (Р-1).
///
/// Своего троттлинга у неё нет — он в контуре, и он **отдельный** от
/// планового: плановая проверка час назад не должна глушить реакцию на
/// живой симптом «YouTube поменялся».
pub fn trigger_broken_extraction_check(app: &AppHandle) {
    let app = app.clone();

    tauri::async_runtime::spawn(async move {
        let controller = app.state::<Arc<UpdateController>>().inner().clone();
        if controller
            .begin_background_check(
                CheckTrigger::BrokenExtraction,
                crate::clock::monotonic_now(),
            )
            .is_none()
        {
            return;
        }

        eprintln!(
            "yt-dlp update: скачивание упало сбоем yt-dlp — проверяю обновление внепланово (С-13)"
        );
        run(app.clone(), Action::Check(CheckTrigger::BrokenExtraction)).await;
    });
}

/// Что именно предстоит сделать конвейеру.
///
/// Одно перечисление вместо трёх почти одинаковых функций: собирается
/// окружение одинаково, и различие в одну строку не стоит трёх копий
/// сборки, которые разойдутся на первой же новой зависимости.
enum Action {
    Check(CheckTrigger),
    BundledPin,
    /// Несёт, куда отдать первый ответ команды отката (TL-66).
    Rollback(ytdlp::RollbackReply),
}

fn spawn(app: AppHandle, action: Action) {
    tauri::async_runtime::spawn(run(app, action));
}

/// Собирает боевое окружение и выполняет действие.
///
/// Всё, что нужно конвейеру, берётся из состояния приложения — того же,
/// которым живут остальные команды: реестр процессов (TL-10), отметки
/// занятости установок (Ф-7), очередь загрузок (E4) и один на
/// процесс HTTP-клиент (переиспользование соединений и пула TLS).
async fn run(app: AppHandle, action: Action) {
    let Ok(data_dir) = app.path().app_data_dir() else {
        eprintln!(
            "yt-dlp update: каталог данных приложения не определяется — обновление пропущено"
        );
        return;
    };

    let controller = app.state::<Arc<UpdateController>>().inner().clone();
    let transport = app.state::<GithubTransport>();
    let registry = app.state::<ChildRegistry>();
    let in_use = app.state::<InUse>();
    let session = app.state::<ytdlp::Session>();
    let scheduler = app.state::<Arc<QueueScheduler>>().inner().clone();

    let sink = AppUpdateSink(app.clone());
    let boundary = QueueBoundary(scheduler);

    let job = UpdateJob::new(
        &data_dir,
        bundled_archive(&app),
        transport.inner(),
        registry.inner(),
        in_use.inner(),
        session.inner(),
        &sink,
        &boundary,
    );

    match action {
        Action::Check(trigger) => job.check(&controller, trigger).await,
        Action::BundledPin => job.bundled_pin(&controller).await,
        Action::Rollback(reply) => job.rollback(&controller, reply).await,
    }
}

/// Путь к вложенному в бандл архиву yt-dlp — источник установки для
/// С-10.
///
/// `None` — ресурс не резолвится; для контура это не отказ, а «этой
/// дорогой сегодня не пойдём»: сетевое обновление от вложенного архива
/// не зависит вовсе.
fn bundled_archive(app: &AppHandle) -> Option<std::path::PathBuf> {
    app.path()
        .resolve(
            ytdlp::BUNDLED_ARCHIVE_RESOURCE,
            tauri::path::BaseDirectory::Resource,
        )
        .ok()
}

/// Приёмник событий контура: снимок уходит в `ytdlp://update` всем окнам.
///
/// Канал отдельный от `ytdlp://prepare`, и это не косметика: на том
/// поднимается полноэкранный блокирующий экран подготовки первого
/// запуска, и его появление посреди фонового обновления было бы прямой
/// регрессией Р-1. Разрешение канала выдано контрактной задачей (TL-53)
/// в `capabilities/main.json`; эта задача его только использует.
struct AppUpdateSink(AppHandle);

impl UpdateSink for AppUpdateSink {
    fn emit(&self, snapshot: YtDlpUpdateSnapshot) {
        use tauri::Emitter as _;

        // Неотправленное событие не повод бросать обновление: работа
        // важнее индикатора, а снимок фронтенд заберёт командой при
        // следующем открытии экрана (то же решение, что в подготовке
        // yt-dlp и в скачивании ролика).
        if let Err(err) = self.0.emit(ytdlp::UPDATE_EVENT, snapshot) {
            eprintln!("yt-dlp update: событие не отправлено: {err}");
        }
    }
}

/// Граница задач для контура — очередь загрузок (E4).
///
/// До E4 здесь стоял слот E3, и разница не в имени: «задач нет» —
/// свойство очереди, а не одной задачи. Пока за завершившейся задачей
/// стоит следующая, граница не наступает сама собой — её наступление
/// объявляет планировщик, и он же ждёт на ней контур (Р-7 E4), чтобы
/// прогрев не соревновался с новой загрузкой (Н-3).
///
/// Держит `Arc`, а не ссылку: конвейер живёт отдельной задачей рантайма
/// и переживает возврат из команды, которая его затеяла.
struct QueueBoundary(Arc<QueueScheduler>);

impl TaskBoundary for QueueBoundary {
    fn is_busy(&self) -> bool {
        self.0.is_active()
    }

    fn wait(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        Box::pin(self.0.wait_for_task_boundary())
    }
}
