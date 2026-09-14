//! Домен `sidecar`: разрешение путей, запуск и получение версии
//! sidecar-бинарников (yt-dlp, ffmpeg, deno) через механизм `externalBin` Tauri
//! (Ф-5–Ф-8 эпика E1, см. `epics/E1-karkas-i-sidecar.md` в tube-leak-docs).
//!
//! Разделение по подмодулям отражает независимо тестируемые части:
//! - [`resolve`] — путь к бинарнику по имени (`externalBin`/target triple);
//! - [`process`] — запуск с аргументами, захват stdout, таймаут, классификация
//!   ошибок ОС в типизированные [`error::SidecarError`]; там же
//!   построчное чтение stdout идущего процесса ([`run_streaming`]) — для
//!   скачивания, где вывод это ход работы, а не результат (E3);
//! - [`registry`] — реестр PID ещё выполняющихся sidecar-процессов и их
//!   синхронное массовое убийство на выходе из приложения (TL-10);
//! - [`version`] — разбор строки версии `yt-dlp --version` / `ffmpeg -version`
//!   / `deno --version`;
//! - [`deno`] — окружение, с которым запускается deno (TL-110): без
//!   проверки обновлений и с кэшем в каталоге данных приложения; путь и
//!   окружение одним значением и аргументы рантайма для yt-dlp (TL-109).
//!
//! Композиция этих частей в команду `check_sidecar` (проверка всех трёх
//! бинарников параллельно, конвертация в `crate::types::SidecarCheckReport`,
//! конкретные значения таймаута) реализована в `crate::commands::sidecar`
//! (TL-5), которая и является единственным потребителем публичного API
//! этого модуля. [`ChildRegistry`] дополнительно управляется как
//! Tauri-состояние в `main.rs` (TL-10).

mod deno;
mod error;
mod process;
mod registry;
mod resolve;
mod version;

#[cfg(test)]
pub use deno::testing as deno_testing;
pub use deno::{DenoEnv, DenoLaunch, YtDlpJsRuntime};
pub use error::SidecarError;
pub use process::{
    run, run_cancellable, run_streaming, run_with_env, stderr_tail, RunHandle, RunOutput,
    StreamedRun,
};
// Предел обрезки нужен только тем, кто его проверяет: продакшен-код зовёт
// `stderr_tail`, а само число сверяют тесты обоих потребителей — команды
// служебного экрана и классификации разбора.
#[cfg(test)]
pub use process::STDERR_TAIL_MAX_CHARS;
pub use registry::ChildRegistry;
pub use resolve::resolve_sidecar_path;
pub use version::{parse_deno_version, parse_ffmpeg_version, parse_ytdlp_version, SidecarVersion};
