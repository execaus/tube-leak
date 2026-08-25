//! Домен `sidecar`: разрешение путей, запуск и получение версии
//! sidecar-бинарников (yt-dlp, ffmpeg) через механизм `externalBin` Tauri
//! (Ф-5–Ф-8 эпика E1, см. `epics/E1-karkas-i-sidecar.md` в tube-leak-docs).
//!
//! Разделение по подмодулям отражает независимо тестируемые части:
//! - [`resolve`] — путь к бинарнику по имени (`externalBin`/target triple);
//! - [`process`] — запуск с аргументами, захват stdout, таймаут, классификация
//!   ошибок ОС в типизированные [`error::SidecarError`];
//! - [`version`] — разбор строки версии `yt-dlp --version` / `ffmpeg -version`.
//!
//! Композиция этих частей в команду `check_sidecar` (проверка обоих
//! бинарников параллельно, конвертация в `crate::types::SidecarCheckReport`,
//! конкретные значения таймаута) — задача TL-5, здесь не реализуется.
//!
//! Публичное API этого модуля пока не вызывается из `commands::sidecar`
//! (там всё ещё stub из TL-3) — допуски ниже временные, до TL-5, по
//! аналогии с `#[allow(dead_code)]` на контрактных enum в `types.rs`.
#![allow(dead_code, unused_imports)]

mod error;
mod process;
mod resolve;
mod version;

pub use error::SidecarError;
pub use process::run;
pub use resolve::resolve_sidecar_path;
pub use version::{parse_ffmpeg_version, parse_ytdlp_version};
