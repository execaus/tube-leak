//! Доступ к фикстурам вывода внешних процессов для тестов скачивания
//! (Ф-11).
//!
//! Только для тестов: модуль объявлен под `#[cfg(test)]` и в сборку
//! приложения не попадает. Читать фикстуры из двух модулей сразу (разбор
//! строк и агрегация) двумя копиями кода — гарантированное расхождение,
//! поэтому конверт и его сверка живут в одном месте.
//!
//! Наборов три, и конверты у них разные: `ytdlp-download/progress/` —
//! stdout идущей загрузки (TL-41), `ytdlp-download/outcomes/` — код
//! завершения и stderr законченной попытки yt-dlp (TL-43),
//! `ffmpeg-merge/` — то же для процесса склейки, но инструмент другой
//! (TL-42). Формат каждого конверта и правила пересъёмки — в README рядом
//! с ним.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

/// Все фикстуры прогресса. Порядок — от простого к частному.
///
/// Список объявлен здесь, а не выводится обходом каталога: тест обязан
/// краснеть, если фикстуру удалили, а не тихо проверять на одну меньше.
pub const PROGRESS_FIXTURES: &[&str] = &[
    "video-and-audio.json",
    "audio-only.json",
    "hls-fragmented.json",
    "resume-interrupted.json",
    "resume-continued.json",
    "already-downloaded.json",
];

/// Все фикстуры исходов запуска (TL-43). Порядок — от успеха к частному.
///
/// Отдельный набор, а не продолжение [`PROGRESS_FIXTURES`], потому что в
/// нём другой конверт и другой предмет: там stdout идущей загрузки, здесь
/// код завершения и stderr законченной попытки.
pub const OUTCOME_FIXTURES: &[&str] = &[
    "success-audio-only.json",
    "disk-full.json",
    "stale-format.json",
    "destination-read-only.json",
    "video-unavailable.json",
    "sign-in-required.json",
    "ytdlp-failure-outdated.json",
    "connection-lost-mid-download.json",
    "stalled-killed-by-watchdog.json",
];

/// Все фикстуры одного запуска с селектором через запятую (TL-48).
///
/// Четвёртый набор и четвёртый конверт: stdout, stderr и код **одного и
/// того же** запуска вместе, плюс листинг папки назначения до и после.
/// Сняты вложенным yt-dlp с argv приложения против локального HTTP-сервера,
/// без обращения к YouTube, — как именно, в README рядом с набором.
pub const SINGLE_LAUNCH_FIXTURES: &[&str] = &[
    "video-and-audio.json",
    "video-only.json",
    "video-already-downloaded.json",
    "one-format-missing.json",
    "video-404.json",
    "phrase-in-title.json",
];

/// Чем в снятом выводе заменён абсолютный путь папки назначения съёмки.
pub const DESTINATION_PLACEHOLDER: &str = "<destination>";

/// Снятый запуск: конверт `single-launch/<имя>.json`.
#[derive(Debug)]
pub struct SingleLaunch {
    pub capture: Capture,
    /// Код завершения; `None` — процесс убит сигналом.
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    /// Что лежало в папке назначения до запуска.
    pub listing_before: Vec<String>,
    /// Что осталось после.
    pub listing_after: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SingleLaunchEnvelope {
    #[serde(rename = "_capture")]
    capture: Capture,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    listing_before: Vec<String>,
    listing_after: Vec<String>,
}

/// Снятый запуск одного `-f V,A`.
pub fn single_launch(name: &str) -> SingleLaunch {
    let path = single_launch_dir().join(name);
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("фикстура {} не читается: {err}", path.display()));
    let envelope: SingleLaunchEnvelope = serde_json::from_str(&raw)
        .unwrap_or_else(|err| panic!("фикстура {} — не тот конверт: {err}", path.display()));

    SingleLaunch {
        capture: envelope.capture,
        exit_code: envelope.exit_code,
        stdout: envelope.stdout,
        stderr: envelope.stderr,
        listing_before: envelope.listing_before,
        listing_after: envelope.listing_after,
    }
}

/// Имена файлов, реально лежащих в каталоге `single-launch`, по алфавиту.
pub fn single_launch_files_on_disk() -> Vec<String> {
    json_files_in(&single_launch_dir())
}

/// Обстоятельства съёмки — всё, что нужно, чтобы фикстуру можно было
/// повторить и чтобы её нельзя было тихо оставить протухшей.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capture {
    /// Чем интересна эта фикстура — тот же текст, что в README.
    #[allow(dead_code)]
    pub note: String,
    /// Аргументы запуска; вместо значения шаблона стоит плейсхолдер
    /// `<progressTemplate>` — подставляется из соседнего поля.
    pub argv: Vec<String>,
    pub progress_template: String,
    pub yt_dlp_version: String,
    #[allow(dead_code)]
    pub captured_at: String,
    /// `live` — исход снят как есть; `modelled` — обстоятельства
    /// воспроизведены (см. [`Capture::is_live`]).
    #[serde(default)]
    reality: Option<String>,
}

impl Capture {
    /// Снят ли исход живьём.
    ///
    /// Поле есть только у фикстур исходов (TL-43) и отсутствует у фикстур
    /// прогресса (TL-41), снятых раньше: там все до одной живые, и
    /// приписывать им признак задним числом значило бы править чужой
    /// снятый материал. Отсюда `Option` — и отсюда же тест, требующий,
    /// чтобы у **исхода** признак был обязательно: молчание здесь
    /// означало бы «неизвестно», а неизвестного происхождения фикстур в
    /// этом проекте не бывает.
    #[allow(dead_code)]
    pub fn is_live(&self) -> bool {
        match self.reality.as_deref() {
            Some("live") => true,
            Some("modelled") => false,
            other => panic!(
                "у фикстуры исхода обязано быть поле _capture.reality со \
                 значением live или modelled, а не {other:?}"
            ),
        }
    }
}

#[derive(Debug, Deserialize)]
struct Envelope {
    #[serde(rename = "_capture")]
    capture: Capture,
    stdout: String,
}

/// Снятый исход одной попытки скачивания.
#[derive(Debug)]
pub struct Outcome {
    pub capture: Capture,
    /// Код завершения; `None` — процесс убит сигналом.
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OutcomeEnvelope {
    #[serde(rename = "_capture")]
    capture: Capture,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// Снятый исход запуска: конверт `outcomes/<имя>.json`.
pub fn outcome(name: &str) -> Outcome {
    let path = outcomes_dir().join(name);
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("фикстура {} не читается: {err}", path.display()));
    let envelope: OutcomeEnvelope = serde_json::from_str(&raw)
        .unwrap_or_else(|err| panic!("фикстура {} — не тот конверт: {err}", path.display()));

    Outcome {
        capture: envelope.capture,
        exit_code: envelope.exit_code,
        stdout: envelope.stdout,
        stderr: envelope.stderr,
    }
}

/// Имена файлов, реально лежащих в каталоге фикстур исходов, по алфавиту.
pub fn outcome_files_on_disk() -> Vec<String> {
    json_files_in(&outcomes_dir())
}

fn read(name: &str) -> Envelope {
    let path = progress_dir().join(name);
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("фикстура {} не читается: {err}", path.display()));
    serde_json::from_str(&raw)
        .unwrap_or_else(|err| panic!("фикстура {} — не тот конверт: {err}", path.display()))
}

/// Снятый stdout фикстуры целиком, как пришёл.
pub fn stdout(name: &str) -> String {
    read(name).stdout
}

/// Обстоятельства съёмки фикстуры.
pub fn capture(name: &str) -> Capture {
    read(name).capture
}

/// Метаданные ролика из фикстур разбора E2.
///
/// Нужны ровно затем, чтобы сверить оценку размера, которую отдаёт
/// карточка, с числом байт из строки прогресса того же ролика: два эпика
/// говорят об одной величине, и разойтись им нельзя молча.
pub fn probe_metadata(name: &str) -> Value {
    let path = fixtures_root().join("ytdlp-probe").join(name);
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("фикстура {} не читается: {err}", path.display()));
    serde_json::from_str(&raw)
        .unwrap_or_else(|err| panic!("фикстура {} — не JSON: {err}", path.display()))
}

/// Версия yt-dlp из пина `binaries.lock.json` — та, что реально
/// вкладывается в приложение.
///
/// Тот же приём, что в `probe::quality` и `probe::classify` (TL-30/31):
/// фикстуры заморожены, а yt-dlp нет, поэтому смена пина обязана ломать
/// тест и заставлять переснять фикстуры.
pub fn pinned_yt_dlp_version() -> String {
    pinned_version("ytDlp")
}

/// Версия ffmpeg из того же пина — та, которой сняты фикстуры склейки
/// (TL-42).
///
/// Сторож ровно того же смысла, что и у yt-dlp: формулировки ошибок и
/// коды завершения задаёт апстрим ffmpeg, а фикстуры заморожены. Отличие
/// одно — вложенный бинарник называет себя строкой с суффиксом сборщика
/// (`9.0.1-https://www.martin-riedl.de`), поэтому фикстура хранит и
/// сырую строку тоже, а тест сводит её к этому значению тем же
/// `parse_ffmpeg_version`, которым пользуется служебный экран E1.
pub fn pinned_ffmpeg_version() -> String {
    pinned_version("ffmpeg")
}

fn pinned_version(tool: &str) -> String {
    let path = manifest_dir().join("binaries.lock.json");
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("пин {} не читается: {err}", path.display()));
    let pin: Value = serde_json::from_str(&raw)
        .unwrap_or_else(|err| panic!("пин {} — не JSON: {err}", path.display()));

    pin.get(tool)
        .and_then(|entry| entry.get("version"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("в пине объявлена версия {tool}"))
        .to_owned()
}

/// Имена файлов, реально лежащих в каталоге фикстур прогресса, по
/// алфавиту.
///
/// Нужны сторожу «список в коде и каталог не разошлись»: фикстура, о
/// которой [`PROGRESS_FIXTURES`] не знает, выглядела бы покрытым случаем,
/// не будучи им.
pub fn files_on_disk() -> Vec<String> {
    json_files_in(&progress_dir())
}

/// Все фикстуры склейки (TL-42). Порядок — успехи, потом отказы.
///
/// Третий набор с третьим конвертом, и снят он **другим инструментом**:
/// здесь вывод ffmpeg, а не yt-dlp. Общего с исходами скачивания у него
/// только форма («что осталось от процесса»), поэтому и каталог свой —
/// `tests/fixtures/ffmpeg-merge/`.
pub const MERGE_FIXTURES: &[&str] = &[
    "success-mp4-h264-aac.json",
    "success-webm-vp9-opus.json",
    "success-mkv-mixed.json",
    "success-awkward-filename.json",
    "container-refuses-codec.json",
    "input-missing.json",
    "input-truncated.json",
    "output-not-writable.json",
    "disk-full-mid-merge.json",
    "killed-mid-merge.json",
];

/// Обстоятельства съёмки фикстуры склейки.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeCapture {
    /// Чем интересна эта фикстура — тот же текст, что в README.
    #[allow(dead_code)]
    pub note: String,
    /// Аргументы запуска с плейсхолдерами `<video>`, `<audio>`,
    /// `<output>` вместо путей (сами пути вели во временный каталог
    /// съёмки и ничего не значат).
    pub argv: Vec<String>,
    /// Версия из пина, которой снята фикстура.
    pub ffmpeg_version: String,
    /// Первая строка `ffmpeg -version` вложенного бинарника — как есть,
    /// с суффиксом сборщика.
    pub ffmpeg_version_raw: String,
    #[allow(dead_code)]
    pub captured_at: String,
    reality: String,
}

impl MergeCapture {
    /// Снят ли исход живьём. Обязательное поле — по той же причине, что и
    /// у исходов скачивания: неизвестного происхождения фикстур в этом
    /// проекте не бывает.
    pub fn is_live(&self) -> bool {
        match self.reality.as_str() {
            "live" => true,
            "modelled" => false,
            other => panic!(
                "у фикстуры склейки обязано быть поле _capture.reality со \
                 значением live или modelled, а не {other:?}"
            ),
        }
    }
}

/// Снятый исход одного запуска ffmpeg.
#[derive(Debug)]
pub struct MergeOutcome {
    pub capture: MergeCapture,
    /// Код завершения; `None` — процесс убит сигналом.
    pub exit_code: Option<i32>,
    /// Сколько байт занимал файл результата после завершения процесса;
    /// `None` — файла не появилось вовсе. Это факт о диске, ради которого
    /// и существует подчистка в [`crate::download::merge`].
    pub output_left_bytes: Option<u64>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MergeEnvelope {
    #[serde(rename = "_capture")]
    capture: MergeCapture,
    exit_code: Option<i32>,
    output_left_bytes: Option<u64>,
    stdout: String,
    stderr: String,
}

/// Снятый исход склейки: конверт `ffmpeg-merge/<имя>.json`.
pub fn merge_outcome(name: &str) -> MergeOutcome {
    let path = merge_dir().join(name);
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("фикстура {} не читается: {err}", path.display()));
    let envelope: MergeEnvelope = serde_json::from_str(&raw)
        .unwrap_or_else(|err| panic!("фикстура {} — не тот конверт: {err}", path.display()));

    MergeOutcome {
        capture: envelope.capture,
        exit_code: envelope.exit_code,
        output_left_bytes: envelope.output_left_bytes,
        stdout: envelope.stdout,
        stderr: envelope.stderr,
    }
}

/// Имена файлов, реально лежащих в каталоге фикстур склейки, по алфавиту.
pub fn merge_files_on_disk() -> Vec<String> {
    json_files_in(&merge_dir())
}

fn json_files_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("каталог {} не читается: {err}", dir.display()))
        .map(|entry| {
            entry
                .unwrap_or_else(|err| panic!("запись каталога {}: {err}", dir.display()))
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.ends_with(".json"))
        .collect();
    names.sort();
    names
}

fn manifest_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn fixtures_root() -> PathBuf {
    manifest_dir().join("tests").join("fixtures")
}

fn progress_dir() -> PathBuf {
    fixtures_root().join("ytdlp-download").join("progress")
}

fn single_launch_dir() -> PathBuf {
    fixtures_root().join("ytdlp-download").join("single-launch")
}

fn outcomes_dir() -> PathBuf {
    fixtures_root().join("ytdlp-download").join("outcomes")
}

fn merge_dir() -> PathBuf {
    fixtures_root().join("ffmpeg-merge")
}
