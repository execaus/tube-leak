//! Фикстуры подготовки yt-dlp для тестов вне модуля `prepare` (TL-23,
//! TL-21): архив формы onedir и «yt-dlp», который считает свои запуски и
//! умеет зависать до сигнала.
//!
//! Счётчик — не диагностика, а предмет утверждений «запускается один раз»:
//! изнутри процесса приложения увидеть число запусков дочернего бинарника
//! больше нечем. Зависание управляется файлами, а не таймерами: тест
//! дожидается, что процесс действительно висит, и отпускает его сам, так
//! что ни одно утверждение не зависит от того, сколько прошло времени.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::prepare::ProgressSink;
use crate::types::YtDlpPrepareEvent;

/// Имя исполняемого файла в корне фикстурного дерева.
pub const EXECUTABLE: &str = "yt-dlp_fake";

/// Приёмник, которому события подготовки не нужны.
pub struct SilentSink;

impl ProgressSink for SilentSink {
    fn emit(&self, _event: YtDlpPrepareEvent) {}
}

/// Архив формы апстримного onedir-ассета: исполняемый `executable` в
/// корне плюс `_internal`.
pub fn write_onedir_zip(path: &Path, executable: &str, script: &str) {
    let file = File::create(path).expect("create archive");
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);

    zip.start_file(executable, options.unix_permissions(0o755))
        .expect("start_file");
    zip.write_all(script.as_bytes()).expect("write");

    zip.add_directory("_internal/", options.unix_permissions(0o755))
        .expect("add_directory");
    zip.start_file("_internal/lib.so", options.unix_permissions(0o755))
        .expect("start_file");
    zip.write_all(b"pretend-shared-library").expect("write");

    zip.finish().expect("finish");
}

/// Каталог, через который тест управляет фикстурным yt-dlp и считает его
/// запуски.
pub struct Control {
    dir: PathBuf,
}

/// Предел опросов при ожидании зависшего процесса: 400 шагов по 25 мс.
/// Это бюджет рандеву, а не утверждение о длительности: на занятой машине
/// запуск процесса стоит дороже, и ждут здесь только его старта.
const HANG_POLL_LIMIT: u32 = 400;
const HANG_POLL_STEP: std::time::Duration = std::time::Duration::from_millis(25);

impl Control {
    pub fn new(dir: &Path) -> Self {
        fs::create_dir_all(dir).expect("каталог управления");
        Self {
            dir: dir.to_path_buf(),
        }
    }

    /// Текст «yt-dlp»: записывает запуск, при файле `hang` отмечается
    /// `hanging` и ждёт файла `release`, затем печатает `version`.
    pub fn script(&self, version: &str) -> String {
        let dir = self.dir.display();
        format!(
            "#!/bin/sh\n\
             echo run >> \"{dir}/launches\"\n\
             if [ -f \"{dir}/hang\" ]; then\n\
             \x20 : > \"{dir}/hanging\"\n\
             \x20 while [ ! -f \"{dir}/release\" ]; do sleep 0.05; done\n\
             fi\n\
             echo {version}\n"
        )
    }

    /// Сколько раз фикстурный yt-dlp запускался.
    pub fn launches(&self) -> usize {
        fs::read_to_string(self.dir.join("launches"))
            .map(|text| text.lines().count())
            .unwrap_or(0)
    }

    /// Следующие запуски зависают до [`Self::release`].
    pub fn hang(&self) {
        let _ = fs::remove_file(self.dir.join("release"));
        let _ = fs::remove_file(self.dir.join("hanging"));
        fs::write(self.dir.join("hang"), b"").expect("hang");
    }

    /// Отпускает зависшие запуски и последующие не вешает.
    pub fn release(&self) {
        let _ = fs::remove_file(self.dir.join("hang"));
        fs::write(self.dir.join("release"), b"").expect("release");
    }

    /// Ждёт, пока запуск действительно зависнет.
    pub async fn wait_until_hanging(&self) {
        for _ in 0..HANG_POLL_LIMIT {
            if self.dir.join("hanging").exists() {
                return;
            }
            tokio::time::sleep(HANG_POLL_STEP).await;
        }
        panic!("фикстурный yt-dlp так и не запустился в режиме зависания");
    }
}
