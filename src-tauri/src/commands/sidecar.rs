//! `#[tauri::command]` для служебного экрана: проверка sidecar-бинарников.

use crate::types::{self, SidecarCheckReport};

/// Возвращает результат проверки обоих sidecar-бинарников (yt-dlp, ffmpeg).
///
/// Реализация — фиксированные stub-данные: реальный запуск процессов,
/// разбор версии и обработка ошибок ОС появятся в TL-4/TL-5 (домен
/// `crate::sidecar`). Здесь важна только форма контракта, которую зеркалит
/// TS-сторона (TL-2).
#[tauri::command]
pub fn check_sidecar() -> SidecarCheckReport {
    types::stub_report()
}
