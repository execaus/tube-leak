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
use crate::download::filename::sanitized_stem;
use crate::download::fixtures::{self, DESTINATION_PLACEHOLDER, SINGLE_LAUNCH_FIXTURES};
use crate::download::merge::MERGE_TIMEOUT_SECS;
use crate::download::progress::parse_line;
use crate::download::retry::{MAX_ATTEMPTS, NO_PROGRESS_TIMEOUT, SOCKET_TIMEOUT_SECS};
use crate::queue::video_id::canonical_video_id;
use crate::sidecar::RunOutput;
use crate::types::{DownloadPhase, QualityKind, QualitySize, SelectedQuality, YtDlpFailureReason};

// ─────────────────────────── Оснастка ───────────────────────────

/// Прогон задачи с папкой назначения E3: настроек нет (умолчания — системная
/// папка и `{title}`), системная «Загрузки» — `destination`, истории нет.
///
/// Тесты E3/E4 проверяют механизм одной задачи, а не настройки, и держат
/// прежнюю форму вызова: имя затеняет `super::run_task`. Тесты TL-89 зовут
/// настоящий с собственным [`TaskEnv`].
async fn run_task(
    task: &Arc<DownloadTask>,
    launcher: &dyn DownloadLauncher,
    ffmpeg: &dyn FfmpegLauncher,
    sink: &dyn ProgressSink,
    destination: &Path,
) {
    let env = TaskEnv {
        settings: None,
        history: None,
        system_downloads: Some(destination.to_path_buf()),
        today: fixed_today,
    };
    super::run_task(task, launcher, ffmpeg, sink, &env).await;
}

/// Дата тестов, не зависящая от часов: 2031-02-03.
fn fixed_today() -> TemplateDate {
    TemplateDate::new(2031, 2, 3).expect("дата существует")
}

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
    /// Сделать то, что делает пользователь, пока процесс идёт (TL-89:
    /// сменить настройку посреди задачи).
    Hook(Hook),
    /// Сделать с диском то, что сделал бы yt-dlp **по нашим аргументам**
    /// (TL-89): для каждого формата из `-f` назвать файл по шаблону `-o` в
    /// папке `-P` строкой `Destination` и создать его ровно там. Имя и папку
    /// тест не выписывает сам — их берёт из argv оркестрации.
    ///
    /// `partial` — какие потоки оборвать ([`Partial`]): создать `<имя>.part`
    /// (ревью TL-89, B1; выборочно — TL-104).
    /// Без него — как yt-dlp с `--continue` по умолчанию: готовый файл под
    /// этим именем даёт `has already been downloaded`, лежащий `<имя>.part`
    /// докачивается (переименовывается в файл потока с прежним содержимым),
    /// иначе файл пишется заново.
    ///
    /// Раскрытия `$`, `%` и `~` здесь нет, и имя, которое не является одним
    /// именем файла (разделители, `..`), не пишется вовсе — отказ уходит в
    /// [`Call::emulate_failures`], и задача кончается не `Done`. Так сценарий
    /// никогда не пишет за пределы папки теста, а проверку `-o` делает сам
    /// тест по argv.
    Emulate { partial: Partial },
}

/// Какие потоки [`Step::Emulate`] обрывает.
#[derive(Debug, Clone, Copy)]
enum Partial {
    /// Ни одного: все потоки дописаны.
    Nothing,
    /// Все.
    Every,
    /// Только этот формат, остальные дописаны (TL-104: задача оставила один
    /// готовый поток и один `.part`).
    Only(&'static str),
}

impl Partial {
    fn covers(self, format_id: &str) -> bool {
        match self {
            Self::Nothing => false,
            Self::Every => true,
            Self::Only(only) => only == format_id,
        }
    }
}

/// Содержимое `.part`, оставленного оборванным потоком: докачанный файл
/// сохраняет его, и по нему видно, что вчерашнее не скачивалось заново.
const PARTIAL_BYTES: &[u8] = b"partial bytes";

/// Действие посреди сценария.
#[derive(Clone)]
struct Hook(Arc<dyn Fn() + Send + Sync>);

impl std::fmt::Debug for Hook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Hook")
    }
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

    fn hook(mut self, action: impl Fn() + Send + Sync + 'static) -> Self {
        self.steps.push(Step::Hook(Hook(Arc::new(action))));
        self
    }

    fn emulate(mut self) -> Self {
        self.steps.push(Step::Emulate {
            partial: Partial::Nothing,
        });
        self
    }

    fn emulate_partial(mut self) -> Self {
        self.steps.push(Step::Emulate {
            partial: Partial::Every,
        });
        self
    }

    fn emulate_partial_only(mut self, format_id: &'static str) -> Self {
        self.steps.push(Step::Emulate {
            partial: Partial::Only(format_id),
        });
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
    /// Что [`Step::Emulate`] не смог или отказался положить на диск.
    emulate_failures: Vec<String>,
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
                emulate_failures: Vec::new(),
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
                    Step::Hook(hook) => (hook.0)(),
                    Step::Emulate { partial } => {
                        let dir = call
                            .value_of("-P")
                            .and_then(|value| value.strip_prefix("home:"))
                            .expect("-P home:<папка> обязателен")
                            .to_string();
                        let template = call.value_of("-o").expect("-o обязателен").to_string();
                        let selector = call.value_of("-f").expect("-f обязателен").to_string();
                        for format_id in selector.split(',') {
                            let ext = if matches!(format_id, "139" | "140") {
                                "m4a"
                            } else {
                                "mp4"
                            };
                            let name = template
                                .replace("%(format_id)s", format_id)
                                .replace("%(ext)s", ext);
                            if Path::new(&name).file_name() != Some(std::ffi::OsStr::new(&name)) {
                                call.emulate_failures
                                    .push(format!("{name:?} — не одно имя файла"));
                                continue;
                            }
                            let path = Path::new(&dir).join(&name);
                            let part = Path::new(&dir).join(format!("{name}.part"));
                            let partial = partial.covers(format_id);
                            let already = !partial && path.exists();
                            let line = if already {
                                format!("[download] {} has already been downloaded", path.display())
                            } else {
                                format!("[download] Destination: {}", path.display())
                            };
                            call.lines.push(line.clone());
                            call.line_times.push(monotonic_now());
                            call.deadlines.push(on_line(&line));
                            let written = if partial {
                                std::fs::write(&part, PARTIAL_BYTES)
                            } else if already {
                                Ok(())
                            } else if part.exists() {
                                std::fs::rename(&part, &path)
                            } else {
                                std::fs::write(&path, b"stream bytes")
                            };
                            if let Err(err) = written {
                                call.emulate_failures
                                    .push(format!("{}: {err}", path.display()));
                            }
                        }
                    }
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
    /// Содержимое тех же входов в момент склейки (TL-104): по нему видно,
    /// чей поток ушёл в готовый файл, — после склейки входы подчищены.
    input_bytes: StdMutex<Vec<(Vec<u8>, Vec<u8>)>>,
    /// Что сделать с диском, пока ffmpeg работает, — после чтения входов и до
    /// записи результата (TL-106: файл потока пропал во время склейки).
    during: Option<Hook>,
}

impl ScriptedFfmpeg {
    fn with(fails: bool, hangs: bool) -> Self {
        Self {
            fails,
            hangs,
            calls: AtomicUsize::new(0),
            inputs: StdMutex::new(Vec::new()),
            input_bytes: StdMutex::new(Vec::new()),
            during: None,
        }
    }

    fn during(mut self, action: impl Fn() + Send + Sync + 'static) -> Self {
        self.during = Some(Hook(Arc::new(action)));
        self
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

    /// Содержимое файлов, ушедших в последнюю склейку: (видео, звук).
    fn last_input_bytes(&self) -> (Vec<u8>, Vec<u8>) {
        self.input_bytes
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("склейка была")
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
                let read = |arg: &str| {
                    std::fs::read(arg.strip_prefix("file:").unwrap_or(arg)).unwrap_or_default()
                };
                self.input_bytes
                    .lock()
                    .unwrap()
                    .push((read(video), read(audio)));
            }

            if let Some(hook) = &self.during {
                (hook.0)();
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

/// Рабочая основа задачи из названия и ссылки — тем же кодом, что
/// [`build_task`].
fn partial_base(title: &str, url: &str) -> String {
    download_stem(
        &sanitized_stem(title, &video_id_of(url)),
        &PartialId::of_url(url),
    )
}

/// Рабочая основа запроса [`request`]: название и id ролика [`URL`] (TL-104).
const BASE: &str = "aqz-KE-bpKQ.Big Buck Bunny";

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

/// Основа `-o`, с которой снят запуск: значение `-o` без нашего хвоста.
fn shot_base(capture: &fixtures::Capture) -> String {
    const TAIL: &str = ".f%(format_id)s.%(ext)s";
    let at = capture
        .argv
        .iter()
        .position(|arg| arg == "-o")
        .expect("в argv съёмки есть -o");
    let template = &capture.argv[at + 1];
    template
        .strip_suffix(TAIL)
        .unwrap_or_else(|| panic!("шаблон имени съёмки не наш: {template}"))
        .to_string()
}

/// Имена файлов снятого запуска — под рабочей основой приложения.
///
/// Набор `single-launch` снят до TL-104, когда рабочей основой было одно
/// название; теперь перед ним id ролика (`<id>.<название>`, ревью TL-104,
/// S1). Основу из `-o` yt-dlp пишет только в имена файлов (`Destination`,
/// `has already been downloaded`, листинги), поэтому оснастка подставляет
/// её так же, как папку назначения: `<основа съёмки>.f` →
/// `<id>.<основа съёмки>.f`. Что id ролика съёмки перед основой съёмки —
/// ровно то, что построило бы приложение, проверяет
/// `the_single_launch_fixtures_were_shot_with_the_arguments_of_the_app`; что
/// yt-dlp с таким `-o` пишет эти имена, — офлайн-замер в README набора.
fn shot_names_to_app(capture: &fixtures::Capture, text: &str) -> String {
    let shot = shot_base(capture);
    text.replace(
        &format!("{shot}.f"),
        &format!("{}.f", partial_base(&shot, URL)),
    )
}

/// Строки stdout снятого запуска с папкой теста и рабочей основой
/// приложения ([`shot_names_to_app`]).
fn launch_stdout(name: &str, dir: &Path) -> Vec<String> {
    let launch = fixtures::single_launch(name);
    let folder = dir.display().to_string();
    launch
        .stdout
        .lines()
        .map(|line| {
            shot_names_to_app(
                &launch.capture,
                &line.replace(DESTINATION_PLACEHOLDER, &folder),
            )
        })
        .collect()
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
    let rename = |text: &str| shot_names_to_app(&launch.capture, text);
    let mut script = Script {
        exit_code: launch.exit_code,
        stderr: rename(&launch.stderr.replace(DESTINATION_PLACEHOLDER, &folder)),
        ..Script::default()
    };

    let mut on_disk = BTreeSet::new();
    for file in &launch.listing_before {
        let file = rename(file);
        script = script.creates(&file);
        on_disk.insert(file);
    }

    let mut destinations: Vec<String> = Vec::new();
    for line in launch_stdout(name, dir) {
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
        launch
            .listing_after
            .iter()
            .map(|file| rename(file))
            .collect::<Vec<_>>(),
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
            .line(&destination_line(
                dir.path(),
                "aqz-KE-bpKQ.Big Buck Bunny.f140.m4a",
            ))
            .creates("aqz-KE-bpKQ.Big Buck Bunny.f140.m4a")],
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
        let template = output_template(&partial_base(title, URL));

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
        let first = partial_base(title, URL);
        let second = partial_base(title, URL);
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
    let partial = format!("{}.f140.m4a", partial_base(TITLE, URL));
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
    // Третье последствие дыры было самым тихим: подчистка ищет рабочие
    // имена в папке назначения, а раскрытое имя лежит в другом месте —
    // файл остаётся, а панель честно говорит «данные удалены». Сторож
    // именно на это: имя частичного файла обязано быть тем, которое
    // подчистка потом ищет.
    let dir = tempfile::tempdir().unwrap();
    const TITLE: &str = "$HOME-leading 100%(ext)s";
    let mut req = request(streams(None, Some("140")));
    req.title = TITLE.to_string();
    let task = new_task(req);
    let sink = RecordingSink::new();

    let partial = format!("{}.f140.m4a.part", partial_base(TITLE, URL));
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
            .line(&destination_line(
                dir.path(),
                "aqz-KE-bpKQ.Big Buck Bunny.f18.mp4",
            ))
            .creates("aqz-KE-bpKQ.Big Buck Bunny.f18.mp4")],
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
            .line(&destination_line(
                dir.path(),
                "aqz-KE-bpKQ.Big Buck Bunny.f140.m4a",
            ))
            .lines(fixture_progress("audio-only.json", "140"))
            .creates("aqz-KE-bpKQ.Big Buck Bunny.f140.m4a")],
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
            .line(&destination_line(
                dir.path(),
                "aqz-KE-bpKQ.Big Buck Bunny.f133.mp4",
            ))
            .lines(fixture_progress("video-and-audio.json", "133"))
            .creates("aqz-KE-bpKQ.Big Buck Bunny.f133.mp4")
            .line(&destination_line(
                dir.path(),
                "aqz-KE-bpKQ.Big Buck Bunny.f139.m4a",
            ))
            .creates("aqz-KE-bpKQ.Big Buck Bunny.f139.m4a.part")
            .creates("aqz-KE-bpKQ.Big Buck Bunny.f139.m4a.ytdl")
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
                .line(&destination_line(
                    dir.path(),
                    "aqz-KE-bpKQ.Big Buck Bunny.f140.m4a",
                ))
                .creates("aqz-KE-bpKQ.Big Buck Bunny.f140.m4a.part"),
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
                .line(&destination_line(
                    dir.path(),
                    "aqz-KE-bpKQ.Big Buck Bunny.f134.mp4",
                ))
                .lines(fixture_progress("resume-interrupted.json", "134"))
                .creates("aqz-KE-bpKQ.Big Buck Bunny.f134.mp4.part"),
            Script::ok()
                .line(&destination_line(
                    dir.path(),
                    "aqz-KE-bpKQ.Big Buck Bunny.f134.mp4",
                ))
                .line("[download] Resuming download at byte 995883")
                .lines(fixture_progress("resume-continued.json", "134"))
                .creates("aqz-KE-bpKQ.Big Buck Bunny.f134.mp4"),
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
                .line(&destination_line(
                    dir.path(),
                    "aqz-KE-bpKQ.Big Buck Bunny.f140.m4a",
                ))
                .line(sample)
                .creates(&format!("aqz-KE-bpKQ.Big Buck Bunny.f140.m4a.part.{index}")),
        );
    }
    scripts.push(
        Script::ok()
            .line(&destination_line(
                dir.path(),
                "aqz-KE-bpKQ.Big Buck Bunny.f140.m4a",
            ))
            .lines(samples.clone())
            .creates("aqz-KE-bpKQ.Big Buck Bunny.f140.m4a"),
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
                .line(&destination_line(
                    dir.path(),
                    "aqz-KE-bpKQ.Big Buck Bunny.f140.m4a",
                ))
                .creates("aqz-KE-bpKQ.Big Buck Bunny.f140.m4a.part")
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
        dir.path()
            .join("aqz-KE-bpKQ.Big Buck Bunny.f140.m4a.part")
            .exists(),
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
            .creates("aqz-KE-bpKQ.Big Buck Bunny.f133.mp4")
            .line(&already("aqz-KE-bpKQ.Big Buck Bunny.f133.mp4"))
            .creates("aqz-KE-bpKQ.Big Buck Bunny.f139.m4a")
            .line(&already("aqz-KE-bpKQ.Big Buck Bunny.f139.m4a"))],
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
            "aqz-KE-bpKQ.Big Buck Bunny.f133.mp4".to_string(),
            "aqz-KE-bpKQ.Big Buck Bunny.f139.m4a".to_string()
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
            .line(&destination_line(
                dir.path(),
                "aqz-KE-bpKQ.Big Buck Bunny.f139.m4a",
            ))
            .lines(fixture_progress("video-and-audio.json", "139"))
            .creates("aqz-KE-bpKQ.Big Buck Bunny.f139.m4a")
            .line(&destination_line(
                dir.path(),
                "aqz-KE-bpKQ.Big Buck Bunny.f133.mp4",
            ))
            .lines(fixture_progress("video-and-audio.json", "133"))
            .creates("aqz-KE-bpKQ.Big Buck Bunny.f133.mp4")],
    );
    let ffmpeg = ScriptedFfmpeg::merging();
    let sink = RecordingSink::new();

    run_task(&task, &launcher, &ffmpeg, &sink, dir.path()).await;

    assert_eq!(
        ffmpeg.last_input_names(),
        (
            "aqz-KE-bpKQ.Big Buck Bunny.f133.mp4".to_string(),
            "aqz-KE-bpKQ.Big Buck Bunny.f139.m4a".to_string()
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
            "aqz-KE-bpKQ.Big Buck Bunny.f133.mp4".to_string(),
            "aqz-KE-bpKQ.Big Buck Bunny.f139.m4a".to_string()
        )
    );
    assert!(matches!(sink.last(), DownloadProgress::Done { .. }));
}

#[test]
fn only_a_name_built_by_our_template_belongs_to_a_stream() {
    let dir = Path::new("/папка назначения");
    let owns = |format_id: &str, name: &str| is_file_of_stream(BASE, format_id, &dir.join(name));

    assert!(owns("133", "aqz-KE-bpKQ.Big Buck Bunny.f133.mp4"));
    assert!(owns("140-drc", "aqz-KE-bpKQ.Big Buck Bunny.f140-drc.m4a"));

    // Чужой формат, формат с общим началом, другая основа.
    assert!(!owns("133", "aqz-KE-bpKQ.Big Buck Bunny.f139.m4a"));
    assert!(!owns("133", "aqz-KE-bpKQ.Big Buck Bunny.f1333.mp4"));
    assert!(!owns("140", "aqz-KE-bpKQ.Big Buck Bunny.f140-drc.m4a"));
    assert!(!owns("133", "Big Buck Bunny 2.f133.mp4"));
    // Другой ролик с тем же названием и основа без id (TL-104).
    assert!(!owns("133", "YE7VzlLtp-4.Big Buck Bunny.f133.mp4"));
    assert!(!owns("133", "Big Buck Bunny.f133.mp4"));
    // Рабочие хвосты — не файл потока.
    assert!(!owns("133", "aqz-KE-bpKQ.Big Buck Bunny.f133.mp4.part"));
    assert!(!owns("133", "aqz-KE-bpKQ.Big Buck Bunny.f133.temp.mp4"));
    assert!(!owns("133", "aqz-KE-bpKQ.Big Buck Bunny.f133."));
    // Идентификатор с точкой не делает соседа владельцем.
    assert!(owns("sb.0", "aqz-KE-bpKQ.Big Buck Bunny.fsb.0.mhtml"));
    assert!(!owns("sb", "aqz-KE-bpKQ.Big Buck Bunny.fsb.0.mhtml"));
    // Совпадает только имя, а не каталог.
    assert!(!is_file_of_stream(
        BASE,
        "133",
        Path::new("/aqz-KE-bpKQ.Big Buck Bunny.f133.mp4/другое.mp4")
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
            "aqz-KE-bpKQ.Big Buck Bunny.f133.mp4".to_string(),
            "aqz-KE-bpKQ.Big Buck Bunny.f139.m4a".to_string()
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
    let lines: Vec<String> = launch_stdout("one-format-missing.json", dir.path())
        .into_iter()
        .take(4)
        .collect();
    assert!(
        matches!(parse_line(&lines[0]), StdoutLine::SelectedFormats { .. }),
        "сценарий обязан нести перечень"
    );
    let launcher = ScriptedLauncher::new(
        dir.path(),
        vec![Script::failing(1, &connection_lost_stderr())
            .lines(lines)
            .creates("aqz-KE-bpKQ.Big Buck Bunny.f133.mp4.part")],
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
    let lines: Vec<String> = launch_stdout("video-and-audio.json", dir.path())
        .into_iter()
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
        vec![Script::ok()
            .lines(lines)
            .creates("aqz-KE-bpKQ.Big Buck Bunny.f133.mp4")],
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
        dir.path()
            .join("aqz-KE-bpKQ.Big Buck Bunny.f139.m4a")
            .display()
    );
    let second = ScriptedLauncher::new(
        dir.path(),
        vec![Script::ok()
            .creates("aqz-KE-bpKQ.Big Buck Bunny.f139.m4a")
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
                .line(&destination_line(
                    dir.path(),
                    "aqz-KE-bpKQ.Big Buck Bunny.f133.mp4",
                ))
                .lines(fixture_progress("video-and-audio.json", "133"))
                .creates("aqz-KE-bpKQ.Big Buck Bunny.f133.mp4"),
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
        // TL-104: набор снят до id ролика в рабочей основе. Из названия и
        // ссылки съёмки (ролик `aqz-KE-bpKQ`, его id в `[info]` снятого
        // stdout) приложение строит `<id>.<основа съёмки>` — и больше ничем
        // от съёмки не отличается. Id в имена файлов снятого вывода
        // подставляет оснастка ([`shot_names_to_app`]). Основа съёмки —
        // `-o` съёмки без нашего хвоста (`shot_base`); сверять её с тем же
        // `-o` бессмысленно, настоящие проверки — ниже: основа с id против
        // построенной приложением и весь argv против `download_args`.
        let shot_stem = shot_base(&capture);
        let template = output_template(&partial_base(&shot_stem, URL));
        assert_eq!(
            template,
            format!("aqz-KE-bpKQ.{shot_stem}{TAIL}"),
            "{name}: основа имени съёмки с id — не та, что приложение построило бы \
             из названия и ссылки"
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
        let output_at = shot.iter().position(|arg| arg == "-o").expect("-o есть") + 1;
        shot[output_at] = template.clone();
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
                .line(&destination_line(
                    dir.path(),
                    "aqz-KE-bpKQ.Big Buck Bunny.f133.mp4",
                ))
                .lines(launch_progress("video-and-audio.json", "133"))
                .creates("aqz-KE-bpKQ.Big Buck Bunny.f133.mp4"),
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
        partial_base(TITLE, URL),
        format!("aqz-KE-bpKQ.{TITLE}"),
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
        (
            format!("aqz-KE-bpKQ.{TITLE}.f133.mp4"),
            format!("aqz-KE-bpKQ.{TITLE}.f139.m4a")
        )
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
    let destination = destination_line(dir.path(), "aqz-KE-bpKQ.Big Buck Bunny.f133.mp4");
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
            .line(&destination_line(
                dir.path(),
                "aqz-KE-bpKQ.Big Buck Bunny.f140.m4a",
            ))
            .line(&samples[0])
            .line(&samples[1])
            // Та же строка ещё дважды: вывод есть, принятых байт больше
            // не становится.
            .line(&repeated)
            .line(&repeated)
            .creates("aqz-KE-bpKQ.Big Buck Bunny.f140.m4a")],
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
                .line(&destination_line(
                    dir.path(),
                    "aqz-KE-bpKQ.Big Buck Bunny.f140.m4a",
                ))
                .lines(
                    fixture_progress("audio-only.json", "140")
                        .into_iter()
                        .take(3),
                )
                .creates("aqz-KE-bpKQ.Big Buck Bunny.f140.m4a.part"),
            Script::ok()
                .line(&destination_line(
                    dir.path(),
                    "aqz-KE-bpKQ.Big Buck Bunny.f140.m4a",
                ))
                .lines(fixture_progress("audio-only.json", "140"))
                .creates("aqz-KE-bpKQ.Big Buck Bunny.f140.m4a"),
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
            .line(&destination_line(
                dir.path(),
                "aqz-KE-bpKQ.Big Buck Bunny.f140.m4a",
            ))
            .creates("aqz-KE-bpKQ.Big Buck Bunny.f140.m4a")],
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
                .creates("aqz-KE-bpKQ.Big Buck Bunny.f140.m4a.part"),
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
        [
            "aqz-KE-bpKQ.Big Buck Bunny.f133.mp4",
            "aqz-KE-bpKQ.Big Buck Bunny.f139.m4a"
        ],
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
            .line(&destination_line(
                dir.path(),
                "aqz-KE-bpKQ.Big Buck Bunny.f602.mp4",
            ))
            .lines(samples.clone())
            .creates("aqz-KE-bpKQ.Big Buck Bunny.f602.mp4")],
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
                .line(&destination_line(
                    dir.path(),
                    "aqz-KE-bpKQ.Big Buck Bunny.f140.m4a",
                ))
                .lines(samples.iter().take(2).cloned())
                .creates("aqz-KE-bpKQ.Big Buck Bunny.f140.m4a.part"),
            Script::ok()
                .line(&destination_line(
                    dir.path(),
                    "aqz-KE-bpKQ.Big Buck Bunny.f140.m4a",
                ))
                .lines(samples)
                .creates("aqz-KE-bpKQ.Big Buck Bunny.f140.m4a"),
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
    use crate::download::filename::MAX_FILE_NAME_BYTES;
    use crate::download::merge::{working_file_name, MergeContainer};

    // Самый длинный id — запасной вариант (TL-104): ссылка без распознанного id.
    let hashed = PartialId::of_url("https://example.com/video");
    assert_eq!(
        hashed.as_str().len(),
        MAX_PARTIAL_ID_BYTES,
        "сторож стоит не на самом длинном id"
    );

    for title in ["я".repeat(400), "a".repeat(400)] {
        let download = download_stem(&sanitized_stem(&title, "aqz-KE-bpKQ"), &hashed);
        assert!(
            download.starts_with(&format!("{}.", hashed.as_str())),
            "id обязан доезжать до имени целиком: {download}"
        );
        assert!(
            download.len() <= download_stem_budget(),
            "{}",
            download.len()
        );

        // Предел всего, что дописывается к основе частичного: идентификатор
        // формата, расширение в 8 байт и `.part`.
        let longest = format!(
            "{download}.f{}.{}.part",
            "a".repeat(MAX_FORMAT_ID_BYTES),
            "e".repeat(8)
        );
        assert!(
            longest.len() <= MAX_FILE_NAME_BYTES,
            "имя частичного файла длиной {} байт не переживёт ни одну из трёх ФС",
            longest.len()
        );
        for container in [
            MergeContainer::Mp4,
            MergeContainer::Webm,
            MergeContainer::Mkv,
        ] {
            let working = working_file_name(&download, container);
            assert!(
                working.len() <= MAX_FILE_NAME_BYTES,
                "рабочий файл склейки длиной {} байт: {working}",
                working.len()
            );
        }
    }

    // ASCII-название упирается в бюджет байт в байт: сторож стоит на пределе,
    // а не рядом с ним.
    let ascii = download_stem(&sanitized_stem(&"a".repeat(400), "id"), &hashed);
    assert_eq!(ascii.len(), download_stem_budget());
}

#[test]
fn the_partial_name_is_derived_from_the_title_and_the_link_and_nothing_else() {
    // На этом стоит обещание Р-2: после выхода из приложения пользователь
    // вставляет ту же ссылку, ядро строит то же имя, yt-dlp продолжает с
    // места. Любая недетерминированная часть имени это отменяет.
    let first = partial_base(TITLE, URL);
    let second = partial_base(TITLE, URL);

    assert_eq!(first, second);
    assert_eq!(first, BASE);
}

#[test]
fn a_stream_prefix_cannot_swallow_a_neighbouring_file() {
    // Без точки после идентификатора формата префикс «Название.f» совпал
    // бы с посторонним «Название.flv», и атрибуция приписывала бы потоку
    // чужое имя. Подчистку префикс с TL-106 не решает — её держит белый
    // список рабочих имён (`is_working_name_of`).
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

// ─────────────────── Настройки и история (TL-89, эпик E5) ───────────────────

/// Хранилище настроек из `settings.json`, записанного руками.
///
/// Путь своей папки кладётся в файл как есть, без канонизации (правило
/// чтения её не делает): так тест различает «ту же папку» и «ту же, но
/// канонизированную». Настройки, которые хранилище не приняло, роняют тест:
/// иначе он молча проверял бы умолчания.
fn settings_with(
    data: &Path,
    folder: Option<&Path>,
    template: &str,
    attempts: u32,
) -> Arc<SettingsState> {
    let destination = match folder {
        None => serde_json::json!({ "kind": "system" }),
        Some(path) => serde_json::json!({
            "kind": "custom",
            "path": path.to_str().expect("папка теста в UTF-8"),
        }),
    };
    let file = serde_json::json!({
        "version": 1,
        "destinationFolder": destination,
        "nameTemplate": template,
        "maxAttempts": attempts,
    });
    std::fs::create_dir_all(data).unwrap();
    std::fs::write(
        data.join("settings.json"),
        serde_json::to_vec(&file).unwrap(),
    )
    .unwrap();

    let state = SettingsState::open_isolated(Ok(data.to_path_buf()));
    let readout = state.store().expect("хранилище настроек открыто").readout();
    assert!(
        readout.reset_fields.is_empty() && !readout.whole_file_reset,
        "настройки теста не приняты ({template:?}): сброшены {:?}",
        readout.reset_fields
    );
    Arc::new(state)
}

fn history_in(data: &Path) -> Arc<HistoryState> {
    let state = HistoryState::open_isolated(Ok(data.to_path_buf()));
    assert!(state.store().is_ok(), "история теста открыта");
    Arc::new(state)
}

fn records(history: &HistoryState) -> Vec<crate::storage::history::HistoryRecord> {
    history
        .store()
        .expect("история открыта")
        .page(None)
        .expect("страница истории читается")
        .records
}

fn env_with(
    settings: Option<Arc<SettingsState>>,
    history: Option<Arc<HistoryState>>,
    system_downloads: Option<&Path>,
) -> TaskEnv {
    TaskEnv {
        settings,
        history,
        system_downloads: system_downloads.map(Path::to_path_buf),
        today: fixed_today,
    }
}

fn custom(path: &Path) -> crate::types::FolderDisplay {
    crate::types::FolderDisplay::Custom {
        path: path.to_str().expect("UTF-8").to_string(),
    }
}

fn audio_task() -> Arc<DownloadTask> {
    new_task(request(streams(None, Some("140"))))
}

/// Всё под `root` — каталоги и файлы, пути через `/`, по порядку.
fn tree(root: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).expect("каталог читается").flatten() {
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .expect("под корнем")
                .components()
                .map(|part| part.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            out.push(relative);
            if entry.file_type().expect("тип").is_dir() {
                walk(root, &path, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

#[tokio::test]
async fn a_custom_folder_from_the_settings_reaches_the_process_the_file_the_panel_and_the_history()
{
    // К-4 [авто]: папка из настроек доезжает до `run_task` (`-P`), до файла,
    // до `Done.folderDisplay` и до записи истории.
    let root = tempfile::tempdir().unwrap();
    let system = root.path().join("Загрузки");
    let own = root.path().join("своя папка");
    std::fs::create_dir(&system).unwrap();
    std::fs::create_dir(&own).unwrap();
    let data = root.path().join("data");

    let history = history_in(&data);
    let env = env_with(
        Some(settings_with(&data, Some(&own), "{title}", 8)),
        Some(Arc::clone(&history)),
        Some(&system),
    );
    let sink = RecordingSink::new();
    let launcher = ScriptedLauncher::new(&own, vec![Script::ok().emulate()]);

    super::run_task(
        &audio_task(),
        &launcher,
        &ScriptedFfmpeg::merging(),
        &sink,
        &env,
    )
    .await;

    assert_eq!(
        launcher.calls()[0].value_of("-P"),
        Some(format!("home:{}", own.display()).as_str()),
        "yt-dlp получил папку из настроек"
    );
    assert_eq!(
        sink.last(),
        DownloadProgress::Done {
            file_name: "Big Buck Bunny.m4a".to_string(),
            folder_display: custom(&own),
        }
    );
    assert_eq!(dir_listing(&own), ["Big Buck Bunny.m4a"]);
    assert_eq!(
        dir_listing(&system),
        Vec::<String>::new(),
        "не в «Загрузки»"
    );

    let records = records(&history);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].folder, own);
    assert_eq!(records[0].file_name, "Big Buck Bunny.m4a");
    assert_eq!(
        folder_display(&records[0].folder, Some(&system)),
        custom(&own),
        "панель «Готово» и история называют папку одинаково"
    );
}

#[tokio::test]
async fn a_settings_change_during_a_task_waits_for_the_next_task() {
    // К-4, Р-4: смена папки и шаблона посреди задачи не трогает идущую и
    // достаётся ожидающей. Обе задачи поставлены **до** старта первой —
    // вторая стоит в очереди, пока меняются настройки.
    let root = tempfile::tempdir().unwrap();
    let folder_a = root.path().join("A");
    let folder_b = root.path().join("B");
    std::fs::create_dir(&folder_a).unwrap();
    std::fs::create_dir(&folder_b).unwrap();
    let data = root.path().join("data");

    let settings = settings_with(&data, Some(&folder_a), "{title}", 8);
    let env = env_with(Some(Arc::clone(&settings)), None, None);

    let first = audio_task();
    let mut other = request(streams(None, Some("140")));
    other.url = "https://www.youtube.com/watch?v=dQw4w9WgXcQ".to_string();
    other.title = "Never Gonna Give You Up".to_string();
    let second = new_task(other);

    let changer = Arc::clone(&settings);
    let path_b = folder_b.to_str().unwrap().to_string();
    let change = move || {
        let store = changer.store().expect("хранилище настроек");
        store
            .set(&crate::types::SettingsPatch::DestinationFolder(
                crate::types::DestinationFolder::Custom {
                    path: path_b.clone(),
                },
            ))
            .expect("папка сохранена");
        store
            .set(&crate::types::SettingsPatch::NameTemplate(
                "{id} — {title}".to_string(),
            ))
            .expect("шаблон сохранён");
    };

    let sink_a = RecordingSink::new();
    let launcher_a = ScriptedLauncher::new(&folder_a, vec![Script::ok().hook(change).emulate()]);
    super::run_task(
        &first,
        &launcher_a,
        &ScriptedFfmpeg::merging(),
        &sink_a,
        &env,
    )
    .await;

    assert_eq!(
        sink_a.last(),
        DownloadProgress::Done {
            file_name: "Big Buck Bunny.m4a".to_string(),
            folder_display: custom(&folder_a),
        },
        "идущая задача держит папку и имя своего старта"
    );

    let sink_b = RecordingSink::new();
    let launcher_b = ScriptedLauncher::new(&folder_b, vec![Script::ok().emulate()]);
    super::run_task(
        &second,
        &launcher_b,
        &ScriptedFfmpeg::merging(),
        &sink_b,
        &env,
    )
    .await;

    // Сохранение канонизирует путь папки (TL-87) — ожидание берёт то же.
    let saved_b = std::fs::canonicalize(&folder_b).unwrap();
    assert_eq!(
        sink_b.last(),
        DownloadProgress::Done {
            file_name: "dQw4w9WgXcQ — Never Gonna Give You Up.m4a".to_string(),
            folder_display: custom(&saved_b),
        },
        "ожидавшая задача получила настройки своего старта"
    );
    assert_eq!(dir_listing(&folder_a), ["Big Buck Bunny.m4a"]);
    assert_eq!(
        dir_listing(&folder_b),
        ["dQw4w9WgXcQ — Never Gonna Give You Up.m4a"]
    );
}

#[tokio::test]
async fn an_id_and_title_template_gives_the_expected_name() {
    let dir = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let env = env_with(
        Some(settings_with(data.path(), None, "{id} — {title}", 8)),
        None,
        Some(dir.path()),
    );
    let sink = RecordingSink::new();
    let launcher = ScriptedLauncher::new(dir.path(), vec![Script::ok().emulate()]);

    super::run_task(
        &audio_task(),
        &launcher,
        &ScriptedFfmpeg::merging(),
        &sink,
        &env,
    )
    .await;

    assert_eq!(
        sink.last(),
        DownloadProgress::Done {
            file_name: "aqz-KE-bpKQ — Big Buck Bunny.m4a".to_string(),
            folder_display: crate::types::FolderDisplay::SystemDownloads,
        }
    );
    assert_eq!(
        dir_listing(dir.path()),
        ["aqz-KE-bpKQ — Big Buck Bunny.m4a"]
    );
}

#[tokio::test]
async fn the_default_template_names_every_file_exactly_like_e3_did() {
    // Ф-12, К-5: умолчание `{title}` из настоящего хранилища (файла нет —
    // умолчания) даёт байт в байт имя E3 — `sanitized_stem(название,
    // video_id_of(ссылка))` — на корпусе названий E3, включая запасное имя.
    let mut titles: Vec<String> = HOSTILE_TITLES.iter().map(|t| (*t).to_string()).collect();
    titles.extend(
        [
            TITLE,
            "CON",
            "отчёт\u{202e}gpj.exe",
            "a/b\\c:d*e?f\"g<h>i|j",
            "..",
            "???",
            "   ",
            "Название. ",
        ]
        .map(str::to_string),
    );
    titles.push("я".repeat(400));

    for title in titles {
        let dir = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let settings = Arc::new(SettingsState::open_isolated(Ok(data.path().to_path_buf())));
        let env = env_with(Some(settings), None, Some(dir.path()));
        let mut req = request(streams(None, Some("140")));
        req.title = title.clone();
        let sink = RecordingSink::new();
        let launcher = ScriptedLauncher::new(dir.path(), vec![Script::ok().emulate()]);

        super::run_task(
            &new_task(req),
            &launcher,
            &ScriptedFfmpeg::merging(),
            &sink,
            &env,
        )
        .await;

        let e3 = format!("{}.m4a", sanitized_stem(&title, &video_id_of(URL)));
        assert_eq!(
            sink.last(),
            DownloadProgress::Done {
                file_name: e3.clone(),
                folder_display: crate::types::FolderDisplay::SystemDownloads,
            },
            "название {title:?}"
        );
        assert_eq!(dir_listing(dir.path()), [e3], "название {title:?}");
    }
}

#[tokio::test]
async fn the_fallback_name_takes_the_id_e3_took_even_where_the_canonical_id_differs() {
    // F3 ревью TL-86: `{id}` и запасное имя — `video_id_of`, как в E3, а не
    // канонический id очереди. У `youtu.be/ID#t=10` они разные: канонический
    // разбор отбрасывает фрагмент, нестрогий E3 — нет.
    const LINK: &str = "https://youtu.be/aqz-KE-bpKQ#t=10";
    const NOTHING_LEFT: &str = "???";
    let e3_id = video_id_of(LINK);
    let canonical = canonical_video_id(LINK).expect("форма разбирается TL-72");
    assert_ne!(
        sanitized_stem(NOTHING_LEFT, &e3_id),
        sanitized_stem(NOTHING_LEFT, canonical.as_str()),
        "проверять нечего, если на этой ссылке запасные имена совпадают"
    );

    for (template, stem) in [
        ("{title}", sanitized_stem(NOTHING_LEFT, &e3_id)),
        ("{id}", sanitized_stem(&e3_id, &e3_id)),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let env = env_with(
            Some(settings_with(data.path(), None, template, 8)),
            None,
            Some(dir.path()),
        );
        let mut req = request(streams(None, Some("140")));
        req.url = LINK.to_string();
        req.title = NOTHING_LEFT.to_string();
        let sink = RecordingSink::new();
        let launcher = ScriptedLauncher::new(dir.path(), vec![Script::ok().emulate()]);

        super::run_task(
            &new_task(req),
            &launcher,
            &ScriptedFfmpeg::merging(),
            &sink,
            &env,
        )
        .await;

        assert_eq!(
            sink.last(),
            DownloadProgress::Done {
                file_name: format!("{stem}.m4a"),
                folder_display: crate::types::FolderDisplay::SystemDownloads,
            },
            "шаблон {template}"
        );
    }
}

/// Шов даты «следующего дня» после [`fixed_today`]: 2031-02-04.
fn next_day() -> TemplateDate {
    TemplateDate::new(2031, 2, 4).expect("дата существует")
}

#[tokio::test]
async fn the_date_in_the_name_is_the_completion_date_from_the_clock_seam() {
    // Ф-12: `{date}` — дата завершения из шва часов. Шов зовётся только перед
    // финализацией: в рабочие имена дата не входит вовсе (ревью TL-89, B1),
    // и `-o` запуска её не несёт. Что дата — именно дня завершения, а не
    // старта, доказывают тесты B1 со сменой дня между стартами.
    let dir = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let mut env = env_with(
        Some(settings_with(data.path(), None, "{date} {title}", 8)),
        None,
        Some(dir.path()),
    );
    env.today = next_day;
    let sink = RecordingSink::new();
    let launcher = ScriptedLauncher::new(dir.path(), vec![Script::ok().emulate()]);

    super::run_task(
        &audio_task(),
        &launcher,
        &ScriptedFfmpeg::merging(),
        &sink,
        &env,
    )
    .await;

    assert_eq!(
        sink.last(),
        DownloadProgress::Done {
            file_name: "2031-02-04 Big Buck Bunny.m4a".to_string(),
            folder_display: crate::types::FolderDisplay::SystemDownloads,
        }
    );
    assert_eq!(dir_listing(dir.path()), ["2031-02-04 Big Buck Bunny.m4a"]);
    assert_eq!(
        launcher.calls()[0].value_of("-o"),
        Some("aqz-KE-bpKQ.Big Buck Bunny.f%(format_id)s.%(ext)s"),
        "рабочее имя — основа E3, без даты и шаблона"
    );
}

#[test]
fn the_production_date_is_today_by_the_clock_the_preview_uses() {
    // Та же функция, что у предпросмотра (TL-91): `clock::today_utc`.
    let format = |(year, month, day): (i64, u32, u32)| format!("{year:04}-{month:02}-{day:02}");
    let before = format(clock::today_utc());
    let date = today_utc_date().to_string();
    let after = format(clock::today_utc());
    assert!(
        date == before || date == after,
        "{date} — не сегодня по UTC ({before})"
    );
}

#[tokio::test]
async fn one_attempt_from_the_settings_fails_after_the_first_interruption() {
    // К-6: настройка 1 — после первой же неудачи `connectionLost`, без пауз.
    let dir = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let env = env_with(
        Some(settings_with(data.path(), None, "{title}", 1)),
        None,
        Some(dir.path()),
    );
    let sink = RecordingSink::new();
    let launcher = ScriptedLauncher::new(
        dir.path(),
        vec![Script::failing(1, &connection_lost_stderr())
            .line(&destination_line(
                dir.path(),
                "aqz-KE-bpKQ.Big Buck Bunny.f140.m4a",
            ))
            .creates("aqz-KE-bpKQ.Big Buck Bunny.f140.m4a.part")],
    );

    super::run_task(
        &audio_task(),
        &launcher,
        &ScriptedFfmpeg::merging(),
        &sink,
        &env,
    )
    .await;

    assert_eq!(launcher.calls().len(), 1, "ровно одна попытка");
    let DownloadProgress::Failed { error } = sink.last() else {
        panic!("ожидался отказ, а не {:?}", sink.last());
    };
    assert_eq!(error.kind, DownloadErrorKind::ConnectionLost);
    assert!(
        error.message.contains(": 1 попыток"),
        "исчерпано 1 из 1: {}",
        error.message
    );
    assert!(
        !sink.events().iter().any(|p| matches!(
            p,
            DownloadProgress::Downloading(DownloadingState::WaitingRetry { .. })
        )),
        "паузы перед повтором при пределе 1 не бывает"
    );
}

fn waiting_attempts(sink: &RecordingSink) -> Vec<crate::types::DownloadAttempt> {
    let mut attempts: Vec<crate::types::DownloadAttempt> = sink
        .events()
        .iter()
        .filter_map(|progress| match progress {
            DownloadProgress::Downloading(DownloadingState::WaitingRetry { attempt, .. }) => {
                Some(*attempt)
            }
            _ => None,
        })
        .collect();
    attempts.dedup();
    attempts
}

#[tokio::test]
async fn the_attempt_total_is_taken_at_the_start_and_survives_a_change_of_the_setting() {
    // Ф-13, К-6: предел 3 из настроек — в `total` событий; смена настройки
    // на 5 посреди задачи его не трогает: задача сдаётся после третьей.
    let dir = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let settings = settings_with(data.path(), None, "{title}", 3);
    let env = env_with(Some(Arc::clone(&settings)), None, Some(dir.path()));
    let failing = || {
        Script::failing(1, &connection_lost_stderr())
            .line(&destination_line(
                dir.path(),
                "aqz-KE-bpKQ.Big Buck Bunny.f140.m4a",
            ))
            .creates("aqz-KE-bpKQ.Big Buck Bunny.f140.m4a.part")
    };
    let changer = Arc::clone(&settings);
    let scripts = vec![
        failing().hook(move || {
            changer
                .store()
                .expect("хранилище")
                .set(&crate::types::SettingsPatch::MaxAttempts(5))
                .expect("число попыток сохранено");
        }),
        failing(),
        failing(),
    ];
    let launcher = ScriptedLauncher::new(dir.path(), scripts);
    let sink = RecordingSink::new();

    tokio::time::pause();
    super::run_task(
        &audio_task(),
        &launcher,
        &ScriptedFfmpeg::merging(),
        &sink,
        &env,
    )
    .await;
    tokio::time::resume();

    assert_eq!(
        launcher.calls().len(),
        3,
        "попыток ровно три — предел старта"
    );
    assert_eq!(
        waiting_attempts(&sink)
            .iter()
            .map(|a| (a.number, a.total))
            .collect::<Vec<_>>(),
        [(2, 3), (3, 3)]
    );
    assert!(matches!(
        sink.last(),
        DownloadProgress::Failed { error } if error.kind == DownloadErrorKind::ConnectionLost
    ));
}

#[tokio::test]
async fn twenty_attempts_from_the_settings_show_twenty_in_the_events() {
    let dir = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let env = env_with(
        Some(settings_with(data.path(), None, "{title}", 20)),
        None,
        Some(dir.path()),
    );
    let launcher = ScriptedLauncher::new(
        dir.path(),
        vec![
            Script::failing(1, &connection_lost_stderr())
                .line(&destination_line(
                    dir.path(),
                    "aqz-KE-bpKQ.Big Buck Bunny.f140.m4a",
                ))
                .creates("aqz-KE-bpKQ.Big Buck Bunny.f140.m4a.part"),
            Script::ok().emulate(),
        ],
    );
    let sink = RecordingSink::new();

    tokio::time::pause();
    super::run_task(
        &audio_task(),
        &launcher,
        &ScriptedFfmpeg::merging(),
        &sink,
        &env,
    )
    .await;
    tokio::time::resume();

    assert_eq!(
        waiting_attempts(&sink)
            .iter()
            .map(|a| (a.number, a.total))
            .collect::<Vec<_>>(),
        [(2, 20)]
    );
    assert!(matches!(sink.last(), DownloadProgress::Done { .. }));
}

/// Приёмник, который на событии `done` смотрит, есть ли уже запись в
/// истории (Ф-3: запись раньше, чем о готовой задаче узнает окно).
struct SinkSeeingHistory {
    inner: RecordingSink,
    history: Arc<HistoryState>,
    records_at_done: StdMutex<Vec<usize>>,
}

impl ProgressSink for SinkSeeingHistory {
    fn emit(&self, event: DownloadProgressEvent) {
        if matches!(event.progress, DownloadProgress::Done { .. }) {
            let count = self
                .history
                .store()
                .expect("история")
                .page(None)
                .expect("страница")
                .records
                .len();
            self.records_at_done.lock().unwrap().push(count);
        }
        self.inner.emit(event);
    }
}

#[tokio::test]
async fn a_done_task_writes_exactly_one_record_and_failed_or_cancelled_write_none() {
    // Р-1, Р-2, К-2 [авто]: Done — одна запись с верными полями, записанная
    // раньше события; Failed и Cancelled — ни одной; запись переживает новое
    // открытие того же файла.
    let root = tempfile::tempdir().unwrap();
    let downloads = root.path().join("Загрузки");
    std::fs::create_dir(&downloads).unwrap();
    let data = root.path().join("data");
    let history = history_in(&data);

    // Done.
    let env = env_with(None, Some(Arc::clone(&history)), Some(&downloads));
    let sink = SinkSeeingHistory {
        inner: RecordingSink::new(),
        history: Arc::clone(&history),
        records_at_done: StdMutex::new(Vec::new()),
    };
    let launcher = ScriptedLauncher::new(&downloads, vec![Script::ok().emulate()]);
    let before = clock::now_unix_secs();
    super::run_task(
        &audio_task(),
        &launcher,
        &ScriptedFfmpeg::merging(),
        &sink,
        &env,
    )
    .await;
    let after = clock::now_unix_secs();

    let DownloadProgress::Done {
        file_name,
        folder_display: shown,
    } = sink.inner.last()
    else {
        panic!("ожидался Done, а не {:?}", sink.inner.last());
    };
    assert_eq!(
        *sink.records_at_done.lock().unwrap(),
        [1],
        "к событию done запись уже в базе"
    );

    // Failed: папки назначения нет.
    let missing = root.path().join("нет такой папки");
    let failed_env = env_with(None, Some(Arc::clone(&history)), Some(&missing));
    let failed_sink = RecordingSink::new();
    super::run_task(
        &audio_task(),
        &ScriptedLauncher::new(&missing, Vec::new()),
        &ScriptedFfmpeg::merging(),
        &failed_sink,
        &failed_env,
    )
    .await;
    assert!(matches!(
        failed_sink.last(),
        DownloadProgress::Failed { .. }
    ));

    // Cancelled.
    let cancelled = audio_task();
    cancelled.cancel().await;
    let cancelled_sink = RecordingSink::new();
    super::run_task(
        &cancelled,
        &ScriptedLauncher::new(&downloads, Vec::new()),
        &ScriptedFfmpeg::merging(),
        &cancelled_sink,
        &env,
    )
    .await;
    assert!(matches!(
        cancelled_sink.last(),
        DownloadProgress::Cancelled { .. }
    ));

    let records = records(&history);
    assert_eq!(
        records.len(),
        1,
        "Done — одна запись, Failed и Cancelled — ни одной"
    );
    let record = &records[0];
    assert_eq!(record.video_id, "aqz-KE-bpKQ", "канонический id TL-72");
    assert_eq!(record.url, URL);
    assert_eq!(record.title, TITLE);
    assert_eq!(
        record.quality,
        SelectedQuality {
            kind: QualityKind::Standard,
            height_px: Some(720),
        }
    );
    assert_eq!(record.file_name, file_name);
    assert_eq!(record.folder, downloads);
    assert_eq!(
        record.size_bytes,
        std::fs::metadata(downloads.join(&file_name)).unwrap().len()
    );
    assert!((before..=after).contains(&record.finished_at_unix_secs));
    assert_eq!(
        folder_display(&record.folder, Some(&downloads)),
        shown,
        "системная папка названа одинаково на панели и в истории"
    );
    assert_eq!(shown, crate::types::FolderDisplay::SystemDownloads);

    // К-2: новое открытие того же файла отдаёт запись первой.
    let reopened = crate::storage::history::HistoryStore::open_isolated(&data).expect("открытие");
    let page = reopened.page(None).expect("страница");
    assert_eq!(page.records.first().map(|r| r.id), Some(record.id));
}

#[cfg(unix)]
#[tokio::test]
async fn a_refused_insert_leaves_the_task_done_and_leaves_a_notice() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    let downloads = root.path().join("Загрузки");
    std::fs::create_dir(&downloads).unwrap();
    let data = root.path().join("data");
    let history = history_in(&data);
    let env = env_with(None, Some(Arc::clone(&history)), Some(&downloads));
    let sink = RecordingSink::new();
    let launcher = ScriptedLauncher::new(&downloads, vec![Script::ok().emulate()]);

    // Журнал SQLite в каталоге только на чтение не создаётся — вставка
    // отказывает на настоящем диске (приём `history_tests`).
    std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o555)).unwrap();
    super::run_task(
        &audio_task(),
        &launcher,
        &ScriptedFfmpeg::merging(),
        &sink,
        &env,
    )
    .await;
    std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(
        sink.last(),
        DownloadProgress::Done {
            file_name: "Big Buck Bunny.m4a".to_string(),
            folder_display: crate::types::FolderDisplay::SystemDownloads,
        },
        "отказ записи в историю не меняет исход (Н-4)"
    );
    assert_eq!(dir_listing(&downloads), ["Big Buck Bunny.m4a"]);
    let page = history.store().unwrap().page(None).unwrap();
    assert!(page.records.is_empty());
    assert_eq!(
        page.notices,
        vec![crate::types::HistoryNotice::LastWriteFailed {
            cause: HistoryWriteFailure::NoAccess
        }]
    );
}

#[tokio::test]
async fn without_a_history_the_task_is_still_done() {
    for history in [
        None,
        Some(Arc::new(HistoryState::open_isolated(Err(
            "каталог данных не определяется".to_string(),
        )))),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let env = env_with(None, history, Some(dir.path()));
        let sink = RecordingSink::new();
        let launcher = ScriptedLauncher::new(dir.path(), vec![Script::ok().emulate()]);

        super::run_task(
            &audio_task(),
            &launcher,
            &ScriptedFfmpeg::merging(),
            &sink,
            &env,
        )
        .await;

        assert!(matches!(sink.last(), DownloadProgress::Done { .. }));
        assert_eq!(dir_listing(dir.path()), ["Big Buck Bunny.m4a"]);
    }
}

#[tokio::test]
async fn no_hostile_template_or_title_puts_a_file_outside_the_folder_or_into_a_subfolder() {
    // К-5 через настоящий путь оркестрации: шаблон из файла настроек, файл
    // потока — по argv (`Step::Emulate`), финализация — своя. Что доказывает
    // каждая из двух проверок (ревью TL-89, S1):
    //
    // - **argv**: `-o` запуска равен `output_template(download_stem(основа
    //   E3, id ролика))` и до нашего хвоста состоит только из символов
    //   `template_safe` — вместе с id ролика в основе (TL-104).
    //   В шаблон yt-dlp не уходит ничего, что он раскрыл бы (`$`, `%`, ведущая
    //   `~`, разделители), и ничего от шаблона настроек. Раскрытия сценарий не
    //   моделирует, поэтому доказывает это только argv, а не обход диска.
    //   Мутация «сырое название в `-o`» краснеет здесь, до исхода задачи.
    // - **обход ФС**: после `Done` под корнем ровно папка назначения и один
    //   файл в ней. Он ловит пояс финализации — `candidate_name` санитизирует
    //   основу сам и не выпускает разделителей — и отсутствие подпапок. Чего
    //   он **не** доказывает: что финальная основа прошла `file_stem`
    //   (подстановка и санитизация E3). Мутация «финальная основа — сырое
    //   название» здесь зелёная, её держит `candidate_name`; ловят её тесты
    //   имени (`the_default_template_names_every_file_exactly_like_e3_did` и
    //   соседние, сверяющие имя готового файла байт в байт).
    const TEMPLATES: &[&str] = &[
        "{title}",
        "../{title}",
        "..\\{title}",
        "{title}/../../escape",
        "/{id}",
        "C:\\{id}",
        "~/{title}",
        "$HOME/{title}",
        // `${HOME}` — не шаблон (`{HOME}` вне белого списка) и до оркестрации
        // не доходит; раскрытие доллара проверяется литералом `$HOME`.
        "$HOME-{id}",
        "%(ext)s{title}",
        "%%(title)s{id}",
        "{id}/{title}",
        "\u{202e}{title}",
        "{quality}",
        ".{date}.",
        "CON{quality}",
        "{title}\n{id}",
        "  {title}  ",
        "{title}.part",
        "{title}:{id}*?",
        // Финальное имя под префиксом частичного файла и склейки (B1; с
        // TL-104 в префиксе id ролика): подчистка на `Done` обязана не
        // тронуть готовый файл — с TL-106 он не рабочее имя.
        "{id}.{title}.f140.{id}",
        "{id}.{title}.tl-merging.{id}",
    ];
    let mut titles: Vec<String> = HOSTILE_TITLES.iter().map(|t| (*t).to_string()).collect();
    titles.extend(
        [
            "..",
            "../../etc/passwd",
            "CON",
            "a/b\\c",
            "{id}",
            "$HOME",
            ".hidden",
            "\u{202e}gpj.exe",
            "",
        ]
        .map(str::to_string),
    );
    titles.push("я/".repeat(200));

    let mut checked = 0;
    for (index, template) in TEMPLATES.iter().enumerate() {
        for title in &titles {
            let root = tempfile::tempdir().unwrap();
            let dest = root.path().join("dest");
            std::fs::create_dir(&dest).unwrap();
            let data = tempfile::tempdir().unwrap();
            let env = env_with(
                Some(settings_with(data.path(), Some(&dest), template, 1)),
                None,
                None,
            );
            let mut req = request(streams(None, Some("140")));
            req.title = title.clone();
            // Чётные шаблоны — со ступенью, нечётные — без: `{quality}` бывает
            // и пустым.
            req.quality.height_px = (index % 2 == 0).then_some(720);
            let sink = RecordingSink::new();
            let launcher = ScriptedLauncher::new(&dest, vec![Script::ok().emulate()]);

            super::run_task(
                &new_task(req),
                &launcher,
                &ScriptedFfmpeg::merging(),
                &sink,
                &env,
            )
            .await;

            // argv — раньше исхода задачи: неверный `-o` обязан краснеть
            // здесь, а не побочным отказом.
            let calls = launcher.calls();
            assert_eq!(calls.len(), 1, "шаблон {template:?}, название {title:?}");
            let output = calls[0].value_of("-o").expect("-o обязателен");
            assert_eq!(
                output,
                output_template(&partial_base(title, URL)),
                "шаблон {template:?}, название {title:?}: -o не из рабочей основы E3"
            );
            let head = output
                .strip_suffix(".f%(format_id)s.%(ext)s")
                .unwrap_or_else(|| panic!("хвост -o не наш: {output:?}"));
            assert!(
                !head.is_empty() && head.chars().all(template_safe),
                "шаблон {template:?}, название {title:?}: в -o символ вне белого списка: {output:?}"
            );
            assert_eq!(
                calls[0].emulate_failures,
                Vec::<String>::new(),
                "шаблон {template:?}, название {title:?}"
            );

            let DownloadProgress::Done { file_name, .. } = sink.last() else {
                panic!("шаблон {template:?}, название {title:?}: {:?}", sink.last());
            };
            assert_eq!(
                tree(root.path()),
                ["dest".to_string(), format!("dest/{file_name}")],
                "шаблон {template:?}, название {title:?}: файл вне папки назначения или подпапка"
            );
            assert!(dest.join(&file_name).is_file());
            checked += 1;
        }
    }
    assert_eq!(checked, TEMPLATES.len() * titles.len());
}

#[tokio::test]
async fn a_retry_after_the_folder_changed_downloads_again_into_the_new_folder() {
    // Ф-14, Р-4: «Повторить» — новый старт с текущими настройками. Папка
    // сменилась — забранные потоки лежат в прежней, задача качается с нуля в
    // новую, прежние частичные остаются, где лежали.
    let root = tempfile::tempdir().unwrap();
    let folder_a = root.path().join("A");
    let folder_b = root.path().join("B");
    std::fs::create_dir(&folder_a).unwrap();
    std::fs::create_dir(&folder_b).unwrap();
    let settings = settings_with(&root.path().join("data"), Some(&folder_a), "{title}", 8);
    let env = env_with(Some(Arc::clone(&settings)), None, None);
    let task = new_task(request(streams(Some("133"), Some("139"))));

    let sink = RecordingSink::new();
    let first = ScriptedLauncher::new(&folder_a, vec![Script::ok().emulate()]);
    super::run_task(&task, &first, &ScriptedFfmpeg::failing(), &sink, &env).await;
    assert!(matches!(
        sink.last(),
        DownloadProgress::Failed { error } if error.kind == DownloadErrorKind::MergeFailed
    ));
    let left_in_a = dir_listing(&folder_a);
    assert_eq!(left_in_a.len(), 2, "оба потока сохранены: {left_in_a:?}");

    settings
        .store()
        .unwrap()
        .set(&crate::types::SettingsPatch::DestinationFolder(
            crate::types::DestinationFolder::Custom {
                path: folder_b.to_str().unwrap().to_string(),
            },
        ))
        .unwrap();
    task.set_progress(DownloadProgress::Queued);

    let second = ScriptedLauncher::new(&folder_b, vec![Script::ok().emulate()]);
    let ffmpeg = ScriptedFfmpeg::merging();
    super::run_task(&task, &second, &ffmpeg, &sink, &env).await;

    let saved_b = std::fs::canonicalize(&folder_b).unwrap();
    assert_eq!(
        second.formats_asked(),
        ["133,139"],
        "качаются оба потока заново"
    );
    assert_eq!(
        second.calls()[0].value_of("-P"),
        Some(format!("home:{}", saved_b.display()).as_str())
    );
    assert_eq!(
        sink.last(),
        DownloadProgress::Done {
            file_name: "Big Buck Bunny.mp4".to_string(),
            folder_display: custom(&saved_b),
        }
    );
    assert_eq!(dir_listing(&folder_b), ["Big Buck Bunny.mp4"]);
    assert_eq!(
        dir_listing(&folder_a),
        left_in_a,
        "прежние частичные на месте"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn the_folder_is_named_from_the_path_as_given_on_the_panel_and_in_the_history() {
    // Без канонизации ни с одной стороны: путь через символьную ссылку —
    // тот же путь и на панели, и в истории, для системной и своей папки.
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().unwrap();
    let real_downloads = root.path().join("настоящие Загрузки");
    let real_own = root.path().join("настоящая своя");
    std::fs::create_dir(&real_downloads).unwrap();
    std::fs::create_dir(&real_own).unwrap();
    let downloads = root.path().join("Загрузки");
    let own = root.path().join("своя");
    symlink(&real_downloads, &downloads).unwrap();
    symlink(&real_own, &own).unwrap();
    assert_ne!(std::fs::canonicalize(&own).unwrap(), own);

    for (folder, expected) in [
        (None, crate::types::FolderDisplay::SystemDownloads),
        (Some(own.as_path()), custom(&own)),
    ] {
        let data = tempfile::tempdir().unwrap();
        let history = history_in(data.path());
        let env = env_with(
            Some(settings_with(data.path(), folder, "{title}", 8)),
            Some(Arc::clone(&history)),
            Some(&downloads),
        );
        let target = folder.unwrap_or(&downloads);
        let sink = RecordingSink::new();
        let launcher = ScriptedLauncher::new(target, vec![Script::ok().emulate()]);

        super::run_task(
            &audio_task(),
            &launcher,
            &ScriptedFfmpeg::merging(),
            &sink,
            &env,
        )
        .await;

        let DownloadProgress::Done {
            folder_display: shown,
            ..
        } = sink.last()
        else {
            panic!("ожидался Done, а не {:?}", sink.last());
        };
        let records = records(&history);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].folder, target);
        assert_eq!(shown, expected);
        assert_eq!(
            folder_display(&records[0].folder, Some(&downloads)),
            shown,
            "панель и история называют папку {folder:?} одинаково"
        );
    }
}

// ───────────── Рабочие имена не зависят от шаблона и даты (ревью TL-89, B1) ─────────────

#[tokio::test]
async fn a_retry_after_a_failed_merge_on_the_next_day_merges_yesterdays_streams_and_leaves_only_the_file(
) {
    // B1, сценарий ревьюера 1: шаблон с `{date}`, склейка отказала 3-го,
    // «Повторить» 4-го. Потоки не качаются заново, а после готовности в
    // папке только готовый файл — вчерашних частичных не остаётся.
    let dir = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let settings = settings_with(data.path(), None, "{date} {title}", 8);
    let today = env_with(Some(Arc::clone(&settings)), None, Some(dir.path()));
    let mut tomorrow = env_with(Some(settings), None, Some(dir.path()));
    tomorrow.today = next_day;
    let task = new_task(request(streams(Some("133"), Some("139"))));
    // Один сценарий на оба старта: второй запуск yt-dlp уронил бы тест.
    let launcher = ScriptedLauncher::new(dir.path(), vec![Script::ok().emulate()]);
    let sink = RecordingSink::new();

    super::run_task(&task, &launcher, &ScriptedFfmpeg::failing(), &sink, &today).await;
    assert!(matches!(
        sink.last(),
        DownloadProgress::Failed { error } if error.kind == DownloadErrorKind::MergeFailed
    ));
    assert_eq!(
        dir_listing(dir.path()),
        [
            "aqz-KE-bpKQ.Big Buck Bunny.f133.mp4",
            "aqz-KE-bpKQ.Big Buck Bunny.f139.m4a"
        ],
        "в рабочих именах нет ни даты, ни шаблона"
    );

    task.set_progress(DownloadProgress::Queued);
    super::run_task(
        &task,
        &launcher,
        &ScriptedFfmpeg::merging(),
        &sink,
        &tomorrow,
    )
    .await;

    assert_eq!(
        launcher.calls().len(),
        1,
        "вчерашние потоки не качаются заново"
    );
    assert_eq!(
        sink.last(),
        DownloadProgress::Done {
            file_name: "2031-02-04 Big Buck Bunny.mp4".to_string(),
            folder_display: crate::types::FolderDisplay::SystemDownloads,
        }
    );
    assert_eq!(
        dir_listing(dir.path()),
        ["2031-02-04 Big Buck Bunny.mp4"],
        "частичные файлы подчищены"
    );
}

#[tokio::test]
async fn a_task_restored_on_the_next_day_resumes_yesterdays_part_file_and_leaves_only_the_file() {
    // B1, сценарий ревьюера 2: 3-го поток оборвался (`.part` сохранён),
    // приложение перезапущено, очередь восстановила задачу из снимка тем же
    // запросом, и 4-го она стартует заново. Имя частичного то же, `.part`
    // докачивается, а не качается рядом под новым именем.
    let dir = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let settings = settings_with(data.path(), None, "{date} {title}", 1);
    let today = env_with(Some(Arc::clone(&settings)), None, Some(dir.path()));
    let mut tomorrow = env_with(Some(settings), None, Some(dir.path()));
    tomorrow.today = next_day;

    let yesterday = ScriptedLauncher::new(
        dir.path(),
        vec![Script::failing(1, &connection_lost_stderr()).emulate_partial()],
    );
    let sink = RecordingSink::new();
    super::run_task(
        &audio_task(),
        &yesterday,
        &ScriptedFfmpeg::merging(),
        &sink,
        &today,
    )
    .await;
    assert!(matches!(
        sink.last(),
        DownloadProgress::Failed { error } if error.kind == DownloadErrorKind::ConnectionLost
    ));
    assert_eq!(
        dir_listing(dir.path()),
        ["aqz-KE-bpKQ.Big Buck Bunny.f140.m4a.part"]
    );

    // Перезапуск: новая задача из того же запроса (снимок очереди, Ф-9 E4).
    let restored = audio_task();
    let next = ScriptedLauncher::new(dir.path(), vec![Script::ok().emulate()]);
    let sink = RecordingSink::new();
    super::run_task(
        &restored,
        &next,
        &ScriptedFfmpeg::merging(),
        &sink,
        &tomorrow,
    )
    .await;

    assert_eq!(
        next.calls()[0].value_of("-o"),
        yesterday.calls()[0].value_of("-o"),
        "имя частичного файла то же, что вчера"
    );
    let done = "2031-02-04 Big Buck Bunny.m4a";
    assert_eq!(
        sink.last(),
        DownloadProgress::Done {
            file_name: done.to_string(),
            folder_display: crate::types::FolderDisplay::SystemDownloads,
        }
    );
    assert_eq!(
        dir_listing(dir.path()),
        [done],
        "вчерашний .part не остался"
    );
    assert_eq!(
        std::fs::read(dir.path().join(done)).unwrap(),
        PARTIAL_BYTES,
        "готовый файл — докачанный вчерашний .part, а не новая загрузка"
    );
}

#[tokio::test]
async fn a_template_change_between_starts_does_not_stop_the_resume() {
    // B1: смена шаблона между стартами забранные потоки не забывает — задачу
    // сбрасывает только смена папки. Имя готового файла — по новому шаблону.
    let dir = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let settings = settings_with(data.path(), None, "{title}", 8);
    let env = env_with(Some(Arc::clone(&settings)), None, Some(dir.path()));
    let task = new_task(request(streams(Some("133"), Some("139"))));
    let launcher = ScriptedLauncher::new(dir.path(), vec![Script::ok().emulate()]);
    let sink = RecordingSink::new();

    super::run_task(&task, &launcher, &ScriptedFfmpeg::failing(), &sink, &env).await;
    assert!(matches!(
        sink.last(),
        DownloadProgress::Failed { error } if error.kind == DownloadErrorKind::MergeFailed
    ));

    settings
        .store()
        .unwrap()
        .set(&crate::types::SettingsPatch::NameTemplate(
            "{id} {title}".to_string(),
        ))
        .unwrap();
    task.set_progress(DownloadProgress::Queued);
    super::run_task(&task, &launcher, &ScriptedFfmpeg::merging(), &sink, &env).await;

    assert_eq!(launcher.calls().len(), 1, "потоки не качаются заново");
    assert_eq!(
        sink.last(),
        DownloadProgress::Done {
            file_name: "aqz-KE-bpKQ Big Buck Bunny.mp4".to_string(),
            folder_display: crate::types::FolderDisplay::SystemDownloads,
        }
    );
    assert_eq!(dir_listing(dir.path()), ["aqz-KE-bpKQ Big Buck Bunny.mp4"]);
}

#[tokio::test]
async fn a_final_name_under_a_partial_prefix_survives_the_cleanup_on_done() {
    // Следствие B1: финальное имя строит шаблон независимо от рабочего, и
    // оно может начинаться с префикса потока или склейки. С TL-106 подчистка
    // удаляет только точные рабочие имена, и такое имя в них не входит;
    // пощада своего файла (`Cleanup::apply_sparing`) здесь уже не нужна — её
    // держит `a_finished_file_that_took_the_name_of_a_vanished_stream_survives_the_cleanup_on_done`.
    for (template, video, audio, expected) in [
        (
            "{id}.{title}.f140.{id}",
            None,
            "140",
            "aqz-KE-bpKQ.Big Buck Bunny.f140.aqz-KE-bpKQ.m4a",
        ),
        (
            "{id}.{title}.tl-merging.{id}",
            None,
            "140",
            "aqz-KE-bpKQ.Big Buck Bunny.tl-merging.aqz-KE-bpKQ.m4a",
        ),
        (
            "{id}.{title}.f133.{id}",
            Some("133"),
            "139",
            "aqz-KE-bpKQ.Big Buck Bunny.f133.aqz-KE-bpKQ.mp4",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let env = env_with(
            Some(settings_with(data.path(), None, template, 8)),
            None,
            Some(dir.path()),
        );
        let launcher = ScriptedLauncher::new(dir.path(), vec![Script::ok().emulate()]);
        let sink = RecordingSink::new();

        super::run_task(
            &new_task(request(streams(video, Some(audio)))),
            &launcher,
            &ScriptedFfmpeg::merging(),
            &sink,
            &env,
        )
        .await;

        assert_eq!(
            sink.last(),
            DownloadProgress::Done {
                file_name: expected.to_string(),
                folder_display: crate::types::FolderDisplay::SystemDownloads,
            },
            "шаблон {template}"
        );
        assert_eq!(dir_listing(dir.path()), [expected], "шаблон {template}");
    }
}

// ───────────── Отказы записи истории до вставки (ревью TL-89, S2) ─────────────

#[test]
fn a_finished_file_gone_before_its_size_was_taken_leaves_a_storage_failed_notice() {
    // `metadata` готового файла не снялся: файл удалили между финализацией и
    // `stat`. Записи нет, пометка — в первой странице истории.
    let data = tempfile::tempdir().unwrap();
    let folder = tempfile::tempdir().unwrap();
    let history = history_in(data.path());
    let task = audio_task();

    write_done(
        &history,
        &task.id,
        task.request(),
        folder.path().to_path_buf(),
        "Big Buck Bunny.m4a".to_string(),
    );

    let page = history.store().unwrap().page(None).unwrap();
    assert!(page.records.is_empty());
    assert_eq!(
        page.notices,
        vec![crate::types::HistoryNotice::LastWriteFailed {
            cause: HistoryWriteFailure::StorageFailed
        }]
    );
}

#[tokio::test]
async fn a_panic_in_the_history_write_leaves_a_storage_failed_notice() {
    // Паника на потоке блокирующего пула приходит `JoinError`: исход задачи
    // она не трогает (зовущий не падает), а пометка ставится.
    let data = tempfile::tempdir().unwrap();
    let folder = tempfile::tempdir().unwrap();
    let history = history_in(data.path());
    let env = env_with(None, Some(Arc::clone(&history)), None);

    record_done(
        &audio_task(),
        &env,
        folder.path(),
        "Big Buck Bunny.m4a",
        |_, _, _, _, _| panic!("запись истории упала (намеренно, тест S2)"),
    )
    .await;

    let page = history.store().unwrap().page(None).unwrap();
    assert!(page.records.is_empty());
    assert_eq!(
        page.notices,
        vec![crate::types::HistoryNotice::LastWriteFailed {
            cause: HistoryWriteFailure::StorageFailed
        }]
    );
}

/// Задача, которую отменяет [`cancelling_write`]. Своя на единственный тест.
static CANCEL_DURING_WRITE: StdMutex<Option<Arc<DownloadTask>>> = StdMutex::new(None);

/// Тело записи истории, которое посреди записи поднимает флаг отмены задачи
/// и пишет по-настоящему.
fn cancelling_write(
    history: &HistoryState,
    task_id: &str,
    request: &StartDownloadRequest,
    folder: PathBuf,
    file_name: String,
) {
    let task = CANCEL_DURING_WRITE
        .lock()
        .unwrap()
        .clone()
        .expect("задача теста");
    // Флаг, а не `DownloadTask::cancel`: процесса к этому моменту нет
    // (`set_child(None)` раньше записи), и убивать нечего.
    task.cancel.cancel();
    write_done(history, task_id, request, folder, file_name);
}

#[tokio::test]
async fn a_cancel_during_the_history_write_leaves_one_record_and_one_done() {
    let dir = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let history = history_in(data.path());
    let env = env_with(None, Some(Arc::clone(&history)), Some(dir.path()));
    let task = audio_task();
    *CANCEL_DURING_WRITE.lock().unwrap() = Some(Arc::clone(&task));
    let launcher = ScriptedLauncher::new(dir.path(), vec![Script::ok().emulate()]);
    let sink = RecordingSink::new();

    run_task_with(
        &task,
        &launcher,
        &ScriptedFfmpeg::merging(),
        &sink,
        &env,
        cancelling_write,
    )
    .await;

    assert!(task.cancel.is_cancelled(), "отмена пришла во время записи");
    let done = DownloadProgress::Done {
        file_name: "Big Buck Bunny.m4a".to_string(),
        folder_display: crate::types::FolderDisplay::SystemDownloads,
    };
    let terminal: Vec<DownloadProgress> = sink
        .events()
        .into_iter()
        .filter(DownloadProgress::is_terminal)
        .collect();
    assert_eq!(terminal, std::slice::from_ref(&done), "исход один — Done");
    assert_eq!(task.snapshot(), done);
    assert_eq!(records(&history).len(), 1, "запись одна");
    assert_eq!(dir_listing(dir.path()), ["Big Buck Bunny.m4a"]);
}

// ───────────── Разные ролики с одним названием (TL-104) ─────────────

/// Другой ролик для тестов TL-104: свой id, название задаёт тест.
const OTHER_URL: &str = "https://www.youtube.com/watch?v=YE7VzlLtp-4";

/// Имена и содержимое файлов папки, по имени: видно и что лежит, и что
/// чужое не переписано.
fn contents(dir: &Path) -> Vec<(String, Vec<u8>)> {
    dir_listing(dir)
        .into_iter()
        .map(|name| {
            let bytes = std::fs::read(dir.join(&name)).expect("файл читается");
            (name, bytes)
        })
        .collect()
}

#[tokio::test]
async fn partial_files_of_another_video_with_the_same_title_are_neither_resumed_nor_taken_nor_removed(
) {
    // TL-104. Первая задача оборвалась с сохранением частичного: видео
    // дописано, звук — `.part`. Вторая — **другой** ролик с тем же названием
    // и теми же форматами в той же папке — завершается. С одной рабочей
    // основой на двоих вторая приняла бы чужое видео за своё («has already
    // been downloaded»), докачала бы чужой `.part`, а подчистка на `Done`
    // удалила бы оба. Потом повтор первой обязан докачать свои частичные
    // файлы и не качать готовое заново.
    //
    // yt-dlp — `Step::Emulate`: имя и папку берёт из argv, а `--continue`
    // ведёт себя как у настоящего (готовый файл — «уже скачан», `.part` —
    // докачивается с прежним содержимым). Проверки — по `-o`, по строкам
    // запуска, по байтам, ушедшим в склейку, и обходом папки.
    const TITLE: &str = "Трейлер";
    let dir = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let env = env_with(
        Some(settings_with(data.path(), None, "{title}", 1)),
        None,
        Some(dir.path()),
    );
    let task_of = |url: &str| {
        let mut req = request(streams(Some("133"), Some("139")));
        req.url = url.to_string();
        req.title = TITLE.to_string();
        new_task(req)
    };
    let first = task_of(URL);
    let second = task_of(OTHER_URL);

    // ── Первая задача: обрыв, частичное сохранено ──
    let interrupted = ScriptedLauncher::new(
        dir.path(),
        vec![Script::failing(1, &connection_lost_stderr()).emulate_partial_only("139")],
    );
    let sink = RecordingSink::new();
    super::run_task(
        &first,
        &interrupted,
        &ScriptedFfmpeg::merging(),
        &sink,
        &env,
    )
    .await;
    let DownloadProgress::Failed { error } = sink.last() else {
        panic!("ожидался отказ первой задачи: {:?}", sink.last());
    };
    assert_eq!(error.kind, DownloadErrorKind::ConnectionLost);
    assert_eq!(error.partial_data, PartialData::Kept);
    assert_eq!(
        interrupted.calls()[0].value_of("-o"),
        Some("aqz-KE-bpKQ.Трейлер.f%(format_id)s.%(ext)s")
    );
    let left_by_first = contents(dir.path());
    assert_eq!(
        left_by_first,
        [
            (
                "aqz-KE-bpKQ.Трейлер.f133.mp4".to_string(),
                b"stream bytes".to_vec()
            ),
            (
                "aqz-KE-bpKQ.Трейлер.f139.m4a.part".to_string(),
                PARTIAL_BYTES.to_vec()
            ),
        ],
        "первая задача оставила готовое видео и оборванный звук"
    );

    // ── Вторая задача: другой ролик, то же название ──
    let other = ScriptedLauncher::new(dir.path(), vec![Script::ok().emulate()]);
    let ffmpeg = ScriptedFfmpeg::merging();
    let sink = RecordingSink::new();
    super::run_task(&second, &other, &ffmpeg, &sink, &env).await;

    let call = &other.calls()[0];
    assert_eq!(
        call.value_of("-o"),
        Some("YE7VzlLtp-4.Трейлер.f%(format_id)s.%(ext)s"),
        "у другого ролика своя рабочая основа"
    );
    assert_eq!(call.lines.len(), 2, "{:?}", call.lines);
    assert!(
        call.lines
            .iter()
            .all(|line| line.starts_with("[download] Destination: ")),
        "оба потока качаются сами: ни «уже скачан», ни докачки чужого — {:?}",
        call.lines
    );
    assert_eq!(
        ffmpeg.last_input_names(),
        (
            "YE7VzlLtp-4.Трейлер.f133.mp4".to_string(),
            "YE7VzlLtp-4.Трейлер.f139.m4a".to_string()
        )
    );
    assert_eq!(
        ffmpeg.last_input_bytes(),
        (b"stream bytes".to_vec(), b"stream bytes".to_vec()),
        "в склейку ушли свои потоки, а не чужой дописанный `.part`"
    );
    assert_eq!(
        sink.last(),
        DownloadProgress::Done {
            file_name: "Трейлер.mp4".to_string(),
            folder_display: crate::types::FolderDisplay::SystemDownloads,
        }
    );
    let mut expected = left_by_first.clone();
    expected.push(("Трейлер.mp4".to_string(), b"merged bytes".to_vec()));
    expected.sort();
    assert_eq!(
        contents(dir.path()),
        expected,
        "частичные файлы первой задачи на месте и не переписаны"
    );

    // ── Повтор первой: докачка своего ──
    first.set_progress(DownloadProgress::Queued);
    let resumed = ScriptedLauncher::new(dir.path(), vec![Script::ok().emulate()]);
    let ffmpeg = ScriptedFfmpeg::merging();
    let sink = RecordingSink::new();
    super::run_task(&first, &resumed, &ffmpeg, &sink, &env).await;

    let call = &resumed.calls()[0];
    assert_eq!(
        call.value_of("-o"),
        interrupted.calls()[0].value_of("-o"),
        "повтор строит те же рабочие имена"
    );
    assert_eq!(call.lines.len(), 2, "{:?}", call.lines);
    assert!(
        call.lines[0].ends_with("aqz-KE-bpKQ.Трейлер.f133.mp4 has already been downloaded"),
        "готовое видео не качается заново: {:?}",
        call.lines
    );
    assert!(
        call.lines[1].starts_with("[download] Destination: ")
            && call.lines[1].ends_with("aqz-KE-bpKQ.Трейлер.f139.m4a"),
        "звук докачивается под своим именем: {:?}",
        call.lines
    );
    assert_eq!(
        ffmpeg.last_input_bytes(),
        (b"stream bytes".to_vec(), PARTIAL_BYTES.to_vec()),
        "в склейку ушли своё готовое видео и свой докачанный `.part`"
    );
    assert_eq!(
        sink.last(),
        DownloadProgress::Done {
            file_name: "Трейлер (2).mp4".to_string(),
            folder_display: crate::types::FolderDisplay::SystemDownloads,
        }
    );
    assert_eq!(
        dir_listing(dir.path()),
        ["Трейлер (2).mp4", "Трейлер.mp4"],
        "обе задачи готовы, частичных файлов не осталось"
    );
}

#[tokio::test]
async fn a_link_without_a_recognised_id_puts_only_its_hash_into_the_partial_name() {
    // TL-104, К-5 для id. `video_id_of` на чужой ссылке отдаёт что угодно из
    // адреса — `../`, `$HOME`, `%(…)s`, обратную косую, пустоту, — а
    // плейсхолдер YouTube (`videoseries`) и чужой хост проходят форму id. В
    // рабочую основу из этого не попадает ни байта: только хеш ссылки.
    // Проверки — `-o` в argv, обход ФС и различимость ссылок.
    const LINKS: &[&str] = &[
        "https://example.com/watch?v=../../escape",
        "https://example.com/watch?v=$HOME",
        "https://example.com/watch?v=~/leading",
        "https://example.com/watch?v=%(title)s",
        "https://example.com/watch?v=a\\b",
        "https://example.com/watch?v=",
        "https://example.com/../../etc/passwd",
        "https://example.com/CON",
        "https://youtube.com.example/watch?v=aqz-KE-bpKQ",
        "https://www.youtube.com/embed/videoseries?list=PL1",
        "https://www.youtube.com/embed/videoseries?list=PL2",
        "https://www.youtube.com/watch?v=aqz-KE-bpK",
    ];
    const TAIL: &str = ".f%(format_id)s.%(ext)s";
    // Корпус не пустой по сути: сырой id хоть одной ссылки увёл бы имя из
    // папки или раскрылся бы в шаблоне, а плейсхолдер форму id проходит.
    assert!(LINKS.iter().any(|link| video_id_of(link).contains('/')));
    assert!(LINKS.iter().any(|link| video_id_of(link).contains('$')));
    assert!(LINKS.iter().any(|link| video_id_of(link) == "videoseries"));

    let mut hashes = BTreeSet::new();
    for link in LINKS {
        assert_eq!(
            canonical_video_id(link),
            None,
            "«{link}» — не разобранная форма"
        );
        let root = tempfile::tempdir().unwrap();
        let dest = root.path().join("dest");
        std::fs::create_dir(&dest).unwrap();
        let mut req = request(streams(None, Some("140")));
        req.url = (*link).to_string();
        let launcher = ScriptedLauncher::new(&dest, vec![Script::ok().emulate()]);
        let sink = RecordingSink::new();

        run_task(
            &new_task(req),
            &launcher,
            &ScriptedFfmpeg::merging(),
            &sink,
            &dest,
        )
        .await;

        let calls = launcher.calls();
        let output = calls[0].value_of("-o").expect("-o обязателен");
        let head = output
            .strip_suffix(TAIL)
            .unwrap_or_else(|| panic!("«{link}»: хвост -o не наш: {output:?}"));
        let id = head
            .strip_suffix(".Big Buck Bunny")
            .unwrap_or_else(|| panic!("«{link}»: -o не из названия и id: {output:?}"));
        assert!(
            id.len() == PARTIAL_ID_HASH_HEX
                && id
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')),
            "«{link}»: в имени не хеш ссылки, а {id:?}"
        );
        assert!(
            head.chars().all(template_safe),
            "«{link}»: в -o символ вне белого списка: {output:?}"
        );
        assert_eq!(calls[0].emulate_failures, Vec::<String>::new(), "«{link}»");
        let DownloadProgress::Done { file_name, .. } = sink.last() else {
            panic!("«{link}»: {:?}", sink.last());
        };
        assert_eq!(
            tree(root.path()),
            ["dest".to_string(), format!("dest/{file_name}")],
            "«{link}»: файл вне папки назначения или подпапка"
        );
        assert!(
            hashes.insert(id.to_string()),
            "«{link}»: хеш совпал с хешем другой ссылки корпуса"
        );
    }
}

#[test]
fn a_recognised_link_puts_the_video_id_itself_into_the_partial_name() {
    for url in [
        "https://www.youtube.com/watch?v=aqz-KE-bpKQ",
        "https://www.youtube.com/watch?v=aqz-KE-bpKQ&list=PL1&index=2",
        "https://youtu.be/aqz-KE-bpKQ?si=Kx1yQ7wSomething",
        "https://www.youtube.com/shorts/aqz-KE-bpKQ",
        "  https://m.youtube.com/shorts/aqz-KE-bpKQ?feature=share  ",
    ] {
        assert_eq!(PartialId::of_url(url).as_str(), "aqz-KE-bpKQ", "«{url}»");
    }
    assert_eq!(PartialId::of_url(OTHER_URL).as_str(), "YE7VzlLtp-4");
}

#[test]
fn the_fallback_id_is_a_fixed_alphabet_hash_of_the_link() {
    // Опубликованный вектор: sha256("abc") = ba7816bf8f01cfea414140de…
    assert_eq!(url_digest("abc"), "ba7816bf8f01cfea");
    assert_eq!(
        url_digest("  abc "),
        "ba7816bf8f01cfea",
        "пробелы по краям — та же ссылка"
    );

    let first = PartialId::of_url("https://example.com/video/1");
    assert_eq!(
        first.as_str(),
        PartialId::of_url("https://example.com/video/1").as_str(),
        "та же ссылка — то же имя (Р-2)"
    );
    assert_ne!(
        first.as_str(),
        PartialId::of_url("https://example.com/video/2").as_str()
    );
    assert_eq!(first.as_str().len(), PARTIAL_ID_HASH_HEX);
}

// ───────────── Название с чужим префиксом (ревью TL-104, S1) ─────────────

#[tokio::test]
async fn a_title_carrying_another_videos_stream_prefix_keeps_its_partials() {
    // Воспроизведение ревью. Ролик `YE7VzlLtp-4` называется
    // «Трейлер.aqz-KE-bpKQ.f139» и оборвался с сохранением частичного. При
    // основе `<название>.<id>` его файлы `Трейлер.aqz-KE-bpKQ.f139.YE7VzlLtp-4.…`
    // лежали под префиксом потока `Трейлер.aqz-KE-bpKQ.f139.` ролика «Трейлер»
    // (`aqz-KE-bpKQ`). Два следствия, и тест держит оба в разных папках, чтобы
    // одно не маскировало другое:
    //
    // - `Keep`: отказ ролика «Трейлер», не создавшего ничего, отвечал «частичное
    //   сохранено» — чужими файлами;
    // - `Done`: подчистка ролика «Трейлер» удаляла чужие файлы.
    //
    // С основой `<id>.<название>` имя другого ролика начинается с его id, а в
    // алфавите id точки нет: под префикс `aqz-KE-bpKQ.` оно не попадает.
    const VICTIM_TITLE: &str = "Трейлер.aqz-KE-bpKQ.f139";
    let task_of = |url: &str, title: &str| {
        let mut req = request(streams(Some("133"), Some("139")));
        req.url = url.to_string();
        req.title = title.to_string();
        new_task(req)
    };

    // Жертва оставляет частичные файлы в папке; возвращает папку, окружение
    // и то, что осталось.
    async fn victim_leaves_partials(
        task: &Arc<DownloadTask>,
    ) -> (TempDir, TempDir, TaskEnv, Vec<(String, Vec<u8>)>) {
        let dir = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let env = env_with(
            Some(settings_with(data.path(), None, "{title}", 1)),
            None,
            Some(dir.path()),
        );
        let launcher = ScriptedLauncher::new(
            dir.path(),
            vec![Script::failing(1, &connection_lost_stderr()).emulate_partial_only("139")],
        );
        let sink = RecordingSink::new();
        super::run_task(task, &launcher, &ScriptedFfmpeg::merging(), &sink, &env).await;
        let DownloadProgress::Failed { error } = sink.last() else {
            panic!("ожидался отказ жертвы: {:?}", sink.last());
        };
        assert_eq!(error.partial_data, PartialData::Kept);
        let left = contents(dir.path());
        (dir, data, env, left)
    }

    // ── Keep: отказ без собственных файлов ──
    let victim = task_of(OTHER_URL, VICTIM_TITLE);
    let (keep_dir, _keep_data, keep_env, keep_left) = victim_leaves_partials(&victim).await;
    let victim_names: Vec<&str> = keep_left.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        victim_names,
        [
            "YE7VzlLtp-4.Трейлер.aqz-KE-bpKQ.f139.f133.mp4",
            "YE7VzlLtp-4.Трейлер.aqz-KE-bpKQ.f139.f139.m4a.part",
        ],
        "жертва оставила своё видео и оборванный звук под основой со своим id"
    );
    let failing = ScriptedLauncher::new(
        keep_dir.path(),
        vec![Script::failing(1, &connection_lost_stderr())],
    );
    let sink = RecordingSink::new();
    super::run_task(
        &task_of(URL, "Трейлер"),
        &failing,
        &ScriptedFfmpeg::merging(),
        &sink,
        &keep_env,
    )
    .await;
    let keep_answer = match sink.last() {
        DownloadProgress::Failed { error } => Some(error.partial_data),
        _ => None,
    };
    let after_keep = contents(keep_dir.path());

    // ── Done: подчистка готовой задачи ──
    let victim = task_of(OTHER_URL, VICTIM_TITLE);
    let (done_dir, _done_data, done_env, done_left) = victim_leaves_partials(&victim).await;
    let ok = ScriptedLauncher::new(done_dir.path(), vec![Script::ok().emulate()]);
    let sink = RecordingSink::new();
    super::run_task(
        &task_of(URL, "Трейлер"),
        &ok,
        &ScriptedFfmpeg::merging(),
        &sink,
        &done_env,
    )
    .await;
    assert!(
        matches!(sink.last(), DownloadProgress::Done { .. }),
        "{:?}",
        sink.last()
    );
    let mut expected_after_done = done_left.clone();
    expected_after_done.push(("Трейлер.mp4".to_string(), b"merged bytes".to_vec()));
    expected_after_done.sort();

    // Одной проверкой: мутация обязана показать оба следствия сразу.
    assert_eq!(
        (keep_answer, after_keep, contents(done_dir.path())),
        (
            Some(PartialData::NothingCreated),
            keep_left,
            expected_after_done
        ),
        "чужие частичные файлы: `Keep` не принимает их за свои, подчистка на `Done` \
         их не трогает"
    );
}

#[test]
fn a_partial_id_never_carries_a_dot_so_its_prefix_is_unambiguous() {
    // Однозначность рабочих имён между роликами держится на алфавите id (doc
    // `PartialId`). Константная проверка стоит у самого алфавита; здесь —
    // что ни одна ссылка не проводит в id ничего вне него, включая точку в
    // позиции id и формы, которые белый список отвергает.
    for byte in 0..=u8::MAX {
        if partial_id_byte(byte) {
            assert!(
                byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_',
                "в алфавите id байт {byte:#04x}"
            );
        }
    }
    const LINKS: &[&str] = &[
        URL,
        OTHER_URL,
        "https://www.youtube.com/watch?v=aqz-KE-bp.Q",
        "https://www.youtube.com/watch?v=.qz-KE-bpKQ",
        "https://youtu.be/aqz-KE-bp.Q",
        "https://www.youtube.com/shorts/aqz.KE-bpKQ",
        "https://www.youtube.com/embed/videoseries?list=PL1",
        "https://example.com/watch?v=a.b",
        "https://example.com/../..",
        "",
    ];
    for link in LINKS {
        let id = PartialId::of_url(link);
        assert!(
            !id.as_str().is_empty() && id.as_str().bytes().all(partial_id_byte),
            "«{link}»: id {:?} вне алфавита",
            id.as_str()
        );
        assert!(!id.as_str().contains('.'), "«{link}»: точка в id");
        let base = partial_base("Трейлер.aqz-KE-bpKQ.f139", link);
        assert_eq!(
            base.split_once('.').map(|(head, _)| head),
            Some(id.as_str()),
            "«{link}»: первая точка основы обязана отделять id целиком: {base}"
        );
    }
    // Точка в позиции id — не id: такая ссылка уходит в запасной вариант.
    assert_eq!(
        PartialId::of_url("https://www.youtube.com/watch?v=aqz-KE-bp.Q")
            .as_str()
            .len(),
        PARTIAL_ID_HASH_HEX
    );
}

#[test]
fn no_generated_title_puts_another_videos_file_under_our_prefixes() {
    // Класс, а не случай S1: названия собираются из кусков, среди которых
    // чужие id, их запасные варианты, точки, `f<формат>` и `tl-merging`. Для
    // каждой пары разных роликов ни одно имя, которое приложение строит
    // второму (файл потока, `.part`, `.ytdl`, файл склейки), не лежит под
    // префиксом первого и не приписывается его потоку.
    use crate::download::merge::{working_file_name, MergeContainer};

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
        fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
            let len = u64::try_from(xs.len()).unwrap();
            &xs[usize::try_from(self.next() % len).unwrap()]
        }
    }

    const URLS: &[&str] = &[
        URL,
        OTHER_URL,
        "https://example.com/a",
        "https://example.com/b",
    ];
    const FORMATS: &[&str] = &["133", "139", "140", "140-drc", "sb", "sb.0"];
    const EXTS: &[&str] = &["mp4", "m4a", "webm", "mhtml"];
    let hashes: Vec<String> = URLS[2..]
        .iter()
        .map(|url| PartialId::of_url(url).as_str().to_string())
        .collect();
    let mut pieces: Vec<String> = [
        "Трейлер",
        "a",
        "😀",
        ".",
        " ",
        "f",
        "140",
        "133",
        "-",
        "_",
        ".f140.",
        ".f133",
        "sb",
        ".0",
        "%",
        "$",
        "/",
        "CON",
        "tl-merging",
        ".tl-merging.",
        "m4a",
        "part",
        "aqz-KE-bpKQ",
        "aqz-KE-bpKQ.",
        ".aqz-KE-bpKQ.f139",
        "YE7VzlLtp-4.",
        ".YE7VzlLtp-4",
    ]
    .map(str::to_string)
    .to_vec();
    for hash in &hashes {
        pieces.push(format!("{hash}."));
        pieces.push(format!(".{hash}.f133."));
    }
    let title = |rng: &mut Rng| {
        let count = 1 + rng.next() % 8;
        (0..count)
            .map(|_| rng.pick(&pieces).clone())
            .collect::<String>()
    };

    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut checks = 0usize;
    for _ in 0..1000 {
        let first_title = title(&mut rng);
        let second_title = match rng.next() % 3 {
            0 => first_title.clone(),
            1 => format!("{first_title}{}", title(&mut rng)),
            _ => title(&mut rng),
        };
        let (first_url, second_url) = (*rng.pick(URLS), *rng.pick(URLS));
        if first_url == second_url {
            continue;
        }
        assert_ne!(
            PartialId::of_url(first_url).as_str(),
            PartialId::of_url(second_url).as_str()
        );
        let first = partial_base(&first_title, first_url);
        let second = partial_base(&second_title, second_url);
        for format in FORMATS {
            for other_format in FORMATS {
                for ext in EXTS {
                    for name in [
                        format!("{second}.f{other_format}.{ext}"),
                        format!("{second}.f{other_format}.{ext}.part"),
                        format!("{second}.f{other_format}.{ext}.ytdl"),
                        working_file_name(&second, MergeContainer::Mp4),
                    ] {
                        checks += 1;
                        assert!(
                            !name.starts_with(&stream_prefix(&first, format))
                                && !is_stream_working_name(
                                    &first,
                                    format,
                                    &UNNAMED_STREAM_EXTENSIONS,
                                    &name
                                )
                                && !is_merge_working_name(&first, &ALL_MERGE_CONTAINERS, &name)
                                && !is_file_of_stream(&first, format, Path::new(&name)),
                            "имя {name:?} другого ролика под префиксом основы {first:?}, \
                             поток {format}"
                        );
                    }
                }
            }
        }
    }
    assert!(checks > 100_000, "корпус выродился: {checks}");
}

// ─────────── Отметка «забран» сверяется с диском (TL-105) ───────────

/// Имя файла потока задачи [`request`] — рабочая основа [`BASE`], формат и
/// расширение.
fn stream_name(format_id: &str, extension: &str) -> String {
    format!("{BASE}.f{format_id}.{extension}")
}

#[tokio::test]
async fn the_same_video_in_another_quality_removes_the_shared_audio_and_the_retry_downloads_it_again(
) {
    // TL-105 (воспроизведение ревью TL-104). A (`133+139`) оборвалась: звук
    // забран целиком и отмечен, видео — `.part`. B — тот же ролик в другом
    // качестве (`134+139`, дубль разрешён E4) — принимает звук A как «уже
    // скачан» (байты те же), доходит до `Done`, и её подчистка удаляет
    // `f139.m4a` (решение ведущего (б): подчистка не различает принятое и
    // скачанное). Повтор A обязан спросить звук у yt-dlp снова, а не отдать
    // склейке файл, которого нет.
    let dir = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let env = env_with(
        Some(settings_with(data.path(), None, "{title}", 1)),
        None,
        Some(dir.path()),
    );
    let a = new_task(request(streams(Some("133"), Some("139"))));
    let b = new_task(request(streams(Some("134"), Some("139"))));
    let video = stream_name("133", "mp4");
    let audio = stream_name("139", "m4a");

    // ── A: звук забран, видео оборвано ──
    let video_part = dir.path().join(format!("{video}.part"));
    let interrupted = ScriptedLauncher::new(
        dir.path(),
        vec![Script::failing(1, &connection_lost_stderr())
            .line(&destination_line(dir.path(), &video))
            .hook(move || std::fs::write(&video_part, PARTIAL_BYTES).unwrap())
            .line(&destination_line(dir.path(), &audio))
            .lines(fixture_progress("video-and-audio.json", "139"))
            .creates(&audio)],
    );
    let sink = RecordingSink::new();
    super::run_task(&a, &interrupted, &ScriptedFfmpeg::merging(), &sink, &env).await;
    let DownloadProgress::Failed { error } = sink.last() else {
        panic!("A обязана оборваться: {:?}", sink.last());
    };
    assert_eq!(error.kind, DownloadErrorKind::ConnectionLost);
    assert_eq!(error.partial_data, PartialData::Kept);
    assert_eq!(
        contents(dir.path()),
        [
            (format!("{video}.part"), PARTIAL_BYTES.to_vec()),
            (audio.clone(), b"stream bytes".to_vec()),
        ]
    );

    // ── B: тот же ролик, другое качество ──
    let other = ScriptedLauncher::new(dir.path(), vec![Script::ok().emulate()]);
    let sink = RecordingSink::new();
    super::run_task(&b, &other, &ScriptedFfmpeg::merging(), &sink, &env).await;
    let call = &other.calls()[0];
    assert_eq!(call.value_of("-f"), Some("134,139"));
    assert!(
        call.lines[1].ends_with(&format!("{audio} has already been downloaded")),
        "B принимает звук A как свой: {:?}",
        call.lines
    );
    assert!(
        matches!(sink.last(), DownloadProgress::Done { .. }),
        "{:?}",
        sink.last()
    );
    assert_eq!(
        dir_listing(dir.path()),
        ["Big Buck Bunny.mp4".to_string(), format!("{video}.part")],
        "подчистка B удалила общий звук, `.part` видео A не тронут"
    );

    // ── Повтор A ──
    a.set_progress(DownloadProgress::Queued);
    let retry = ScriptedLauncher::new(dir.path(), vec![Script::ok().emulate()]);
    let ffmpeg = ScriptedFfmpeg::merging();
    let sink = RecordingSink::new();
    super::run_task(&a, &retry, &ffmpeg, &sink, &env).await;

    assert_eq!(
        retry.formats_asked(),
        ["133,139"],
        "звука на диске нет — повтор спрашивает его снова"
    );
    let call = &retry.calls()[0];
    assert!(
        call.lines[1].starts_with("[download] Destination: ") && call.lines[1].ends_with(&audio),
        "звук качается заново: {:?}",
        call.lines
    );
    assert_eq!(ffmpeg.calls(), 1);
    assert_eq!(ffmpeg.last_input_names(), (video.clone(), audio.clone()));
    assert_eq!(
        ffmpeg.last_input_bytes(),
        (PARTIAL_BYTES.to_vec(), b"stream bytes".to_vec()),
        "в склейку ушли докачанное видео A и заново скачанный звук"
    );
    assert_eq!(
        sink.last(),
        DownloadProgress::Done {
            file_name: "Big Buck Bunny (2).mp4".to_string(),
            folder_display: crate::types::FolderDisplay::SystemDownloads,
        }
    );
    assert_eq!(
        dir_listing(dir.path()),
        ["Big Buck Bunny (2).mp4", "Big Buck Bunny.mp4"]
    );
}

#[tokio::test]
async fn a_stream_marked_done_whose_file_the_user_deleted_is_downloaded_again_on_retry() {
    // TL-105. Оба потока забраны, склейка отказала (`mergeFailed`, частичное
    // сохранено). Пользователь почистил папку — звука больше нет. Повтор
    // обязан скачать звук заново, а не склеивать отсутствующий файл.
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(Some("133"), Some("139"))));
    let sink = RecordingSink::new();
    let video = stream_name("133", "mp4");
    let audio = stream_name("139", "m4a");

    let first = ScriptedLauncher::new(dir.path(), two_stream_scripts(dir.path()));
    run_task(&task, &first, &ScriptedFfmpeg::failing(), &sink, dir.path()).await;
    let DownloadProgress::Failed { error } = sink.last() else {
        panic!("склейка обязана отказать: {:?}", sink.last());
    };
    assert_eq!(error.kind, DownloadErrorKind::MergeFailed);
    assert_eq!(dir_listing(dir.path()), [video.clone(), audio.clone()]);
    let video_bytes = std::fs::read(dir.path().join(&video)).unwrap();

    std::fs::remove_file(dir.path().join(&audio)).unwrap();

    task.set_progress(DownloadProgress::Queued);
    let retry = ScriptedLauncher::new(dir.path(), vec![Script::ok().emulate()]);
    let ffmpeg = ScriptedFfmpeg::merging();
    run_task(&task, &retry, &ffmpeg, &sink, dir.path()).await;

    assert_eq!(retry.formats_asked(), ["139"], "видео на месте, звука нет");
    assert_eq!(ffmpeg.calls(), 1);
    assert_eq!(
        ffmpeg.last_input_bytes(),
        (video_bytes, b"stream bytes".to_vec()),
        "в склейку ушли лежавшее видео и заново скачанный звук"
    );
    assert!(
        matches!(sink.last(), DownloadProgress::Done { .. }),
        "{:?}",
        sink.last()
    );
    assert_eq!(dir_listing(dir.path()), ["Big Buck Bunny.mp4"]);
}

#[tokio::test]
async fn a_stream_file_gone_between_the_download_and_the_merge_is_not_merged_and_the_retry_downloads_it(
) {
    // TL-105, вторая проверка — перед склейкой. Видео забрано прошлым
    // стартом и на его начале лежит на месте; пока идёт докачка звука, файл
    // видео удаляют. Склейка отсутствующего файла не запускается, отказ
    // сохраняет частичное, а следующий повтор спрашивает видео снова.
    let dir = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let env = env_with(
        Some(settings_with(data.path(), None, "{title}", 1)),
        None,
        Some(dir.path()),
    );
    let task = new_task(request(streams(Some("133"), Some("139"))));
    let video = stream_name("133", "mp4");
    let audio = stream_name("139", "m4a");

    let interrupted = ScriptedLauncher::new(
        dir.path(),
        vec![Script::failing(1, &connection_lost_stderr())
            .line(&destination_line(dir.path(), &video))
            .lines(fixture_progress("video-and-audio.json", "133"))
            .creates(&video)
            .line(&destination_line(dir.path(), &audio))],
    );
    let sink = RecordingSink::new();
    super::run_task(&task, &interrupted, &ScriptedFfmpeg::merging(), &sink, &env).await;
    assert!(
        matches!(&sink.last(), DownloadProgress::Failed { error } if error.kind == DownloadErrorKind::ConnectionLost),
        "{:?}",
        sink.last()
    );

    task.set_progress(DownloadProgress::Queued);
    let video_path = dir.path().join(&video);
    let during = ScriptedLauncher::new(
        dir.path(),
        vec![Script::ok()
            .hook(move || std::fs::remove_file(&video_path).unwrap())
            .emulate()],
    );
    let ffmpeg = ScriptedFfmpeg::merging();
    let sink = RecordingSink::new();
    super::run_task(&task, &during, &ffmpeg, &sink, &env).await;

    assert_eq!(during.formats_asked(), ["139"], "на старте видео лежало");
    assert_eq!(
        ffmpeg.calls(),
        0,
        "склейка отсутствующего файла не запускается"
    );
    let DownloadProgress::Failed { error } = sink.last() else {
        panic!("склеивать нечего — отказ: {:?}", sink.last());
    };
    assert_eq!(error.kind, DownloadErrorKind::MergeFailed);
    assert_eq!(error.partial_data, PartialData::Kept);
    assert!(error.retryable);

    task.set_progress(DownloadProgress::Queued);
    let retry = ScriptedLauncher::new(dir.path(), vec![Script::ok().emulate()]);
    let ffmpeg = ScriptedFfmpeg::merging();
    let sink = RecordingSink::new();
    super::run_task(&task, &retry, &ffmpeg, &sink, &env).await;

    assert_eq!(retry.formats_asked(), ["133"], "видео спрашивается снова");
    assert_eq!(ffmpeg.last_input_names(), (video.clone(), audio.clone()));
    assert_eq!(
        ffmpeg.last_input_bytes(),
        (b"stream bytes".to_vec(), b"stream bytes".to_vec()),
        "оба входа склейки лежат на диске"
    );
    assert!(
        matches!(sink.last(), DownloadProgress::Done { .. }),
        "{:?}",
        sink.last()
    );
    assert_eq!(dir_listing(dir.path()), ["Big Buck Bunny.mp4"]);
}

// ─────────── Подчистка по точным рабочим именам (TL-106) ───────────

/// Имена, которые yt-dlp пишет для файла потока `<основа>.f<формат>.<расширение>`,
/// — все формы из замера TL-106 (doc `is_ytdlp_tail`), по одной на форму.
fn measured_working_names(format_id: &str, extension: &str) -> Vec<String> {
    let file = stream_name(format_id, extension);
    vec![
        file.clone(),
        format!("{file}.part"),
        format!("{file}.ytdl"),
        format!("{file}.part-Frag3"),
        format!("{file}.part-Frag17.part"),
        format!("{BASE}.f{format_id}.temp.{extension}"),
    ]
}

#[tokio::test]
async fn a_finished_file_of_another_task_under_a_stream_prefix_survives_the_cleanup_on_done() {
    // TL-106 (воспроизведение исполнителя TL-105). Шаблон `{id}.{title}.f139`
    // кладёт готовый файл склейки под префикс потока `<основа>.f139.`. Первая
    // задача доходит до `Done`; затем тот же ролик качается ещё дважды — в
    // другом качестве (`134+139`) и в том же (`133+139`). Готовый файл первой
    // обязан пережить `Done` обеих байт в байт, а каждая запись истории —
    // ссылаться на существующий файл. Проверка обходом папки.
    let dir = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let history_data = tempfile::tempdir().unwrap();
    let history = history_in(history_data.path());
    let env = env_with(
        Some(settings_with(data.path(), None, "{id}.{title}.f139", 1)),
        Some(Arc::clone(&history)),
        Some(dir.path()),
    );
    let first_name = format!("{BASE}.f139.mp4");
    assert!(
        first_name.starts_with(&stream_prefix(BASE, "139")),
        "сценарий обязан класть готовый файл под префикс потока"
    );

    let launcher = ScriptedLauncher::new(dir.path(), vec![Script::ok().emulate()]);
    let sink = RecordingSink::new();
    super::run_task(
        &new_task(request(streams(Some("133"), Some("139")))),
        &launcher,
        &ScriptedFfmpeg::merging(),
        &sink,
        &env,
    )
    .await;
    assert_eq!(
        sink.last(),
        DownloadProgress::Done {
            file_name: first_name.clone(),
            folder_display: crate::types::FolderDisplay::SystemDownloads,
        }
    );
    assert_eq!(dir_listing(dir.path()), std::slice::from_ref(&first_name));
    // Своё содержимое у готового файла первой: подмена другим файлом под тем
    // же именем была бы видна.
    const FIRST_BYTES: &[u8] = b"finished file of the first task";
    std::fs::write(dir.path().join(&first_name), FIRST_BYTES).unwrap();

    let mut expected = vec![(first_name.clone(), FIRST_BYTES.to_vec())];
    for (video, suffix) in [("134", "(2)"), ("133", "(3)")] {
        let launcher = ScriptedLauncher::new(dir.path(), vec![Script::ok().emulate()]);
        let sink = RecordingSink::new();
        super::run_task(
            &new_task(request(streams(Some(video), Some("139")))),
            &launcher,
            &ScriptedFfmpeg::merging(),
            &sink,
            &env,
        )
        .await;
        assert_eq!(launcher.formats_asked(), [format!("{video},139")]);
        let name = format!("{BASE}.f139 {suffix}.mp4");
        assert_eq!(
            sink.last(),
            DownloadProgress::Done {
                file_name: name.clone(),
                folder_display: crate::types::FolderDisplay::SystemDownloads,
            },
            "видео {video}"
        );
        expected.push((name, b"merged bytes".to_vec()));
        expected.sort();
        assert_eq!(
            contents(dir.path()),
            expected,
            "после `Done` задачи с видео {video} готовый файл первой на месте и не переписан"
        );
    }

    let recorded = records(&history);
    assert_eq!(recorded.len(), 3);
    for record in &recorded {
        let path = record.file_path().expect("у записи есть путь");
        assert!(
            path.is_file(),
            "запись «{}» ссылается на отсутствующий файл",
            record.file_name
        );
    }
    assert!(recorded.iter().any(|record| record.file_name == first_name));
}

#[tokio::test]
async fn a_users_file_under_the_working_stem_survives_the_cleanup_on_cancel_failure_and_done() {
    // TL-106. Имена, лишь начинающиеся с рабочей основы, — не рабочие.
    // Подчистка не удаляет их ни на одном исходе, а `Keep` не выдаёт их за
    // частичное. Среди них — почти-хвосты: номер фрагмента не числом, лишнее
    // после `.part` и `.ytdl`, `.temp.` с другим расширением, почти-имя склейки.
    let foreign: Vec<String> = [
        "f140.заметки.txt",
        "f140.m4a.bak",
        "f140.m4a.part.old",
        "f140.m4a.part-Frag",
        "f140.m4a.part-Frag3x",
        "f140.m4a.part-Frag3.part.part",
        "f140.m4a.ytdl.txt",
        "f140.temp.txt",
        "f140.m4a (2).m4a",
        "f133.заметки.txt",
        "f139.m4a.part-Frag-1",
        "tl-merging.заметки.txt",
        "tl-merging.mp4.part",
    ]
    .iter()
    .map(|tail| format!("{BASE}.{tail}"))
    .collect();
    let mut sorted_foreign = foreign.clone();
    sorted_foreign.sort();

    // (исход, запрос, сценарий, отмена через мс)
    let cancel_after_naming = |dir: &Path| {
        Script::ok()
            .line(&destination_line(dir, &stream_name("140", "m4a")))
            .creates(&format!("{}.part", stream_name("140", "m4a")))
            .hangs()
    };
    let stale_before_naming = |_: &Path| {
        Script::failing(1, &fixtures::outcome("stale-format.json").stderr)
            .creates(&format!("{}.part", stream_name("140", "m4a")))
    };
    let lost_without_files = |_: &Path| Script::failing(1, &connection_lost_stderr());
    let merged = |_: &Path| Script::ok().emulate();
    type Scenario<'a> = (
        &'a str,
        QualityStreams,
        &'a dyn Fn(&Path) -> Script,
        Option<u64>,
    );
    let scenarios: [Scenario<'_>; 4] = [
        (
            "отмена",
            streams(None, Some("140")),
            &cancel_after_naming,
            Some(50),
        ),
        (
            "отказ с удалением",
            streams(None, Some("140")),
            &stale_before_naming,
            None,
        ),
        (
            "отказ с сохранением",
            streams(None, Some("140")),
            &lost_without_files,
            None,
        ),
        ("готово", streams(Some("133"), Some("139")), &merged, None),
    ];

    for (outcome, quality_streams, script, cancel_after) in scenarios {
        let dir = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let env = env_with(
            Some(settings_with(data.path(), None, "{title}", 1)),
            None,
            Some(dir.path()),
        );
        for name in &foreign {
            std::fs::write(dir.path().join(name), b"user bytes").unwrap();
        }
        let task = new_task(request(quality_streams));
        let launcher = ScriptedLauncher::new(dir.path(), vec![script(dir.path())]);
        if let Some(millis) = cancel_after {
            let canceller = Arc::clone(&task);
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(millis)).await;
                canceller.cancel().await;
            });
        }
        let sink = RecordingSink::new();
        super::run_task(&task, &launcher, &ScriptedFfmpeg::merging(), &sink, &env).await;

        let (answer, mut expected) = match sink.last() {
            DownloadProgress::Cancelled { partial_data } => (partial_data, sorted_foreign.clone()),
            DownloadProgress::Failed { error } => (error.partial_data, sorted_foreign.clone()),
            DownloadProgress::Done { file_name, .. } => {
                let mut names = sorted_foreign.clone();
                names.push(file_name);
                (PartialData::Removed, names)
            }
            other => panic!("{outcome}: {other:?}"),
        };
        expected.sort();
        let expected_answer = match outcome {
            "отмена" | "отказ с удалением" | "готово" => {
                PartialData::Removed
            }
            _ => PartialData::NothingCreated,
        };
        assert_eq!(
            answer, expected_answer,
            "{outcome}: чужое не считается частичным"
        );
        assert_eq!(
            dir_listing(dir.path()),
            expected,
            "{outcome}: подчистка тронула имя, которое не рабочее"
        );
        for name in &foreign {
            assert_eq!(
                std::fs::read(dir.path().join(name)).unwrap(),
                b"user bytes",
                "{outcome}: {name} переписан"
            );
        }
    }
}

#[tokio::test]
async fn every_measured_working_name_is_removed_on_cancel_and_on_a_removing_failure() {
    // TL-106. Все формы имён из замера yt-dlp (doc `is_ytdlp_tail`) и файлы
    // склейки удаляются: на отмене, когда оба потока названы (расширение —
    // фактическое, контейнер склейки — точный), и на отказе класса «удалить»
    // до строки `Destination` (расширения — `UNNAMED_STREAM_EXTENSIONS`, все
    // контейнеры склейки). Тест намеренно перечисляет каждую форму: убрать
    // любую из белого списка — и в папке останется её файл.
    // ── Отмена: потоки названы ──
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(Some("133"), Some("139"))));
    let mut script = Script::ok().line(&destination_line(dir.path(), &stream_name("133", "mp4")));
    for name in measured_working_names("133", "mp4") {
        script = script.creates(&name);
    }
    script = script.line(&destination_line(dir.path(), &stream_name("139", "m4a")));
    for name in measured_working_names("139", "m4a") {
        script = script.creates(&name);
    }
    let script = script
        .creates(&working_file_name(BASE, MergeContainer::Mp4))
        .hangs();
    assert_eq!(dir_listing(dir.path()), Vec::<String>::new());
    let launcher = ScriptedLauncher::new(dir.path(), vec![script]);
    let canceller = Arc::clone(&task);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        canceller.cancel().await;
    });
    let sink = RecordingSink::new();
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
        "отмена: осталась рабочая форма, которой нет в белом списке"
    );

    // ── Отказ с удалением: потоки не названы ──
    let dir = tempfile::tempdir().unwrap();
    let task = new_task(request(streams(Some("133"), Some("139"))));
    let mut script = Script::failing(1, &fixtures::outcome("stale-format.json").stderr);
    let mut created = 0;
    for extension in UNNAMED_STREAM_EXTENSIONS {
        for format_id in ["133", "139"] {
            for name in measured_working_names(format_id, extension) {
                script = script.creates(&name);
                created += 1;
            }
        }
    }
    for container in ALL_MERGE_CONTAINERS {
        script = script.creates(&working_file_name(BASE, container));
        created += 1;
    }
    let launcher = ScriptedLauncher::new(dir.path(), vec![script]);
    let sink = RecordingSink::new();
    run_task(
        &task,
        &launcher,
        &ScriptedFfmpeg::merging(),
        &sink,
        dir.path(),
    )
    .await;
    let DownloadProgress::Failed { error } = sink.last() else {
        panic!("устаревший формат — отказ: {:?}", sink.last());
    };
    assert_eq!(error.kind, DownloadErrorKind::StaleFormat);
    assert_eq!(error.partial_data, PartialData::Removed);
    assert_eq!(created, 3 * 2 * 6 + 3, "корпус имён выродился");
    assert_eq!(
        dir_listing(dir.path()),
        Vec::<String>::new(),
        "отказ с удалением: осталась рабочая форма, которой нет в белом списке"
    );
}

#[tokio::test]
async fn a_finished_file_that_took_the_name_of_a_vanished_stream_survives_the_cleanup_on_done() {
    // TL-106, пояс `Cleanup::apply_sparing`. Шаблон `{id}.{title}.f133` у
    // склейки в mp4 даёт финальное имя `<основа>.f133.mp4` — точное рабочее
    // имя видеопотока. Файл потока удаляют, пока идёт склейка (ffmpeg его уже
    // прочёл), финализация занимает освободившееся имя, и подчистка без пощады
    // удалила бы готовый файл, о котором задача только что сказала «готово».
    let dir = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let env = env_with(
        Some(settings_with(data.path(), None, "{id}.{title}.f133", 1)),
        None,
        Some(dir.path()),
    );
    let video = stream_name("133", "mp4");
    let video_path = dir.path().join(&video);
    let ffmpeg =
        ScriptedFfmpeg::merging().during(move || std::fs::remove_file(&video_path).unwrap());
    let launcher = ScriptedLauncher::new(dir.path(), vec![Script::ok().emulate()]);
    let sink = RecordingSink::new();

    super::run_task(
        &new_task(request(streams(Some("133"), Some("139")))),
        &launcher,
        &ffmpeg,
        &sink,
        &env,
    )
    .await;

    assert_eq!(ffmpeg.calls(), 1);
    assert_eq!(
        sink.last(),
        DownloadProgress::Done {
            file_name: video.clone(),
            folder_display: crate::types::FolderDisplay::SystemDownloads,
        },
        "финализация заняла рабочее имя пропавшего потока"
    );
    assert_eq!(
        contents(dir.path()),
        [(video, b"merged bytes".to_vec())],
        "готовый файл на месте, рабочих файлов нет"
    );
}

#[test]
fn every_merge_container_is_a_working_name_of_an_unnamed_merge() {
    // Состав `ALL_MERGE_CONTAINERS` — все варианты `MergeContainer`. `match`
    // без `_`: с новым вариантом тест не соберётся, пока его не впишут сюда и
    // в список.
    for container in [
        MergeContainer::Mp4,
        MergeContainer::Webm,
        MergeContainer::Mkv,
    ] {
        let listed = match container {
            MergeContainer::Mp4 | MergeContainer::Webm | MergeContainer::Mkv => {
                ALL_MERGE_CONTAINERS.contains(&container)
            }
        };
        assert!(listed, "{container:?} нет в ALL_MERGE_CONTAINERS");
        assert!(is_merge_working_name(
            BASE,
            &ALL_MERGE_CONTAINERS,
            &working_file_name(BASE, container)
        ));
    }
    assert_eq!(ALL_MERGE_CONTAINERS.len(), 3);
}
