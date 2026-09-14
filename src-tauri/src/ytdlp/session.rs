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
use super::prepare::{self, BackgroundOutcome, BackgroundWarmup, ProgressSink, WarmLaunch};
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

    /// Выполняет фоновый прогрев, выданный [`Self::prepare`], и отмечает
    /// его завершение — в том числе если future бросили недовыполненным.
    pub async fn run_background(
        &self,
        warmup: BackgroundWarmup,
        registry: &ChildRegistry,
    ) -> BackgroundOutcome {
        struct InFlight<'a>(&'a Session);

        impl Drop for InFlight<'_> {
            fn drop(&mut self) {
                self.0.lock().background_in_flight = false;
            }
        }

        let _in_flight = InFlight(self);
        warmup.run(registry).await
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

        let (outcome, ()) = tokio::join!(session.run_background(first, &fixture.registry), async {
            fixture.control.wait_until_hanging().await;
            fixture.control.release();
        });
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
}
