//! Распаковка onedir-архива yt-dlp в каталог данных приложения (TL-12).
//!
//! # Целостность
//!
//! Каждая запись zip несёт CRC32, и он проверяется здесь самим фактом
//! дочитывания записи до конца: `zip::read::ZipFile` сверяет контрольную
//! сумму на EOF и возвращает ошибку при расхождении. Поэтому оборванный
//! или побитый архив не превращается в «успешно распакованное» дерево —
//! распаковка падает на первой же несошедшейся записи, а `.staging-*`
//! удаляется вызывающей стороной ([`super::prepare`]).
//!
//! Отдельной сверки sha256 архива на рантайме нет намеренно: контрольная
//! сумма из пина проверяется при доставке ассета
//! (`scripts/fetch-binaries`), сам архив приезжает внутри бандла и на
//! macOS/Windows покрыт подписью приложения, а CRC32 на каждой записи
//! ловит ту же порчу дешевле — по мере распаковки, без второго прохода по
//! 54 МиБ.
//!
//! # Безопасность путей
//!
//! Имена внутри архива — недоверенный ввод (CLAUDE.md: «Ни один компонент
//! пути не строится из непроверенного ввода»). Используется
//! `ZipFile::enclosed_name`, который отвергает абсолютные пути, `..` и
//! прочие попытки выйти за каталог назначения; запись с таким именем — не
//! повод «почистить» путь и продолжить, а повод отказаться от архива
//! целиком.

use std::fs::{self, File};
use std::io;
use std::path::{Component, Path, PathBuf};

use zip::ZipArchive;

use super::error::PrepareError;

/// Что получилось после распаковки.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unpacked {
    /// Имя исполняемого файла в корне дерева.
    pub executable: String,
    /// Число распакованных обычных файлов.
    pub file_count: u64,
    /// Их суммарный размер.
    pub total_bytes: u64,
}

/// Распаковывает `archive` в `dest` (каталог должен не существовать или
/// быть пустым — вызывающая сторона готовит `.staging-*` сама).
///
/// `on_progress(done_bytes, total_bytes)` вызывается по мере записи —
/// достаточно часто, чтобы прогресс двигался, но не на каждый байт (см.
/// [`PROGRESS_STEP_BYTES`]).
pub fn unpack(
    archive_path: &Path,
    dest: &Path,
    on_progress: &mut dyn FnMut(u64, u64),
) -> Result<Unpacked, PrepareError> {
    let file = File::open(archive_path).map_err(|err| {
        if err.kind() == io::ErrorKind::NotFound {
            PrepareError::ArchiveMissing {
                path: archive_path.display().to_string(),
            }
        } else {
            PrepareError::ArchiveCorrupted {
                reason: format!("{}: {err}", archive_path.display()),
            }
        }
    })?;

    let mut archive = ZipArchive::new(file).map_err(|err| PrepareError::ArchiveCorrupted {
        reason: format!("{}: {err}", archive_path.display()),
    })?;

    // Полный размер дерева нужен до распаковки, чтобы прогресс считался
    // от него, а не «сколько-то из неизвестного». `by_index_raw` не
    // распаковывает данные — только читает заголовок записи.
    let mut total_bytes = 0_u64;
    for index in 0..archive.len() {
        if let Ok(entry) = archive.by_index_raw(index) {
            total_bytes += entry.size();
        }
    }

    fs::create_dir_all(dest).map_err(|err| PrepareError::UnpackFailed {
        reason: format!("{}: {err}", dest.display()),
    })?;

    let mut written = 0_u64;
    let mut reported = 0_u64;
    let mut file_count = 0_u64;
    let mut executables_at_root = Vec::new();

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|err| PrepareError::ArchiveCorrupted {
                reason: format!("запись {index}: {err}"),
            })?;

        let Some(relative) = entry.enclosed_name() else {
            return Err(PrepareError::ArchiveCorrupted {
                reason: format!(
                    "запись {index} ведёт за пределы каталога назначения: {}",
                    entry.name()
                ),
            });
        };

        let target = dest.join(&relative);

        if entry.is_dir() {
            fs::create_dir_all(&target).map_err(|err| PrepareError::UnpackFailed {
                reason: format!("{}: {err}", target.display()),
            })?;
            continue;
        }

        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|err| PrepareError::UnpackFailed {
                reason: format!("{}: {err}", parent.display()),
            })?;
        }

        let mode = entry.unix_mode();
        let mut out = File::create(&target).map_err(|err| PrepareError::UnpackFailed {
            reason: format!("{}: {err}", target.display()),
        })?;

        // `io::copy` дочитывает запись до конца, а `ZipFile` на EOF
        // сверяет CRC32 — вот здесь и ловится повреждённый архив.
        // Прогресс обновляется поэтапно, для этого чтение идёт через
        // счётчик, а не одним вызовом `io::copy`.
        copy_with_progress(&mut entry, &mut out, &mut |chunk| {
            written += chunk;
            if written - reported >= PROGRESS_STEP_BYTES || written == total_bytes {
                reported = written;
                on_progress(written, total_bytes);
            }
        })
        .map_err(|err| classify_copy_error(&target, err))?;

        drop(out);
        apply_mode(&target, mode)?;

        file_count += 1;
        if is_root_executable(&relative, mode) {
            executables_at_root.push(
                relative
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default()
                    .to_string(),
            );
        }
    }

    on_progress(total_bytes, total_bytes);

    if executables_at_root.len() != 1 {
        return Err(PrepareError::LayoutUnexpected {
            reason: format!(
                "в корне архива {} исполняемых файлов вместо одного{}",
                executables_at_root.len(),
                if executables_at_root.is_empty() {
                    String::new()
                } else {
                    format!(": {}", executables_at_root.join(", "))
                }
            ),
        });
    }
    let executable = executables_at_root.remove(0);

    Ok(Unpacked {
        executable,
        file_count,
        total_bytes,
    })
}

/// Насколько должен вырасти объём записанного, чтобы стоило сообщить о
/// прогрессе. Один мегабайт на 124 МиБ дерева — около 124 обновлений на
/// всю распаковку: индикатор движется плавно, а событий не больше, чем
/// успевает отрисовать WebView.
const PROGRESS_STEP_BYTES: u64 = 1024 * 1024;

/// Размер буфера чтения. 64 КиБ — обычный компромисс: заметно больше
/// размера страницы, заметно меньше кеша L2.
const COPY_BUFFER_BYTES: usize = 64 * 1024;

fn copy_with_progress(
    reader: &mut impl io::Read,
    writer: &mut impl io::Write,
    on_chunk: &mut dyn FnMut(u64),
) -> io::Result<()> {
    let mut buffer = vec![0_u8; COPY_BUFFER_BYTES];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok(());
        }
        writer.write_all(&buffer[..read])?;
        on_chunk(read as u64);
    }
}

/// Отличает «архив побит» от «диск не принял».
///
/// `zip` сообщает о несошедшемся CRC32 как об `io::Error` с
/// `ErrorKind::InvalidData` — это порча архива, а не отказ файловой
/// системы, и пользователю про них надо говорить разное: переустановить
/// приложение против освободить место.
fn classify_copy_error(target: &Path, err: io::Error) -> PrepareError {
    if err.kind() == io::ErrorKind::InvalidData {
        PrepareError::ArchiveCorrupted {
            reason: format!("{}: {err}", target.display()),
        }
    } else {
        PrepareError::UnpackFailed {
            reason: format!("{}: {err}", target.display()),
        }
    }
}

/// Переносит права из архива на распакованный файл.
///
/// Без этого исполняемый файл yt-dlp и сотня `.so` внутри дерева
/// оказались бы неисполняемыми, и подготовка «успешно» оставляла бы
/// нерабочую установку. На Windows прав в этом смысле нет, и архив
/// апстрима их не несёт (создан на FAT-совместимой системе — `unix_mode`
/// там `None`), поэтому шаг применим только к Unix.
#[cfg(unix)]
fn apply_mode(path: &Path, mode: Option<u32>) -> Result<(), PrepareError> {
    use std::os::unix::fs::PermissionsExt;

    let Some(mode) = mode else {
        return Ok(());
    };

    fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o777)).map_err(|err| {
        PrepareError::UnpackFailed {
            reason: format!("права {}: {err}", path.display()),
        }
    })
}

#[cfg(not(unix))]
fn apply_mode(_path: &Path, _mode: Option<u32>) -> Result<(), PrepareError> {
    Ok(())
}

/// Является ли запись исполняемым файлом в корне дерева.
///
/// Имя исполняемого файла у каждой платформы своё (`yt-dlp_macos`,
/// `yt-dlp_linux`, `yt-dlp.exe`), поэтому оно не захардкожено, а
/// определяется по свойству: единственный запускаемый файл в корне архива.
/// На Unix признак — бит выполнения из архива; в windows-ассете апстрима
/// прав нет вовсе (архив создан не Unix-системой), и там признак —
/// расширение `.exe`.
fn is_root_executable(relative: &Path, mode: Option<u32>) -> bool {
    let at_root = relative.components().count() == 1
        && matches!(relative.components().next(), Some(Component::Normal(_)));
    if !at_root {
        return false;
    }

    if let Some(mode) = mode {
        if mode & 0o111 != 0 {
            return true;
        }
    }

    relative
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
}

/// Удаляет каталог, если он есть; отсутствие каталога — не ошибка.
pub fn remove_dir_if_exists(dir: &Path) -> Result<(), PrepareError> {
    match fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(PrepareError::UnpackFailed {
            reason: format!("удаление {}: {err}", dir.display()),
        }),
    }
}

/// Переименовывает полностью распакованное дерево на его окончательное
/// место — единственный момент, в который установка «появляется».
///
/// `rename` в пределах одной файловой системы атомарен: каталог
/// назначения либо ещё не существует, либо уже полон. Это и есть ответ на
/// прерванную подготовку — недостроенное дерево физически не может
/// оказаться по рабочему пути.
pub fn promote(staging: &Path, install: &Path) -> Result<(), PrepareError> {
    fs::rename(staging, install).map_err(|err| PrepareError::UnpackFailed {
        reason: format!(
            "перенос {} в {}: {err}",
            staging.display(),
            install.display()
        ),
    })
}

/// Собирает пути `.staging-*` в `root` — мусор от прерванных подготовок.
pub fn stale_staging_dirs(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };

    entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(".staging-"))
        })
        .map(|entry| entry.path())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    /// Собирает zip, похожий по форме на апстримный onedir-ассет:
    /// исполняемый файл в корне плюс каталог `_internal` рядом.
    fn write_onedir_zip(path: &Path, executable_name: &str, executable_mode: u32) {
        let file = File::create(path).expect("fixture archive must be creatable");
        let mut zip = ZipWriter::new(file);
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);

        zip.start_file(executable_name, stored.unix_permissions(executable_mode))
            .expect("start_file");
        zip.write_all(b"#!/bin/sh\necho 2026.08.19\n")
            .expect("write");

        zip.add_directory("_internal/", stored.unix_permissions(0o755))
            .expect("add_directory");

        zip.start_file("_internal/lib.so", stored.unix_permissions(0o755))
            .expect("start_file");
        zip.write_all(b"shared-library-bytes").expect("write");

        zip.start_file("_internal/data.txt", stored.unix_permissions(0o644))
            .expect("start_file");
        zip.write_all(b"plain data").expect("write");

        zip.finish().expect("finish");
    }

    fn unpack_fixture(archive: &Path, dest: &Path) -> Result<Unpacked, PrepareError> {
        unpack(archive, dest, &mut |_, _| {})
    }

    #[test]
    fn unpacks_the_tree_and_finds_the_single_root_executable() {
        let dir = tempdir().expect("tempdir");
        let archive = dir.path().join("yt-dlp.zip");
        write_onedir_zip(&archive, "yt-dlp_macos", 0o755);

        let dest = dir.path().join("staging");
        let unpacked = unpack_fixture(&archive, &dest).expect("распаковка обязана пройти");

        assert_eq!(unpacked.executable, "yt-dlp_macos");
        assert_eq!(unpacked.file_count, 3);
        assert!(dest.join("_internal/lib.so").exists());
        assert!(dest.join("_internal/data.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn preserves_the_execute_bit_of_every_entry_that_had_it() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().expect("tempdir");
        let archive = dir.path().join("yt-dlp.zip");
        write_onedir_zip(&archive, "yt-dlp_macos", 0o755);

        let dest = dir.path().join("staging");
        unpack_fixture(&archive, &dest).expect("распаковка обязана пройти");

        let mode = |relative: &str| {
            fs::metadata(dest.join(relative))
                .expect("файл обязан существовать")
                .permissions()
                .mode()
                & 0o777
        };

        assert_eq!(mode("yt-dlp_macos"), 0o755);
        // Сотня .so внутри дерева тоже должна остаться исполняемой: их
        // грузит dyld, и именно на них уходит время прогрева.
        assert_eq!(mode("_internal/lib.so"), 0o755);
        assert_eq!(mode("_internal/data.txt"), 0o644);
    }

    #[test]
    fn reports_progress_that_ends_at_the_full_size() {
        let dir = tempdir().expect("tempdir");
        let archive = dir.path().join("yt-dlp.zip");
        write_onedir_zip(&archive, "yt-dlp_macos", 0o755);

        let mut updates = Vec::new();
        let dest = dir.path().join("staging");
        let unpacked = unpack(&archive, &dest, &mut |done, total| {
            updates.push((done, total))
        })
        .expect("распаковка обязана пройти");

        let (done, total) = *updates.last().expect("хотя бы одно обновление прогресса");
        assert_eq!(done, total);
        assert_eq!(total, unpacked.total_bytes);
        assert!(
            updates.iter().all(|(done, total)| done <= total),
            "прогресс не может превышать сто процентов: {updates:?}"
        );
    }

    #[test]
    fn refuses_an_archive_whose_entry_escapes_the_destination() {
        let dir = tempdir().expect("tempdir");
        let archive = dir.path().join("evil.zip");
        {
            let file = File::create(&archive).expect("create");
            let mut zip = ZipWriter::new(file);
            let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            zip.start_file("../escaped.txt", stored)
                .expect("start_file");
            zip.write_all(b"nope").expect("write");
            zip.finish().expect("finish");
        }

        let dest = dir.path().join("staging");
        let error = unpack_fixture(&archive, &dest).expect_err("выход за каталог недопустим");

        assert!(
            matches!(error, PrepareError::ArchiveCorrupted { .. }),
            "{error}"
        );
        assert!(
            !dir.path().join("escaped.txt").exists(),
            "файл не должен появиться за пределами каталога назначения"
        );
    }

    #[test]
    fn refuses_a_file_that_is_not_a_zip_at_all() {
        let dir = tempdir().expect("tempdir");
        let archive = dir.path().join("not-a-zip.zip");
        fs::write(&archive, b"this is not an archive").expect("write");

        let error = unpack_fixture(&archive, &dir.path().join("staging"))
            .expect_err("не-zip не может распаковаться");

        assert!(
            matches!(error, PrepareError::ArchiveCorrupted { .. }),
            "{error}"
        );
    }

    #[test]
    fn reports_a_missing_archive_separately_from_a_broken_one() {
        let dir = tempdir().expect("tempdir");

        let error = unpack_fixture(&dir.path().join("absent.zip"), &dir.path().join("staging"))
            .expect_err("отсутствующий архив — ошибка");

        assert!(
            matches!(error, PrepareError::ArchiveMissing { .. }),
            "{error}"
        );
    }

    #[test]
    fn detects_a_corrupted_entry_through_its_crc32() {
        // Правим байты полезной нагрузки, не трогая CRC32 в заголовке —
        // ровно то, что делает битый сектор или оборванная закачка.
        let dir = tempdir().expect("tempdir");
        let archive = dir.path().join("yt-dlp.zip");
        {
            let file = File::create(&archive).expect("create");
            let mut zip = ZipWriter::new(file);
            let deflated =
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            zip.start_file("yt-dlp_macos", deflated.unix_permissions(0o755))
                .expect("start_file");
            zip.write_all(b"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
                .expect("write");
            zip.finish().expect("finish");
        }

        let mut bytes = fs::read(&archive).expect("read");
        let offset = bytes
            .windows(4)
            .position(|window| window == b"AAAA")
            .expect("полезная нагрузка обязана найтись");
        bytes[offset] = b'B';
        fs::write(&archive, &bytes).expect("write");

        let error = unpack_fixture(&archive, &dir.path().join("staging"))
            .expect_err("несошедшийся CRC32 обязан быть замечен");

        assert!(
            matches!(error, PrepareError::ArchiveCorrupted { .. }),
            "{error}"
        );
    }

    #[test]
    fn refuses_an_archive_without_a_single_root_executable() {
        let dir = tempdir().expect("tempdir");
        let archive = dir.path().join("no-exe.zip");
        {
            let file = File::create(&archive).expect("create");
            let mut zip = ZipWriter::new(file);
            let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            zip.start_file("readme.txt", stored.unix_permissions(0o644))
                .expect("start_file");
            zip.write_all(b"no executable here").expect("write");
            zip.finish().expect("finish");
        }

        let error = unpack_fixture(&archive, &dir.path().join("staging"))
            .expect_err("дерево без исполняемого файла непригодно");

        assert!(
            matches!(error, PrepareError::LayoutUnexpected { .. }),
            "{error}"
        );
    }

    #[test]
    fn refuses_an_archive_with_two_root_executables() {
        let dir = tempdir().expect("tempdir");
        let archive = dir.path().join("two-exe.zip");
        {
            let file = File::create(&archive).expect("create");
            let mut zip = ZipWriter::new(file);
            let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            for name in ["yt-dlp_macos", "yt-dlp_other"] {
                zip.start_file(name, stored.unix_permissions(0o755))
                    .expect("start_file");
                zip.write_all(b"#!/bin/sh\n").expect("write");
            }
            zip.finish().expect("finish");
        }

        let error = unpack_fixture(&archive, &dir.path().join("staging"))
            .expect_err("гадать, что запускать, нельзя");

        assert!(
            matches!(error, PrepareError::LayoutUnexpected { .. }),
            "{error}"
        );
    }

    #[test]
    fn recognises_a_windows_style_executable_without_unix_permissions() {
        // Апстримный yt-dlp_win.zip создан не Unix-системой: прав в нём
        // нет вовсе, и единственный признак — расширение.
        assert!(is_root_executable(Path::new("yt-dlp.exe"), None));
        assert!(is_root_executable(Path::new("yt-dlp.EXE"), None));
        assert!(!is_root_executable(Path::new("readme.txt"), None));
        assert!(!is_root_executable(Path::new("_internal/lib.exe"), None));
        assert!(!is_root_executable(
            Path::new("_internal/tool"),
            Some(0o755)
        ));
    }

    #[test]
    fn promotes_staging_to_the_install_directory_atomically() {
        let dir = tempdir().expect("tempdir");
        let staging = dir.path().join(".staging-x");
        let install = dir.path().join("x");
        fs::create_dir_all(staging.join("_internal")).expect("mkdir");
        fs::write(staging.join("marker"), b"1").expect("write");

        promote(&staging, &install).expect("перенос обязан пройти");

        assert!(!staging.exists());
        assert!(install.join("marker").exists());
    }

    #[test]
    fn lists_only_staging_leftovers() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join(".staging-a")).expect("mkdir");
        fs::create_dir_all(dir.path().join(".staging-b")).expect("mkdir");
        fs::create_dir_all(dir.path().join("2026.08.19-abc")).expect("mkdir");
        fs::write(dir.path().join("2026.08.19-abc.json"), b"{}").expect("write");

        let mut found: Vec<_> = stale_staging_dirs(dir.path())
            .into_iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        found.sort();

        assert_eq!(found, vec![".staging-a", ".staging-b"]);
    }

    #[test]
    fn removing_a_missing_directory_is_not_an_error() {
        let dir = tempdir().expect("tempdir");
        remove_dir_if_exists(&dir.path().join("never-existed")).expect("отсутствие — не ошибка");
    }
}
