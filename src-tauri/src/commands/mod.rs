//! Тонкий слой `#[tauri::command]` над доменной логикой ядра (Ф-1 CLAUDE.md).
//!
//! Команды здесь не содержат бизнес-логики — только вызов доменных модулей
//! (`crate::sidecar`, `crate::ytdlp` и далее) и адаптацию их результата под
//! контракт, зеркалируемый в `src/types/` фронтендом.

mod sidecar;
mod ytdlp;

pub use sidecar::check_sidecar;
pub use ytdlp::{prepare_ytdlp, start_ytdlp_preparation, PreparationLock};
