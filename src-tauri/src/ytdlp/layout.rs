//! Раскладка каталога установки yt-dlp в каталоге данных приложения и
//! манифест распакованного дерева (TL-12).
//!
//! # Что где лежит
//!
//! ```text
//! <app_data>/yt-dlp/
//!   2026.08.19-07e54b086530/          дерево ровно как в архиве
//!     yt-dlp_macos                    исполняемый файл (имя зависит от ОС)
//!     _internal/…
//!   2026.08.19-07e54b086530.json      манифест этой установки
//!   .staging-2026.08.19-07e54b086530/ временный каталог распаковки
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

/// Версия формата манифеста. Растёт, когда меняется смысл полей: чужую
/// версию проще переустановить (30 секунд), чем угадывать её семантику.
const MANIFEST_SCHEMA_VERSION: u32 = 1;

/// Идентификатор сборки yt-dlp, вложенной в это приложение.
pub fn bundled_build_id() -> String {
    build_id(BUNDLED_VERSION, BUNDLED_SHA256)
}

fn build_id(version: &str, sha256: &str) -> String {
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

    /// Каталог, в который идёт распаковка до атомарного переименования.
    pub fn staging_dir(&self, build_id: &str) -> PathBuf {
        self.root.join(format!("{STAGING_PREFIX}{build_id}"))
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
        let json = serde_json::to_vec_pretty(self).map_err(|err| PrepareError::UnpackFailed {
            reason: format!("сериализация манифеста: {err}"),
        })?;

        let temp_path = path.with_extension("json.tmp");
        let write = || -> io::Result<()> {
            fs::write(&temp_path, &json)?;
            fs::rename(&temp_path, path)
        };

        write().map_err(|err| {
            let _ = fs::remove_file(&temp_path);
            PrepareError::UnpackFailed {
                reason: format!("запись манифеста {}: {err}", path.display()),
            }
        })
    }

    /// Читает манифест. `None` — файла нет либо он не разбирается: и то,
    /// и другое означает «готовой установки нет», разница между ними
    /// ни на что не влияет.
    pub fn read(path: &Path) -> Option<Self> {
        let raw = fs::read(path).ok()?;
        serde_json::from_slice(&raw).ok()
    }
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

/// Собирает манифест по фактически распакованному дереву.
pub fn manifest_for(dir: &Path, executable: &str) -> Result<Manifest, PrepareError> {
    let (file_count, total_bytes) =
        measure_tree(dir).map_err(|err| PrepareError::UnpackFailed {
            reason: format!("обход {}: {err}", dir.display()),
        })?;

    Ok(Manifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        yt_dlp_version: BUNDLED_VERSION.to_string(),
        archive_sha256: BUNDLED_SHA256.to_string(),
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

        let manifest = manifest_for(&dir, "yt-dlp-test").expect("manifest must be collectable");
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
        let layout = Layout::new(Path::new("/data"));
        assert_ne!(
            layout.staging_dir("build"),
            layout.install_dir("build"),
            "распаковка идёт рядом с целевым каталогом, а не в него"
        );
        assert_eq!(layout.install_dir("build").parent(), Some(layout.root()));
        assert_eq!(layout.staging_dir("build").parent(), Some(layout.root()));
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
