//! Точка входа приложения.
//!
//! # Политика содержимого (CSP, TL-28)
//!
//! Сама политика живёт в `app.security.csp` в `tauri.conf.json` — JSON
//! комментариев не держит, поэтому обоснование записано здесь; менять её
//! без чтения этого блока не стоит.
//!
//! ```text
//! default-src 'self'; img-src 'self' https://i.ytimg.com;
//! style-src 'self'; script-src 'self';
//! connect-src ipc: http://ipc.localhost;
//! object-src 'none'; base-uri 'self'; form-action 'none'; frame-src 'none'
//! ```
//!
//! `img-src` — единственное послабление наружу, и оно нужно ради превью
//! ролика в карточке (решение владельца Р-2): картинку грузит сам webview,
//! напрямую с CDN, без промежуточных серверов. Хост ровно один: yt-dlp
//! отдаёт в поле `thumbnail` ссылки только на `i.ytimg.com` — и `/vi/…jpg`,
//! и `/vi_webp/…webp`, и варианты с `?sqp=…` (сверено на живой выдаче:
//! четыре ролика, включая Shorts; во всех — и в поле `thumbnail`, и во
//! всём списке `thumbnails` — хост один). Соседние хосты
//! (`img.youtube.com`, `yt3.ggpht.com`) в выдаче не встречаются и потому не
//! разрешены: выданное разрешение потом трудно забрать. Если такая ссылка
//! однажды придёт, превью просто не отрисуется — `VideoThumbnail`
//! показывает плейсхолдер на `error`, а не ломает карточку.
//!
//! `connect-src` перечисляет транспорт IPC, а не «наш сервер»: `invoke`
//! в Tauri 2 — это `fetch` на `ipc://localhost` (macOS, Linux) либо
//! `http://ipc.localhost` (Windows). Без этих источников вызовы команд не
//! отваливаются заметно, а тихо сползают на запасной путь `postMessage` —
//! ровно тот класс молчаливого отказа, что стоил приёмки в TL-24.
//!
//! `'unsafe-inline'` в `style-src` **сознательно не выдан**, хотя `:style`
//! и `v-show` во фронтенде есть. CSP запрещает разбор атрибута `style`, а
//! Vue ставит стили через CSSOM (`el.style.*`), которого запрет не
//! касается. Проверено на собранном бандле: полоса прогресса экрана
//! подготовки получает ширину по процентам, плейсхолдер превью прячется
//! через `v-show`, нарушений политики за весь холодный старт и разбор
//! ссылки — ноль.
//!
//! Проверять правки политики можно только на бандле: в `tauri dev` CSP не
//! применяется вовсе (`AppManager::csp` отдаёт `None`, пока не собрано с
//! фичей `custom-protocol`).

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
