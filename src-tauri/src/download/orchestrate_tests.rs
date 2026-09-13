//! Тесты оркестрации задачи скачивания (TL-44).
//!
//! Настоящих процессов здесь нет и сети тоже: запуск подменён сценарием
//! ([`ScriptedLauncher`]), склейка — фальшивым ffmpeg. Зато **строки
//! прогресса настоящие** — они берутся из фикстур, снятых живьём с
//! yt-dlp (`tests/fixtures/ytdlp-download/`), и проходят тот же разбор и
//! ту же агрегацию, что в бою. Сценарий задаёт только то, чего в фикстуре
//! нет: когда процесс кончится, каким кодом, что он создаст на диске и
//! что при этом окажется в stderr.
//!
//! Файлы при этом настоящие: сценарий их создаёт, подчистка удаляет,
//! финализация переименовывает — «что осталось на диске» проверяется
//! обходом каталога, а не утверждением о намерениях.

use std::collections::{BTreeSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};

use tempfile::TempDir;

use super::*;
use crate::download::fixtures::{self, DESTINATION_PLACEHOLDER, SINGLE_LAUNCH_FIXTURES};
use crate::download::merge::MERGE_TIMEOUT_SECS;
use crate::download::progress::parse_line;
use crate::download::retry::{MAX_ATTEMPTS, NO_PROGRESS_TIMEOUT, SOCKET_TIMEOUT_SECS};
use crate::queue::video_id::canonical_video_id;
use crate::sidecar::RunOutput;
use crate::types::{DownloadPhase, QualityKind, QualitySize, SelectedQuality, YtDlpFailureReason};

// ─────────────────────────── Оснастка ───────────────────────────

/// Что делает подменённый запуск.
#[derive(Debug, Clone)]
enum Step {
    /// Отдать строку stdout.
    Line(String),
    /// Создать файл в папке назначения — то, что по ходу дела делает
    /// сам yt-dlp.
    Creates(String),
    /// Висеть, пока не убьют. Так выглядит процесс, который отменяют.
    HangUntilCancelled,
    /// Сдвинуть управляемые часы рантайма — процесс молчит это время.
    /// Работает только под `tokio::time::pause()` и без него падает, а не
    /// ждёт по-настоящему.
    Advance(Duration),
}

/// Сценарий одного запуска.
#[derive(Debug, Clone, Default)]
struct Script {
    steps: Vec<Step>,
    exit_code: Option<i32>,
    stderr: String,
    /// Запуск снят сроком бездействия (сторож).
    deadline_expired: bool,
}

impl Script {
    fn ok() -> Self {
        Self {
            exit_code: Some(0),
            ..Self::default()
        }
    }

    fn failing(exit_code: i32, stderr: &str) -> Self {
        Self {
            exit_code: Some(exit_code),
            stderr: stderr.to_string(),
            ..Self::default()
        }
    }

    /// Попытка, снятая сторожем продвижения.
    fn stalled() -> Self {
        Self {
            exit_code: None,
            deadline_expired: true,
            ..Self::default()
        }
    }

    fn line(mut self, line: &str) -> Self {
        self.steps.push(Step::Line(line.to_string()));
        self
    }

    fn lines(mut self, lines: impl IntoIterator<Item = String>) -> Self {
        self.steps.extend(lines.into_iter().map(Step::Line));
        self
    }

    fn creates(mut self, name: &str) -> Self {
        self.steps.push(Step::Creates(name.to_string()));
        self
    }

    fn hangs(mut self) -> Self {
        self.steps.push(Step::HangUntilCancelled);
        self
    }

    fn advance(mut self, by: Duration) -> Self {
        self.steps.push(Step::Advance(by));
        self
    }
}

/// Один состоявшийся запуск: argv и сроки, которые оркестрация выставила.
#[derive(Debug, Clone)]
struct Call {
    argv: Vec<String>,
    /// Срок, с которым запуск начался.
    first_deadline: Instant,
    /// Строки stdout, отданные запуском, по порядку.
    lines: Vec<String>,
    /// Момент каждой строки по часам оркестрации ([`monotonic_now`]).
    line_times: Vec<Instant>,
    /// Сроки, возвращённые обработчиком строк, по одному на строку.
    deadlines: Vec<Option<Instant>>,
}

impl Call {
    fn value_of(&self, flag: &str) -> Option<&str> {
        let index = self.argv.iter().position(|arg| arg == flag)?;
        self.argv.get(index + 1).map(String::as_str)
    }
}

/// Подменённый запуск yt-dlp: отдаёт заготовленные строки и код.
struct ScriptedLauncher {
    dir: PathBuf,
    scripts: StdMutex<VecDeque<Script>>,
    calls: StdMutex<Vec<Call>>,
    /// Задача, чьё состояние снимается после каждой строки.
    observed: StdMutex<Option<Arc<DownloadTask>>>,
    /// (строка, состояние задачи сразу после её обработки).
    ///
    /// Снимок, а не события приёмника: троттлинг глотает события одного
    /// вида, а снимок обновляется на каждой строке (doc `Emitter::emit`),
    /// и только по нему видно, к какому потоку отнесена **каждая** строка.
    snapshots: StdMutex<Vec<(String, DownloadProgress)>>,
}

impl ScriptedLauncher {
    fn new(dir: &Path, scripts: Vec<Script>) -> Self {
        Self {
            dir: dir.to_path_buf(),
            scripts: StdMutex::new(scripts.into()),
            calls: StdMutex::new(Vec::new()),
            observed: StdMutex::new(None),
            snapshots: StdMutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    fn formats_asked(&self) -> Vec<String> {
        self.calls()
            .iter()
            .map(|call| call.value_of("-f").expect("-f обязателен").to_string())
            .collect()
    }

    fn observe(&self, task: &Arc<DownloadTask>) {
        *self.observed.lock().unwrap() = Some(Arc::clone(task));
    }

    fn snapshots(&self) -> Vec<(String, DownloadProgress)> {
        self.snapshots.lock().unwrap().clone()
    }
}

impl DownloadLauncher for ScriptedLauncher {
    fn launch<'a>(
        &'a self,
        args: &'a [&'a str],
        handle: &'a RunHandle,
        first_deadline: Instant,
        on_line: &'a mut (dyn FnMut(&str) -> Option<Instant> + Send),
    ) -> Pin<Box<dyn Future<Output = Result<StreamedRun, SidecarError>> + Send + 'a>> {
        Box::pin(async move {
            let script = self
                .scripts
                .lock()
                .unwrap()
                .pop_front()
                .expect("сценариев запуска меньше, чем запусков");

            let mut call = Call {
                argv: args.iter().map(|arg| (*arg).to_string()).collect(),
                first_deadline,
                lines: Vec::new(),
                line_times: Vec::new(),
                deadlines: Vec::new(),
            };

            for step in &script.steps {
                match step {
                    Step::Line(line) => {
                        call.lines.push(line.clone());
                        call.line_times.push(monotonic_now());
                        call.deadlines.push(on_line(line));
                        let observed = self.observed.lock().unwrap().clone();
                        if let Some(task) = observed {
                            self.snapshots
                                .lock()
                                .unwrap()
                                .push((line.clone(), task.snapshot()));
                        }
                    }
                    Step::Creates(name) => {
                        std::fs::write(self.dir.join(name), b"stream bytes")
                            .expect("сценарий обязан уметь создать файл");
                    }
                    Step::HangUntilCancelled => handle.cancelled().await,
                    Step::Advance(by) => tokio::time::advance(*by).await,
                }
            }

            self.calls.lock().unwrap().push(call);

            Ok(StreamedRun {
                exit_code: script.exit_code,
                stderr: script.stderr.clone(),
                deadline_expired: script.deadline_expired,
            })
        })
    }
}

/// Подменённый ffmpeg: пишет файл результата и отвечает успехом.
struct ScriptedFfmpeg {
    /// Отказать вместо успеха.
    fails: bool,
    /// Висеть, пока не убьют.
    hangs: bool,
    calls: AtomicUsize,
    /// Входы каждой склейки: (видео, звук) — значения двух `-i` по порядку.
    inputs: StdMutex<Vec<(String, String)>>,
}

impl ScriptedFfmpeg {
    fn with(fails: bool, hangs: bool) -> Self {
        Self {
            fails,
            hangs,
            calls: AtomicUsize::new(0),
            inputs: StdMutex::new(Vec::new()),
        }
    }

    fn merging() -> Self {
        Self::with(false, false)
    }

    fn failing() -> Self {
        Self::with(true, false)
    }

    fn hanging() -> Self {
        Self::with(false, true)
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// Имена файлов, ушедших в склейку видео и звуком, — по последней
    /// склейке.
    fn last_input_names(&self) -> (String, String) {
        let name = |path: &str| {
            Path::new(path.strip_prefix("file:").unwrap_or(path))
                .file_name()
                .expect("вход склейки — путь к файлу")
                .to_string_lossy()
                .into_owned()
        };
        let inputs = self.inputs.lock().unwrap();
        let (video, audio) = inputs.last().expect("склейка была");
        (name(video), name(audio))
    }
}

impl FfmpegLauncher for ScriptedFfmpeg {
    fn launch<'a>(
        &'a self,
        args: &'a [&'a str],
        _timeout: Duration,
        handle: &'a RunHandle,
    ) -> Pin<Box<dyn Future<Output = Result<RunOutput, SidecarError>> + Send + 'a>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let inputs: Vec<String> = args
                .windows(2)
                .filter(|pair| pair[0] == "-i")
                .map(|pair| pair[1].to_string())
                .collect();
            if let [video, audio] = inputs.as_slice() {
                self.inputs
                    .lock()
                    .unwrap()
                    .push((video.clone(), audio.clone()));
            }

            if self.hangs {
                handle.cancelled().await;
            }
            if self.fails {
                return Err(SidecarError::NonZeroExit {
                    code: 234,
                    stderr: "Only VP8 or VP9 or AV1 video … are supported for WebM".to_string(),
                });
            }

            // Последний аргумент — выход с префиксом `file:`; настоящий
            // ffmpeg именно его и создаёт.
            let output = args
                .last()
                .and_then(|arg| arg.strip_prefix("file:"))
                .expect("последний аргумент склейки — путь результата");
            std::fs::write(output, b"merged bytes").expect("склейка обязана создать файл");

            Ok(RunOutput {
                stdout: String::new(),
                stderr: String::new(),
            })
        })
    }
}

/// Приёмник, который всё запоминает и попутно сторожит Ф-8.
struct RecordingSink {
    events: StdMutex<Vec<DownloadProgress>>,
    /// Папка назначения и финальное имя: на каждом событии до `done`
    /// проверяется, что файла под финальным именем ещё нет.
    watch: Option<(PathBuf, String)>,
    violations: StdMutex<Vec<String>>,
}

impl RecordingSink {
    fn new() -> Self {
        Self {
            events: StdMutex::new(Vec::new()),
            watch: None,
            violations: StdMutex::new(Vec::new()),
        }
    }

    /// Тот же приёмник, но следящий за требованием Ф-8: файла под
    /// финальным именем не бывает ни в одной нетерминальной фазе.
    fn watching(dir: &Path, final_name: &str) -> Self {
        Self {
            watch: Some((dir.to_path_buf(), final_name.to_string())),
            ..Self::new()
        }
    }

    fn events(&self) -> Vec<DownloadProgress> {
        self.events.lock().unwrap().clone()
    }

    fn phases(&self) -> Vec<DownloadPhase> {
        self.events().iter().map(DownloadProgress::phase).collect()
    }

    fn last(&self) -> DownloadProgress {
        self.events().last().cloned().expect("хоть одно событие")
    }

    fn violations(&self) -> Vec<String> {
        self.violations.lock().unwrap().clone()
    }
}

impl ProgressSink for RecordingSink {
    fn emit(&self, event: DownloadProgressEvent) {
        if let Some((dir, name)) = &self.watch {
            if !event.progress.is_terminal() && dir.join(name).exists() {
                self.violations.lock().unwrap().push(format!(
                    "{name} появился уже в фазе {:?}",
                    event.progress.phase()
                ));
            }
        }
        self.events.lock().unwrap().push(event.progress);
    }
}

const URL: &str = "https://www.youtube.com/watch?v=aqz-KE-bpKQ";
const TITLE: &str = "Big Buck Bunny";

fn streams(video: Option<&str>, audio: Option<&str>) -> QualityStreams {
    QualityStreams {
        video_format_id: video.map(str::to_string),
        audio_format_id: audio.map(str::to_string),
    }
}

fn request(streams: QualityStreams) -> StartDownloadRequest {
    StartDownloadRequest {
        url: URL.to_string(),
        title: TITLE.to_string(),
        quality: SelectedQuality {
            kind: QualityKind::Standard,
            height_px: Some(720),
        },
        streams,
        size: QualitySize::Known {
            bytes: LAUNCH_ITEM_BYTES,
        },
    }
}

/// Оценка размера пункта `133 + 139` в запросе — сумма размеров, которые
/// отдавал локальный сервер при съёмке `single-launch` (894 838 + 323 730).
/// Та же величина, что дал бы разбор E2: сумма `filesize` форматов пункта.
const LAUNCH_ITEM_BYTES: u64 = 894_838 + 323_730;

/// Строки прогресса одного формата из снятой живьём фикстуры.
fn fixture_progress(fixture: &str, format_id: &str) -> Vec<String> {
    fixtures::stdout(fixture)
        .lines()
        .filter(|line| {
            matches!(parse_line(line), StdoutLine::Progress(sample) if sample.format_id == format_id)
        })
        .map(str::to_string)
        .collect()
}

/// `[download] Destination: <папка>/<имя>` — строка, которой yt-dlp
/// сообщает путь потока.
fn destination_line(dir: &Path, name: &str) -> String {
    format!("[download] Destination: {}", dir.join(name).display())
}

/// Задача, созданная штатным путём — через те же проверки, что делает
/// постановка в очередь.
///
/// Идентификатор выдаёт счётчик тестов, а не планировщик: очередь этим
/// файлом не проверяется вовсе (её тесты — в `crate::queue::scheduler`),
/// а два соседних вызова обязаны давать разные задачи.
fn new_task(request: StartDownloadRequest) -> Arc<DownloadTask> {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let id = format!("dl-тест-{}", NEXT.fetch_add(1, Ordering::SeqCst));
    build_task(id, request).expect("запрос обязан быть принят")
}

/// Имена файлов в папке, отсортированные.
fn dir_listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("папка назначения обязана существовать")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Полный прогон задачи с двумя потоками до готового файла.
async fn run_two_streams(
    dir: &TempDir,
    sink: &RecordingSink,
    scripts: Vec<Script>,
    ffmpeg: &ScriptedFfmpeg,
) -> Arc<DownloadTask> {
    let task = new_task(request(streams(Some("133"), Some("139"))));
    let launcher = ScriptedLauncher::new(dir.path(), scripts);

    run_task(&task, &launcher, ffmpeg, sink, dir.path()).await;
    task
}

/// Сценарий успешного скачивания двух потоков **одним** запуском (TL-48) —
/// снятый запуск `-f 133,139` (`single-launch/video-and-audio.json`).
fn two_stream_scripts(dir: &Path) -> Vec<Script> {
    vec![launch_script("video-and-audio.json", dir)]
}

/// Сценарий из снятой фикстуры одного запуска (TL-48).
///
/// Строки, код и stderr — как сняты, с папкой теста вместо плейсхолдера.
/// От себя сценарий добавляет только то, что yt-dlp делает с диском:
/// файлы из `listingBefore` появляются до первой строки, файл потока — на
/// строке `finished` его формата, под именем из его `Destination`. Что
/// из этого вышло, сверяется с `listingAfter` той же съёмки: иначе
/// сценарий мог бы тихо разойтись с диском, который видел настоящий
/// yt-dlp.
fn launch_script(name: &str, dir: &Path) -> Script {
    launch_script_with(name, dir, |_| None)
}

/// То же, но перед строкой сдвигаются управляемые часы на то, что вернёт
/// `pause` (только под `tokio::time::pause()`).
fn launch_script_with(
    name: &str,
    dir: &Path,
    mut pause: impl FnMut(&str) -> Option<Duration>,
) -> Script {
    let launch = fixtures::single_launch(name);
    let folder = dir.display().to_string();
    let mut script = Script {
        exit_code: launch.exit_code,
        stderr: launch.stderr.replace(DESTINATION_PLACEHOLDER, &folder),
        ..Script::default()
    };

    let mut on_disk = BTreeSet::new();
    for file in &launch.listing_before {
        script = script.creates(file);
        on_disk.insert(file.clone());
    }

    let mut destinations: Vec<String> = Vec::new();
    for raw in launch.stdout.lines() {
        let line = raw.replace(DESTINATION_PLACEHOLDER, &folder);
        if let Some(by) = pause(&line) {
            script = script.advance(by);
        }
        script = script.line(&line);

        match parse_line(&line) {
            StdoutLine::Destination { path } => destinations.push(
                Path::new(path)
                    .file_name()
                    .expect("Destination называет файл")
                    .to_string_lossy()
                    .into_owned(),
            ),
            StdoutLine::Progress(sample) if sample.status == SampleStatus::Finished => {
                let marker = format!(".f{}.", sample.format_id);
                if let Some(file) = destinations.iter().find(|file| file.contains(&marker)) {
                    script = script.creates(file);
                    on_disk.insert(file.clone());
                }
            }
            _ => {}
        }
    }

    assert_eq!(
        on_disk.into_iter().collect::<Vec<_>>(),
        launch.listing_after,
        "{name}: сценарий разошёлся с тем, что осталось на диске при съёмке"
    );
    script
}

/// Строки прогресса одного формата из снятой фикстуры одного запуска.
fn launch_progress(name: &str, format_id: &str) -> Vec<String> {
    fixtures::single_launch(name)
        .stdout
        .lines()
        .filter(|line| {
            matches!(parse_line(line), StdoutLine::Progress(sample) if sample.format_id == format_id)
        })
        .map(str::to_string)
        .collect()
}

/// Ни одна строка не пришла позже срока, выставленного перед ней: настоящий
/// запуск ([`crate::sidecar::run_streaming`]) снял бы процесс по сроку.
fn assert_no_line_outlives_its_deadline(call: &Call) {
    let mut deadline = Some(call.first_deadline);
    for (index, at) in call.line_times.iter().enumerate() {
        if let Some(deadline) = deadline {
            assert!(
                *at < deadline,
                "строка {index} {:?} пришла позже срока на {:?} — процесс был бы снят",
                call.lines[index],
                at.duration_since(deadline)
            );
        }
        deadline = call.deadlines[index];
    }
}

// ─────────────────────── Аргументы запуска (Ф-1) ───────────────────────

#[test]
fn the_source_side_throttle_matches_ours() {
    // Два числа с одним смыслом: наш троттлинг событий и `--progress-delta`
    // у самого yt-dlp. Разойдись они — вторая половина решения тихо
    // перестала бы соответствовать первой.
    let seconds: f64 = PROGRESS_DELTA_ARG
        .parse()
        .expect("--progress-delta обязан быть числом");
    assert_eq!(
        Duration::from_secs_f64(seconds),
        PROGRESS_THROTTLE,
        "порог у источника обязан совпадать с нашим"
    );
}

#[test]
fn the_socket_timeout_argument_matches_the_policy() {
    let seconds: u64 = SOCKET_TIMEOUT_ARG
        .parse()
        .expect("--socket-timeout обязан быть числом");
    assert_eq!(
        seconds, SOCKET_TIMEOUT_SECS,
        "значение обязано совпадать с тем, на котором стоит расчёт сторожа"
    );
    assert!(
        seconds * 2 <= NO_PROGRESS_TIMEOUT.as_secs(),
        "таймаут сокета обязан оставлять yt-dlp хотя бы одну свою попытку \
         переподключиться до того, как попытку заберёт сторож"
    );
}

#[tokio::test]
async fn every_launch_carries_the_arguments_the_parser_and_the_watchdog_stand_on() {
    let dir = tempfile::tempdir().unwrap();
    let sink = RecordingSink::new();
    run_two_streams(
        &dir,
        &sink,
        two_stream_scripts(dir.path()),
        &ScriptedFfmpeg::merging(),
    )
    .await;

    let launcher_calls = {
        let task = new_task(request(streams(Some("133"), Some("139"))));
        let launcher = ScriptedLauncher::new(dir.path(), two_stream_scripts(dir.path()));
        run_task(
            &task,
            &launcher,
            &ScriptedFfmpeg::merging(),
            &RecordingSink::new(),
            dir.path(),
        )
        .await;
        launcher.calls()
    };

    assert_eq!(
        launcher_calls.len(),
        1,
        "один запуск на задачу — одно извлечение адреса (TL-48)"
    );
    for call in &launcher_calls {
        assert_eq!(
            call.value_of("--progress-template"),
            Some(PROGRESS_TEMPLATE),
            "разбор читает свой шаблон — запуск обязан его нести"
        );
        assert!(
            call.argv.iter().any(|arg| arg == "--newline"),
            "без --newline полоса пишется возвратом каретки в одну строку"
        );
        assert_eq!(call.value_of("--progress-delta"), Some(PROGRESS_DELTA_ARG));
        assert_eq!(call.value_of("--socket-timeout"), Some(SOCKET_TIMEOUT_ARG));
        assert!(call.argv.iter().any(|arg| arg == "--no-playlist"));
    }
}

#[tokio::test]
async fn one_launch_asks_for_every_stream_separately() {
    // TL-48: один запуск, одно извлечение адреса. Объединённый запрос
    // (`133+139`) yt-dlp склеил бы сам — своим ffmpeg, найденным в PATH, —
    // и Ф-9 перестал бы выполняться; запятая просит те же два формата
    // по отдельности (замер в шапке `orchestrate`).
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(Some("133"), Some("139"))));
    let launcher = ScriptedLauncher::new(dir.path(), two_stream_scripts(dir.path()));

    run_task(
        &task,
        &launcher,
        &ScriptedFfmpeg::merging(),
        &RecordingSink::new(),
        dir.path(),
    )
    .await;

    let formats: Vec<String> = launcher
        .calls()
        .iter()
        .map(|call| call.value_of("-f").expect("-f обязателен").to_string())
        .collect();
    assert_eq!(
        formats,
        ["133,139"],
        "оба потока одним запуском, каждый — отдельным результатом выбора"
    );
    for format in &formats {
        assert!(
            !format.contains('+'),
            "«+» у -f означает объединение потоков, то есть чужую склейку"
        );
    }
}

#[tokio::test]
async fn the_url_is_the_last_argument_and_stands_after_the_separator() {
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(None, Some("140"))));
    let launcher = ScriptedLauncher::new(
        dir.path(),
        vec![Script::ok()
            .line(&destination_line(dir.path(), "Big Buck Bunny.f140.m4a"))
            .creates("Big Buck Bunny.f140.m4a")],
    );

    run_task(
        &task,
        &launcher,
        &ScriptedFfmpeg::merging(),
        &RecordingSink::new(),
        dir.path(),
    )
    .await;

    let argv = &launcher.calls()[0].argv;
    assert_eq!(argv.last().map(String::as_str), Some(URL));
    assert_eq!(
        argv[argv.len() - 2],
        "--",
        "ссылка обязана стоять сразу за разделителем аргументов"
    );
}

/// Названия, которые шаблон вывода yt-dlp понял бы не как текст.
///
/// Каждое снято прямым запуском вложенного бинарника (`--print filename`,
/// macOS, пин 2026.08.19) — это не выдуманные строки, а измеренные
/// раскрытия; таблица с результатами в doc `template_safe`.
pub(crate) const HOSTILE_TITLES: [&str; 9] = [
    "Скидка 100%(ext)s навсегда",
    "A$HOME B",
    "$HOME-leading",
    "braced ${HOME} here",
    "double $$HOME",
    "~",
    "~/leading",
    "%HOME% и back\\slash",
    "$$$%%%~~~",
];

#[test]
fn nothing_the_template_would_expand_ever_reaches_it() {
    // Проверяется **класс**, а не случай: у шаблона два механизма
    // раскрытия — поля (`%`) и переменные окружения с домашним каталогом
    // (`$`, ведущая `~`), — и оба закрыты одним правилом «в шаблон едет
    // только заведомо инертное». Перечисление опасного здесь уже
    // подводило дважды, поэтому утверждение сформулировано о всей
    // основе целиком, а не о списке символов, которые мы вспомнили.
    const TAIL: &str = ".f%(format_id)s.%(ext)s";

    for title in HOSTILE_TITLES {
        let stem = sanitized_stem(title, "aqz-KE-bpKQ");
        let template = output_template(&download_stem(&stem));

        let head = template
            .strip_suffix(TAIL)
            .unwrap_or_else(|| panic!("«{title}»: хвост шаблона обязан быть нашим: {template}"));

        assert!(!head.is_empty(), "«{title}»: основа обязана быть непустой");
        for ch in head.chars() {
            assert!(
                template_safe(ch),
                "«{title}»: в шаблон уехал символ {ch:?} — шаблон {template}"
            );
        }
        // Прямые следствия таблицы замеров, выписанные отдельно: то, что
        // уводило файл из папки назначения.
        assert!(!head.contains('$'), "«{title}»: доллар раскрылся бы");
        assert!(!head.contains('%'), "«{title}»: процент начал бы поле");
        assert!(
            !head.starts_with('~'),
            "«{title}»: ведущая тильда сделала бы путь домашним"
        );
    }
}

#[test]
fn a_hostile_title_still_makes_a_deterministic_partial_name() {
    // Обещание Р-2 держится на детерминированности, а не на совпадении с
    // финальным именем: вставил ту же ссылку — получил ту же основу.
    for title in HOSTILE_TITLES {
        let first = download_stem(&sanitized_stem(title, "aqz-KE-bpKQ"));
        let second = download_stem(&sanitized_stem(title, "aqz-KE-bpKQ"));
        assert_eq!(first, second, "«{title}»");
    }
}

#[tokio::test]
async fn a_title_with_a_percent_and_a_dollar_reaches_a_file_inside_the_destination() {
    // Сквозной тест того самого класса, на котором прошлый точечный не
    // сработал: название несёт **и** процент, **и** доллар, и проверяется
    // не строка шаблона, а файл на диске — и то, что рядом с папкой
    // назначения ничего не появилось.
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("назначение");
    std::fs::create_dir(&dir).unwrap();

    const TITLE: &str = "Скидка 100%(ext)s и $HOME внутри";
    let mut req = request(streams(None, Some("140")));
    req.title = TITLE.to_string();
    let task = new_task(req);
    let sink = RecordingSink::new();

    // Сценарий кладёт файл ровно туда, куда его положил бы yt-dlp по
    // нашему шаблону: имя частичного файла тест выводит тем же кодом.
    let partial = format!(
        "{}.f140.m4a",
        download_stem(&sanitized_stem(TITLE, "aqz-KE-bpKQ"))
    );
    let launcher = ScriptedLauncher::new(
        &dir,
        vec![Script::ok()
            .line(&destination_line(&dir, &partial))
            .creates(&partial)],
    );

    run_task(&task, &launcher, &ScriptedFfmpeg::merging(), &sink, &dir).await;

    // Финальное имя сохраняет название целиком — и процент, и доллар:
    // его строит финализация уже после того, как yt-dlp сделал своё дело.
    let expected = format!("{}.m4a", sanitized_stem(TITLE, "aqz-KE-bpKQ"));
    assert!(
        expected.contains('%') && expected.contains('$'),
        "проверять нечего, если название потеряло опасные символы: {expected}"
    );
    assert_eq!(
        sink.last(),
        DownloadProgress::Done {
            file_name: expected.clone(),
            folder_display: crate::types::FolderDisplay::SystemDownloads,
        }
    );
    assert_eq!(dir_listing(&dir), [expected]);
    assert_eq!(
        dir_listing(root.path()),
        ["назначение"],
        "рядом с папкой назначения не должно появиться ничего: раскрытая \
         переменная увела бы файл именно сюда"
    );
}

#[tokio::test]
async fn a_hostile_title_is_still_cleaned_up_after_a_cancel() {
    // Третье последствие дыры было самым тихим: подчистка ищет по
    // префиксу в папке назначения, а раскрытое имя лежит в другом месте —
    // файл остаётся, а панель честно говорит «данные удалены». Сторож
    // именно на это: имя частичного файла обязано быть тем, которое
    // подчистка потом ищет.
    let dir = tempfile::tempdir().unwrap();
    const TITLE: &str = "$HOME-leading 100%(ext)s";
    let mut req = request(streams(None, Some("140")));
    req.title = TITLE.to_string();
    let task = new_task(req);
    let sink = RecordingSink::new();

    let partial = format!(
        "{}.f140.m4a.part",
        download_stem(&sanitized_stem(TITLE, "aqz-KE-bpKQ"))
    );
    let launcher = ScriptedLauncher::new(dir.path(), vec![Script::ok().creates(&partial).hangs()]);

    let canceller = Arc::clone(&task);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        canceller.cancel().await;
    });

    run_task(
        &task,
        &launcher,
        &ScriptedFfmpeg::merging(),
        &sink,
        dir.path(),
    )
    .await;

    assert_eq!(
        sink.last(),
        DownloadProgress::Cancelled {
            partial_data: PartialData::Removed
        },
        "«удалено» обязано означать, что удалять было что"
    );
    assert_eq!(dir_listing(dir.path()), Vec::<String>::new());
}

#[test]
fn a_format_id_that_is_not_one_is_refused_whole() {
    for id in [
        "137+140",      // объединение потоков — чужая склейка
        "--exec",       // флаг под видом идентификатора
        "137;rm -rf /", // попытка из другого мира
        "",             // пустой
        "a".repeat(MAX_FORMAT_ID_BYTES + 1).as_str(),
        ".hidden", // имя частичного файла стало бы скрытым
    ] {
        assert!(!usable_format_id(id), "«{id}» не идентификатор формата");
    }

    for id in ["137", "140-drc", "hls-1080", "616_2", "sb.0"] {
        assert!(usable_format_id(id), "«{id}» — настоящий идентификатор");
    }
}

#[test]
fn an_unusable_format_id_never_reaches_a_process() {
    let Err(rejection) = build_task(
        "dl-негодный".to_string(),
        request(streams(Some("137+140"), Some("140"))),
    ) else {
        panic!("такой запрос обязан быть отклонён");
    };

    assert!(matches!(
        rejection,
        DownloadCommandRejection::NoStreamsSelected
    ));
}

#[test]
fn a_link_that_is_not_a_link_never_reaches_a_process() {
    let mut broken = request(streams(Some("137"), None));
    broken.url = "-o--".to_string();

    let Err(rejection) = build_task("dl-не-ссылка".to_string(), broken) else {
        panic!("не-ссылка обязана быть отклонена");
    };

    assert!(matches!(rejection, DownloadCommandRejection::InvalidUrl));
}

#[test]
fn an_empty_selection_is_refused() {
    let Err(rejection) = build_task("dl-пусто".to_string(), request(streams(None, None)))
    else {
        panic!("пункт без потоков скачивать нечем");
    };

    assert!(matches!(
        rejection,
        DownloadCommandRejection::NoStreamsSelected
    ));
}

// ─────────────────────── Полные циклы ───────────────────────

#[tokio::test]
async fn two_streams_go_through_merging_to_one_ready_file() {
    let dir = tempfile::tempdir().unwrap();
    let sink = RecordingSink::watching(dir.path(), "Big Buck Bunny.mp4");
    let ffmpeg = ScriptedFfmpeg::merging();

    run_two_streams(&dir, &sink, two_stream_scripts(dir.path()), &ffmpeg).await;

    assert_eq!(
        sink.phases().first(),
        Some(&DownloadPhase::Fetching),
        "первым виден шаг «Подготовка»"
    );
    assert!(
        sink.phases().contains(&DownloadPhase::Merging),
        "у двух потоков шаг «Склейка» обязан быть"
    );
    assert_eq!(ffmpeg.calls(), 1, "склейка ровно одна");
    assert_eq!(
        sink.last(),
        DownloadProgress::Done {
            file_name: "Big Buck Bunny.mp4".to_string(),
            folder_display: crate::types::FolderDisplay::SystemDownloads,
        }
    );
    assert_eq!(
        sink.violations(),
        Vec::<String>::new(),
        "файла под финальным именем не бывает до готовности (Ф-8)"
    );
    assert_eq!(
        dir_listing(dir.path()),
        ["Big Buck Bunny.mp4"],
        "штатный исход не оставляет мусора (Н-4): ни потоков, ни рабочего \
         файла склейки"
    );
}

#[tokio::test]
async fn the_percent_reaches_a_hundred_and_never_goes_backwards() {
    let dir = tempfile::tempdir().unwrap();
    let sink = RecordingSink::new();
    run_two_streams(
        &dir,
        &sink,
        two_stream_scripts(dir.path()),
        &ScriptedFfmpeg::merging(),
    )
    .await;

    let percents: Vec<u8> = sink
        .events()
        .iter()
        .filter_map(|progress| match progress {
            DownloadProgress::Downloading(DownloadingState::Running { percent, .. }) => *percent,
            _ => None,
        })
        .map(DownloadPercent::value)
        .collect();

    assert!(!percents.is_empty(), "процент обязан появиться");
    assert_eq!(percents.last(), Some(&100), "к концу — сто процентов");
    assert!(
        percents.windows(2).all(|pair| pair[0] <= pair[1]),
        "переход видео → звук не выглядит откатом назад: {percents:?}"
    );
}

#[tokio::test]
async fn a_progressive_format_never_enters_merging() {
    // С-3: у пункта заполнен только видеопоток, звук уже внутри него.
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(Some("18"), None)));
    let sink = RecordingSink::new();
    let ffmpeg = ScriptedFfmpeg::merging();
    let launcher = ScriptedLauncher::new(
        dir.path(),
        vec![Script::ok()
            .line(&destination_line(dir.path(), "Big Buck Bunny.f18.mp4"))
            .creates("Big Buck Bunny.f18.mp4")],
    );

    run_task(&task, &launcher, &ffmpeg, &sink, dir.path()).await;

    assert_eq!(task.plan, DownloadPlan::SingleStream);
    assert!(
        !sink.phases().contains(&DownloadPhase::Merging),
        "шага «Склейка» в жизни такой задачи не бывает вовсе"
    );
    assert_eq!(ffmpeg.calls(), 0, "ffmpeg не запускается");
    assert_eq!(
        sink.last(),
        DownloadProgress::Done {
            file_name: "Big Buck Bunny.mp4".to_string(),
            folder_display: crate::types::FolderDisplay::SystemDownloads,
        }
    );
    assert_eq!(dir_listing(dir.path()), ["Big Buck Bunny.mp4"]);
}

#[tokio::test]
async fn audio_only_keeps_the_extension_the_stream_actually_has() {
    // С-2: расширение — по фактическому контейнеру, а не «всегда .mp4».
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(None, Some("140"))));
    let sink = RecordingSink::new();
    let ffmpeg = ScriptedFfmpeg::merging();
    let launcher = ScriptedLauncher::new(
        dir.path(),
        vec![Script::ok()
            .line(&destination_line(dir.path(), "Big Buck Bunny.f140.m4a"))
            .lines(fixture_progress("audio-only.json", "140"))
            .creates("Big Buck Bunny.f140.m4a")],
    );

    run_task(&task, &launcher, &ffmpeg, &sink, dir.path()).await;

    assert_eq!(ffmpeg.calls(), 0);
    assert_eq!(
        sink.last(),
        DownloadProgress::Done {
            file_name: "Big Buck Bunny.m4a".to_string(),
            folder_display: crate::types::FolderDisplay::SystemDownloads,
        }
    );

    // У задачи с одним потоком подписи потока не бывает: различать нечего.
    let streams_shown: Vec<Option<DownloadStream>> = sink
        .events()
        .iter()
        .filter_map(|progress| match progress {
            DownloadProgress::Downloading(DownloadingState::Running { stream, .. }) => {
                Some(*stream)
            }
            _ => None,
        })
        .collect();
    assert!(
        streams_shown.iter().all(Option::is_none),
        "подпись «Скачиваем видео/звук» рисуется только при двух потоках"
    );
}

#[tokio::test]
async fn a_second_copy_of_the_same_video_gets_a_suffix_and_leaves_the_first_alone() {
    // С-12 и К-4: молчаливой перезаписи не бывает ни в каком сценарии.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Big Buck Bunny.mp4"), "первая копия").unwrap();

    let sink = RecordingSink::new();
    run_two_streams(
        &dir,
        &sink,
        two_stream_scripts(dir.path()),
        &ScriptedFfmpeg::merging(),
    )
    .await;

    assert_eq!(
        sink.last(),
        DownloadProgress::Done {
            file_name: "Big Buck Bunny (2).mp4".to_string(),
            folder_display: crate::types::FolderDisplay::SystemDownloads,
        }
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("Big Buck Bunny.mp4")).unwrap(),
        "первая копия",
        "существующий файл не изменён"
    );
}

// ─────────────────────── Отмена по фазам (Ф-4) ───────────────────────

#[tokio::test]
async fn cancelling_in_queued_says_nothing_was_created() {
    // Первая строка таблицы «Отмена по фазам»: убивать нечего, файлов
    // не появлялось — и текст панели не должен утверждать обратного.
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(Some("133"), Some("139"))));
    let sink = RecordingSink::new();
    let launcher = ScriptedLauncher::new(dir.path(), Vec::new());

    task.cancel().await;
    run_task(
        &task,
        &launcher,
        &ScriptedFfmpeg::merging(),
        &sink,
        dir.path(),
    )
    .await;

    assert!(launcher.calls().is_empty(), "процесса не было");
    assert_eq!(
        sink.last(),
        DownloadProgress::Cancelled {
            partial_data: PartialData::NothingCreated
        }
    );
}

#[tokio::test]
async fn cancelling_in_fetching_kills_the_process_and_leaves_no_files() {
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(Some("133"), Some("139"))));
    let sink = RecordingSink::new();
    // Процесс, который висит до убийства: строк прогресса ещё не было.
    let launcher = ScriptedLauncher::new(dir.path(), vec![Script::ok().hangs()]);

    let canceller = Arc::clone(&task);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        canceller.cancel().await;
    });

    run_task(
        &task,
        &launcher,
        &ScriptedFfmpeg::merging(),
        &sink,
        dir.path(),
    )
    .await;

    assert_eq!(
        sink.last(),
        DownloadProgress::Cancelled {
            partial_data: PartialData::NothingCreated
        }
    );
    assert_eq!(dir_listing(dir.path()), Vec::<String>::new());
}

#[tokio::test]
async fn cancelling_in_downloading_removes_every_partial_file() {
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(Some("133"), Some("139"))));
    let sink = RecordingSink::new();
    // Первый поток скачан целиком, второй качается и оставляет `.part`,
    // а рядом — служебный хвост фрагментного протокола.
    // Один запуск на оба потока (TL-48): убить надо тот же процесс, что
    // уже отдал видео.
    let launcher = ScriptedLauncher::new(
        dir.path(),
        vec![Script::ok()
            .line(&destination_line(dir.path(), "Big Buck Bunny.f133.mp4"))
            .lines(fixture_progress("video-and-audio.json", "133"))
            .creates("Big Buck Bunny.f133.mp4")
            .line(&destination_line(dir.path(), "Big Buck Bunny.f139.m4a"))
            .creates("Big Buck Bunny.f139.m4a.part")
            .creates("Big Buck Bunny.f139.m4a.ytdl")
            .hangs()],
    );

    let canceller = Arc::clone(&task);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        canceller.cancel().await;
    });

    run_task(
        &task,
        &launcher,
        &ScriptedFfmpeg::merging(),
        &sink,
        dir.path(),
    )
    .await;

    assert_eq!(
        sink.last(),
        DownloadProgress::Cancelled {
            partial_data: PartialData::Removed
        }
    );
    assert_eq!(
        dir_listing(dir.path()),
        Vec::<String>::new(),
        "подчистка при отмене полная и без исключений: и целый поток, и \
         `.part`, и служебный хвост"
    );
}

#[tokio::test]
async fn cancelling_during_the_pause_before_a_retry_answers_at_once() {
    // Строка таблицы «Ожидание повтора»: убивать нечего, отменяется
    // отложенный таймер, а накопленное всё равно удаляется — раз
    // пользователь отменил, докачивать в будущем нечего.
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(None, Some("140"))));
    let sink = RecordingSink::new();
    let launcher = ScriptedLauncher::new(
        dir.path(),
        vec![
            Script::failing(1, "ERROR: unable to download video data: Read timed out")
                .line(&destination_line(dir.path(), "Big Buck Bunny.f140.m4a"))
                .creates("Big Buck Bunny.f140.m4a.part"),
        ],
    );

    let canceller = Arc::clone(&task);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(120)).await;
        canceller.cancel().await;
    });

    let started = Instant::now();
    run_task(
        &task,
        &launcher,
        &ScriptedFfmpeg::merging(),
        &sink,
        dir.path(),
    )
    .await;
    let elapsed = started.elapsed();

    assert!(
        sink.events().iter().any(|progress| matches!(
            progress,
            DownloadProgress::Downloading(DownloadingState::WaitingRetry { .. })
        )),
        "пауза перед повтором обязана быть видна отдельным состоянием"
    );
    assert_eq!(
        sink.last(),
        DownloadProgress::Cancelled {
            partial_data: PartialData::Removed
        }
    );
    assert!(
        elapsed < Duration::from_secs(3),
        "«Отменить» в паузе обязано отвечать секундами, а не дожидаться \
         пятисекундной паузы (Н-3); ответ занял {elapsed:?}"
    );
    assert_eq!(dir_listing(dir.path()), Vec::<String>::new());
}

#[tokio::test]
async fn cancelling_in_merging_removes_both_streams_and_the_half_merged_file() {
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(Some("133"), Some("139"))));
    let sink = RecordingSink::new();
    let ffmpeg = ScriptedFfmpeg::hanging();
    let launcher = ScriptedLauncher::new(dir.path(), two_stream_scripts(dir.path()));

    let canceller = Arc::clone(&task);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(80)).await;
        canceller.cancel().await;
    });

    run_task(&task, &launcher, &ffmpeg, &sink, dir.path()).await;

    assert_eq!(ffmpeg.calls(), 1, "склейка успела начаться");
    assert_eq!(
        sink.last(),
        DownloadProgress::Cancelled {
            partial_data: PartialData::Removed
        }
    );
    assert_eq!(
        dir_listing(dir.path()),
        Vec::<String>::new(),
        "и оба потока, и недосклеенный результат"
    );
}

// ─────────────────────── Повторы и докачка ───────────────────────

/// stderr оборванной попытки — снятый живьём (С-6).
fn connection_lost_stderr() -> String {
    fixtures::outcome("connection-lost-mid-download.json").stderr
}

#[tokio::test]
async fn an_interrupted_attempt_is_retried_and_the_percent_does_not_fall_to_zero() {
    // К-6 буквально: обрыв на 995 883 байтах, следующая попытка
    // продолжает с 996 907 — обе серии сняты живьём.
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(Some("134"), None)));
    let sink = RecordingSink::new();
    let launcher = ScriptedLauncher::new(
        dir.path(),
        vec![
            Script::failing(1, &connection_lost_stderr())
                .line(&destination_line(dir.path(), "Big Buck Bunny.f134.mp4"))
                .lines(fixture_progress("resume-interrupted.json", "134"))
                .creates("Big Buck Bunny.f134.mp4.part"),
            Script::ok()
                .line(&destination_line(dir.path(), "Big Buck Bunny.f134.mp4"))
                .line("[download] Resuming download at byte 995883")
                .lines(fixture_progress("resume-continued.json", "134"))
                .creates("Big Buck Bunny.f134.mp4"),
        ],
    );

    run_task(
        &task,
        &launcher,
        &ScriptedFfmpeg::merging(),
        &sink,
        dir.path(),
    )
    .await;

    let percents: Vec<u8> = sink
        .events()
        .iter()
        .filter_map(|progress| match progress {
            DownloadProgress::Downloading(
                DownloadingState::Running { percent, .. }
                | DownloadingState::WaitingRetry { percent, .. },
            ) => *percent,
            _ => None,
        })
        .map(DownloadPercent::value)
        .collect();

    assert!(
        percents.windows(2).all(|pair| pair[0] <= pair[1]),
        "процент не откатывается ни в паузе, ни на новой попытке: {percents:?}"
    );
    assert!(
        percents.iter().any(|percent| *percent > 0),
        "к паузе уже что-то скачано"
    );
    assert_eq!(
        sink.last(),
        DownloadProgress::Done {
            file_name: "Big Buck Bunny.mp4".to_string(),
            folder_display: crate::types::FolderDisplay::SystemDownloads,
        }
    );
    assert_eq!(launcher.calls().len(), 2, "ровно две попытки");
}

#[tokio::test]
async fn any_advance_resets_the_attempt_counter() {
    // С-6: час загрузки на нестабильном Wi-Fi не исчерпывает лимит
    // суммированием редких обрывов. Обрывов здесь больше, чем попыток в
    // лимите, но каждый со скачанными байтами между ними.
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(None, Some("140"))));
    let sink = RecordingSink::new();

    let samples = fixture_progress("audio-only.json", "140");
    let mut scripts: Vec<Script> = Vec::new();
    for (index, sample) in samples.iter().take(MAX_ATTEMPTS as usize + 2).enumerate() {
        scripts.push(
            Script::failing(1, &connection_lost_stderr())
                .line(&destination_line(dir.path(), "Big Buck Bunny.f140.m4a"))
                .line(sample)
                .creates(&format!("Big Buck Bunny.f140.m4a.part.{index}")),
        );
    }
    scripts.push(
        Script::ok()
            .line(&destination_line(dir.path(), "Big Buck Bunny.f140.m4a"))
            .lines(samples.clone())
            .creates("Big Buck Bunny.f140.m4a"),
    );
    let expected_calls = scripts.len();
    let launcher = ScriptedLauncher::new(dir.path(), scripts);

    run_task(
        &task,
        &launcher,
        &ScriptedFfmpeg::merging(),
        &sink,
        dir.path(),
    )
    .await;

    assert_eq!(
        launcher.calls().len(),
        expected_calls,
        "продвижение между обрывами обязано обнулять счётчик — иначе \
         лимит наказывал бы за длину ролика, а не за качество связи"
    );
    let numbers: Vec<u32> = sink
        .events()
        .iter()
        .filter_map(|progress| match progress {
            DownloadProgress::Downloading(DownloadingState::WaitingRetry { attempt, .. }) => {
                Some(attempt.number)
            }
            _ => None,
        })
        .collect();
    assert!(
        numbers.iter().all(|number| *number == 2),
        "после продвижения следующая попытка снова вторая: {numbers:?}"
    );
    assert!(matches!(sink.last(), DownloadProgress::Done { .. }));
}

#[tokio::test]
async fn attempts_without_a_single_byte_run_out_and_become_connection_lost() {
    // С-7: попытки закончились, а сеть не восстановилась.
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(None, Some("140"))));
    let sink = RecordingSink::new();
    let scripts: Vec<Script> = (0..MAX_ATTEMPTS)
        .map(|_| {
            Script::failing(1, &connection_lost_stderr())
                .line(&destination_line(dir.path(), "Big Buck Bunny.f140.m4a"))
                .creates("Big Buck Bunny.f140.m4a.part")
        })
        .collect();
    let launcher = ScriptedLauncher::new(dir.path(), scripts);

    // Паузы политики растут от пяти секунд — ждать их по-настоящему тест
    // не станет: время в этом рантайме управляемое.
    tokio::time::pause();
    run_task(
        &task,
        &launcher,
        &ScriptedFfmpeg::merging(),
        &sink,
        dir.path(),
    )
    .await;
    tokio::time::resume();

    assert_eq!(launcher.calls().len(), MAX_ATTEMPTS as usize);
    let DownloadProgress::Failed { error } = sink.last() else {
        panic!(
            "исчерпание попыток обязано кончиться отказом, а не {:?}",
            sink.last()
        );
    };
    assert_eq!(error.kind, DownloadErrorKind::ConnectionLost);
    assert_eq!(
        error.partial_data,
        PartialData::Kept,
        "скачанное сохраняется: «Повторить» продолжит с места"
    );
    assert!(error.retryable);
    let details = error
        .details
        .expect("«Подробнее» обязано нести детали последней прерванной попытки");
    assert!(
        details
            .stderr_tail
            .is_some_and(|tail| tail.contains("ERROR")),
        "иначе под «Подробнее» не было бы ничего, кроме слова «попытки \
         закончились»"
    );
    assert!(
        dir.path().join("Big Buck Bunny.f140.m4a.part").exists(),
        "частичное осталось на диске"
    );
}

#[tokio::test]
async fn a_stream_already_on_disk_is_not_downloaded_again() {
    // Ф-11 и обязанность повтора после неудачной склейки: поток, который
    // качать не пришлось, не присылает ни одной строки прогресса —
    // включая `finished`. Без отметки о его готовности задача навсегда
    // упиралась бы в неполный процент.
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(Some("133"), Some("139"))));
    let sink = RecordingSink::new();
    let already = |name: &str| {
        format!(
            "[download] {} has already been downloaded",
            dir.path().join(name).display()
        )
    };
    let launcher = ScriptedLauncher::new(
        dir.path(),
        vec![Script::ok()
            .creates("Big Buck Bunny.f133.mp4")
            .line(&already("Big Buck Bunny.f133.mp4"))
            .creates("Big Buck Bunny.f139.m4a")
            .line(&already("Big Buck Bunny.f139.m4a"))],
    );

    run_task(
        &task,
        &launcher,
        &ScriptedFfmpeg::merging(),
        &sink,
        dir.path(),
    )
    .await;

    assert!(matches!(sink.last(), DownloadProgress::Done { .. }));
    assert_eq!(dir_listing(dir.path()), ["Big Buck Bunny.mp4"]);
}

// ─────────────── Атрибуция по формату и имени файла (TL-48) ───────────────

/// Подпись потока в состоянии задачи.
fn stream_of_snapshot(progress: &DownloadProgress) -> Option<DownloadStream> {
    match progress {
        DownloadProgress::Downloading(DownloadingState::Running { stream, .. }) => *stream,
        _ => None,
    }
}

fn percent_of_snapshot(progress: &DownloadProgress) -> Option<u8> {
    match progress {
        DownloadProgress::Downloading(DownloadingState::Running { percent, .. }) => {
            percent.map(DownloadPercent::value)
        }
        _ => None,
    }
}

#[tokio::test]
async fn every_progress_line_of_one_launch_goes_to_the_stream_of_its_format() {
    // Критерий 3 задачи: процесс один, потоков два — факт запуска больше
    // ничего не говорит о том, чей это вывод. Строки сняты запуском
    // `-f 133,139` (`single-launch/video-and-audio.json`), и каждая
    // проверяется по отдельности через
    // снимок задачи — троттлинг событий здесь ничего не прячет.
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(Some("133"), Some("139"))));
    let launcher = ScriptedLauncher::new(dir.path(), two_stream_scripts(dir.path()));
    launcher.observe(&task);
    let ffmpeg = ScriptedFfmpeg::merging();

    run_task(&task, &launcher, &ffmpeg, &RecordingSink::new(), dir.path()).await;

    let video_lines = launch_progress("video-and-audio.json", "133");
    let audio_lines = launch_progress("video-and-audio.json", "139");
    assert!(
        !video_lines.is_empty() && !audio_lines.is_empty(),
        "фикстура обязана нести строки обоих форматов"
    );

    let mut checked = (0, 0);
    let mut last_video_percent = None;
    for (line, snapshot) in launcher.snapshots() {
        let expected = if video_lines.contains(&line) {
            checked.0 += 1;
            DownloadStream::Video
        } else if audio_lines.contains(&line) {
            checked.1 += 1;
            DownloadStream::Audio
        } else {
            continue;
        };
        assert_eq!(
            stream_of_snapshot(&snapshot),
            Some(expected),
            "строка {line:?} отнесена не к своему потоку"
        );
        if expected == DownloadStream::Video {
            last_video_percent = percent_of_snapshot(&snapshot);
        }
    }
    assert_eq!(checked, (video_lines.len(), audio_lines.len()));

    // Байты тоже легли к своему потоку: к концу видео у звука ещё нет
    // точного размера, и знаменатель — оценка пункта из запроса
    // (`LAUNCH_ITEM_BYTES`, правило агрегации «оценка E2 для потоков без
    // точного размера»). Видео закончилось на своей доле: 894 838 из
    // 1 218 568 — 73 %, а не на сотне, как было бы, уйди строки звука
    // в видео.
    assert_eq!(last_video_percent, Some(73));
    assert_eq!(
        percent_of_snapshot(&launcher.snapshots().last().unwrap().1),
        Some(100)
    );
    assert_eq!(
        ffmpeg.last_input_names(),
        (
            "Big Buck Bunny.f133.mp4".to_string(),
            "Big Buck Bunny.f139.m4a".to_string()
        )
    );
}

#[tokio::test]
async fn stream_files_are_attributed_by_name_and_not_by_the_order_of_lines() {
    // yt-dlp обходит `-f 133,139` в порядке селектора, но ядро на порядок
    // не опирается: здесь звук назван первым. Сопоставление «первый
    // незабранный поток — первому пути» склеило бы звук как видео.
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(Some("133"), Some("139"))));
    let launcher = ScriptedLauncher::new(
        dir.path(),
        vec![Script::ok()
            .line(&destination_line(dir.path(), "Big Buck Bunny.f139.m4a"))
            .lines(fixture_progress("video-and-audio.json", "139"))
            .creates("Big Buck Bunny.f139.m4a")
            .line(&destination_line(dir.path(), "Big Buck Bunny.f133.mp4"))
            .lines(fixture_progress("video-and-audio.json", "133"))
            .creates("Big Buck Bunny.f133.mp4")],
    );
    let ffmpeg = ScriptedFfmpeg::merging();
    let sink = RecordingSink::new();

    run_task(&task, &launcher, &ffmpeg, &sink, dir.path()).await;

    assert_eq!(
        ffmpeg.last_input_names(),
        (
            "Big Buck Bunny.f133.mp4".to_string(),
            "Big Buck Bunny.f139.m4a".to_string()
        ),
        "видео и звук склейки — по именам файлов"
    );
    assert!(matches!(sink.last(), DownloadProgress::Done { .. }));
}

#[tokio::test]
async fn a_launch_that_finds_the_video_on_disk_closes_it_and_downloads_only_the_audio() {
    // Снятый повтор (`single-launch/video-already-downloaded.json`): видео
    // уже на диске, yt-dlp печатает для него `has already been downloaded`
    // и `finished` без байт (М-1), затем качает звук. Видео закрывается по
    // имени файла, без единой строки `downloading`, и в проценте весит свой
    // полный размер.
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(Some("133"), Some("139"))));
    let launcher = ScriptedLauncher::new(
        dir.path(),
        vec![launch_script("video-already-downloaded.json", dir.path())],
    );
    launcher.observe(&task);
    let ffmpeg = ScriptedFfmpeg::merging();
    let sink = RecordingSink::new();

    run_task(&task, &launcher, &ffmpeg, &sink, dir.path()).await;

    let audio_lines = launch_progress("video-already-downloaded.json", "139");
    let audio_percents: Vec<Option<u8>> = launcher
        .snapshots()
        .iter()
        .filter(|(line, _)| audio_lines.contains(line))
        .map(|(_, snapshot)| percent_of_snapshot(snapshot))
        .collect();
    assert_eq!(audio_percents.len(), audio_lines.len());
    assert_eq!(
        audio_percents.first(),
        Some(&Some(73)),
        "на первом байте звука видео уже весит свои 894 838 из 1 218 568: \
         {audio_percents:?}"
    );
    assert_eq!(audio_percents.last(), Some(&Some(100)));
    assert_eq!(launcher.formats_asked(), ["133,139"]);
    assert_eq!(
        ffmpeg.last_input_names(),
        (
            "Big Buck Bunny.f133.mp4".to_string(),
            "Big Buck Bunny.f139.m4a".to_string()
        )
    );
    assert!(matches!(sink.last(), DownloadProgress::Done { .. }));
}

#[test]
fn only_a_name_built_by_our_template_belongs_to_a_stream() {
    let dir = Path::new("/папка назначения");
    let owns = |format_id: &str, name: &str| {
        is_file_of_stream("Big Buck Bunny", format_id, &dir.join(name))
    };

    assert!(owns("133", "Big Buck Bunny.f133.mp4"));
    assert!(owns("140-drc", "Big Buck Bunny.f140-drc.m4a"));

    // Чужой формат, формат с общим началом, другая основа.
    assert!(!owns("133", "Big Buck Bunny.f139.m4a"));
    assert!(!owns("133", "Big Buck Bunny.f1333.mp4"));
    assert!(!owns("140", "Big Buck Bunny.f140-drc.m4a"));
    assert!(!owns("133", "Big Buck Bunny 2.f133.mp4"));
    // Рабочие хвосты — не файл потока.
    assert!(!owns("133", "Big Buck Bunny.f133.mp4.part"));
    assert!(!owns("133", "Big Buck Bunny.f133.temp.mp4"));
    assert!(!owns("133", "Big Buck Bunny.f133."));
    // Идентификатор с точкой не делает соседа владельцем.
    assert!(owns("sb.0", "Big Buck Bunny.fsb.0.mhtml"));
    assert!(!owns("sb", "Big Buck Bunny.fsb.0.mhtml"));
    // Совпадает только имя, а не каталог.
    assert!(!is_file_of_stream(
        "Big Buck Bunny",
        "133",
        Path::new("/Big Buck Bunny.f133.mp4/другое.mp4")
    ));
}

#[test]
fn the_selector_joins_the_streams_with_a_comma_and_never_a_plus() {
    assert_eq!(format_selector(["133", "139"]), "133,139");
    assert_eq!(format_selector(["140"]), "140");
}

#[tokio::test]
async fn a_retry_after_an_interrupted_launch_asks_only_for_what_is_still_missing() {
    // Снятые запуски. У `-f 133,139` адрес видео отвечает 404, а звук
    // скачивается целиком, код 1 (`single-launch/video-404.json`): отказ
    // одного формата не останавливает другой. Повтор обязан заказать только
    // видео (`single-launch/video-only.json`) — звук уже забран.
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(Some("133"), Some("139"))));
    let launcher = ScriptedLauncher::new(
        dir.path(),
        vec![
            launch_script("video-404.json", dir.path()),
            launch_script("video-only.json", dir.path()),
        ],
    );
    let ffmpeg = ScriptedFfmpeg::merging();
    let sink = RecordingSink::new();

    tokio::time::pause();
    run_task(&task, &launcher, &ffmpeg, &sink, dir.path()).await;
    tokio::time::resume();

    assert_eq!(launcher.formats_asked(), ["133,139", "133"]);
    assert_eq!(
        ffmpeg.last_input_names(),
        (
            "Big Buck Bunny.f133.mp4".to_string(),
            "Big Buck Bunny.f139.m4a".to_string()
        ),
        "видео из первой попытки не потеряно"
    );
    assert!(matches!(sink.last(), DownloadProgress::Done { .. }));
}

#[tokio::test]
async fn a_format_that_fell_out_of_the_selection_is_a_stale_format() {
    // Р-1 ревью TL-48, снятый вывод (`single-launch/one-format-missing.json`):
    // в метаданных нет 139, yt-dlp выбирает только 133, качает видео и
    // выходит с кодом 0 при пустом stderr. До TL-48 отдельный запуск на 139
    // давал `staleFormat` — тот же класс обязан получиться и здесь, а не
    // «ошибка склейки» с повтором, который не поможет никогда.
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(Some("133"), Some("139"))));
    let sink = RecordingSink::new();
    let ffmpeg = ScriptedFfmpeg::merging();
    let launcher = ScriptedLauncher::new(
        dir.path(),
        vec![launch_script("one-format-missing.json", dir.path())],
    );

    run_task(&task, &launcher, &ffmpeg, &sink, dir.path()).await;

    let DownloadProgress::Failed { error } = sink.last() else {
        panic!("формат пропал — отказ: {:?}", sink.last());
    };
    assert_eq!(error.kind, DownloadErrorKind::StaleFormat);
    assert!(!error.retryable, "повтор тем же форматом бесполезен");
    assert_eq!(
        error.partial_data,
        PartialData::Removed,
        "скачанное видео без звука пункта не нужно — как у staleFormat до TL-48"
    );
    assert_eq!(dir_listing(dir.path()), Vec::<String>::new());
    assert_eq!(launcher.calls().len(), 1);
    assert_eq!(ffmpeg.calls(), 0, "склейки не было");
}

#[tokio::test]
async fn a_format_that_fell_out_of_the_selection_of_an_interrupted_launch_is_stale_too() {
    // Перечень печатается до первого байта: если он уже без 139, обрыв
    // после него повтором этот формат не вернёт. Сценарий собран из снятых
    // строк `one-format-missing.json` (перечень, Destination, два байта) и
    // снятого stderr обрыва; вместе живьём они не снимались.
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(Some("133"), Some("139"))));
    let sink = RecordingSink::new();
    let folder = dir.path().display().to_string();
    let lines: Vec<String> = fixtures::single_launch("one-format-missing.json")
        .stdout
        .lines()
        .take(4)
        .map(|line| line.replace(DESTINATION_PLACEHOLDER, &folder))
        .collect();
    assert!(
        matches!(parse_line(&lines[0]), StdoutLine::SelectedFormats { .. }),
        "сценарий обязан нести перечень"
    );
    let launcher = ScriptedLauncher::new(
        dir.path(),
        vec![Script::failing(1, &connection_lost_stderr())
            .lines(lines)
            .creates("Big Buck Bunny.f133.mp4.part")],
    );

    tokio::time::pause();
    run_task(
        &task,
        &launcher,
        &ScriptedFfmpeg::merging(),
        &sink,
        dir.path(),
    )
    .await;
    tokio::time::resume();

    let DownloadProgress::Failed { error } = sink.last() else {
        panic!("формат пропал — отказ, а не повторы: {:?}", sink.last());
    };
    assert_eq!(error.kind, DownloadErrorKind::StaleFormat);
    assert_eq!(launcher.calls().len(), 1, "повторов не было");
    assert_eq!(dir_listing(dir.path()), Vec::<String>::new());
}

#[tokio::test]
async fn a_selected_stream_whose_file_was_never_named_is_asked_for_again() {
    // Прежняя ветка, названная честно: перечень называет оба формата, код 0,
    // а файла звука yt-dlp не назвал. Что стало с потоком, не узнать;
    // склеивать нечего, и повтор спрашивает только звук. Сценарий собран из
    // снятого `video-and-audio.json`, обрезанного на `finished` видео, —
    // живьём такого вывода не видели.
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(Some("133"), Some("139"))));
    let sink = RecordingSink::new();
    let folder = dir.path().display().to_string();
    let lines: Vec<String> = fixtures::single_launch("video-and-audio.json")
        .stdout
        .lines()
        .map(|line| line.replace(DESTINATION_PLACEHOLDER, &folder))
        .take_while(|line| !line.contains(".f139."))
        .collect();
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("@tl-progress|finished|") && line.ends_with("|133")),
        "сценарий доходит до конца видео"
    );
    let first = ScriptedLauncher::new(
        dir.path(),
        vec![Script::ok().lines(lines).creates("Big Buck Bunny.f133.mp4")],
    );
    run_task(&task, &first, &ScriptedFfmpeg::merging(), &sink, dir.path()).await;

    let DownloadProgress::Failed { error } = sink.last() else {
        panic!("без файла звука склеивать нечего: {:?}", sink.last());
    };
    assert_eq!(error.kind, DownloadErrorKind::MergeFailed);
    assert!(error.retryable);

    task.set_progress(DownloadProgress::Queued);
    let already = format!(
        "[download] {} has already been downloaded",
        dir.path().join("Big Buck Bunny.f139.m4a").display()
    );
    let second = ScriptedLauncher::new(
        dir.path(),
        vec![Script::ok()
            .creates("Big Buck Bunny.f139.m4a")
            .line(&already)],
    );
    run_task(
        &task,
        &second,
        &ScriptedFfmpeg::merging(),
        &sink,
        dir.path(),
    )
    .await;

    assert_eq!(second.formats_asked(), ["139"]);
    assert!(matches!(sink.last(), DownloadProgress::Done { .. }));
}

#[tokio::test]
async fn a_failure_that_removes_the_streams_also_forgets_that_they_were_done() {
    // `videoUnavailable` повторяем и при этом удаляет частичное. Видео
    // было забрано до отказа — после подчистки его на диске нет, и повтор
    // обязан заказать его снова, а не склеивать отсутствующий файл.
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(Some("133"), Some("139"))));
    let sink = RecordingSink::new();
    let first = ScriptedLauncher::new(
        dir.path(),
        vec![
            Script::failing(1, &fixtures::outcome("video-unavailable.json").stderr)
                .line(&destination_line(dir.path(), "Big Buck Bunny.f133.mp4"))
                .lines(fixture_progress("video-and-audio.json", "133"))
                .creates("Big Buck Bunny.f133.mp4"),
        ],
    );
    run_task(&task, &first, &ScriptedFfmpeg::merging(), &sink, dir.path()).await;

    let DownloadProgress::Failed { error } = sink.last() else {
        panic!("недоступный ролик — отказ: {:?}", sink.last());
    };
    assert_eq!(error.kind, DownloadErrorKind::VideoUnavailable);
    assert_eq!(error.partial_data, PartialData::Removed);
    assert!(
        error.retryable,
        "класс повторяем — иначе сценарию неоткуда взяться"
    );

    task.set_progress(DownloadProgress::Queued);
    let second = ScriptedLauncher::new(dir.path(), two_stream_scripts(dir.path()));
    run_task(
        &task,
        &second,
        &ScriptedFfmpeg::merging(),
        &sink,
        dir.path(),
    )
    .await;

    assert_eq!(second.formats_asked(), ["133,139"]);
    assert!(matches!(sink.last(), DownloadProgress::Done { .. }));
}

#[test]
fn the_single_launch_fixtures_were_shot_with_the_arguments_of_the_app() {
    // Утверждение README набора «сняты с argv приложения» — проверяемое:
    // argv съёмки обязан совпасть с тем, что строит `download_args`, кроме
    // хвоста — вместо `-- <ссылка>` у съёмки `--load-info-json <файл>`.
    // Сменят аргументы запуска — этот тест покраснеет, и набор переснимут.
    const TAIL: &str = ".f%(format_id)s.%(ext)s";

    for name in SINGLE_LAUNCH_FIXTURES {
        let capture = fixtures::single_launch(name).capture;
        let value_of = |flag: &str| -> String {
            let at = capture
                .argv
                .iter()
                .position(|arg| arg == flag)
                .unwrap_or_else(|| panic!("{name}: в argv съёмки нет {flag}"));
            capture.argv[at + 1].clone()
        };
        let selector = value_of("-f");
        let template = value_of("-o");
        let stem = template
            .strip_suffix(TAIL)
            .unwrap_or_else(|| panic!("{name}: шаблон имени не наш: {template}"));
        assert_eq!(
            download_stem(&sanitized_stem(stem, "aqz-KE-bpKQ")),
            stem,
            "{name}: основа имени съёмки — не та, что приложение построило бы из названия"
        );

        let destination = format!("home:{DESTINATION_PLACEHOLDER}");
        let mut expected: Vec<String> = download_args(&selector, &destination, &template, URL)
            .into_iter()
            .map(|arg| {
                if arg == PROGRESS_TEMPLATE {
                    "<progressTemplate>".to_string()
                } else {
                    arg.to_string()
                }
            })
            .collect();
        let expected_tail = expected.split_off(expected.len() - 2);
        assert_eq!(expected_tail, ["--", URL]);

        let mut shot = capture.argv.clone();
        let shot_tail = shot.split_off(shot.len() - 2);
        assert_eq!(shot_tail, ["--load-info-json", "<infoJson>"], "{name}");
        assert_eq!(shot, expected, "{name}: съёмка шла не с argv приложения");
    }
}

#[tokio::test]
async fn after_a_removing_failure_the_new_download_rearms_the_watchdog_and_the_percent() {
    // Р-2 ревью TL-48. `videoUnavailable` повторяем и удаляет частичное:
    // видео, забранное целиком, стёрто. Повтор качает его с нуля, и каждый
    // его новый байт обязан продлевать срок сторожа, а процент — начинаться
    // с нуля, а не стоять на удалённом.
    //
    // Время управляемое: строка раз в 3 с. Видеопоток повтора тянется
    // дольше порога сторожа, и срок, не продлённый его байтами, истёк бы
    // посреди здоровой загрузки.
    tokio::time::pause();
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(Some("133"), Some("139"))));
    let sink = RecordingSink::new();

    let first = ScriptedLauncher::new(
        dir.path(),
        vec![
            Script::failing(1, &fixtures::outcome("video-unavailable.json").stderr)
                .line(&destination_line(dir.path(), "Big Buck Bunny.f133.mp4"))
                .lines(launch_progress("video-and-audio.json", "133"))
                .creates("Big Buck Bunny.f133.mp4"),
        ],
    );
    first.observe(&task);
    run_task(&task, &first, &ScriptedFfmpeg::merging(), &sink, dir.path()).await;
    let DownloadProgress::Failed { error } = sink.last() else {
        panic!("недоступный ролик — отказ: {:?}", sink.last());
    };
    assert_eq!(
        (error.kind, error.partial_data),
        (DownloadErrorKind::VideoUnavailable, PartialData::Removed)
    );
    assert_eq!(
        first
            .snapshots()
            .last()
            .and_then(|(_, snapshot)| percent_of_snapshot(snapshot)),
        Some(73),
        "видео было забрано целиком до отказа"
    );

    task.set_progress(DownloadProgress::Queued);
    let second = ScriptedLauncher::new(
        dir.path(),
        vec![launch_script_with(
            "video-and-audio.json",
            dir.path(),
            |_| Some(Duration::from_secs(3)),
        )],
    );
    second.observe(&task);
    run_task(
        &task,
        &second,
        &ScriptedFfmpeg::merging(),
        &sink,
        dir.path(),
    )
    .await;
    tokio::time::resume();

    let call = &second.calls()[0];
    let mut high_water = 0;
    let mut advancing = 0;
    for (index, line) in call.lines.iter().enumerate() {
        let StdoutLine::Progress(sample) = parse_line(line) else {
            continue;
        };
        let Some(bytes) = sample.downloaded_bytes else {
            continue;
        };
        if sample.format_id != "133" || bytes <= high_water {
            continue;
        }
        high_water = bytes;
        advancing += 1;
        assert_eq!(
            call.deadlines[index],
            Some(call.line_times[index] + NO_PROGRESS_TIMEOUT),
            "строка {index} {line:?}: новые байты повтора обязаны продлить срок"
        );
    }
    assert_eq!(advancing, 6, "у видео повтора шесть строк с новыми байтами");
    assert_no_line_outlives_its_deadline(call);

    let first_video_line = &launch_progress("video-and-audio.json", "133")[0];
    let (_, after_first_byte) = second
        .snapshots()
        .into_iter()
        .find(|(line, _)| line == first_video_line)
        .expect("строка была");
    assert_eq!(
        percent_of_snapshot(&after_first_byte),
        Some(0),
        "1 024 байта из 1 218 568 — ноль, а не 73 удалённых процента"
    );
    assert!(matches!(sink.last(), DownloadProgress::Done { .. }));
}

#[tokio::test]
async fn the_gap_between_two_streams_of_one_launch_gets_a_full_window() {
    // М-2 ревью TL-48. До TL-48 звук качал новый процесс, и промежуток до
    // его первого байта держал полный срок от старта процесса. Теперь
    // процесс один, и тот же промежуток обязан получить полный срок от
    // начала потока — первой строки, называющей его файл.
    //
    // Строки сняты (`single-launch/video-and-audio.json`), время растянуто:
    // строка раз в секунду, а стык — 15 с от `finished` видео до
    // `Destination` звука и ещё 15 с до его первого байта. Локальный сервер
    // съёмки отвечал за миллисекунды; растяжение — единственное допущение.
    tokio::time::pause();
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(Some("133"), Some("139"))));
    let sink = RecordingSink::new();
    let seam = Duration::from_secs(15);
    let mut previous = String::new();
    let script = launch_script_with("video-and-audio.json", dir.path(), |line| {
        let pause = match parse_line(&previous) {
            StdoutLine::Progress(sample)
                if sample.status == SampleStatus::Finished && sample.format_id == "133" =>
            {
                seam
            }
            StdoutLine::Destination { path } if path.ends_with(".f139.m4a") => seam,
            _ => Duration::from_secs(1),
        };
        previous = line.to_string();
        Some(pause)
    });
    let launcher = ScriptedLauncher::new(dir.path(), vec![script]);

    run_task(
        &task,
        &launcher,
        &ScriptedFfmpeg::merging(),
        &sink,
        dir.path(),
    )
    .await;
    tokio::time::resume();

    let call = &launcher.calls()[0];
    let audio_start = call
        .lines
        .iter()
        .position(|line| {
            matches!(parse_line(line), StdoutLine::Destination { path } if path.ends_with(".f139.m4a"))
        })
        .expect("Destination звука был");
    assert_eq!(
        (
            call.line_times[audio_start].duration_since(call.line_times[audio_start - 1]),
            call.line_times[audio_start + 1].duration_since(call.line_times[audio_start]),
        ),
        (seam, seam),
        "стык обязан быть растянут — иначе тест ничего не проверяет"
    );
    assert_eq!(
        call.deadlines[audio_start],
        Some(call.line_times[audio_start] + NO_PROGRESS_TIMEOUT),
        "у звука полный срок от начала потока"
    );
    assert_no_line_outlives_its_deadline(call);
    assert!(matches!(sink.last(), DownloadProgress::Done { .. }));
}

#[tokio::test]
async fn a_title_carrying_the_already_downloaded_phrase_still_closes_its_stream() {
    // М-3 ревью TL-48, снятый вывод (`single-launch/phrase-in-title.json`):
    // звук уже на диске, а в названии стоит та самая фраза. Путь обязан
    // кончаться перед последней, иначе файл звука не принадлежит никому.
    const TITLE: &str = "Big Buck Bunny has already been downloaded";
    let dir = tempfile::tempdir().unwrap();
    let mut req = request(streams(Some("133"), Some("139")));
    req.title = TITLE.to_string();
    let task = new_task(req);
    assert_eq!(
        download_stem(&sanitized_stem(TITLE, "aqz-KE-bpKQ")),
        TITLE,
        "фраза целиком доходит до имени частичного файла"
    );
    let ffmpeg = ScriptedFfmpeg::merging();
    let sink = RecordingSink::new();
    let launcher = ScriptedLauncher::new(
        dir.path(),
        vec![launch_script("phrase-in-title.json", dir.path())],
    );

    run_task(&task, &launcher, &ffmpeg, &sink, dir.path()).await;

    assert_eq!(
        sink.last(),
        DownloadProgress::Done {
            file_name: format!("{TITLE}.mp4"),
            folder_display: crate::types::FolderDisplay::SystemDownloads,
        }
    );
    assert_eq!(
        ffmpeg.last_input_names(),
        (format!("{TITLE}.f133.mp4"), format!("{TITLE}.f139.m4a"))
    );
}

#[tokio::test]
async fn a_repeated_line_naming_the_same_stream_does_not_extend_the_watchdog() {
    // Граница потока продлевает срок один раз на поток за запуск (doc
    // `note_stream_start`): замерший процесс, повторяющий строку о том же
    // файле, жить на ней не может. Сценарий собран из снятых строк видео и
    // той же `Destination`, повторённой дважды, — живьём yt-dlp так не делал.
    tokio::time::pause();
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(Some("133"), Some("139"))));
    let destination = destination_line(dir.path(), "Big Buck Bunny.f133.mp4");
    let video = launch_progress("video-and-audio.json", "133");
    let launcher = ScriptedLauncher::new(
        dir.path(),
        vec![
            Script::failing(1, &fixtures::outcome("video-unavailable.json").stderr)
                .line(&destination)
                .lines(video.iter().take(3).cloned())
                .advance(Duration::from_secs(15))
                .line(&destination)
                .advance(Duration::from_secs(15))
                .line(&destination),
        ],
    );

    run_task(
        &task,
        &launcher,
        &ScriptedFfmpeg::merging(),
        &RecordingSink::new(),
        dir.path(),
    )
    .await;
    tokio::time::resume();

    let call = &launcher.calls()[0];
    let last_byte = 3;
    assert!(matches!(
        parse_line(&call.lines[last_byte]),
        StdoutLine::Progress(_)
    ));
    let deadline = call.deadlines[last_byte].expect("срок выставлен");
    assert_eq!(
        (call.deadlines[4], call.deadlines[5]),
        (Some(deadline), Some(deadline)),
        "повтор строки о том же файле срок не двигает"
    );
    assert!(
        call.line_times[5] >= deadline,
        "без новых байт третья `Destination` приходит уже за сроком — процесс был бы снят"
    );
}

// ─────────────────── Сторож продвижения и подготовки ───────────────────

#[tokio::test]
async fn the_stall_watchdog_is_armed_from_the_last_advance_not_from_the_last_line() {
    // Обязанность, найденная замером в TL-41: замерший поток либо не
    // печатает ничего, либо печатает «Read timed out», ничего не
    // принимая. Срок обязан двигаться от **принятых байт**, а не от
    // факта вывода строки.
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(None, Some("140"))));
    let samples = fixture_progress("audio-only.json", "140");
    let repeated = samples[1].clone();
    let launcher = ScriptedLauncher::new(
        dir.path(),
        vec![Script::ok()
            .line(&destination_line(dir.path(), "Big Buck Bunny.f140.m4a"))
            .line(&samples[0])
            .line(&samples[1])
            // Та же строка ещё дважды: вывод есть, принятых байт больше
            // не становится.
            .line(&repeated)
            .line(&repeated)
            .creates("Big Buck Bunny.f140.m4a")],
    );

    run_task(
        &task,
        &launcher,
        &ScriptedFfmpeg::merging(),
        &RecordingSink::new(),
        dir.path(),
    )
    .await;

    let call = &launcher.calls()[0];
    let deadlines: Vec<Instant> = call
        .deadlines
        .iter()
        .map(|deadline| deadline.expect("срок обязан быть выставлен на каждой строке"))
        .collect();

    assert!(
        deadlines[2] > deadlines[1],
        "строка с новыми байтами обязана перевооружить таймер"
    );
    assert_eq!(
        deadlines[3], deadlines[2],
        "строка без новых байт таймер не двигает — иначе замерший поток \
         продлевал бы себе жизнь собственными жалобами"
    );
    assert_eq!(deadlines[4], deadlines[3]);
}

#[tokio::test]
async fn a_stalled_stream_is_retried_like_a_lost_connection() {
    // С-8: пользователь не видит отдельного «поток завис» — он видит
    // ожидание повтора.
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(None, Some("140"))));
    let sink = RecordingSink::new();
    let launcher = ScriptedLauncher::new(
        dir.path(),
        vec![
            Script::stalled()
                .line(&destination_line(dir.path(), "Big Buck Bunny.f140.m4a"))
                .lines(
                    fixture_progress("audio-only.json", "140")
                        .into_iter()
                        .take(3),
                )
                .creates("Big Buck Bunny.f140.m4a.part"),
            Script::ok()
                .line(&destination_line(dir.path(), "Big Buck Bunny.f140.m4a"))
                .lines(fixture_progress("audio-only.json", "140"))
                .creates("Big Buck Bunny.f140.m4a"),
        ],
    );

    tokio::time::pause();
    run_task(
        &task,
        &launcher,
        &ScriptedFfmpeg::merging(),
        &sink,
        dir.path(),
    )
    .await;
    tokio::time::resume();

    assert!(
        sink.events().iter().any(|progress| matches!(
            progress,
            DownloadProgress::Downloading(DownloadingState::WaitingRetry { .. })
        )),
        "зависшая попытка уходит в тот же цикл повторов, что и обрыв"
    );
    assert!(matches!(sink.last(), DownloadProgress::Done { .. }));
}

#[tokio::test]
async fn silence_during_preparation_is_a_yt_dlp_failure_not_a_lost_connection() {
    // Мёртвая сеть на подготовке печатает строку каждые 10 с (замер в doc
    // FETCH_SILENCE_TIMEOUT) и до этого срока не доходит. Значит срок,
    // сработавший **до первой строки прогресса**, означает зависший
    // процесс, а не сеть, — и класс у него свой.
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(None, Some("140"))));
    let sink = RecordingSink::new();
    let launcher = ScriptedLauncher::new(dir.path(), vec![Script::stalled()]);

    run_task(
        &task,
        &launcher,
        &ScriptedFfmpeg::merging(),
        &sink,
        dir.path(),
    )
    .await;

    let DownloadProgress::Failed { error } = sink.last() else {
        panic!("зависшая подготовка обязана кончиться отказом");
    };
    assert_eq!(error.kind, DownloadErrorKind::YtDlpFailure);
    assert_eq!(error.reason, Some(YtDlpFailureReason::Generic));
    assert_eq!(
        launcher.calls().len(),
        1,
        "в цикл повторов зависшая подготовка не уходит"
    );
}

#[tokio::test]
async fn the_first_deadline_of_every_launch_is_the_preparation_one() {
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(None, Some("140"))));
    let before = monotonic_now();
    let launcher = ScriptedLauncher::new(
        dir.path(),
        vec![Script::ok()
            .line(&destination_line(dir.path(), "Big Buck Bunny.f140.m4a"))
            .creates("Big Buck Bunny.f140.m4a")],
    );

    run_task(
        &task,
        &launcher,
        &ScriptedFfmpeg::merging(),
        &RecordingSink::new(),
        dir.path(),
    )
    .await;

    let first = launcher.calls()[0].first_deadline;
    assert!(first >= before + FETCH_SILENCE_TIMEOUT);
    assert!(first <= monotonic_now() + FETCH_SILENCE_TIMEOUT);
}

// ─────────────────── Классы отказа и судьба частичного ───────────────────

#[tokio::test]
async fn a_stale_format_removes_what_it_downloaded() {
    // С-10: докачка того же формата невозможна по построению, и хранить
    // огрызок незачем.
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(None, Some("140"))));
    let sink = RecordingSink::new();
    let launcher = ScriptedLauncher::new(
        dir.path(),
        vec![
            Script::failing(1, &fixtures::outcome("stale-format.json").stderr)
                .creates("Big Buck Bunny.f140.m4a.part"),
        ],
    );

    run_task(
        &task,
        &launcher,
        &ScriptedFfmpeg::merging(),
        &sink,
        dir.path(),
    )
    .await;

    let DownloadProgress::Failed { error } = sink.last() else {
        panic!("устаревший формат — отказ");
    };
    assert_eq!(error.kind, DownloadErrorKind::StaleFormat);
    assert_eq!(error.partial_data, PartialData::Removed);
    assert!(
        !error.retryable,
        "повтор тем же форматом заведомо бесполезен"
    );
    assert_eq!(dir_listing(dir.path()), Vec::<String>::new());
    assert_eq!(launcher.calls().len(), 1, "класс без повторов");
}

#[tokio::test]
async fn a_class_that_surfaced_before_the_first_byte_says_nothing_was_created() {
    // Общая оговорка таблицы: отказ до первого принятого байта даёт
    // `nothingCreated` — удалять было нечего, и говорить «данные удалены»
    // было бы неправдой.
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(None, Some("140"))));
    let sink = RecordingSink::new();
    let launcher = ScriptedLauncher::new(
        dir.path(),
        vec![Script::failing(
            1,
            &fixtures::outcome("video-unavailable.json").stderr,
        )],
    );

    run_task(
        &task,
        &launcher,
        &ScriptedFfmpeg::merging(),
        &sink,
        dir.path(),
    )
    .await;

    let DownloadProgress::Failed { error } = sink.last() else {
        panic!("недоступный ролик — отказ");
    };
    assert_eq!(error.kind, DownloadErrorKind::VideoUnavailable);
    assert_eq!(error.partial_data, PartialData::NothingCreated);
}

#[tokio::test]
async fn a_failed_merge_keeps_both_streams_and_no_half_merged_file() {
    // С-11: оба потока целы — повтор пересобирает файл без повторного
    // скачивания; недосклеенного под финальным именем не бывает никогда.
    let dir = tempfile::tempdir().unwrap();
    let sink = RecordingSink::new();
    let ffmpeg = ScriptedFfmpeg::failing();
    run_two_streams(&dir, &sink, two_stream_scripts(dir.path()), &ffmpeg).await;

    let DownloadProgress::Failed { error } = sink.last() else {
        panic!("отказ склейки — отказ задачи");
    };
    assert_eq!(error.kind, DownloadErrorKind::MergeFailed);
    assert_eq!(error.partial_data, PartialData::Kept);
    assert!(error.retryable);
    assert_eq!(
        dir_listing(dir.path()),
        ["Big Buck Bunny.f133.mp4", "Big Buck Bunny.f139.m4a"],
        "оба потока на месте, рабочего файла склейки нет"
    );
}

#[tokio::test]
async fn a_retry_after_a_failed_merge_only_merges_again() {
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(Some("133"), Some("139"))));
    let sink = RecordingSink::new();

    let failing = ScriptedFfmpeg::failing();
    let launcher = ScriptedLauncher::new(dir.path(), two_stream_scripts(dir.path()));
    run_task(&task, &launcher, &failing, &sink, dir.path()).await;
    assert_eq!(launcher.calls().len(), 1, "оба потока одним запуском");

    // Повтор — продолжение той же задачи: тот же id, ни одного нового
    // запуска yt-dlp, только склейка. Решение «повторять ли» принимает
    // очередь (`crate::queue::scheduler`), здесь проверяется то, что
    // делает сам воркер, когда та вернула задачу в `Queued`.
    task.set_progress(DownloadProgress::Queued);
    let working = ScriptedFfmpeg::merging();
    let empty = ScriptedLauncher::new(dir.path(), Vec::new());
    run_task(&task, &empty, &working, &sink, dir.path()).await;

    assert!(
        empty.calls().is_empty(),
        "повтор после неудачной склейки не качает заново"
    );
    assert_eq!(working.calls(), 1);
    assert_eq!(
        sink.last(),
        DownloadProgress::Done {
            file_name: "Big Buck Bunny.mp4".to_string(),
            folder_display: crate::types::FolderDisplay::SystemDownloads,
        }
    );
    assert_eq!(dir_listing(dir.path()), ["Big Buck Bunny.mp4"]);
}

#[tokio::test]
async fn a_missing_destination_folder_fails_the_task_and_not_the_command() {
    // Момент обнаружения — до запуска процесса, но отказ всё равно
    // приезжает событием фазы, а не реджектом промиса: иначе один класс
    // ошибки рисовался бы то панелью, то отказом команды.
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("нет такой папки");
    let task = new_task(request(streams(None, Some("140"))));
    let sink = RecordingSink::new();
    let launcher = ScriptedLauncher::new(&missing, Vec::new());

    run_task(
        &task,
        &launcher,
        &ScriptedFfmpeg::merging(),
        &sink,
        &missing,
    )
    .await;

    assert!(launcher.calls().is_empty(), "процесса не было вовсе");
    let DownloadProgress::Failed { error } = sink.last() else {
        panic!("недоступная папка — отказ задачи");
    };
    assert_eq!(error.kind, DownloadErrorKind::DestinationUnavailable);
    assert_eq!(error.partial_data, PartialData::NothingCreated);
}

// ─────────────────────── Троттлинг событий ───────────────────────

#[tokio::test]
async fn a_burst_of_progress_lines_does_not_become_a_burst_of_events() {
    // Ф-2: частота эмита ограничена, чтобы не заваливать webview.
    // Фикстура фрагментного потока — та самая, что печатает 15,7 строк в
    // секунду (замер TL-41).
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(Some("602"), None)));
    let sink = RecordingSink::new();
    let samples = fixture_progress("hls-fragmented.json", "602");
    assert!(
        samples.len() > 20,
        "фикстура обязана быть длинной, иначе троттлинг нечем проверять"
    );
    let launcher = ScriptedLauncher::new(
        dir.path(),
        vec![Script::ok()
            .line(&destination_line(dir.path(), "Big Buck Bunny.f602.mp4"))
            .lines(samples.clone())
            .creates("Big Buck Bunny.f602.mp4")],
    );

    run_task(
        &task,
        &launcher,
        &ScriptedFfmpeg::merging(),
        &sink,
        dir.path(),
    )
    .await;

    let running = sink
        .events()
        .iter()
        .filter(|progress| {
            matches!(
                progress,
                DownloadProgress::Downloading(DownloadingState::Running { .. })
            )
        })
        .count();
    assert!(
        running < samples.len(),
        "{running} событий на {} строк — троттлинг не сработал",
        samples.len()
    );
    assert!(
        sink.last().is_terminal(),
        "терминальный переход не глотается троттлингом ни при какой частоте"
    );
}

#[tokio::test]
async fn the_shape_of_the_state_always_gets_through() {
    // Смена вида состояния (`running` ⇄ `waitingRetry`) обязана
    // проезжать мимо троттлинга: иначе панель показывала бы «качается»
    // всю паузу перед повтором.
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(None, Some("140"))));
    let sink = RecordingSink::new();
    let samples = fixture_progress("audio-only.json", "140");
    let launcher = ScriptedLauncher::new(
        dir.path(),
        vec![
            Script::failing(1, &connection_lost_stderr())
                .line(&destination_line(dir.path(), "Big Buck Bunny.f140.m4a"))
                .lines(samples.iter().take(2).cloned())
                .creates("Big Buck Bunny.f140.m4a.part"),
            Script::ok()
                .line(&destination_line(dir.path(), "Big Buck Bunny.f140.m4a"))
                .lines(samples)
                .creates("Big Buck Bunny.f140.m4a"),
        ],
    );

    tokio::time::pause();
    run_task(
        &task,
        &launcher,
        &ScriptedFfmpeg::merging(),
        &sink,
        dir.path(),
    )
    .await;
    tokio::time::resume();

    let shapes: Vec<Option<&'static str>> = sink.events().iter().map(downloading_shape).collect();
    assert!(
        shapes.contains(&Some("waitingRetry")),
        "пауза перед повтором обязана быть видна: {shapes:?}"
    );
    assert!(
        shapes.contains(&Some("running")),
        "и возврат к скачиванию — тоже"
    );
}

// ─────────────────────── Имена частичных файлов ───────────────────────

#[test]
fn the_partial_name_of_the_longest_title_still_fits_the_file_system() {
    let stem = sanitized_stem(&"я".repeat(400), "aqz-KE-bpKQ");
    let download = download_stem(&stem);
    let longest = format!("{download}.f{}.mhtml.part", "a".repeat(MAX_FORMAT_ID_BYTES));

    assert!(
        longest.len() <= crate::download::filename::MAX_FILE_NAME_BYTES,
        "имя частичного файла длиной {} байт не переживёт ни одну из трёх ФС",
        longest.len()
    );
}

#[test]
fn the_partial_name_is_derived_from_the_title_and_nothing_else() {
    // На этом стоит обещание Р-2: после выхода из приложения пользователь
    // вставляет ту же ссылку, ядро строит то же имя, yt-dlp продолжает с
    // места. Любая недетерминированная часть имени это отменяет.
    let first = download_stem(&sanitized_stem(TITLE, "aqz-KE-bpKQ"));
    let second = download_stem(&sanitized_stem(TITLE, "aqz-KE-bpKQ"));

    assert_eq!(first, second);
    assert_eq!(first, "Big Buck Bunny");
}

#[test]
fn a_stream_prefix_cannot_swallow_a_neighbouring_file() {
    // Без точки после идентификатора формата префикс «Название.f» совпал
    // бы с посторонним «Название.flv» — то есть подчистка удаляла бы
    // чужой файл.
    let prefix = stream_prefix("Название", "137");

    assert!(!"Название.flv".starts_with(&prefix));
    assert!("Название.f137.mp4".starts_with(&prefix));
    assert!("Название.f137.mp4.part".starts_with(&prefix));
}

#[test]
fn the_merge_timeout_is_the_one_the_merge_module_declares() {
    // Склейку ведёт TL-42 со своим таймаутом; оркестрация его не
    // переопределяет и не дублирует — сторож на случай, если однажды
    // захочется.
    assert_eq!(MERGE_TIMEOUT_SECS, 1800);
}

#[test]
fn a_video_id_is_taken_from_the_link_without_parsing_youtube() {
    assert_eq!(video_id_of(URL), "aqz-KE-bpKQ");
    assert_eq!(
        video_id_of("https://www.youtube.com/watch?v=aqz-KE-bpKQ&list=PL1"),
        "aqz-KE-bpKQ"
    );
    assert_eq!(video_id_of("https://youtu.be/aqz-KE-bpKQ"), "aqz-KE-bpKQ");
    // Ничего не распознали — это не беда: запасное имя нужно только там,
    // где от названия ничего не осталось, и оно всё равно проходит через
    // белый список санитизации.
    assert_eq!(video_id_of("https://example.com/"), "example.com");
}

#[test]
fn the_fallback_stem_id_agrees_with_the_canonical_id_wherever_that_one_exists() {
    // На ссылку в ядре смотрят два места, и это осознанно: здесь — ради
    // запасного имени файла (Ф-6 E3), в `queue::video_id` — ради
    // тождества ролика при сравнении дублей (Ф-8/Р-5 E4, TL-72). Задачи
    // разные: тамошний разбор строгий (белый список форм, «не понял» —
    // отказ), здешний нарочно нестрогий (на чужом адресе он отдаёт хоть
    // что-нибудь, лишь бы имя файла было не пустым).
    //
    // Сторож ровно об одном: там, где строгий разбор говорит «это ролик
    // N», нестрогий обязан давать ту же строку. Разъедутся — значит одно
    // из двух мест втихую разошлось с формой ссылки, и увидеть это
    // должен тест, а не пользователь по чужому имени файла.
    for url in [
        "https://www.youtube.com/watch?v=aqz-KE-bpKQ",
        "http://youtube.com/watch?v=aqz-KE-bpKQ",
        "https://m.youtube.com/watch?v=aqz-KE-bpKQ",
        "HTTPS://WWW.YOUTUBE.COM/watch?v=aqz-KE-bpKQ",
        "https://www.youtube.com/watch?v=aqz-KE-bpKQ&list=PL1&index=2",
        "https://www.youtube.com/watch?app=desktop&v=aqz-KE-bpKQ",
        "https://www.youtube.com/watch?v=aqz-KE-bpKQ#t=10",
        "https://youtu.be/aqz-KE-bpKQ",
        "https://youtu.be/aqz-KE-bpKQ/",
        "https://youtu.be/aqz-KE-bpKQ?si=Kx1yQ7wSomething",
        "https://www.youtube.com/shorts/aqz-KE-bpKQ",
        "  https://m.youtube.com/shorts/aqz-KE-bpKQ?feature=share  ",
    ] {
        let canonical = canonical_video_id(url)
            .unwrap_or_else(|| panic!("«{url}» — разбираемая форма (TL-72)"));

        assert_eq!(
            video_id_of(url),
            canonical.as_str(),
            "запасное имя и канонический id разошлись на форме «{url}»"
        );
    }
}
