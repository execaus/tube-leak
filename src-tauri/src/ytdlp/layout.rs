//! Раскладка каталога установки yt-dlp в каталоге данных приложения и
//! манифест распакованного дерева (TL-12).
//!
//! # Что где лежит
//!
//! ```text
//! <app_data>/yt-dlp/
//!   2026.08.19-07e54b086530/            дерево ровно как в архиве
//!     yt-dlp_macos                      исполняемый файл (имя зависит от ОС)
//!     _internal/…
//!   2026.08.19-07e54b086530.json        манифест этой установки
//!   2026.08.19-07e54b086530.repair.json счётчик безуспешных переустановок
//!   .staging-2026.08.19-07e54b086530-1f3c…/ временный каталог распаковки
//! ```
//!
//! Имя каталога — версия плюс первые 12 символов sha256 архива. Версии
//! одной достаточно, чтобы различать релизы, но не чтобы различать
//! «тот же релиз, другой ассет»; sha256 закрывает и это, и случай, когда
//! приложение обновилось, а версия yt-dlp формально не менялась.
//!
//! # Почему манифест снаружи каталога, а не внутри
//!
//! Каталог установки — точная копия содержимого архива, и это свойство
//! используется при проверке: число файлов и суммарный размер сверяются с
//! записанными. Файл манифеста внутри каталога ломал бы сверку сам собой.
//!
//! # Почему у `.staging-*` случайный суффикс
//!
//! Имя каталога распаковки не выводится из build id: [`Layout::create_staging_dir`]
//! добавляет к нему случайный суффикс и создаёт каталог `create_dir`, то
//! есть отказом, если путь уже занят. Предсказуемое имя означало бы, что
//! между уборкой остатков и созданием каталога любой процесс того же
//! пользователя может подложить туда символическую ссылку, и распаковка
//! ушла бы по ней наружу. Суффикс стоит ноль, а попутно снимает вопрос
//! столкновения двух экземпляров приложения на одном каталоге данных
//! (полное решение межпроцессной гонки — отдельная задача).
//!
//! # Что гарантирует манифест
//!
//! Каталог установки появляется только через `rename` уже полностью
//! распакованного `.staging-*` (см. [`super::unpack`]), а манифест
//! пишется после этого — тоже через временный файл и `rename`. Отсюда
//! инвариант, на который опирается [`validate`]: **есть манифест —
//! значит распаковка дошла до конца**. Прерванная на середине подготовка
//! (питание, `kill`) оставляет либо `.staging-*` без каталога установки,
//! либо каталог установки без манифеста; ни то, ни другое не считается
//! готовым к работе, и подготовка выполняется заново.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::error::PrepareError;

/// Версия yt-dlp из `binaries.lock.json` — пробрасывается `build.rs`,
/// чтобы пин оставался единственным местом, где она записана.
pub const BUNDLED_VERSION: &str = env!("TUBE_LEAK_YTDLP_VERSION");

/// sha256 вложенного onedir-архива, оттуда же.
pub const BUNDLED_SHA256: &str = env!("TUBE_LEAK_YTDLP_SHA256");

/// Путь вложенного архива внутри ресурсов бандла. Должен совпадать с
/// `bundle.resources` в `tauri.conf.json` и с `RESOURCE_RELATIVE_PATH` в
/// `build.rs`.
pub const BUNDLED_ARCHIVE_RESOURCE: &str = "resources/yt-dlp.zip";

/// Подкаталог каталога данных приложения, в котором живут установки yt-dlp.
const INSTALL_ROOT_DIR: &str = "yt-dlp";

/// Префикс каталога, в который идёт распаковка до атомарного переименования.
const STAGING_PREFIX: &str = ".staging-";

/// Префикс файла, в который принимается скачиваемый архив обновления
/// (TL-56), до того как он проверен суммой и распакован.
///
/// Отдельный префикс, а не [`STAGING_PREFIX`], и не косметика: уборка
/// остатков различает их по типу объекта — `.staging-*` это каталог,
/// `.download-*` файл, — и складывать их под одно имя значило бы удалять
/// одно кодом для другого. Общее у них ровно одно свойство, ради которого
/// префикс вообще есть: имя начинается с точки, то есть остаток
/// прерванной работы никогда не выглядит установкой (та адресуется
/// `<версия>-<sha12>`).
const DOWNLOAD_PREFIX: &str = ".download-";

/// Суффикс файла со счётчиком безуспешных переустановок.
const REPAIR_SUFFIX: &str = ".repair.json";

/// Суффикс файла со счётчиком безуспешных попыток **установить
/// обновление** этого build id (TL-56).
///
/// Форма записи та же ([`RepairLog`]), а файл — другой, и это не
/// дублирование. [`REPAIR_SUFFIX`] принадлежит подготовке первого запуска:
/// по нему `super::prepare` решает, стоит ли ещё раз переустанавливать
/// вложенный в бандл архив, и после [`super::prepare`]`::MAX_REPAIR_ATTEMPTS`
/// отказывает сразу. У С-10 build id обновления и build id пина совпадают
/// (тот же архив, тот же путь установки) — пиши контур свои неудачи в тот
/// же файл, неудача скачивания обновления считалась бы неудачей починки
/// и однажды запретила бы подготовку рабочей установки на старте. Разные
/// файлы делают это невыразимым.
#[allow(dead_code)] // Читает и пишет журнал `fetch`, зовёт его TL-58.
const UPDATE_ATTEMPT_SUFFIX: &str = ".update.json";

/// Сколько имён каталога распаковки пробовать, прежде чем сдаться.
const STAGING_NAME_ATTEMPTS: u8 = 4;

/// Непредсказуемый суффикс имени каталога распаковки.
///
/// `RandomState` берёт ключи SipHash из системного источника случайности
/// один раз на процесс и меняет их от экземпляра к экземпляру, поэтому
/// значения различаются и между запусками, и между вызовами. Криптостойкость
/// здесь не требуется — требуется невозможность подготовить путь заранее;
/// ради этого тащить в зависимости генератор случайных чисел незачем.
fn random_suffix() -> u64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};

    let mut hasher = RandomState::new().build_hasher();
    hasher.write_usize(std::process::id() as usize);
    hasher.write_u64(crate::clock::now_unix_nanos());
    hasher.finish()
}

/// Версия формата манифеста. Растёт, когда меняется смысл полей: чужую
/// версию проще переустановить (30 секунд), чем угадывать её семантику.
const MANIFEST_SCHEMA_VERSION: u32 = 1;

/// Идентификатор сборки yt-dlp, вложенной в это приложение.
pub fn bundled_build_id() -> String {
    build_id(BUNDLED_VERSION, BUNDLED_SHA256)
}

/// Идентификатор сборки yt-dlp: версия апстрима плюс начало суммы её
/// архива.
///
/// Публичный, а не внутренний, с TL-56: обновление знает версию и сумму
/// кандидата из метаданных релиза до того, как хоть байт скачан, и
/// адресует ими и каталог установки, и журнал попыток. Считать этот
/// идентификатор вторым способом рядом значило бы завести второе место,
/// где раскладка каталога данных описана.
pub fn build_id(version: &str, sha256: &str) -> String {
    let short = sha256.get(..12).unwrap_or(sha256);
    format!("{version}-{short}")
}

/// Пути установки yt-dlp внутри каталога данных приложения.
#[derive(Debug, Clone)]
pub struct Layout {
    root: PathBuf,
}

impl Layout {
    /// `data_dir` — каталог данных приложения
    /// (`AppHandle::path().app_data_dir()`).
    pub fn new(data_dir: &Path) -> Self {
        Self {
            root: data_dir.join(INSTALL_ROOT_DIR),
        }
    }

    /// Каталог, внутри которого живут все установки.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Каталог конкретной установки.
    pub fn install_dir(&self, build_id: &str) -> PathBuf {
        self.root.join(build_id)
    }

    /// Файл манифеста конкретной установки.
    pub fn manifest_path(&self, build_id: &str) -> PathBuf {
        self.root.join(format!("{build_id}.json"))
    }

    /// Файл со счётчиком безуспешных переустановок этой установки.
    ///
    /// Лежит рядом с манифестом, а не внутри каталога установки: каталог
    /// сносится на каждой переустановке, а счётчик обязан её пережить —
    /// в этом весь его смысл.
    pub fn repair_path(&self, build_id: &str) -> PathBuf {
        self.root.join(format!("{build_id}{REPAIR_SUFFIX}"))
    }

    /// Файл со счётчиком безуспешных попыток установить обновление до
    /// этого build id (TL-56).
    ///
    /// Живёт там же и по той же причине, что [`Self::repair_path`], но
    /// отдельным файлом — см. [`UPDATE_ATTEMPT_SUFFIX`].
    #[allow(dead_code)] // Потребитель — `super::fetch`, его — TL-58.
    pub fn update_attempt_path(&self, build_id: &str) -> PathBuf {
        self.root.join(format!("{build_id}{UPDATE_ATTEMPT_SUFFIX}"))
    }

    /// Создаёт файл, в который пойдёт приём скачиваемого архива, под
    /// именем, которое нельзя предугадать, и отдаёт его вместе с путём.
    ///
    /// `create_new` (то есть `O_EXCL`), а не «открыть или создать», ровно
    /// по тому же доводу, что `create_dir` в [`Self::create_staging_dir`]:
    /// занятый путь — не «уже готово», а чужой объект под именем, которое
    /// мы считали своим, и лить в него шестьдесят мегабайт из сети нельзя.
    /// Вместе со случайным суффиксом это и есть гарантия «файл приёма
    /// создали мы».
    #[allow(dead_code)] // Потребитель — `super::fetch`, его — TL-58.
    pub fn create_download_file(
        &self,
        build_id: &str,
    ) -> Result<(PathBuf, fs::File), PrepareError> {
        let mut last_error = None;

        for _ in 0..STAGING_NAME_ATTEMPTS {
            let candidate = self.root.join(format!(
                "{DOWNLOAD_PREFIX}{build_id}-{suffix:016x}",
                suffix = random_suffix()
            ));
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&candidate)
            {
                Ok(file) => return Ok((candidate, file)),
                Err(err) => last_error = Some((candidate, err)),
            }
        }

        let (path, err) = last_error.expect("цикл выполняется хотя бы раз");
        Err(PrepareError::UnpackFailed {
            reason: format!("файл приёма архива {}: {err}", path.display()),
        })
    }

    /// Собирает пути `.download-*` — недокачанные архивы от прерванных
    /// обновлений (С-7).
    ///
    /// Сканирование живёт здесь, а не рядом с уборкой `.staging-*` в
    /// [`super::unpack`], потому что здесь объявлен префикс: единственный
    /// способ не разойтись с именем, которым файл создаётся, — не
    /// повторять литерал в другом модуле.
    pub fn stale_downloads(&self) -> Vec<PathBuf> {
        let Ok(entries) = fs::read_dir(&self.root) else {
            return Vec::new();
        };

        entries
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with(DOWNLOAD_PREFIX))
            })
            .map(|entry| entry.path())
            .collect()
    }

    /// Создаёт каталог, в который пойдёт распаковка, под именем, которое
    /// нельзя предугадать, и возвращает его путь.
    ///
    /// Создание — `create_dir`, а не `create_dir_all`: занятый путь здесь
    /// не «уже готово», а чужой объект по имени, которое мы считали
    /// своим, и распаковываться в него нельзя. Вместе со случайным
    /// суффиксом это и есть гарантия «каталог распаковки создали мы»
    /// (см. doc модуля).
    pub fn create_staging_dir(&self, build_id: &str) -> Result<PathBuf, PrepareError> {
        let mut last_error = None;

        // Несколько попыток — не про вероятность столкнуться суффиксами
        // (она исчезающе мала), а про то, чтобы единичный отказ не ронял
        // подготовку целиком.
        for _ in 0..STAGING_NAME_ATTEMPTS {
            let candidate = self.root.join(format!(
                "{STAGING_PREFIX}{build_id}-{suffix:016x}",
                suffix = random_suffix()
            ));
            match fs::create_dir(&candidate) {
                Ok(()) => return Ok(candidate),
                Err(err) => last_error = Some((candidate, err)),
            }
        }

        let (path, err) = last_error.expect("цикл выполняется хотя бы раз");
        Err(PrepareError::UnpackFailed {
            reason: format!("каталог распаковки {}: {err}", path.display()),
        })
    }

    /// Создаёт корневой каталог установок.
    pub fn create_root(&self) -> Result<(), PrepareError> {
        fs::create_dir_all(&self.root).map_err(|err| PrepareError::DataDirUnavailable {
            reason: format!("{}: {err}", self.root.display()),
        })
    }
}

/// Манифест распакованного дерева.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema_version: u32,
    /// Версия yt-dlp по пину, из которого распаковано дерево.
    pub yt_dlp_version: String,
    /// sha256 архива, из которого распаковано дерево.
    pub archive_sha256: String,
    /// Имя исполняемого файла в корне дерева — не захардкожено, потому что
    /// у каждой платформы оно своё (`yt-dlp_macos`, `yt-dlp_linux`,
    /// `yt-dlp.exe`), а угадывать по маске значит однажды угадать неверно.
    pub executable: String,
    /// Сколько обычных файлов должно быть в дереве.
    pub file_count: u64,
    /// Суммарный размер этих файлов в байтах.
    pub total_bytes: u64,
    /// Когда дерево было распаковано (RFC 3339, UTC) — только для
    /// диагностики.
    pub unpacked_at: String,
}

impl Manifest {
    /// Пишет манифест атомарно: сначала временный файл рядом, затем
    /// `rename`. Половина JSON под именем манифеста означала бы «установка
    /// готова» при неполном дереве.
    pub fn write_atomic(&self, path: &Path) -> Result<(), PrepareError> {
        write_json_atomic(path, self, "манифеста")
    }

    /// Читает манифест. `None` — файла нет либо он не разбирается: и то,
    /// и другое означает «готовой установки нет», разница между ними
    /// ни на что не влияет.
    pub fn read(path: &Path) -> Option<Self> {
        read_json(path)
    }
}

/// Память о том, что переустановка этой установки уже выполнялась и не
/// помогла.
///
/// Нужна ровно против одного сценария: дерево сходится с манифестом, но
/// принципиально не запускается. Без счётчика приложение переустанавливало
/// бы 124 МиБ и грело их ~35 с **при каждом запуске**, каждый раз с тем же
/// исходом. Файл лежит рядом с манифестом ([`Layout::repair_path`]),
/// поэтому переживает и переустановку дерева, и перезапуск приложения;
/// удачный запуск его убирает ([`RepairLog::clear`]).
///
/// Запись адресована конкретному build id: другая версия yt-dlp или другой
/// архив — другое имя файла и, значит, чистая история. Это же и первый
/// выход из терминального состояния, второй — остывание по времени
/// (см. `super::prepare`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairLog {
    pub schema_version: u32,
    /// Сколько переустановок уже выполнено и не помогло.
    pub attempts: u32,
    /// Время последней попытки, секунды с эпохи Unix. В числе, а не
    /// строкой, потому что по нему считается остывание.
    pub last_attempt_unix: u64,
    /// То же время в RFC 3339 — только для чтения человеком в логе.
    pub last_attempt_at: String,
    /// Чем установка была признана нерабочей в последний раз.
    pub last_reason: String,
}

impl RepairLog {
    /// Пустая история: так выглядит установка, которую ещё не чинили.
    pub fn empty() -> Self {
        Self {
            schema_version: MANIFEST_SCHEMA_VERSION,
            attempts: 0,
            last_attempt_unix: 0,
            last_attempt_at: String::new(),
            last_reason: String::new(),
        }
    }

    /// Читает историю. Нечитаемая или чужая по версии формата запись —
    /// то же самое, что её отсутствие: счётчик не то состояние, ради
    /// которого стоит отказывать в работе.
    pub fn read(path: &Path) -> Self {
        read_json(path)
            .filter(|log: &Self| log.schema_version == MANIFEST_SCHEMA_VERSION)
            .unwrap_or_else(Self::empty)
    }

    /// Возвращает историю с ещё одной учтённой попыткой.
    pub fn with_attempt(&self, reason: &str, now_unix: u64) -> Self {
        Self {
            schema_version: MANIFEST_SCHEMA_VERSION,
            attempts: self.attempts.saturating_add(1),
            last_attempt_unix: now_unix,
            last_attempt_at: crate::clock::now_iso8601(),
            last_reason: reason.to_string(),
        }
    }

    /// Записывает историю атомарно.
    pub fn write_atomic(&self, path: &Path) -> Result<(), PrepareError> {
        write_json_atomic(path, self, "истории починки")
    }

    /// Убирает историю: установка запустилась, помнить нечего.
    ///
    /// Отсутствие файла — не ошибка и обычное дело: на исправной машине
    /// этот файл не появляется никогда.
    pub fn clear(path: &Path) {
        let _ = fs::remove_file(path);
    }
}

/// Пишет JSON через временный файл рядом и `rename`.
///
/// Половина JSON под рабочим именем — это либо «установка готова» при
/// неполном дереве (манифест), либо нечитаемый счётчик (история починки);
/// первое опаснее, но чинится одинаково.
fn write_json_atomic<T: Serialize>(path: &Path, value: &T, what: &str) -> Result<(), PrepareError> {
    let json = serde_json::to_vec_pretty(value).map_err(|err| PrepareError::UnpackFailed {
        reason: format!("сериализация {what}: {err}"),
    })?;

    let temp_path = path.with_extension("json.tmp");
    let write = || -> io::Result<()> {
        fs::write(&temp_path, &json)?;
        fs::rename(&temp_path, path)
    };

    write().map_err(|err| {
        let _ = fs::remove_file(&temp_path);
        PrepareError::UnpackFailed {
            reason: format!("запись {what} {}: {err}", path.display()),
        }
    })
}

/// Читает JSON. `None` — файла нет либо он не разбирается.
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let raw = fs::read(path).ok()?;
    serde_json::from_slice(&raw).ok()
}

/// Готовая к запуску установка yt-dlp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// Каталог дерева.
    pub dir: PathBuf,
    /// Полный путь к исполняемому файлу внутри дерева.
    pub executable: PathBuf,
    /// Версия yt-dlp по манифесту (не по запуску — запуском её получает
    /// [`super::prepare`]).
    pub version: String,
}

/// Проверяет, что установка `build_id` на месте и пригодна к запуску.
///
/// Проверка намеренно дешёвая — она выполняется на **каждом** запуске
/// приложения, а бюджет служебного экрана десять секунд на всё (Н-2).
/// Поэтому сверяются число файлов и суммарный размер (обход 134 записей —
/// единицы миллисекунд), а не хеши содержимого: пересчёт sha256 по 124 МиБ
/// стоил бы столько же, сколько сам запуск yt-dlp, и удваивал бы время
/// нормального старта ради сценария, которого при штатной работе не
/// бывает. Порча уже распакованного дерева ловится этой сверкой в
/// подавляющем большинстве случаев (удаление, обрезание, частичная
/// перезапись меняют либо число файлов, либо суммарный размер), а порча
/// при самой распаковке — CRC32 каждой записи архива (см.
/// [`super::unpack`]).
pub fn validate(layout: &Layout, build_id: &str) -> Result<Installed, InvalidInstall> {
    let manifest_path = layout.manifest_path(build_id);
    let Some(manifest) = Manifest::read(&manifest_path) else {
        return Err(InvalidInstall::NoManifest);
    };

    if manifest.schema_version != MANIFEST_SCHEMA_VERSION {
        return Err(InvalidInstall::ForeignSchema {
            found: manifest.schema_version,
        });
    }

    let dir = layout.install_dir(build_id);
    let executable = dir.join(&manifest.executable);
    if !is_executable_file(&executable) {
        return Err(InvalidInstall::ExecutableMissing);
    }

    let (file_count, total_bytes) =
        measure_tree(&dir).map_err(|_| InvalidInstall::TreeUnreadable)?;
    if file_count != manifest.file_count || total_bytes != manifest.total_bytes {
        return Err(InvalidInstall::TreeMismatch {
            expected_files: manifest.file_count,
            found_files: file_count,
            expected_bytes: manifest.total_bytes,
            found_bytes: total_bytes,
        });
    }

    Ok(Installed {
        dir,
        executable,
        version: manifest.yt_dlp_version,
    })
}

/// Почему установка не годится. Все варианты ведут к одному и тому же
/// действию — распаковать заново, — но различаются в логе: по ним видно,
/// подготовку прервали или дерево испортили после неё.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidInstall {
    /// Манифеста нет или он не разбирается: подготовка не доходила до
    /// конца.
    NoManifest,
    /// Манифест от другой версии формата (приложение откатили назад).
    ForeignSchema { found: u32 },
    /// Исполняемого файла нет либо он потерял бит выполнения.
    ExecutableMissing,
    /// Дерево не обходится (нет прав, отсутствует каталог).
    TreeUnreadable,
    /// Дерево на месте, но не совпадает с манифестом.
    TreeMismatch {
        expected_files: u64,
        found_files: u64,
        expected_bytes: u64,
        found_bytes: u64,
    },
}

impl std::fmt::Display for InvalidInstall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoManifest => write!(f, "манифест отсутствует или не читается"),
            Self::ForeignSchema { found } => write!(
                f,
                "манифест версии формата {found}, поддерживается {MANIFEST_SCHEMA_VERSION}"
            ),
            Self::ExecutableMissing => write!(f, "исполняемый файл отсутствует или не исполняем"),
            Self::TreeUnreadable => write!(f, "каталог установки не обходится"),
            Self::TreeMismatch {
                expected_files,
                found_files,
                expected_bytes,
                found_bytes,
            } => write!(
                f,
                "дерево не совпадает с манифестом: файлов {found_files} вместо {expected_files}, \
                 байт {found_bytes} вместо {expected_bytes}"
            ),
        }
    }
}

/// Чем архив себя называет: версия yt-dlp и sha256 самого архива.
///
/// До TL-56 обеих величин было ровно по одной на приложение — пин бандла,
/// — и [`manifest_for`] брал их из констант. С обновлением их стало две
/// пары (пин и кандидат из релиза апстрима), и константа в манифесте
/// означала бы, что установка версии Y записывает в свой манифест версию
/// X: `validate` этого не заметила бы (она сверяет только схему, число
/// файлов и объём), а служебный экран и уборка читают версию именно
/// оттуда.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchiveIdentity<'a> {
    /// Версия yt-dlp в апстримном формате `YYYY.MM.DD`.
    pub version: &'a str,
    /// sha256 архива, шестнадцатеричная.
    pub sha256: &'a str,
}

impl ArchiveIdentity<'static> {
    /// То, что вложено в бандл этого приложения.
    pub fn bundled() -> Self {
        Self {
            version: BUNDLED_VERSION,
            sha256: BUNDLED_SHA256,
        }
    }
}

impl ArchiveIdentity<'_> {
    /// Идентификатор сборки, которым адресуются каталог установки, её
    /// манифест и журналы рядом.
    pub fn build_id(&self) -> String {
        build_id(self.version, self.sha256)
    }
}

/// Собирает манифест по фактически распакованному дереву.
pub fn manifest_for(
    dir: &Path,
    executable: &str,
    identity: ArchiveIdentity<'_>,
) -> Result<Manifest, PrepareError> {
    let (file_count, total_bytes) =
        measure_tree(dir).map_err(|err| PrepareError::UnpackFailed {
            reason: format!("обход {}: {err}", dir.display()),
        })?;

    Ok(Manifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        yt_dlp_version: identity.version.to_string(),
        archive_sha256: identity.sha256.to_string(),
        executable: executable.to_string(),
        file_count,
        total_bytes,
        unpacked_at: crate::clock::now_iso8601(),
    })
}

/// Число обычных файлов и их суммарный размер в дереве.
fn measure_tree(dir: &Path) -> io::Result<(u64, u64)> {
    let mut files = 0_u64;
    let mut bytes = 0_u64;
    let mut stack = vec![dir.to_path_buf()];

    while let Some(current) = stack.pop() {
        for entry in fs::read_dir(&current)? {
            let entry = entry?;
            // `DirEntry::metadata` на Unix не идёт по символической
            // ссылке (в отличие от `fs::metadata`) — ссылка считается
            // собой, а не тем, куда указывает. Это то, что нужно: иначе
            // содержимое цели попало бы в сумму дважды.
            let metadata = entry.metadata()?;
            if metadata.is_dir() {
                stack.push(entry.path());
            } else {
                files += 1;
                bytes += metadata.len();
            }
        }
    }

    Ok((files, bytes))
}

/// Существует ли путь и можно ли его запускать.
///
/// На Unix проверяется бит выполнения: без него `spawn` вернёт `EACCES`,
/// и подготовка, формально «успешная», оставит нерабочее дерево. На
/// Windows битов доступа в этом смысле нет — достаточно того, что файл
/// существует.
fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_file(path: &Path, contents: &[u8], executable: bool) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("failed to create fixture dir");
        }
        fs::write(path, contents).expect("failed to write fixture file");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = if executable { 0o755 } else { 0o644 };
            fs::set_permissions(path, fs::Permissions::from_mode(mode))
                .expect("failed to chmod fixture file");
        }
        #[cfg(not(unix))]
        let _ = executable;
    }

    /// Раскладывает правдоподобную установку и возвращает её build id.
    fn install_fixture(data_dir: &Path) -> (Layout, String) {
        let layout = Layout::new(data_dir);
        layout.create_root().expect("root must be creatable");
        let build_id = bundled_build_id();
        let dir = layout.install_dir(&build_id);

        write_file(&dir.join("yt-dlp-test"), b"#!/bin/sh\n", true);
        write_file(&dir.join("_internal/lib.so"), b"0123456789", false);

        let manifest = manifest_for(&dir, "yt-dlp-test", ArchiveIdentity::bundled())
            .expect("manifest must be collectable");
        manifest
            .write_atomic(&layout.manifest_path(&build_id))
            .expect("manifest must be writable");

        (layout, build_id)
    }

    #[test]
    fn build_id_combines_version_with_a_short_archive_hash() {
        assert_eq!(
            build_id("2026.08.19", "07e54b0865303c864006925913bce2604f8"),
            "2026.08.19-07e54b086530"
        );
    }

    #[test]
    fn build_id_tolerates_a_hash_shorter_than_the_prefix_it_takes() {
        assert_eq!(build_id("1.0", "abc"), "1.0-abc");
    }

    #[test]
    fn staging_and_install_directories_never_collide() {
        let dir = tempdir().expect("tempdir");
        let layout = Layout::new(dir.path());
        layout.create_root().expect("root must be creatable");

        let staging = layout
            .create_staging_dir("build")
            .expect("staging must be creatable");

        assert_ne!(
            staging,
            layout.install_dir("build"),
            "распаковка идёт рядом с целевым каталогом, а не в него"
        );
        assert_eq!(layout.install_dir("build").parent(), Some(layout.root()));
        assert_eq!(staging.parent(), Some(layout.root()));
        assert!(staging.is_dir(), "каталог распаковки обязан быть создан");
    }

    #[test]
    fn every_staging_directory_gets_a_name_that_cannot_be_guessed() {
        // Предсказуемое имя позволяло бы подложить по нему символическую
        // ссылку между уборкой остатков и созданием каталога.
        let dir = tempdir().expect("tempdir");
        let layout = Layout::new(dir.path());
        layout.create_root().expect("root must be creatable");

        let names: std::collections::HashSet<String> = (0..8)
            .map(|_| {
                layout
                    .create_staging_dir("build")
                    .expect("staging must be creatable")
                    .file_name()
                    .expect("staging path has a file name")
                    .to_string_lossy()
                    .to_string()
            })
            .collect();

        assert_eq!(names.len(), 8, "имена обязаны различаться: {names:?}");
        assert!(
            names
                .iter()
                .all(|name| name.starts_with(&format!("{STAGING_PREFIX}build-"))),
            "уборка остатков ищет их по префиксу: {names:?}"
        );
    }

    #[test]
    fn refuses_a_staging_path_that_is_already_taken() {
        // Единственный способ, которым занятый путь может встретиться, —
        // кто-то его подложил; распаковываться в него нельзя.
        let dir = tempdir().expect("tempdir");
        let layout = Layout::new(dir.path());

        let error = layout
            .create_staging_dir("build")
            .expect_err("без корневого каталога создавать негде");

        assert!(
            matches!(error, PrepareError::UnpackFailed { .. }),
            "{error}"
        );
    }

    #[test]
    fn the_repair_log_lives_next_to_the_manifest_and_not_inside_the_tree() {
        let layout = Layout::new(Path::new("/data"));
        let repair = layout.repair_path("build");

        assert_eq!(repair.parent(), Some(layout.root()));
        assert!(
            !repair.starts_with(layout.install_dir("build")),
            "иначе переустановка стирала бы память о своих же неудачах"
        );
        assert_ne!(repair, layout.manifest_path("build"));
    }

    #[test]
    fn the_repair_log_counts_attempts_and_survives_a_round_trip() {
        let dir = tempdir().expect("tempdir");
        let (layout, build_id) = install_fixture(dir.path());
        let path = layout.repair_path(&build_id);

        assert_eq!(
            RepairLog::read(&path),
            RepairLog::empty(),
            "у нетронутой установки истории починки нет"
        );

        let first = RepairLog::empty().with_attempt("не запускается", 1_000);
        first.write_atomic(&path).expect("write");
        assert_eq!(RepairLog::read(&path), first);
        assert_eq!(first.attempts, 1);

        let second = RepairLog::read(&path).with_attempt("снова не запускается", 2_000);
        second.write_atomic(&path).expect("write");
        let read_back = RepairLog::read(&path);
        assert_eq!(read_back.attempts, 2);
        assert_eq!(read_back.last_attempt_unix, 2_000);
        assert_eq!(read_back.last_reason, "снова не запускается");

        RepairLog::clear(&path);
        assert_eq!(RepairLog::read(&path).attempts, 0);
        assert!(!path.exists());
    }

    #[test]
    fn an_unreadable_repair_log_is_treated_as_no_history() {
        // Счётчик — страховка, а не условие работы: испорченный файл не
        // повод отказывать в подготовке.
        let dir = tempdir().expect("tempdir");
        let (layout, build_id) = install_fixture(dir.path());
        let path = layout.repair_path(&build_id);
        fs::write(&path, b"{ not json").expect("write");

        assert_eq!(RepairLog::read(&path), RepairLog::empty());
    }

    #[test]
    fn a_repair_log_from_another_schema_version_is_ignored() {
        let dir = tempdir().expect("tempdir");
        let (layout, build_id) = install_fixture(dir.path());
        let path = layout.repair_path(&build_id);
        let mut log = RepairLog::empty().with_attempt("причина", 1_000);
        log.schema_version = MANIFEST_SCHEMA_VERSION + 1;
        log.write_atomic(&path).expect("write");

        assert_eq!(RepairLog::read(&path).attempts, 0);
    }

    #[test]
    fn writing_the_repair_log_leaves_no_temporary_file_behind() {
        let dir = tempdir().expect("tempdir");
        let (layout, build_id) = install_fixture(dir.path());
        let path = layout.repair_path(&build_id);
        RepairLog::empty()
            .with_attempt("причина", 1_000)
            .write_atomic(&path)
            .expect("write");

        let leftovers: Vec<_> = fs::read_dir(layout.root())
            .expect("root must be readable")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.ends_with(".tmp"))
            .collect();

        assert!(
            leftovers.is_empty(),
            "остались временные файлы: {leftovers:?}"
        );
    }

    #[test]
    fn validates_a_complete_installation() {
        let dir = tempdir().expect("tempdir");
        let (layout, build_id) = install_fixture(dir.path());

        let installed = validate(&layout, &build_id).expect("свежая установка обязана проходить");

        assert_eq!(installed.dir, layout.install_dir(&build_id));
        assert_eq!(installed.executable.file_name().unwrap(), "yt-dlp-test");
        assert_eq!(installed.version, BUNDLED_VERSION);
    }

    #[test]
    fn rejects_an_installation_whose_manifest_never_got_written() {
        // Ровно то, что остаётся после подготовки, прерванной между
        // переименованием дерева и записью манифеста.
        let dir = tempdir().expect("tempdir");
        let (layout, build_id) = install_fixture(dir.path());
        fs::remove_file(layout.manifest_path(&build_id)).expect("manifest must be removable");

        assert_eq!(
            validate(&layout, &build_id),
            Err(InvalidInstall::NoManifest)
        );
    }

    #[test]
    fn rejects_an_installation_that_lost_a_file_after_unpacking() {
        let dir = tempdir().expect("tempdir");
        let (layout, build_id) = install_fixture(dir.path());
        fs::remove_file(layout.install_dir(&build_id).join("_internal/lib.so"))
            .expect("file must be removable");

        let error = validate(&layout, &build_id).expect_err("неполное дерево не годится");
        assert!(
            matches!(error, InvalidInstall::TreeMismatch { .. }),
            "{error}"
        );
    }

    #[test]
    fn rejects_an_installation_whose_file_was_truncated_in_place() {
        // Число файлов то же — ловится только суммарным размером.
        let dir = tempdir().expect("tempdir");
        let (layout, build_id) = install_fixture(dir.path());
        write_file(
            &layout.install_dir(&build_id).join("_internal/lib.so"),
            b"01",
            false,
        );

        let error = validate(&layout, &build_id).expect_err("обрезанный файл не годится");
        assert!(
            matches!(error, InvalidInstall::TreeMismatch { .. }),
            "{error}"
        );
    }

    #[test]
    fn rejects_an_installation_without_its_executable() {
        let dir = tempdir().expect("tempdir");
        let (layout, build_id) = install_fixture(dir.path());
        fs::remove_file(layout.install_dir(&build_id).join("yt-dlp-test"))
            .expect("executable must be removable");

        assert_eq!(
            validate(&layout, &build_id),
            Err(InvalidInstall::ExecutableMissing)
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_an_installation_whose_executable_lost_the_execute_bit() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().expect("tempdir");
        let (layout, build_id) = install_fixture(dir.path());
        fs::set_permissions(
            layout.install_dir(&build_id).join("yt-dlp-test"),
            fs::Permissions::from_mode(0o644),
        )
        .expect("chmod must succeed");

        assert_eq!(
            validate(&layout, &build_id),
            Err(InvalidInstall::ExecutableMissing)
        );
    }

    #[test]
    fn rejects_a_manifest_written_by_another_schema_version() {
        let dir = tempdir().expect("tempdir");
        let (layout, build_id) = install_fixture(dir.path());
        let path = layout.manifest_path(&build_id);
        let mut manifest = Manifest::read(&path).expect("manifest must be readable");
        manifest.schema_version = MANIFEST_SCHEMA_VERSION + 1;
        manifest
            .write_atomic(&path)
            .expect("manifest must be writable");

        assert_eq!(
            validate(&layout, &build_id),
            Err(InvalidInstall::ForeignSchema {
                found: MANIFEST_SCHEMA_VERSION + 1
            })
        );
    }

    #[test]
    fn manifest_round_trips_through_disk() {
        let dir = tempdir().expect("tempdir");
        let (layout, build_id) = install_fixture(dir.path());
        let path = layout.manifest_path(&build_id);

        let manifest = Manifest::read(&path).expect("manifest must be readable");
        assert_eq!(manifest.schema_version, MANIFEST_SCHEMA_VERSION);
        assert_eq!(manifest.archive_sha256, BUNDLED_SHA256);
        assert_eq!(manifest.file_count, 2);
        assert_eq!(manifest.total_bytes, "#!/bin/sh\n".len() as u64 + 10);
        assert!(manifest.unpacked_at.ends_with('Z'));
    }

    #[test]
    fn writing_a_manifest_leaves_no_temporary_file_behind() {
        let dir = tempdir().expect("tempdir");
        let (layout, build_id) = install_fixture(dir.path());

        let leftovers: Vec<_> = fs::read_dir(layout.root())
            .expect("root must be readable")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.ends_with(".tmp"))
            .collect();

        assert!(
            leftovers.is_empty(),
            "остались временные файлы: {leftovers:?}"
        );
        assert!(layout.manifest_path(&build_id).exists());
    }
}
