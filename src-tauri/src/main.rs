mod types;

use types::SidecarCheckReport;

/// Возвращает результат проверки обоих sidecar-бинарников (yt-dlp, ffmpeg).
///
/// Реализация — фиксированные stub-данные: реальный запуск процессов,
/// разбор версии и обработка ошибок ОС появятся в TL-4/TL-5. Здесь важна
/// только форма контракта, которую зеркалит TS-сторона (TL-2).
#[tauri::command]
fn check_sidecar() -> SidecarCheckReport {
    types::stub_report()
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![check_sidecar])
        .run(tauri::generate_context!())
        .unwrap_or_else(|err| {
            eprintln!("error while running tauri application: {err}");
            std::process::exit(1);
        });
}
