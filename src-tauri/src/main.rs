//! Точка входа приложения.
//!
//! # Политика содержимого (CSP, TL-28)
//!
//! Сама политика живёт в `app.security.csp` в `tauri.conf.json` — JSON
//! комментариев не держит, поэтому обоснование записано здесь; менять её
//! без чтения этого блока не стоит. Сторож на случай, если всё-таки
//! тронут не читая, — `tests/content_security_policy.rs`.
//!
//! ```text
//! default-src 'self'; img-src 'self' https://i.ytimg.com;
//! style-src 'self'; script-src 'self';
//! connect-src 'self' ipc: http://ipc.localhost;
//! object-src 'none'; base-uri 'self'; form-action 'none';
//! frame-src 'none'; frame-ancestors 'none'
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
//! разрешены: выданное разрешение потом трудно забрать. Выборка не
//! покрывает музыкальный поддомен, трансляции и элементы плейлистов —
//! известное ограничение, принятое сознательно, потому что отказ мягкий:
//! на неразрешённой ссылке `VideoThumbnail` показывает плейсхолдер по
//! `error` и карточку не ломает.
//!
//! `connect-src` перечисляет транспорт IPC, а не «наш сервер»: `invoke`
//! в Tauri 2 — это `fetch` на `ipc://localhost` (macOS, Linux) либо
//! `http://ipc.localhost` (Windows). Без этих источников вызовы команд не
//! отваливаются заметно, а тихо сползают на запасной путь `postMessage` —
//! ровно тот класс молчаливого отказа, что стоил приёмки в TL-24. `'self'`
//! здесь — для полифилла предзагрузки модулей из сборки Vite: он тянет
//! `link[rel=modulepreload]` через `fetch(href)`, то есть запросом на
//! собственный адрес. Сегодня он простаивает вдвойне — чанк один, да и
//! полифилл выходит сразу, если webview умеет `modulepreload` сам, — но
//! первый же ленивый экран это включит.
//!
//! **Ловушка:** `useHttpsScheme` у окна переводит адрес вызовов на
//! `https://ipc.localhost`, которого в политике нет, — и вызовы молча
//! уезжают на тот же запасной путь. Включать схему можно только вместе с
//! правкой `connect-src` (в тесте на это есть отдельное утверждение).
//!
//! `'unsafe-inline'` в `style-src` **сознательно не выдан**, хотя `:style`
//! и `v-show` во фронтенде есть. CSP запрещает разбор атрибута `style`, а
//! Vue ставит стили через CSSOM (`el.style.*`), которого запрет не
//! касается. Держится это не на дисциплине авторов шаблонов: `transformStyle`
//! в `@vue/compiler-dom` превращает даже литеральный `style="…"` в
//! обычную привязку `:style`, то есть атрибут в DOM не доезжает вовсе.
//! Вне этой гарантии остаётся разметка, минующая компилятор шаблонов
//! (`v-html`, руками написанный `index.html`). Проверено на собранном
//! бандле: полоса прогресса экрана подготовки получает ширину по
//! процентам, плейсхолдер превью прячется через `v-show`, нарушений
//! политики за весь холодный старт и разбор ссылки — ноль.
//!
//! Директивы, которых в политике нет, наследуются от `default-src 'self'`
//! (`font-src`, `media-src`, `worker-src`, `manifest-src`) — это проверено
//! по собранным ассетам: внешних шрифтов, медиа и воркеров в сборке нет.
//! Отдельно выписаны те, что **не** наследуются вовсе: `base-uri`,
//! `form-action`, `frame-ancestors`.
//!
//! # Чего политика не касается
//!
//! Сеть ядра ей не подчиняется вообще: обновление yt-dlp (E6) и апдейтер
//! приложения (E7) ходят наружу из Rust, мимо webview, и расширения
//! источников не требуют. События прогресса скачивания — тоже не про неё,
//! это IPC, а не сеть. Расширять `connect-src` «ради апдейтера» не нужно.
//!
//! # Почему правки политики проверяются только на бандле
//!
//! Заголовок `Content-Security-Policy` навешивается в `AppManager::get_asset`
//! — то есть только там, где фронтенд отдаёт сам Tauri своим протоколом.
//! В `tauri dev` webview идёт на адрес Vite напрямую, html через Tauri не
//! проходит, и вешать заголовок просто некуда. Дело **не** в том, что в
//! dev нет политики: `AppManager::csp` и там возвращает ту же самую
//! (`devCsp`, а при его отсутствии — обычную `csp`). Поэтому задание
//! `devCsp` видимости в dev не вернёт — проверять правку нужно на
//! `npm run tauri build` и запуске `.app` со смонтированного `.dmg`.

mod clock;
mod commands;
mod download;
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
