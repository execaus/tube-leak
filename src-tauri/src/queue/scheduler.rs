//! Планировщик очереди загрузок (TL-73; Ф-1…Ф-5, Ф-7, Ф-8, Р-3, Р-7
//! эпика E4).
//!
//! Сходящийся узел эпика: здесь живёт сама очередь — упорядоченный список
//! задач, единственный активный слот и решение, что запускать следующим.
//! Механизм одной задачи (фазы, повторы, склейка, отмена с подчисткой)
//! этот модуль не трогает: он берёт готовую задачу у
//! [`crate::download::build_task`] и ведёт её [`crate::download::run_task`]
//! чужими руками — через шов [`QueueEnv`].
//!
//! # Что закрывает этот модуль
//!
//! До него ядро жило односерийной моделью E3: один слот, и **второй старт
//! вытеснял идущую загрузку** — постановка заменяла задачу в слоте, а
//! воркер новой добивал предшественницу. Класс отказа «слот занят» к тому
//! моменту уже ушёл из контракта (Ф-2, TL-70), а очереди ещё не было, и
//! окно между этими двумя правками стоило пользователю часовой загрузки
//! на второй клик. Здесь оно закрыто: постановка при занятом слоте — хвост
//! очереди, и ни один путь этого модуля не отменяет активную задачу без
//! просьбы пользователя.
//!
//! # Правила, которых держится планировщик
//!
//! - **Один активный слот** (Р-1): [`QueueState::active`] — не список.
//!   Модель данных при этом N > 1 не запрещает: чтобы включить
//!   параллельность, придётся менять этот тип, а не переписывать задачи.
//! - **Строгий FIFO** (Р-4): следующая — первая нетерминальная в порядке
//!   постановки. Перестановок нет, приоритетов нет; повтор упавшей встаёт
//!   в **хвост** (решение дизайна), потому что второе правило порядка
//!   («ранее созданные вперёд») сделало бы порядок неочевидным.
//! - **Автостарт** (Ф-3): терминальный исход любой задачи освобождает
//!   слот, и следующая стартует без участия пользователя.
//! - **Приостановка после перезапуска** (Р-3): восстановленная с диска
//!   очередь не делает ни одного сетевого шага до `resume_queue`.
//! - **Граница задач** (Ф-7, Р-7): пока слот занят,
//!   [`QueueScheduler::is_active`] истинно; между задачами планировщик
//!   ждёт, пока контур обновления yt-dlp закончит прогрев и переключение,
//!   и на это время несёт в снимке `pauseReason: 'ytDlpUpdate'`.
//!
//! # Почему у команд снаружи один и тот же шов
//!
//! Боевой воркер — это `tauri::async_runtime::spawn` с `AppHandle`
//! внутри, боевое событие — `emit` в окно, а боевая граница обновления —
//! состояние контура E6. Ни одно из трёх нельзя поднять в тесте, и все
//! три собираются там, где есть `AppHandle` (`crate::commands::queue`).
//! Поэтому очередь принимает их одним объектом [`QueueEnv`] на вызов, а
//! не хранит в себе: `Arc<dyn QueueEnv>` переживает возврат из команды и
//! уезжает воркеру, который тем же объектом возвращает управление
//! очереди ([`task_finished`]).

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use super::snapshot::QueueSnapshotEntry;
use super::store::{Saved, SnapshotStore};
use super::video_id::canonical_video_id;
use crate::clock::now_unix_nanos;
use crate::download::{build_task, DownloadCommandRejection, DownloadTask};
use crate::types::{
    DownloadPhase, DownloadProgress, DownloadStarted, PartialData, QueuePauseReason, QueueSnapshot,
    QueueTask, QueueTaskRef, StartDownloadRequest,
};

/// Всё, что очередь умеет делать только чужими руками.
///
/// Три шва в одном объекте, а не три параметра: их всегда передают
/// вместе, собирает их одно место (граница команд), и воркеру нужен весь
/// набор целиком — закончив задачу, он тем же объектом возвращает
/// управление очереди.
pub trait QueueEnv: Send + Sync + 'static {
    /// Отправляет снимок очереди фронтенду — событие `queue://changed`
    /// ([`super::CHANGED_EVENT`]).
    fn emit(&self, snapshot: QueueSnapshot);

    /// Запускает воркера задачи.
    ///
    /// Воркер обязан довести задачу до терминальной фазы и **всегда**
    /// вернуть управление очереди вызовом [`task_finished`] — иначе слот
    /// останется занятым навсегда (запрет Ф-3).
    fn spawn(self: Arc<Self>, scheduler: Arc<QueueScheduler>, task: Arc<DownloadTask>);

    /// Держит ли контур обновления yt-dlp границу задач прямо сейчас
    /// (Р-7).
    ///
    /// Спрашивается ровно на границе — между терминальным исходом одной
    /// задачи и стартом следующей. Истина здесь стоит пользователю
    /// десятков секунд ожидания, поэтому она обязана означать «установка
    /// готова и ждёт прогрева/переключения», а не «контур чем-то занят»:
    /// скачивание архива границы задач не требует и очередь не держит.
    fn update_holds_boundary(&self) -> bool;

    /// Ждёт, пока контур отпустит границу.
    fn wait_for_update(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

/// Изменяемое состояние очереди.
struct QueueState {
    /// Задачи в порядке постановки — и нетерминальные, и ещё не скрытые
    /// терминальные. Порядок значим: он и есть порядок выполнения (Р-4).
    tasks: Vec<Arc<DownloadTask>>,
    /// Задача, которую прямо сейчас ведёт воркер.
    ///
    /// Не «первая нетерминальная», а именно занятый слот: между
    /// терминальным переходом задачи и возвратом её воркера проходит
    /// подчистка, и всё это время слот обязан считаться занятым — иначе
    /// следующий yt-dlp стартовал бы, пока предыдущий доубивают.
    active: Option<Arc<DownloadTask>>,
    /// Очередь восстановлена с диска и ждёт явного продолжения (Р-3).
    awaiting_continue: bool,
    /// Планировщик держит паузу между задачами (Р-7).
    pause_reason: Option<QueuePauseReason>,
}

/// Очередь загрузок: состояние приложения, ровно один экземпляр на
/// процесс.
pub struct QueueScheduler {
    inner: StdMutex<QueueState>,
    /// Счётчик выданных идентификаторов задач.
    counter: AtomicU64,
    /// Будильник «активных задач нет» — граница задач для контура
    /// обновления yt-dlp (Ф-7, Р-2 эпика E6).
    idle: tokio::sync::Notify,
    /// Файл-снимок очереди на диске (Ф-9). `None` — очередь без диска:
    /// так она живёт в тестах планирования, где предмет проверки не
    /// хранилище.
    store: Option<SnapshotStore>,
}

impl QueueScheduler {
    /// Очередь, сохраняющая состав в `store`.
    pub fn new(store: Option<SnapshotStore>) -> Self {
        Self {
            inner: StdMutex::new(QueueState {
                tasks: Vec::new(),
                active: None,
                awaiting_continue: false,
                pause_reason: None,
            }),
            counter: AtomicU64::new(0),
            idle: tokio::sync::Notify::new(),
            store,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, QueueState> {
        // Отравленный мьютекс не повод потерять очередь: под ним список
        // задач и два флага, и паника чужого потока не делает их
        // противоречивыми (тот же приём, что в `ChildRegistry`).
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Идентификатор задачи, непрозрачный для фронтенда.
    ///
    /// Счётчик разводит задачи одного запуска приложения, момент —
    /// задачи разных запусков. Второе перестало быть заделом на будущее
    /// вместе с Ф-9: восстановленная с диска задача сохраняет прежний id,
    /// и он обязан не столкнуться с идентификатором, который выдаст
    /// счётчик этого сеанса.
    fn next_id(&self) -> String {
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        format!("dl-{}-{n}", now_unix_nanos())
    }

    /// Идёт ли сейчас задача скачивания (Ф-7, предикат для контура E6).
    ///
    /// «Идёт» — слот занят, то есть воркер ещё не вернул управление.
    /// Второго определения занятости в приложении заводить нельзя: контур
    /// обновления ждёт ровно ту границу, на которой освобождается слот, и
    /// разойдись эти правила, он ждал бы момента, которого не бывает.
    pub fn is_active(&self) -> bool {
        self.lock().active.is_some()
    }

    /// Ждёт границы задач — момента, когда [`Self::is_active`] ложно.
    ///
    /// Подписка на уведомление включается **до** проверки состояния, и
    /// включается явно ([`tokio::sync::Notified::enable`]): будущее
    /// `notified()` само по себе регистрируется только при первом опросе,
    /// то есть уже после проверки, — и терминальный переход, случившийся
    /// в это окно, был бы потерян. Ждущий уснул бы до следующей задачи,
    /// которой может не быть никогда, а очередь на своей границе ждала бы
    /// его в ответ (Р-7) — два ожидающих друг друга.
    pub async fn wait_for_task_boundary(&self) {
        loop {
            let notified = self.idle.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();

            if !self.is_active() {
                return;
            }
            notified.await;
        }
    }

    /// Снимок очереди: ответ команды `queue_state` и полезная нагрузка
    /// события `queue://changed`.
    pub fn snapshot(&self) -> QueueSnapshot {
        let state = self.lock();
        QueueSnapshot {
            tasks: state.tasks.iter().map(queue_task_of).collect(),
            awaiting_continue: state.awaiting_continue,
            pause_reason: state.pause_reason,
        }
    }

    /// Восстанавливает очередь из файла-снимка — приостановленной (Р-3,
    /// С-7).
    ///
    /// Зовётся один раз при старте приложения, до первого окна. Отказ
    /// чтения не роняет старт: восстанавливать нечего — начинаем с
    /// пустой очереди, назвав причину в логе.
    ///
    /// Задача, которую снимок описал так, что она не проходит проверки
    /// постановки, пропускается поимённо, а не роняет остальные: снимок
    /// пишет прошлая версия приложения, и правила проверки могли с тех
    /// пор ужесточиться.
    pub fn restore(&self) {
        let Some(store) = &self.store else {
            return;
        };

        let entries = match store.load() {
            Ok(entries) => entries,
            Err(err) => {
                eprintln!("queue: снимок не восстановлен ({err}) — очередь начинается пустой");
                return;
            }
        };

        let mut tasks = Vec::with_capacity(entries.len());
        for entry in entries {
            let id = entry.task_id.clone();
            match build_task(entry.task_id, entry.request) {
                Ok(task) => tasks.push(task),
                Err(rejection) => {
                    eprintln!("queue: задача {id} из снимка не восстановлена: {rejection}");
                }
            }
        }

        if tasks.is_empty() {
            return;
        }

        eprintln!(
            "queue: из снимка восстановлено задач: {} — очередь приостановлена до продолжения (Р-3)",
            tasks.len()
        );
        let mut state = self.lock();
        state.tasks = tasks;
        // Приостановка — свойство самого факта восстановления, а не
        // значение из файла: снимок такого поля не несёт намеренно,
        // иначе Р-3 обходился бы правкой файла.
        state.awaiting_continue = true;
    }

    /// Задача по идентификатору либо типизированный отказ.
    ///
    /// Скрытая (`dismiss_queue_task`) и не пережившая перезапуск задача
    /// неотличимы от никогда не существовавшей — и это правда: ядро о них
    /// больше не знает.
    fn task(&self, task_id: &str) -> Result<Arc<DownloadTask>, DownloadCommandRejection> {
        self.lock()
            .tasks
            .iter()
            .find(|task| task.id == task_id)
            .cloned()
            .ok_or_else(|| DownloadCommandRejection::UnknownTask {
                task_id: task_id.to_string(),
            })
    }

    /// Пишет снимок на диск и отправляет событие — то, чем кончается
    /// каждое **структурное** изменение очереди (Ф-9, Ф-6).
    fn commit(&self, env: &Arc<dyn QueueEnv>) {
        self.save();
        env.emit(self.snapshot());
    }

    /// Пишет состав очереди на диск.
    ///
    /// В снимок идут только нетерминальные задачи (Ф-9): терминальные —
    /// материал истории (E5), и хранить их здесь значило бы завести
    /// вторую историю раньше настоящей. Хранилище само не трогает диск,
    /// если состав не изменился, поэтому лишний вызов ничего не стоит, а
    /// событие прогресса состав изменить не может по построению.
    ///
    /// Отказ записи не отменяет работу очереди: пользователь потеряет
    /// список при перезапуске, но текущая загрузка ему важнее.
    fn save(&self) {
        let Some(store) = &self.store else {
            return;
        };

        let entries: Vec<QueueSnapshotEntry> = {
            let state = self.lock();
            state
                .tasks
                .iter()
                .filter(|task| !task.snapshot().is_terminal())
                .map(|task| QueueSnapshotEntry {
                    task_id: task.id.clone(),
                    request: task.request().clone(),
                })
                .collect()
        };

        match store.save(&entries) {
            Ok(Saved::Written) => eprintln!(
                "queue: снимок {} обновлён, нетерминальных задач {}",
                store.path().display(),
                entries.len()
            ),
            Ok(Saved::Unchanged) => {}
            Err(err) => eprintln!("queue: снимок не записан: {err}"),
        }
    }
}

/// Задача очереди в контрактном виде.
fn queue_task_of(task: &Arc<DownloadTask>) -> QueueTask {
    QueueTask {
        task_id: task.id.clone(),
        title: task.request().title.clone(),
        quality: task.request().quality,
        plan: task.plan,
        progress: task.snapshot(),
    }
}

// ─────────────────────── Команда «начать» ───────────────────────

/// Ставит задачу в хвост очереди (Ф-2).
///
/// Возвращается быстро и не ждёт ни одного байта — идентификатором задачи
/// и планом. Слот при этом не проверяется: постановка при активной
/// загрузке — штатный путь, а не отказ. Единственный оставшийся повод
/// отказать — дубль (Ф-8, Р-5).
///
/// Постановка в **приостановленную** очередь (Р-3, до «Продолжить»)
/// принимается и ждёт вместе с остальными, а не запускается сама. Это
/// решение, а не побочный эффект: слот один и порядок строгий, поэтому
/// «начать эту» означало бы либо обогнать восстановленные задачи, о чём
/// пользователь не просил, либо начать первую из них — то есть сделать за
/// него ровно то, от чего Р-3 отказался. Продолжается очередь целиком и
/// одной кнопкой.
///
/// Порядок шагов важен. Проверки запроса ([`build_task`]) идут **до**
/// замка: они не касаются состава очереди, а держать замок на время
/// санитизации имени незачем. Сравнение дублей и постановка — под одним
/// замком: между «дубля нет» и «задача в списке» не должно быть точки,
/// где успеет пройти второй такой же вызов, иначе двойной клик даст ровно
/// то, что Р-5 запрещает.
pub fn start_download(
    scheduler: &Arc<QueueScheduler>,
    request: StartDownloadRequest,
    env: &Arc<dyn QueueEnv>,
) -> Result<DownloadStarted, DownloadCommandRejection> {
    let task = build_task(scheduler.next_id(), request)?;

    {
        let mut state = scheduler.lock();
        if let Some(existing) = duplicate_of(&state.tasks, &task) {
            eprintln!("queue: постановка отклонена дублем (Ф-8)");
            return Err(DownloadCommandRejection::DuplicateTask { existing });
        }
        state.tasks.push(Arc::clone(&task));
    }

    let started = DownloadStarted {
        task_id: task.id.clone(),
        phase: DownloadPhase::Queued,
        plan: task.plan,
    };

    eprintln!("queue: задача {} встала в очередь", task.id);
    scheduler.commit(env);
    pump(scheduler, env);

    Ok(started)
}

/// Нетерминальная задача того же ролика с тем же пунктом качества (Ф-8).
///
/// Правило сравнения — **id ролика плюс идентификаторы потоков**; строка
/// URL в сравнении не участвует: одна и та же ссылка приходит в разных
/// формах записи, и сравнение строк дало бы сторожа, который пропускает
/// ровно то, ради чего заведён.
///
/// # `None` от разбора ссылки — «дублем не считать»
///
/// Канонический id тотален не по формам, а по ответам: форму, которой нет
/// в белом списке, он честно называет неразобранной ([`None`]). Считать
/// две неразобранные ссылки одним роликом нельзя ни при каких условиях —
/// это склеило бы **любые** две такие ссылки в одну задачу, и цена ошибки
/// здесь несимметрична: пропущенный дубль стоит второй загрузки того же
/// ролика (заметно и обратимо), а ложное слипание — не скачанного ролика,
/// о котором пользователь узнает нескоро. Поэтому неразобранная ссылка не
/// сравнивается ни с чем, включая другую неразобранную.
fn duplicate_of(
    tasks: &[Arc<DownloadTask>],
    candidate: &Arc<DownloadTask>,
) -> Option<QueueTaskRef> {
    let request = candidate.request();
    let video_id = canonical_video_id(&request.url)?;

    let existing = tasks
        .iter()
        .filter(|task| !task.snapshot().is_terminal())
        .find(|task| {
            task.request().streams == request.streams
                && canonical_video_id(&task.request().url).is_some_and(|other| other == video_id)
        })?;

    eprintln!(
        "queue: ролик {} с тем же пунктом качества уже стоит задачей {}",
        video_id.as_str(),
        existing.id
    );
    Some(queue_task_of(existing).reference())
}

// ─────────────────── Команды «отменить» и «повторить» ───────────────────

/// Отменяет задачу в любой нетерминальной фазе (Ф-4).
///
/// У активной это ровно то же, что в E3: флаг, убийство процесса и
/// подчистка по фазам — их делает воркер, а он же возвращает управление
/// очереди, и следующая задача стартует штатно (С-3).
///
/// У ожидающей побочных эффектов нет вовсе (С-2): процессов не было —
/// убивать нечего, файлов не было — подчищать нечего. Терминальный
/// переход ей ставит сама очередь, и ставит его **под замком**: иначе
/// планировщик успел бы взять отменяемую задачу в работу между решением и
/// его исполнением.
///
/// Отмена уже терминальной задачи — не ошибка, а ничего: пользователь
/// способен нажать «Отменить» ровно в тот момент, когда приехало `done`.
pub async fn cancel_download(
    scheduler: &Arc<QueueScheduler>,
    task_id: &str,
    env: &Arc<dyn QueueEnv>,
) -> Result<(), DownloadCommandRejection> {
    let task = scheduler.task(task_id)?;

    let waiting = {
        let state = scheduler.lock();
        if task.snapshot().is_terminal() {
            eprintln!("queue: отмена задачи {task_id}, уже завершившейся, — ничего не делаем");
            return Ok(());
        }
        let waiting = !state
            .active
            .as_ref()
            .is_some_and(|active| active.id == task_id);
        if waiting {
            task.set_progress(DownloadProgress::Cancelled {
                partial_data: PartialData::NothingCreated,
            });
        }
        waiting
    };

    eprintln!(
        "queue: отмена задачи {task_id} ({})",
        if waiting {
            "ожидающей"
        } else {
            "активной"
        }
    );
    // Флаг отмены поднимается в обоих случаях: активной он останавливает
    // работу и убивает процесс, ожидающей — закрывает щель на случай,
    // если её всё же кто-то возьмёт в работу.
    task.cancel().await;

    if waiting {
        // Слот отменой ожидающей не освобождается, но состав очереди
        // изменился, а `pump` здесь стоит ради запрета Ф-3: очередь не
        // должна уметь остаться со свободным слотом и ожидающими
        // задачами ни по одному пути.
        scheduler.commit(env);
        pump(scheduler, env);
    }

    Ok(())
}

/// Возвращает упавшую задачу в очередь — в хвост, с тем же id (Ф-5).
///
/// Хвост, а не прежнее место, — решение дизайна: у очереди ровно одно
/// правило порядка (Р-4, порядок постановки), и возврат на прежнее место
/// завёл бы второе — «ранее созданные вперёд», из-за которого задача,
/// повторённая через час, обгоняла бы всё, что пользователь добавил за
/// этот час.
///
/// Задача при этом остаётся той же: тот же id, тот же агрегатор, те же
/// файлы на диске. Чем окажется повтор — докачкой потоков или пересборкой
/// склейки, — решает состояние задачи, а не флаг от фронтенда (семантика
/// E3, не пересматривается).
pub fn retry_download(
    scheduler: &Arc<QueueScheduler>,
    task_id: &str,
    env: &Arc<dyn QueueEnv>,
) -> Result<(), DownloadCommandRejection> {
    let task = scheduler.task(task_id)?;

    if task.snapshot().phase() != DownloadPhase::Failed {
        return Err(DownloadCommandRejection::NotFailed);
    }
    let Some(failure) = task.failure() else {
        return Err(DownloadCommandRejection::NotFailed);
    };
    if !failure.is_retryable() {
        return Err(DownloadCommandRejection::NotRetryable);
    }

    eprintln!("queue: повтор задачи {task_id} после отказа {failure:?} — в хвост очереди");
    task.set_progress(DownloadProgress::Queued);
    {
        let mut state = scheduler.lock();
        if let Some(position) = state.tasks.iter().position(|task| task.id == task_id) {
            let task = state.tasks.remove(position);
            state.tasks.push(task);
        }
    }

    scheduler.commit(env);
    pump(scheduler, env);
    Ok(())
}

// ─────────────────── Команды очереди (контракт TL-70) ───────────────────

/// Снимок очереди — то, что экран запрашивает при монтировании и после
/// перезагрузки webview (С-9).
pub fn queue_snapshot(scheduler: &QueueScheduler) -> QueueSnapshot {
    scheduler.snapshot()
}

/// «Продолжить очередь» — снимает приостановку после перезапуска (Р-3).
///
/// Без параметров: слот один и порядок строгий, поэтому продолжается
/// очередь целиком. Вызов при уже идущей очереди — не ошибка, а ничего, и
/// «ничего» здесь буквально: ни записи на диск, ни события.
pub fn resume_queue(scheduler: &Arc<QueueScheduler>, env: &Arc<dyn QueueEnv>) {
    {
        let mut state = scheduler.lock();
        if !state.awaiting_continue {
            return;
        }
        state.awaiting_continue = false;
    }

    eprintln!("queue: очередь продолжена пользователем");
    scheduler.commit(env);
    pump(scheduler, env);
}

/// «Скрыть» завершённую задачу — убирает её из списка ядра.
///
/// Команда ядра, а не локальное состояние стора: С-9 требует, чтобы
/// перезагрузка webview восстанавливала список по снимку ядра, и
/// скрытие, живущее только во фронтенде, воскрешало бы скрытые задачи.
///
/// Нетерминальная задача отклоняется типизированно: «скрыть» не означает
/// «отменить», и подменять одно другим команда не станет.
pub fn dismiss_queue_task(
    scheduler: &Arc<QueueScheduler>,
    task_id: &str,
    env: &Arc<dyn QueueEnv>,
) -> Result<(), DownloadCommandRejection> {
    let task = scheduler.task(task_id)?;
    if !task.snapshot().is_terminal() {
        return Err(DownloadCommandRejection::TaskNotFinished);
    }

    eprintln!("queue: задача {task_id} скрыта");
    scheduler.lock().tasks.retain(|task| task.id != task_id);
    // Диск при этом не трогается: терминальных задач в снимке нет, и
    // состав нетерминальных скрытием не меняется — хранилище видит те же
    // байты и молчит.
    scheduler.commit(env);
    Ok(())
}

// ─────────────────────────── Планирование ───────────────────────────

/// Воркер задачи закончил — слот свободен (Ф-3, Ф-7, Р-7).
///
/// Зовётся **всегда**, чем бы задача ни кончилась: три терминальные фазы
/// для очереди неразличимы, слот освобождает любая. Порядок шагов —
/// требование, а не стиль:
///
/// 1. слот освобождается;
/// 2. состав уходит на диск и в событие — снимок обязан быть верным
///    раньше, чем кто-то на него посмотрит;
/// 3. будится тот, кто ждёт границы задач (контур E6): он обязан увидеть
///    уже изменившееся состояние, а не догонять его;
/// 4. очередь ждёт, пока контур закончит прогрев и переключение (Р-7);
/// 5. и только теперь стартует следующая.
///
/// Поменяй местами 3 и 4 — и контур узнал бы о границе после того, как
/// его перестали ждать; поменяй 4 и 5 — прогрев ушёл бы в конкуренцию с
/// новой загрузкой за диск, что Н-3 E6 запрещает прямым текстом.
pub async fn task_finished(
    scheduler: &Arc<QueueScheduler>,
    task_id: &str,
    env: &Arc<dyn QueueEnv>,
) {
    {
        let mut state = scheduler.lock();
        if state
            .active
            .as_ref()
            .is_some_and(|active| active.id == task_id)
        {
            state.active = None;
        }
    }

    scheduler.commit(env);
    scheduler.idle.notify_waiters();

    hold_for_update(scheduler, env).await;
    pump(scheduler, env);
}

/// Пауза между задачами ради прогрева и переключения yt-dlp (Р-7, С-8).
///
/// Спрашивается один раз, на границе: контур либо уже держит её (готовая
/// установка ждёт прогрева), либо нет — и тогда очередь не платит ничем.
/// Пауза видима пользователю отдельным полем снимка (`pauseReason`), а не
/// молчаливой задержкой: С-8 требует объяснить редкую задержку в десятки
/// секунд, иначе она выглядит зависанием.
///
/// Пока идёт пауза, слот свободен и [`QueueScheduler::is_active`] ложно —
/// именно этого и ждёт контур, чтобы начать. Ждать друг друга они при
/// этом не могут: очередь объявляет границу до того, как начинает ждать.
async fn hold_for_update(scheduler: &Arc<QueueScheduler>, env: &Arc<dyn QueueEnv>) {
    if !env.update_holds_boundary() {
        return;
    }

    eprintln!("queue: пауза между задачами — контур обновления ставит yt-dlp (Р-7)");
    scheduler.lock().pause_reason = Some(QueuePauseReason::YtDlpUpdate);
    // Только событие: пауза состав очереди не меняет, писать на диск
    // нечего.
    env.emit(scheduler.snapshot());

    env.wait_for_update().await;

    scheduler.lock().pause_reason = None;
    eprintln!("queue: пауза окончена — очередь продолжается");
    env.emit(scheduler.snapshot());
}

/// Запускает следующую задачу, если её есть кому и когда запускать.
///
/// Единственное место, где занимается слот. Три условия отказа —
/// три разных «нельзя», и ни одно не выводится из другого: слот занят
/// (Р-1), очередь приостановлена после перезапуска (Р-3), очередь стоит
/// на границе ради обновления (Р-7).
///
/// Следующая — **первая нетерминальная** в порядке постановки (Р-4).
/// Терминальные остаются на своих местах: их порядок в списке — то, что
/// видит пользователь, а не очередь исполнения.
fn pump(scheduler: &Arc<QueueScheduler>, env: &Arc<dyn QueueEnv>) {
    let next = {
        let mut state = scheduler.lock();
        if state.active.is_some() || state.awaiting_continue || state.pause_reason.is_some() {
            return;
        }

        let Some(task) = state
            .tasks
            .iter()
            .find(|task| !task.snapshot().is_terminal())
            .cloned()
        else {
            return;
        };

        state.active = Some(Arc::clone(&task));
        task
    };

    eprintln!("queue: старт задачи {}", next.id);
    env.emit(scheduler.snapshot());
    Arc::clone(env).spawn(Arc::clone(scheduler), next);
}

#[cfg(test)]
#[path = "scheduler_tests.rs"]
mod tests;
