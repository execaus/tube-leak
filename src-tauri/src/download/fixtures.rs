//! Доступ к фикстурам вывода yt-dlp для тестов скачивания (Ф-11).
//!
//! Только для тестов: модуль объявлен под `#[cfg(test)]` и в сборку
//! приложения не попадает. Читать фикстуры из двух модулей сразу (разбор
//! строк и агрегация) двумя копиями кода — гарантированное расхождение,
//! поэтому конверт и его сверка живут в одном месте.
//!
//! Наборов два, и конверты у них разные: `progress/` — stdout идущей
//! загрузки (TL-41), `outcomes/` — код завершения и stderr законченной
//! попытки (TL-43). Формат каждого конверта и правила пересъёмки — в
//! README рядом с ним.

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
    let path = manifest_dir().join("binaries.lock.json");
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("пин {} не читается: {err}", path.display()));
    let pin: Value = serde_json::from_str(&raw)
        .unwrap_or_else(|err| panic!("пин {} — не JSON: {err}", path.display()));

    pin.get("ytDlp")
        .and_then(|yt_dlp| yt_dlp.get("version"))
        .and_then(Value::as_str)
        .expect("в пине объявлена версия yt-dlp")
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

fn outcomes_dir() -> PathBuf {
    fixtures_root().join("ytdlp-download").join("outcomes")
}
