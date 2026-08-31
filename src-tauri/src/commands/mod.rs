//! Тонкий слой `#[tauri::command]` над доменной логикой ядра (Ф-1 CLAUDE.md).
//!
//! Команды здесь не содержат бизнес-логики — только вызов доменных модулей
//! (`crate::sidecar`, `crate::ytdlp` и далее) и адаптацию их результата под
//! контракт, зеркалируемый в `src/types/` фронтендом.

mod download;
mod probe;
mod queue;
mod sidecar;
mod update;
mod ytdlp;

pub use download::{cancel_download, retry_download, start_download};
pub use probe::{cancel_probe, probe_url};
pub use queue::{dismiss_queue_task, queue_state, resume_queue};
pub use sidecar::check_sidecar;
pub use update::{
    check_ytdlp_update, roll_back_ytdlp, start_ytdlp_update_schedule, ytdlp_update_state,
};
pub use ytdlp::{prepare_ytdlp, start_ytdlp_preparation, PreparationLock};
