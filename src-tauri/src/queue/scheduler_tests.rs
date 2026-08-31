//! Тесты планировщика очереди (TL-73, Ф-11 эпика E4).
//!
//! Сети здесь нет, процессов — почти нет. Почти: один тест ведёт задачу
//! **настоящим** воркером ([`run_task`]) с подменённым запуском, потому
//! что предмет его проверки — то самое окно, ради закрытия которого
//! заведена очередь: второй старт при занятом слоте. Утверждение «первая
//! загрузка не вытеснена» нельзя доказать протоколом вызовов очереди — в
//! вытеснении E3 очередь тоже ничего не «отменяла», задачу добивал
//! воркер; доказывает его только живой воркер, дошедший до конца сам.
//!
//! Остальные тесты подменяют воркера протоколом: исход задачи задаёт сам
//! тест, а очередь проверяется наблюдаемым фактом — составом списка,
//! порядком стартов, снимком на диске и содержимым события, — а не
//! возвращаемым значением команды. Урок TL-71: тест, смотревший на
//! возврат, прошёл мимо проверяемого.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tempfile::TempDir;

use super::*;
use crate::download::merge::FfmpegLauncher;
use crate::download::orchestrate::{run_task, DownloadLauncher, ProgressSink};
use crate::sidecar::{RunHandle, RunOutput, SidecarError, StreamedRun};
use crate::types::{
    DownloadError, DownloadErrorKind, DownloadPercent, DownloadPhase, DownloadProgressEvent,
    DownloadingState, QualityKind, QualitySize, QualityStreams, SelectedQuality,
};

// ─────────────────────────── Оснастка ───────────────────────────

const URL: &str = "https://www.youtube.com/watch?v=aqz-KE-bpKQ";
const OTHER_URL: &str = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";

fn streams(video: Option<&str>, audio: Option<&str>) -> QualityStreams {
    QualityStreams {
        video_format_id: video.map(str::to_string),
        audio_format_id: audio.map(str::to_string),
    }
}

fn request_for(url: &str, streams: QualityStreams) -> StartDownloadRequest {
    StartDownloadRequest {
        url: url.to_string(),
        title: "Big Buck Bunny".to_string(),
        quality: SelectedQuality {
            kind: QualityKind::Standard,
            height_px: Some(720),
        },
        streams,
        size: QualitySize::Known { bytes: 22_157_855 },
    }
}

fn request(streams: QualityStreams) -> StartDownloadRequest {
    request_for(URL, streams)
}

/// Отказ задачи класса `connectionLost` — повторяемого.
fn failed(kind: DownloadErrorKind) -> DownloadProgress {
    DownloadProgress::Failed {
        error: DownloadError {
            kind,
            message: "тестовый отказ".to_string(),
            retryable: kind.is_retryable(),
            reason: None,
            partial_data: PartialData::NothingCreated,
            details: None,
        },
    }
}

fn done() -> DownloadProgress {
    DownloadProgress::Done {
        file_name: "Big Buck Bunny.m4a".to_string(),
    }
}

/// Окружение-протокол: воркер задачу не ведёт, исход задаёт тест.
///
/// Так проверяется планирование, а не скачивание: очередь обязана вести
/// себя одинаково, чем бы задача ни кончилась, и подменённый воркер
/// позволяет назвать исход прямо, вместо того чтобы добиваться его
/// сценарием процесса.
struct TestEnv {
    /// Снимки, уехавшие в `queue://changed`, — в порядке отправки.
    events: StdMutex<Vec<QueueSnapshot>>,
    /// Задачи, которые очередь отдала воркеру, — в порядке стартов.
    started: StdMutex<Vec<Arc<DownloadTask>>>,
    /// Контур обновления держит границу задач (Р-7).
    holds_boundary: AtomicBool,
    /// Сколько раз очередь ждала контур на границе.
    waits: AtomicUsize,
    /// Разрешение продолжить: тест выдаёт его, когда «прогрев закончен».
    release: tokio::sync::Semaphore,
}

impl TestEnv {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            events: StdMutex::new(Vec::new()),
            started: StdMutex::new(Vec::new()),
            holds_boundary: AtomicBool::new(false),
            waits: AtomicUsize::new(0),
            release: tokio::sync::Semaphore::new(0),
        })
    }

    fn events(&self) -> Vec<QueueSnapshot> {
        self.events.lock().unwrap().clone()
    }

    fn started_ids(&self) -> Vec<String> {
        self.started
            .lock()
            .unwrap()
            .iter()
            .map(|task| task.id.clone())
            .collect()
    }

    fn started(&self, index: usize) -> Arc<DownloadTask> {
        self.started.lock().unwrap()[index].clone()
    }
}

impl QueueEnv for TestEnv {
    fn emit(&self, snapshot: QueueSnapshot) {
        self.events.lock().unwrap().push(snapshot);
    }

    fn spawn(self: Arc<Self>, _scheduler: Arc<QueueScheduler>, task: Arc<DownloadTask>) {
        self.started.lock().unwrap().push(task);
    }

    fn update_holds_boundary(&self) -> bool {
        self.holds_boundary.load(Ordering::SeqCst)
    }

    fn wait_for_update(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        self.waits.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            // Семафор, а не «разбудить ждущих»: разрешение, выданное до
            // того, как ждущий пришёл, не теряется — иначе тест зависел
            // бы от того, кто из двух будущих проснулся первым.
            let permit = self.release.acquire().await.expect("семафор жив");
            permit.forget();
        })
    }
}

/// Очередь, её протокол и (когда нужен) её файл-снимок.
struct Harness {
    scheduler: Arc<QueueScheduler>,
    env: Arc<TestEnv>,
    dyn_env: Arc<dyn QueueEnv>,
    dir: Option<TempDir>,
}

impl Harness {
    /// Очередь без диска: предмет проверки — планирование.
    fn new() -> Self {
        Self::build(None)
    }

    /// Очередь со снимком в отдельном каталоге.
    fn with_disk() -> Self {
        let dir = tempfile::tempdir().expect("временный каталог");
        let store = SnapshotStore::new(dir.path());
        let mut harness = Self::build(Some(store));
        harness.dir = Some(dir);
        harness
    }

    fn build(store: Option<SnapshotStore>) -> Self {
        let env = TestEnv::new();
        let dyn_env: Arc<dyn QueueEnv> = env.clone();
        Self {
            scheduler: Arc::new(QueueScheduler::new(store)),
            env,
            dyn_env,
            dir: None,
        }
    }

    fn start(
        &self,
        request: StartDownloadRequest,
    ) -> Result<DownloadStarted, DownloadCommandRejection> {
        start_download(&self.scheduler, request, &self.dyn_env)
    }

    fn start_ok(&self, request: StartDownloadRequest) -> String {
        self.start(request)
            .expect("постановка обязана быть принята")
            .task_id
    }

    async fn cancel(&self, task_id: &str) -> Result<(), DownloadCommandRejection> {
        cancel_download(&self.scheduler, task_id, &self.dyn_env).await
    }

    fn retry(&self, task_id: &str) -> Result<(), DownloadCommandRejection> {
        retry_download(&self.scheduler, task_id, &self.dyn_env)
    }

    fn dismiss(&self, task_id: &str) -> Result<(), DownloadCommandRejection> {
        dismiss_queue_task(&self.scheduler, task_id, &self.dyn_env)
    }

    fn resume(&self) {
        resume_queue(&self.scheduler, &self.dyn_env);
    }

    fn snapshot(&self) -> QueueSnapshot {
        queue_snapshot(&self.scheduler)
    }

    /// Состав очереди: идентификаторы задач в порядке списка.
    fn ids(&self) -> Vec<String> {
        self.snapshot()
            .tasks
            .into_iter()
            .map(|task| task.task_id)
            .collect()
    }

    /// Фазы задач в порядке списка.
    fn phases(&self) -> Vec<DownloadPhase> {
        self.snapshot()
            .tasks
            .iter()
            .map(|task| task.progress.phase())
            .collect()
    }

    /// Доводит стартовавшую задачу до исхода ровно так, как это делает
    /// боевой воркер: сначала терминальное состояние, потом возврат
    /// управления очереди.
    async fn finish(&self, index: usize, progress: DownloadProgress) {
        let task = self.env.started(index);
        task.set_progress(progress);
        task_finished(&self.scheduler, &task.id, &self.dyn_env).await;
    }

    /// Задачи в файле-снимке — то, что переживёт перезапуск.
    fn on_disk(&self) -> Vec<String> {
        let dir = self.dir.as_ref().expect("очередь заведена без диска");
        let store = SnapshotStore::new(dir.path());
        store
            .load()
            .expect("снимок обязан читаться")
            .into_iter()
            .map(|entry| entry.task_id)
            .collect()
    }

    /// Файл-снимок как его видит посторонний: содержимое и момент
    /// последней записи.
    ///
    /// Момент здесь не украшение: «диск не тронут» — утверждение о
    /// записи, а не о байтах. Перезапись того же состава теми же байтами
    /// содержимое не меняет, и проверка по одному содержимому пропустила
    /// бы её целиком.
    fn snapshot_state(&self) -> Option<(Vec<u8>, std::time::SystemTime)> {
        let dir = self.dir.as_ref().expect("очередь заведена без диска");
        let path = dir.path().join(crate::queue::store::SNAPSHOT_FILE_NAME);
        let bytes = std::fs::read(&path).ok()?;
        let modified = std::fs::metadata(&path).ok()?.modified().ok()?;
        Some((bytes, modified))
    }
}

// ─────────────────── Порядок, слот и автостарт ───────────────────

#[tokio::test]
async fn three_tasks_run_one_at_a_time_in_the_order_they_were_added() {
    // Критерий issue буквально: три ссылки подряд встают в очередь и
    // выполняются по одной в порядке постановки.
    let queue = Harness::new();

    let first = queue.start_ok(request(streams(None, Some("140"))));
    let second = queue.start_ok(request_for(OTHER_URL, streams(None, Some("140"))));
    let third = queue.start_ok(request_for(
        "https://www.youtube.com/watch?v=aaaaaaaaaaa",
        streams(None, Some("140")),
    ));

    assert_eq!(
        queue.ids(),
        [first.clone(), second.clone(), third.clone()],
        "порядок списка — порядок постановки (Р-4)"
    );
    assert_eq!(
        queue.env.started_ids(),
        std::slice::from_ref(&first),
        "стартовать обязана ровно одна задача — слот один (Р-1)"
    );

    queue.finish(0, done()).await;
    assert_eq!(
        queue.env.started_ids(),
        [first.clone(), second.clone()],
        "терминальный исход первой запускает вторую без участия пользователя (Ф-3)"
    );

    queue.finish(1, done()).await;
    assert_eq!(
        queue.env.started_ids(),
        [first, second, third],
        "и так далее — строгий FIFO без перестановок"
    );
}

#[tokio::test]
async fn every_terminal_outcome_frees_the_slot() {
    // Ф-3: слот освобождает **любой** из трёх исходов, а не только
    // успешный. Проверяются все три подряд, потому что ветки в
    // планировщике одна, а поводов ошибиться три.
    for outcome in [
        done(),
        failed(DownloadErrorKind::ConnectionLost),
        DownloadProgress::Cancelled {
            partial_data: PartialData::NothingCreated,
        },
    ] {
        let queue = Harness::new();
        let first = queue.start_ok(request(streams(None, Some("140"))));
        let second = queue.start_ok(request_for(OTHER_URL, streams(None, Some("140"))));

        queue.finish(0, outcome.clone()).await;

        assert_eq!(
            queue.env.started_ids(),
            [first, second],
            "исход {outcome:?} обязан освободить слот"
        );
    }
}

#[tokio::test]
async fn a_failed_task_does_not_take_the_next_one_with_it() {
    // С-4: сбой одной задачи не ломает очередь и не трогает соседей.
    let queue = Harness::new();
    let broken = queue.start_ok(request(streams(None, Some("140"))));
    let healthy = queue.start_ok(request_for(OTHER_URL, streams(None, Some("140"))));

    queue
        .finish(0, failed(DownloadErrorKind::ConnectionLost))
        .await;
    queue.finish(1, done()).await;

    let phases = queue.phases();
    assert_eq!(queue.ids(), [broken, healthy]);
    assert_eq!(
        phases,
        [DownloadPhase::Failed, DownloadPhase::Done],
        "исходы задач независимы: {phases:?}"
    );
}

// ─────────────────────── Отмена (Ф-4) ───────────────────────

#[tokio::test]
async fn cancelling_a_waiting_task_touches_neither_the_active_one_nor_the_order() {
    // С-2: у ожидающей отмена без побочных эффектов, активная и остальные
    // не затронуты, порядок сохраняется.
    let queue = Harness::new();
    let active = queue.start_ok(request(streams(None, Some("140"))));
    let doomed = queue.start_ok(request_for(OTHER_URL, streams(None, Some("140"))));
    let last = queue.start_ok(request_for(
        "https://www.youtube.com/watch?v=aaaaaaaaaaa",
        streams(None, Some("140")),
    ));

    queue.cancel(&doomed).await.expect("отмена ожидающей");

    assert_eq!(
        queue.ids(),
        [active.clone(), doomed.clone(), last.clone()],
        "отменённая остаётся на своём месте в списке, порядок не меняется"
    );
    assert_eq!(
        queue.phases(),
        [
            DownloadPhase::Queued,
            DownloadPhase::Cancelled,
            DownloadPhase::Queued
        ],
        "отменена ровно одна задача"
    );
    assert_eq!(
        queue.env.started_ids(),
        std::slice::from_ref(&active),
        "отмена ожидающей никого не запускает и активную не трогает"
    );

    // И следующей пойдёт та, что стояла третьей: отменённую планировщик
    // не берёт.
    queue.finish(0, done()).await;
    assert_eq!(queue.env.started_ids(), [active, last]);
}

#[tokio::test]
async fn cancelling_the_active_task_lets_the_next_one_start() {
    // С-3: отмена активной — как в E3, а очередь продолжает жить.
    let queue = Harness::new();
    let active = queue.start_ok(request(streams(None, Some("140"))));
    let next = queue.start_ok(request_for(OTHER_URL, streams(None, Some("140"))));

    queue.cancel(&active).await.expect("отмена активной");
    // Активную доводит до терминальной фазы её воркер — очередь этого не
    // делает за него, иначе слот освободился бы раньше подчистки.
    assert_eq!(
        queue.env.started_ids(),
        std::slice::from_ref(&active),
        "пока воркер не вернул управление, слот занят"
    );

    queue
        .finish(
            0,
            DownloadProgress::Cancelled {
                partial_data: PartialData::NothingCreated,
            },
        )
        .await;
    assert_eq!(queue.env.started_ids(), [active, next]);
}

#[tokio::test]
async fn cancelling_a_finished_task_is_not_an_error() {
    // Пользователь способен нажать «Отменить» ровно в тот момент, когда
    // приехало `done`.
    let queue = Harness::new();
    let task = queue.start_ok(request(streams(None, Some("140"))));
    queue.finish(0, done()).await;

    queue
        .cancel(&task)
        .await
        .expect("отмена терминальной задачи — не ошибка, а ничего");
    assert_eq!(queue.phases(), [DownloadPhase::Done], "исход не переписан");
}

#[tokio::test]
async fn an_unknown_task_id_is_refused() {
    let queue = Harness::new();
    queue.start_ok(request(streams(None, Some("140"))));

    assert!(matches!(
        queue.cancel("dl-чужой").await,
        Err(DownloadCommandRejection::UnknownTask { .. })
    ));
    assert!(matches!(
        queue.retry("dl-чужой"),
        Err(DownloadCommandRejection::UnknownTask { .. })
    ));
    assert!(matches!(
        queue.dismiss("dl-чужой"),
        Err(DownloadCommandRejection::UnknownTask { .. })
    ));
}

// ─────────────────────── Повтор (Ф-5) ───────────────────────

#[tokio::test]
async fn a_retried_task_keeps_its_id_and_goes_to_the_end_of_the_queue() {
    let queue = Harness::new();
    let broken = queue.start_ok(request(streams(None, Some("140"))));
    let second = queue.start_ok(request_for(OTHER_URL, streams(None, Some("140"))));

    queue
        .finish(0, failed(DownloadErrorKind::ConnectionLost))
        .await;
    queue
        .retry(&broken)
        .expect("класс connectionLost повторяем");

    assert_eq!(
        queue.ids(),
        [second.clone(), broken.clone()],
        "повтор ставит ту же задачу в хвост, а не на прежнее место"
    );
    assert_eq!(
        queue.snapshot().tasks[1].progress.phase(),
        DownloadPhase::Queued,
        "повторённая задача ждёт своей очереди"
    );
    assert_eq!(
        queue.env.started_ids(),
        [broken.clone(), second.clone()],
        "слот занят второй задачей — повтор её не вытесняет"
    );

    queue.finish(1, done()).await;
    assert_eq!(
        queue.env.started_ids(),
        [broken.clone(), second, broken],
        "повторённая стартует, когда до неё дошла очередь, — тем же id"
    );
}

#[tokio::test]
async fn retry_is_refused_for_a_task_that_did_not_fail_and_for_a_hopeless_class() {
    let queue = Harness::new();
    let task = queue.start_ok(request(streams(None, Some("140"))));

    assert!(matches!(
        queue.retry(&task),
        Err(DownloadCommandRejection::NotFailed)
    ));

    queue
        .finish(0, failed(DownloadErrorKind::StaleFormat))
        .await;
    assert!(matches!(
        queue.retry(&task),
        Err(DownloadCommandRejection::NotRetryable)
    ));
    assert_eq!(
        queue.env.started_ids().len(),
        1,
        "отклонённый повтор ничего не запускает"
    );
}

// ─────────────────────── Дубли (Ф-8, Р-5) ───────────────────────

#[tokio::test]
async fn the_same_video_with_the_same_quality_is_refused_and_names_the_existing_task() {
    let queue = Harness::new();
    let existing = queue.start_ok(request(streams(Some("137"), Some("140"))));

    // Та же ссылка в другой форме записи: сравнение идёт по id ролика, а
    // не по строке (Р-5).
    let rejection = queue
        .start(request_for(
            "https://youtu.be/aqz-KE-bpKQ?si=Kx1yQ7wSomething",
            streams(Some("137"), Some("140")),
        ))
        .expect_err("тот же ролик с тем же качеством — дубль");

    let DownloadCommandRejection::DuplicateTask { existing: named } = rejection else {
        panic!("класс отказа обязан быть duplicateTask: {rejection:?}");
    };
    assert_eq!(
        named.task_id, existing,
        "отказ называет существующую задачу"
    );
    assert_eq!(named.title, "Big Buck Bunny");
    assert_eq!(
        queue.ids(),
        [existing],
        "вторая задача в списке не появляется"
    );
}

#[tokio::test]
async fn the_same_video_with_another_quality_is_not_a_duplicate() {
    let queue = Harness::new();
    let first = queue.start_ok(request(streams(Some("137"), Some("140"))));
    let second = queue.start_ok(request(streams(Some("133"), Some("140"))));

    assert_eq!(
        queue.ids(),
        [first, second],
        "другой пункт качества того же ролика — обычная задача"
    );
}

#[tokio::test]
async fn a_terminal_task_does_not_block_the_same_video_again() {
    // Ф-8: в сравнении участвуют только нетерминальные. Повторное
    // скачивание завершённого решено в E3 суффиксом « (N)».
    let queue = Harness::new();
    let first = queue.start_ok(request(streams(None, Some("140"))));
    queue.finish(0, done()).await;

    let again = queue.start_ok(request(streams(None, Some("140"))));
    assert_eq!(queue.ids(), [first, again]);
}

#[tokio::test]
async fn a_link_whose_form_is_not_recognised_is_never_a_duplicate() {
    // Требование, которое легко прочесть наоборот: `None` от канонизации
    // означает «дублем не считать», а не «считать одним роликом».
    // Обратное прочтение склеило бы **любые** две неразобранные ссылки в
    // одну задачу — и вторая никогда бы не скачалась.
    let queue = Harness::new();
    let unknown = "https://example.com/watch?v=aqz-KE-bpKQ";
    assert!(
        canonical_video_id(unknown).is_none(),
        "предпосылка теста: форма ссылки не разбирается"
    );

    let first = queue.start_ok(request_for(unknown, streams(None, Some("140"))));
    let second = queue.start_ok(request_for(unknown, streams(None, Some("140"))));
    let third = queue.start_ok(request_for(
        "https://example.org/другое",
        streams(None, Some("140")),
    ));

    assert_eq!(
        queue.ids(),
        [first, second, third],
        "две неразобранные ссылки — две задачи, а не одна"
    );
}

// ─────────────────────── «Скрыть» ───────────────────────

#[tokio::test]
async fn only_a_finished_task_can_be_dismissed_and_the_disk_does_not_notice() {
    let queue = Harness::with_disk();
    let task = queue.start_ok(request(streams(None, Some("140"))));

    assert!(matches!(
        queue.dismiss(&task),
        Err(DownloadCommandRejection::TaskNotFinished)
    ));

    queue.finish(0, done()).await;
    let before = queue.snapshot_state();
    queue.dismiss(&task).expect("завершённую скрыть можно");

    assert!(
        queue.ids().is_empty(),
        "скрытая задача уходит из списка ядра"
    );
    assert!(matches!(
        queue.cancel(&task).await,
        Err(DownloadCommandRejection::UnknownTask { .. })
    ));
    assert_eq!(
        queue.snapshot_state(),
        before,
        "терминальных задач в снимке нет — скрытие диска не касается"
    );
}

// ─────────── Снимок на диске и восстановление (Ф-9, Р-3) ───────────

#[tokio::test]
async fn the_disk_snapshot_holds_the_non_terminal_tasks_in_order() {
    let queue = Harness::with_disk();
    let first = queue.start_ok(request(streams(None, Some("140"))));
    let second = queue.start_ok(request_for(OTHER_URL, streams(None, Some("140"))));

    assert_eq!(
        queue.on_disk(),
        [first.clone(), second.clone()],
        "снимок пишется при постановке и хранит порядок"
    );

    queue.finish(0, done()).await;
    assert_eq!(
        queue.on_disk(),
        [second],
        "терминальная задача уходит из снимка — её сохранение это история (E5)"
    );
}

#[tokio::test]
async fn progress_events_do_not_touch_the_disk() {
    // Ф-9: писать на структурных изменениях, а не на каждое событие
    // прогресса. Проверяется наблюдаемым фактом — байтами файла.
    let queue = Harness::with_disk();
    queue.start_ok(request(streams(None, Some("140"))));
    let before = queue.snapshot_state().expect("снимок записан постановкой");

    let task = queue.env.started(0);
    for percent in [1_u8, 2, 3] {
        task.set_progress(DownloadProgress::Downloading(DownloadingState::Running {
            stream: None,
            percent: Some(DownloadPercent::new(percent)),
            speed_bytes_per_sec: None,
            eta_secs: None,
            attempt: None,
        }));
        // Повод для записи, которого не должно хватить: состав очереди
        // прогрессом не меняется.
        queue.scheduler.commit(&queue.dyn_env);
    }

    assert_eq!(
        queue.snapshot_state(),
        Some(before),
        "файл обязан остаться тем же — и содержимым, и моментом записи"
    );
}

#[tokio::test]
async fn a_restored_queue_waits_for_the_user_and_starts_nothing() {
    // Р-3 и К-5: после перезапуска очередь видна, но приостановлена, и до
    // явного «Продолжить» ни одна задача не стартует.
    let dir = tempfile::tempdir().expect("временный каталог");

    let ids = {
        let past = Harness::build(Some(SnapshotStore::new(dir.path())));
        let first = past.start_ok(request(streams(None, Some("140"))));
        let second = past.start_ok(request_for(OTHER_URL, streams(None, Some("140"))));
        vec![first, second]
    };

    let queue = Harness::build(Some(SnapshotStore::new(dir.path())));
    queue.scheduler.restore();

    assert_eq!(queue.ids(), ids, "порядок прошлого сеанса сохранён");
    assert!(
        queue.snapshot().awaiting_continue,
        "очередь восстановлена приостановленной (Р-3)"
    );
    assert!(
        queue.env.started_ids().is_empty(),
        "до продолжения не стартует ничего — сетевой активности по задачам нет (Н-1)"
    );

    queue.resume();
    assert!(
        !queue.snapshot().awaiting_continue,
        "банер продолжения исчезает"
    );
    assert_eq!(
        queue.env.started_ids(),
        [ids[0].clone()],
        "продолжение запускает первую по порядку"
    );
}

#[tokio::test]
async fn resuming_a_running_queue_is_nothing_at_all() {
    let queue = Harness::new();
    queue.start_ok(request(streams(None, Some("140"))));
    let events = queue.env.events().len();

    queue.resume();

    assert_eq!(
        queue.env.events().len(),
        events,
        "вызов при неприостановленной очереди — не ошибка и не событие"
    );
}

// ─────────── Граница задач и пауза на обновление (Ф-7, Р-7) ───────────

#[tokio::test]
async fn the_boundary_is_busy_while_a_task_runs_and_free_between_tasks() {
    let queue = Harness::new();
    queue.start_ok(request(streams(None, Some("140"))));

    assert!(
        queue.scheduler.is_active(),
        "пока задача в работе, контур обновления обязан видеть занятость"
    );

    // Ждущий регистрируется до терминального исхода — то есть ровно в том
    // порядке, в каком это делает контур E6.
    let scheduler = Arc::clone(&queue.scheduler);
    let waiting = tokio::spawn(async move { scheduler.wait_for_task_boundary().await });

    // Ждущий обязан **уснуть** до того, как задача кончится, иначе тест
    // проверял бы не будильник, а везение: проснувшийся после исхода
    // возвращается сам, по проверке состояния, и молчащее уведомление
    // осталось бы незамеченным (эта дыра нашлась мутацией). На
    // однопоточном рантайме теста уступка планировщику гарантирует, что
    // задача опрошена и встала на уведомление.
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    assert!(
        !waiting.is_finished(),
        "пока задача идёт, ждущий обязан спать, а не возвращаться"
    );

    queue.finish(0, done()).await;

    tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .expect("граница задач обязана наступить после терминального исхода")
        .expect("ждущий не падает");
    assert!(!queue.scheduler.is_active(), "слот свободен");
}

#[tokio::test]
async fn the_next_task_starts_only_after_the_update_has_finished_at_the_boundary() {
    // Р-7 и Н-3 E6: прогрев (24–36 с дисковой работы) не соревнуется с
    // загрузкой. Пауза между задачами и есть та граница, где контур
    // делает своё дело, — и старт следующей ждёт его окончания.
    let queue = Harness::new();
    let first = queue.start_ok(request(streams(None, Some("140"))));
    let second = queue.start_ok(request_for(OTHER_URL, streams(None, Some("140"))));

    queue.env.holds_boundary.store(true, Ordering::SeqCst);
    let scheduler = Arc::clone(&queue.scheduler);
    let env = Arc::clone(&queue.dyn_env);
    let task = queue.env.started(0);
    task.set_progress(done());
    let finishing = tokio::spawn(async move { task_finished(&scheduler, &task.id, &env).await });

    // Пока контур не отпустил границу, вторая задача не начата, а пауза
    // видна пользователю отдельным полем снимка.
    wait_until(
        "очередь обязана дойти до паузы",
        || queue.env.waits.load(Ordering::SeqCst) == 1,
    )
    .await;
    assert_eq!(
        queue.snapshot().pause_reason,
        Some(QueuePauseReason::YtDlpUpdate),
        "пауза объявлена, а не молчалива (С-8)"
    );
    assert!(
        !queue.scheduler.is_active(),
        "на паузе слот свободен — иначе контур не начал бы вовсе"
    );
    assert_eq!(
        queue.env.started_ids(),
        std::slice::from_ref(&first),
        "старт следующей ждёт окончания прогрева и переключения"
    );

    queue.env.release.add_permits(1);
    finishing.await.expect("граница отработала");

    assert_eq!(queue.env.started_ids(), [first, second]);
    assert_eq!(
        queue.snapshot().pause_reason,
        None,
        "строка паузы исчезает сама, без действий пользователя"
    );
}

#[tokio::test]
async fn without_a_prepared_update_the_boundary_costs_nothing() {
    let queue = Harness::new();
    queue.start_ok(request(streams(None, Some("140"))));
    queue.start_ok(request_for(OTHER_URL, streams(None, Some("140"))));

    queue.finish(0, done()).await;

    assert_eq!(
        queue.env.waits.load(Ordering::SeqCst),
        0,
        "контур границы не держит — ждать нечего"
    );
    assert_eq!(
        queue.env.events().last().expect("событие").pause_reason,
        None,
        "паузы в подавляющем большинстве переходов не бывает вовсе"
    );
}

// ─────────────────── События состава (Ф-6) ───────────────────

#[tokio::test]
async fn every_structural_change_sends_the_whole_snapshot() {
    let queue = Harness::new();
    let first = queue.start_ok(request(streams(None, Some("140"))));
    let second = queue.start_ok(request_for(OTHER_URL, streams(None, Some("140"))));
    queue.finish(0, done()).await;
    queue.dismiss(&first).expect("завершённую скрыть можно");

    let events = queue.env.events();
    assert!(
        events.len() >= 5,
        "события обязаны быть на постановку, старт, исход и скрытие: {}",
        events.len()
    );
    assert_eq!(
        events.last().expect("последнее событие").tasks.len(),
        1,
        "событие несёт весь снимок целиком, а не дельту"
    );
    assert_eq!(
        events.last().expect("последнее событие").tasks[0].task_id,
        second
    );

    // Снимок команды и снимок события — одно и то же значение, иначе
    // экран собирался бы двумя разными способами.
    assert_eq!(events.last().cloned(), Some(queue.snapshot()));
}

// ─────────── Окно вытеснения, закрытое очередью ───────────

/// Запускатель, который висит, пока задачу не отменят.
///
/// Так выглядит идущая загрузка: процесс жив, задача нетерминальна, слот
/// занят. Ровно в этот момент и приходит второй старт.
struct HangingLauncher {
    launches: AtomicUsize,
}

impl DownloadLauncher for HangingLauncher {
    fn launch<'a>(
        &'a self,
        _args: &'a [&'a str],
        handle: &'a RunHandle,
        _first_deadline: Instant,
        _on_line: &'a mut (dyn FnMut(&str) -> Option<Instant> + Send),
    ) -> Pin<Box<dyn Future<Output = Result<StreamedRun, SidecarError>> + Send + 'a>> {
        Box::pin(async move {
            self.launches.fetch_add(1, Ordering::SeqCst);
            handle.cancelled().await;
            Ok(StreamedRun {
                exit_code: None,
                stderr: String::new(),
                deadline_expired: false,
            })
        })
    }
}

/// ffmpeg, которого в этих сценариях не бывает: поток один, склейки нет.
struct NoFfmpeg;

impl FfmpegLauncher for NoFfmpeg {
    fn launch<'a>(
        &'a self,
        _args: &'a [&'a str],
        _timeout: Duration,
        _handle: &'a RunHandle,
    ) -> Pin<Box<dyn Future<Output = Result<RunOutput, SidecarError>> + Send + 'a>> {
        Box::pin(async move {
            panic!("склейки в этом сценарии быть не может")
        })
    }
}

struct SilentSink;

impl ProgressSink for SilentSink {
    fn emit(&self, _event: DownloadProgressEvent) {}
}

/// Окружение с **настоящим** воркером: задачу ведёт `run_task`.
struct LiveEnv {
    launcher: Arc<HangingLauncher>,
    destination: std::path::PathBuf,
}

impl QueueEnv for LiveEnv {
    fn emit(&self, _snapshot: QueueSnapshot) {}

    fn spawn(self: Arc<Self>, scheduler: Arc<QueueScheduler>, task: Arc<DownloadTask>) {
        tokio::spawn(async move {
            let destination = self.destination.clone();
            run_task(
                &task,
                self.launcher.as_ref(),
                &NoFfmpeg,
                &SilentSink,
                &destination,
            )
            .await;
            let env: Arc<dyn QueueEnv> = self;
            task_finished(&scheduler, &task.id, &env).await;
        });
    }

    fn update_holds_boundary(&self) -> bool {
        false
    }

    fn wait_for_update(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async {})
    }
}

/// Ждёт условия, не полагаясь на сон фиксированной длины.
async fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "не дождались: {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn a_second_start_queues_instead_of_evicting_the_running_download() {
    // Окно, ради закрытия которого заведена очередь: до неё второй клик
    // по «Скачать» отменял часовую загрузку — слот занимала новая задача,
    // а её воркер добивал предшественницу. Здесь задачу ведёт настоящий
    // `run_task`, поэтому утверждение проверяется исходом живого воркера,
    // а не протоколом вызовов.
    let dir = tempfile::tempdir().expect("временный каталог");
    let launcher = Arc::new(HangingLauncher {
        launches: AtomicUsize::new(0),
    });
    let live = Arc::new(LiveEnv {
        launcher: Arc::clone(&launcher),
        destination: dir.path().to_path_buf(),
    });
    let env: Arc<dyn QueueEnv> = live.clone();
    let scheduler = Arc::new(QueueScheduler::new(None));

    let first = start_download(&scheduler, request(streams(None, Some("140"))), &env)
        .expect("первая постановка")
        .task_id;
    wait_until(
        "первая задача обязана дойти до процесса",
        || launcher.launches.load(Ordering::SeqCst) == 1,
    )
    .await;

    let second = start_download(
        &scheduler,
        request_for(OTHER_URL, streams(None, Some("140"))),
        &env,
    )
    .expect("постановка при занятом слоте — не отказ (Ф-2)")
    .task_id;

    // Даём вытеснению шанс проявиться: если бы оно осталось, первая
    // задача стала бы терминальной в ближайшие миллисекунды.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let snapshot = queue_snapshot(&scheduler);
    assert_eq!(
        snapshot.tasks.len(),
        2,
        "обе задачи в списке: вторая встала в хвост, а не заменила первую"
    );
    assert_eq!(snapshot.tasks[0].task_id, first);
    assert!(
        !snapshot.tasks[0].progress.is_terminal(),
        "идущая загрузка не отменена вторым стартом: {:?}",
        snapshot.tasks[0].progress
    );
    assert_eq!(
        snapshot.tasks[1].progress.phase(),
        DownloadPhase::Queued,
        "вторая ждёт своей очереди"
    );
    assert_eq!(
        launcher.launches.load(Ordering::SeqCst),
        1,
        "процесс по-прежнему один — вторая задача не запускала своего"
    );

    // И только отмена первой пускает вторую — штатным путём.
    cancel_download(&scheduler, &first, &env)
        .await
        .expect("отмена активной");
    wait_until(
        "вторая задача обязана стартовать после исхода первой",
        || launcher.launches.load(Ordering::SeqCst) == 2,
    )
    .await;

    let snapshot = queue_snapshot(&scheduler);
    assert_eq!(
        snapshot.tasks[0].progress.phase(),
        DownloadPhase::Cancelled,
        "первая кончилась отменой пользователя, а не вытеснением"
    );
    assert_eq!(snapshot.tasks[1].task_id, second);

    cancel_download(&scheduler, &second, &env)
        .await
        .expect("уборка за собой");
    wait_until(
        "вторая задача обязана дойти до исхода",
        || queue_snapshot(&scheduler).tasks[1].progress.is_terminal(),
    )
    .await;
}

#[tokio::test]
async fn a_task_is_run_by_exactly_one_worker_at_a_time() {
    // Р-1 буквально: сколько бы задач ни стояло, запущенный процесс один.
    let dir = tempfile::tempdir().expect("временный каталог");
    let launcher = Arc::new(HangingLauncher {
        launches: AtomicUsize::new(0),
    });
    let live = Arc::new(LiveEnv {
        launcher: Arc::clone(&launcher),
        destination: dir.path().to_path_buf(),
    });
    let env: Arc<dyn QueueEnv> = live.clone();
    let scheduler = Arc::new(QueueScheduler::new(None));

    let mut ids = Vec::new();
    for url in [
        URL,
        OTHER_URL,
        "https://www.youtube.com/watch?v=aaaaaaaaaaa",
    ] {
        ids.push(
            start_download(
                &scheduler,
                request_for(url, streams(None, Some("140"))),
                &env,
            )
            .expect("постановка")
            .task_id,
        );
    }

    wait_until(
        "первая задача обязана дойти до процесса",
        || launcher.launches.load(Ordering::SeqCst) == 1,
    )
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        launcher.launches.load(Ordering::SeqCst),
        1,
        "три задачи в очереди — один запущенный процесс"
    );

    for id in ids {
        cancel_download(&scheduler, &id, &env)
            .await
            .expect("отмена по очереди");
        wait_until(
            "задача обязана дойти до исхода",
            || {
                queue_snapshot(&scheduler)
                    .tasks
                    .iter()
                    .find(|task| task.task_id == id)
                    .is_some_and(|task| task.progress.is_terminal())
            },
        )
        .await;
    }

    assert!(
        queue_snapshot(&scheduler)
            .tasks
            .iter()
            .all(|task| task.progress.phase() == DownloadPhase::Cancelled),
        "все три кончились отменой пользователя"
    );
}
