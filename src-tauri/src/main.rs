mod clock;
mod commands;
mod probe;
mod sidecar;
mod types;
mod ytdlp;

use commands::{
    cancel_probe, check_sidecar, prepare_ytdlp, probe_url, start_ytdlp_preparation, PreparationLock,
};
use probe::ProbeSession;
use sidecar::ChildRegistry;
use tauri::{Manager, RunEvent};

fn main() {
    let app = tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            cancel_probe,
            check_sidecar,
            prepare_ytdlp,
            probe_url
        ])
        .manage(ChildRegistry::new())
        .manage(PreparationLock::new())
        // Состояние «идёт разбор ссылки» (E2): одно на приложение —
        // одновременно выполняется не более одного разбора (Ф-8).
        .manage(ProbeSession::new())
        // Подготовка yt-dlp (TL-12) стартует, не дожидаясь фронтенда:
        // приложение без yt-dlp неработоспособно, и готовить его —
        // обязанность ядра. Фронтенд подписывается на события
        // `ytdlp://prepare` и забирает итог командой `prepare_ytdlp`;
        // повторной работы это не создаёт — подготовка идемпотентна и
        // сериализована мьютексом (см. `commands::ytdlp`).
        .setup(|app| {
            start_ytdlp_preparation(app.handle());
            Ok(())
        })
        .build(tauri::generate_context!())
        .unwrap_or_else(|err| {
            eprintln!("error while building tauri application: {err}");
            std::process::exit(1);
        });

    // Tauri/tao завершают процесс приложения через `std::process::exit`
    // сразу после того, как этот колбэк вернёт управление на `RunEvent::Exit`
    // (см. `tauri::App::run`: «the process is exited directly using
    // `std::process::exit`») — эта функция не выполняет Rust `Drop`-глу
    // вообще ни для чего в процессе. Это единственная надёжная синхронная
    // точка, где ещё можно явно убить процессы sidecar-бинарников
    // (`yt-dlp`/`ffmpeg`), запущенные через `sidecar::process::run`, пока
    // ОС не переродила их на PID 1 (TL-10, дефект Ф-2 эпика E1: `Child`
    // с `kill_on_drop(true)` полагается на `Drop`, который в этом пути не
    // срабатывает — см. doc `sidecar::ChildRegistry`).
    app.run(|app_handle, event| {
        if let RunEvent::Exit = event {
            app_handle.state::<ChildRegistry>().kill_all();
        }
    });
}
