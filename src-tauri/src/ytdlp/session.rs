//! Память подготовки yt-dlp на время сеанса приложения (TL-23, TL-21).
//!
//! # Зачем
//!
//! На тёплом старте yt-dlp запускался трижды, хотя одного запуска
//! достаточно (замер TL-23: 770–874 мс против ~260 мс на запуск). Первый —
//! проба в подготовке, которую `setup` затевает сам. Второй — та же проба в
//! подготовке, которую зовёт фронтенд: он ждёт первую на мьютексе и
//! застаёт готовую установку, но проверяет её запуском заново. Третий —
//! `check_sidecar`, которому нужна версия для служебного экрана.
//!
//! Сеанс сводит это к одному запуску, не трогая контракт с фронтендом:
//!
//! - удачная подготовка запоминается, и вторая дверь в неё отвечает
//!   запомненным итогом без запуска ([`Session::prepare`]);
//! - запуск, которым проба застала дерево тёплым, отдаётся служебному
//!   экрану **один раз** ([`Session::take_warm_launch`]): строка yt-dlp
//!   собирается из того же вывода, из которого собралась бы после
//!   отдельного запуска.
//!
//! # Почему запуск отдаётся один раз, а не на весь сеанс
//!
//! Из-за кнопки «Повторить проверку». Фронтенд по ней снова зовёт
//! подготовку и проверку; подготовка отвечает из памяти, и отдавай сеанс
//! запуск пробы каждой проверке — повтор не запустил бы yt-dlp ни разу, то
//! есть ничего бы не проверил. Поэтому результат пробы достаётся только
//! первой проверке после неё (обычный старт), а любая следующая запускает
//! бинарник заново.
//!
//! # Когда память забывается
//!
//! Сеанс помнит только то, что верно для текущего содержимого корня
//! установок. Контур обновления (E6) его меняет — ставит кандидата,
//! переключает активную запись, откатывается, убирает лишнее, — и после
//! любого из этих шагов память сбрасывается ([`Session::invalidate`]):
//! иначе экран показал бы версию, которой приложение уже не работает.
//! Защита двойная: запуск отдаётся только проверке того же пути, что
//! запускала проба, а переключение меняет путь.
//!
//! На диск ничего не пишется: «дерево тёплое» — свойство ОС, и между
//! запусками приложения его всё равно проверяет проба (см.
//! `super::prepare`, «Почему проверка запуском на каждом старте»).
//!
//! # Фоновый прогрев
//!
//! Если подготовка вернула прогрев, который продолжится в фоне (TL-21),
//! сеанс выдаёт его ровно одному вызывающему и помнит, что он идёт:
//! повторная подготовка после сброса памяти не запустит второй прогрев
//! параллельно первому — проверка подписей в ОС сериализована, и второй
//! прогон только отнял бы у первого диск.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

use super::error::PrepareError;
use super::prepare::{
    self, BackgroundOutcome, BackgroundWarmup, ProgressSink, WarmLaunch, WarmupSink,
};
use crate::sidecar::ChildRegistry;
use crate::types::YtDlpPrepared;

/// Память подготовки на время сеанса: одна на процесс.
#[derive(Debug, Default)]
pub struct Session {
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    /// Растёт при каждом сбросе. Подготовка запоминает итог, только если
    /// сброса не было, пока она шла: иначе итог описывал бы корень
    /// установок до изменения.
    generation: u64,
    prepared: Option<YtDlpPrepared>,
    warm_launch: Option<WarmLaunch>,
    background_in_flight: bool,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // Отравленный мьютекс не повод ронять старт: внутри два снимка и
        // флаг, паника чужого потока не делает их противоречивыми (тот же
        // приём, что в `InUse`).
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Подготовка с памятью сеанса.
    ///
    /// Вызывающий держит `PreparationLock` (`crate::commands::ytdlp`):
    /// сеанс не сериализует подготовки сам, он только помнит их итог.
    ///
    /// Запомненный итог возвращается с `prepared: false` — работу, если
    /// она была, сделал и показал событиями другой вызывающий — и с
    /// длительностью этого вызова. Фоновый прогрев возвращается не больше
    /// одного за раз (см. doc модуля).
    pub async fn prepare(
        &self,
        archive_path: &Path,
        data_dir: &Path,
        registry: &ChildRegistry,
        sink: &dyn ProgressSink,
    ) -> Result<(YtDlpPrepared, Option<BackgroundWarmup>), PrepareError> {
        let started = Instant::now();
        let generation = {
            let state = self.lock();
            if let Some(remembered) = &state.prepared {
                let mut again = remembered.clone();
                again.prepared = false;
                again.duration_ms =
                    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                return Ok((again, None));
            }
            state.generation
        };

        let outcome = prepare::prepare(archive_path, data_dir, registry, sink).await?;

        let mut state = self.lock();
        if state.generation == generation {
            state.prepared = Some(outcome.prepared.clone());
            state.warm_launch = outcome.warm_launch;
        }

        let background = match outcome.background {
            Some(warmup) if !state.background_in_flight => {
                state.background_in_flight = true;
                Some(warmup)
            }
            Some(_) => {
                eprintln!("yt-dlp: фоновый прогрев уже идёт — второй не запускаю");
                None
            }
            None => None,
        };

        Ok((outcome.prepared, background))
    }

    /// Запуск, которым проба подготовки застала дерево тёплым, — если он
    /// был, ещё не отдан и запускался именно `executable`.
    ///
    /// Забирается при любом исходе: отдан, не совпал путь или его не было.
    /// Несовпавший путь означает, что между пробой и проверкой установка
    /// сменилась, и версия пробы про неё ничего не говорит.
    pub fn take_warm_launch(&self, executable: &Path) -> Option<WarmLaunch> {
        let launch = self.lock().warm_launch.take()?;
        if launch.executable == executable {
            Some(launch)
        } else {
            eprintln!(
                "yt-dlp: проба подготовки запускала {}, а проверяется {} — запускаю заново",
                launch.executable.display(),
                executable.display()
            );
            None
        }
    }

    /// Забывает всё, что сеанс помнил о подготовке: корень установок
    /// изменился (E6).
    pub fn invalidate(&self) {
        let mut state = self.lock();
        state.generation = state.generation.wrapping_add(1);
        state.prepared = None;
        state.warm_launch = None;
    }

    /// Выполняет фоновый прогрев, выданный [`Self::prepare`], отмечает его
    /// завершение — в том числе если future бросили недовыполненным — и
    /// сообщает исход в `sink` (TL-21).
    ///
    /// Без события экран не узнавал о конце прогрева вовсе: подготовка его
    /// не ждала, а строка yt-dlp на служебном экране так и оставалась той,
    /// что собрана, пока дерево ещё было холодным.
    ///
    /// Событие уходит **после** того, как снята отметка «прогрев идёт»:
    /// экран по нему зовёт подготовку и проверку, и те обязаны застать
    /// сеанс уже без прогрева. Брошенный future события не шлёт — так
    /// бывает только при выходе из приложения, когда слушать некому.
    pub async fn run_background(
        &self,
        warmup: BackgroundWarmup,
        registry: &ChildRegistry,
        sink: &dyn WarmupSink,
    ) -> BackgroundOutcome {
        struct InFlight<'a>(&'a Session);

        impl Drop for InFlight<'_> {
            fn drop(&mut self) {
                self.0.lock().background_in_flight = false;
            }
        }

        let outcome = {
            let _in_flight = InFlight(self);
            warmup.run(registry).await
        };

        // Отказ фонового прогрева — дерево, которое не запускается (или
        // зависает раз за разом), а не медленная машина. Подготовка,
        // которую помнит сеанс, про него уже неправда: без сброса «Повторить
        // проверку» получила бы из памяти прежний успех и до пробы с
        // переустановкой не дошла бы (Д-1 ревью TL-23). Сброс — до события:
        // экран по нему и зовёт повтор.
        if matches!(outcome, BackgroundOutcome::Failed(_)) {
            self.invalidate();
        }

        sink.warmup_finished(outcome.event());
        outcome
    }

    /// Помнит ли сеанс хоть что-то — вопрос тестов инвалидации.
    #[cfg(test)]
    pub(crate) fn remembers_anything(&self) -> bool {
        let state = self.lock();
        state.prepared.is_some() || state.warm_launch.is_some()
    }

    /// Заполняет память так, как её заполнила бы удачная подготовка, —
    /// для тестов контура обновления, где настоящей подготовки нет.
    #[cfg(test)]
    pub(crate) fn remember_for_test(&self, prepared: YtDlpPrepared, warm_launch: WarmLaunch) {
        let mut state = self.lock();
        state.prepared = Some(prepared);
        state.warm_launch = Some(warm_launch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{YtDlpWarmupEvent, YtDlpWarmupOutcome};
    use crate::ytdlp::layout::{self, Layout, SlowWarmupMark};
    use crate::ytdlp::testing::{Control, SilentSink, EXECUTABLE};
    use std::path::PathBuf;
    use tempfile::{tempdir, TempDir};

    struct Fixture {
        _dir: TempDir,
        archive: PathBuf,
        data_dir: PathBuf,
        control: Control,
        registry: ChildRegistry,
    }

    /// Установка, уже распакованная и прогретая «прошлым запуском
    /// приложения», и счётчик запусков её yt-dlp.
    async fn installed() -> Fixture {
        let dir = tempdir().expect("tempdir");
        let control = Control::new(&dir.path().join("control"));
        let archive = dir.path().join("yt-dlp.zip");
        crate::ytdlp::testing::write_onedir_zip(
            &archive,
            EXECUTABLE,
            &control.script("2026.08.19"),
        );
        let data_dir = dir.path().join("app-data");
        let registry = ChildRegistry::new();

        Session::new()
            .prepare(&archive, &data_dir, &registry, &SilentSink)
            .await
            .expect("первый запуск приложения обязан подготовить yt-dlp");

        Fixture {
            _dir: dir,
            archive,
            data_dir,
            control,
            registry,
        }
    }

    impl Fixture {
        async fn prepare(&self, session: &Session) -> (YtDlpPrepared, Option<BackgroundWarmup>) {
            session
                .prepare(&self.archive, &self.data_dir, &self.registry, &SilentSink)
                .await
                .expect("подготовка обязана пройти")
        }

        fn mark_path(&self) -> PathBuf {
            Layout::new(&self.data_dir)
                .slow_warmup_path(&layout::bundled_build_id().expect("пин проходит проверку"))
        }
    }

    #[tokio::test]
    async fn the_second_door_into_preparation_does_not_launch_yt_dlp_again() {
        let fixture = installed().await;
        let session = Session::new();
        let before = fixture.control.launches();

        let (setup, _) = fixture.prepare(&session).await;
        let (frontend, _) = fixture.prepare(&session).await;

        assert_eq!(
            fixture.control.launches() - before,
            1,
            "пробу делает только первая дверь"
        );
        assert_eq!(setup.version, frontend.version);
        assert_eq!(setup.path, frontend.path);
        assert!(!frontend.prepared, "вторая дверь работы не делала");
    }

    #[tokio::test]
    async fn the_warm_launch_is_given_away_once_and_only_for_its_own_path() {
        let fixture = installed().await;
        let session = Session::new();
        let (prepared, _) = fixture.prepare(&session).await;
        let path = PathBuf::from(&prepared.path);

        assert!(
            session
                .take_warm_launch(&fixture.data_dir.join("elsewhere"))
                .is_none(),
            "запуск другого пути не годится"
        );
        assert!(
            session.take_warm_launch(&path).is_none(),
            "несовпавший путь забирает запуск: установка сменилась"
        );

        let session = Session::new();
        let (prepared, _) = fixture.prepare(&session).await;
        let path = PathBuf::from(&prepared.path);
        let launch = session
            .take_warm_launch(&path)
            .expect("запуск пробы отдаётся");
        assert_eq!(launch.executable, path);
        assert_eq!(launch.output.stdout.trim(), "2026.08.19");
        assert!(
            session.take_warm_launch(&path).is_none(),
            "второй проверке запуск пробы не достаётся — повтор обязан запускать бинарник"
        );
    }

    #[tokio::test]
    async fn a_forgotten_session_probes_again() {
        let fixture = installed().await;
        let session = Session::new();
        fixture.prepare(&session).await;
        let before = fixture.control.launches();

        session.invalidate();
        assert!(!session.remembers_anything());
        let (prepared, _) = fixture.prepare(&session).await;

        assert_eq!(
            fixture.control.launches() - before,
            1,
            "после сброса подготовка обязана проверить дерево запуском"
        );
        assert!(session
            .take_warm_launch(Path::new(&prepared.path))
            .is_some());
    }

    #[tokio::test]
    async fn a_failed_preparation_is_not_remembered() {
        let fixture = installed().await;
        let session = Session::new();
        std::fs::remove_file(&fixture.archive).expect("убрать архив");
        std::fs::remove_dir_all(Layout::new(&fixture.data_dir).root()).expect("убрать установки");

        session
            .prepare(
                &fixture.archive,
                &fixture.data_dir,
                &fixture.registry,
                &SilentSink,
            )
            .await
            .expect_err("архива нет — подготовке не из чего");

        assert!(
            !session.remembers_anything(),
            "отказ не запоминается: повтор обязан попробовать снова"
        );
    }

    #[tokio::test]
    async fn invalidate_during_an_in_flight_prepare_is_not_remembered() {
        // Заметка 7 ревью TL-23: сброс, пришедший, пока подготовка шла,
        // обязан пережить её завершение.
        let fixture = installed().await;
        let session = Session::new();
        fixture.control.hang();
        let ((prepared, _), ()) = tokio::join!(fixture.prepare(&session), async {
            fixture.control.wait_until_hanging().await;
            session.invalidate();
            fixture.control.release();
        });
        assert!(
            !session.remembers_anything(),
            "итог подготовки, пережившей сброс, не запоминается"
        );
        assert!(session
            .take_warm_launch(Path::new(&prepared.path))
            .is_none());
        let before = fixture.control.launches();
        let _ = fixture.prepare(&session).await;
        assert_eq!(
            fixture.control.launches() - before,
            1,
            "после сброса — снова проба"
        );
    }

    #[tokio::test]
    async fn a_background_warm_up_that_fails_makes_the_session_forget_its_preparation() {
        // Д-1 ревью TL-23: без сброса «Повторить проверку» получала из памяти
        // прежний успех и до пробы с переустановкой не доходила.
        let fixture = installed().await;
        let session = Session::new();
        fixture.write_mark();
        fixture.break_executable();

        let (_, background) = fixture.prepare(&session).await;
        assert!(session.remembers_anything(), "предусловие: итог запомнен");

        let outcome = session
            .run_background(
                background.expect("отметка есть — прогрев в фоне"),
                &fixture.registry,
                &RecordingWarmup::default(),
            )
            .await;

        assert!(
            matches!(outcome, BackgroundOutcome::Failed(_)),
            "{outcome:?}"
        );
        assert!(
            !session.remembers_anything(),
            "отказ фонового прогрева сбрасывает память сеанса"
        );
    }

    #[tokio::test]
    async fn only_one_background_warm_up_runs_at_a_time() {
        let fixture = installed().await;
        SlowWarmupMark::recorded(None, 120_000, crate::clock::now_unix_secs())
            .write_atomic(&fixture.mark_path())
            .expect("отметка записывается");
        fixture.control.hang();

        let session = Session::new();
        let (_, first) = fixture.prepare(&session).await;
        let first = first.expect("отметка есть — прогрев уходит в фон");

        session.invalidate();
        let (_, second) = fixture.prepare(&session).await;
        assert!(second.is_none(), "пока первый идёт, второй не выдаётся");

        let sink = RecordingWarmup::default();
        let (outcome, ()) = tokio::join!(
            session.run_background(first, &fixture.registry, &sink),
            async {
                fixture.control.wait_until_hanging().await;
                fixture.control.release();
            }
        );
        assert_eq!(outcome, BackgroundOutcome::Warmed);

        // Прогрев снял отметку, и следующая подготовка идёт обычным путём;
        // выдать фоновый прогрев снова сеанс вправе — прежний закончился.
        SlowWarmupMark::recorded(None, 120_000, crate::clock::now_unix_secs())
            .write_atomic(&fixture.mark_path())
            .expect("отметка записывается");
        session.invalidate();
        let (_, third) = fixture.prepare(&session).await;
        assert!(third.is_some(), "завершившийся прогрев освобождает место");
    }

    /// Приёмник, который запоминает исходы из событий конца прогрева.
    #[derive(Default)]
    struct RecordingWarmup(Mutex<Vec<YtDlpWarmupEvent>>);

    impl RecordingWarmup {
        fn outcomes(&self) -> Vec<YtDlpWarmupOutcome> {
            self.0
                .lock()
                .expect("mutex")
                .iter()
                .map(|event| event.outcome)
                .collect()
        }
    }

    impl WarmupSink for RecordingWarmup {
        fn warmup_finished(&self, event: YtDlpWarmupEvent) {
            self.0.lock().expect("mutex").push(event);
        }
    }

    impl Fixture {
        fn write_mark(&self) {
            SlowWarmupMark::recorded(None, 120_000, crate::clock::now_unix_secs())
                .write_atomic(&self.mark_path())
                .expect("отметка записывается");
        }

        /// Ломает установленный yt-dlp, сохраняя размер: сверка с
        /// манифестом его пропускает, запуск — нет.
        fn break_executable(&self) {
            let path = Layout::new(&self.data_dir)
                .install_dir(&layout::bundled_build_id().expect("пин проходит проверку"))
                .join(EXECUTABLE);
            let size = usize::try_from(std::fs::metadata(&path).expect("размер").len())
                .expect("размер помещается");
            let mut broken = b"#!/bin/sh\nexit 3\n".to_vec();
            broken.resize(size, b'#');
            std::fs::write(&path, broken).expect("сломать, сохранив размер");
        }
    }

    #[tokio::test]
    async fn the_end_of_a_background_warm_up_is_reported_with_its_outcome() {
        // Б-1 ревью TL-21: подготовка прогрев не ждёт, и без события экран
        // не узнавал, что он кончился.
        let fixture = installed().await;
        let session = Session::new();

        // Уложился.
        fixture.write_mark();
        fixture.control.hang();
        let (_, background) = fixture.prepare(&session).await;
        let sink = RecordingWarmup::default();
        let (outcome, ()) = tokio::join!(
            session.run_background(
                background.expect("отметка есть — прогрев в фоне"),
                &fixture.registry,
                &sink,
            ),
            async {
                fixture.control.wait_until_hanging().await;
                assert!(sink.outcomes().is_empty(), "пока прогрев идёт, события нет");
                fixture.control.release();
            }
        );
        assert_eq!(outcome, BackgroundOutcome::Warmed);
        assert_eq!(sink.outcomes(), vec![YtDlpWarmupOutcome::Warmed]);

        // Не запустился.
        fixture.write_mark();
        fixture.break_executable();
        session.invalidate();
        let (_, background) = fixture.prepare(&session).await;
        let sink = RecordingWarmup::default();
        let outcome = session
            .run_background(
                background.expect("отметка есть — прогрев в фоне"),
                &fixture.registry,
                &sink,
            )
            .await;
        assert!(
            matches!(outcome, BackgroundOutcome::Failed(_)),
            "{outcome:?}"
        );
        assert_eq!(sink.outcomes(), vec![YtDlpWarmupOutcome::Failed]);

        // Класс «снова не уложился» — свой, а не один из двух выше.
        assert_eq!(
            BackgroundOutcome::TimedOut.event().outcome,
            YtDlpWarmupOutcome::TimedOut
        );
        assert_eq!(
            serde_json::to_value(BackgroundOutcome::Warmed.event()).expect("сериализуется"),
            serde_json::json!({ "outcome": "warmed" })
        );
    }
}
