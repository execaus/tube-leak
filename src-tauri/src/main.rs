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
mod queue;
mod sidecar;
mod single_instance;
mod types;
mod ytdlp;

use std::sync::Arc;

use commands::{
    cancel_download, cancel_probe, check_sidecar, check_ytdlp_update, clear_history,
    delete_history_record, dismiss_queue_task, history_page, prepare_ytdlp, preview_name_template,
    probe_url, queue_state, resume_queue, retry_download, roll_back_ytdlp, settings_get,
    settings_set, show_in_folder, start_download, start_ytdlp_preparation,
    start_ytdlp_update_schedule, ytdlp_update_state, PreparationLock,
};
use probe::ProbeSession;
use sidecar::ChildRegistry;
use tauri::{Manager, RunEvent};

/// Файл-снимок очереди в каталоге данных приложения (Ф-9, Р-2).
///
/// `None` — каталог данных не определяется или не создаётся: очередь
/// работает без диска, теряя список между запусками, но приложение
/// запускается. Отказать себе в старте из-за файловой системы значило бы
/// сделать неработоспособным всё ради того, что переживает только выход.
fn queue_store(app: &tauri::AppHandle) -> Option<queue::store::SnapshotStore> {
    let data_dir = match app.path().app_data_dir() {
        Ok(dir) => dir,
        Err(err) => {
            eprintln!(
                "queue: каталог данных не определяется ({err}) — очередь без снимка на диске"
            );
            return None;
        }
    };
    if let Err(err) = std::fs::create_dir_all(&data_dir) {
        eprintln!(
            "queue: каталог данных {} не создаётся ({err}) — очередь без снимка на диске",
            data_dir.display()
        );
        return None;
    }
    Some(queue::store::SnapshotStore::new(&data_dir))
}

fn main() {
    let app = tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            cancel_download,
            cancel_probe,
            check_sidecar,
            check_ytdlp_update,
            clear_history,
            delete_history_record,
            dismiss_queue_task,
            history_page,
            prepare_ytdlp,
            preview_name_template,
            probe_url,
            queue_state,
            resume_queue,
            retry_download,
            roll_back_ytdlp,
            settings_get,
            settings_set,
            show_in_folder,
            start_download,
            ytdlp_update_state
        ])
        .manage(ChildRegistry::new())
        .manage(PreparationLock::new())
        // Отметки «этой установкой yt-dlp прямо сейчас пользуется
        // процесс» (Ф-7 эпика E6): их ставит резолв пути, а уважает
        // уборка контура обновления. Состояние одно на процесс —
        // счётчик, а не флаг, потому что одну установку держат
        // несколько процессов сразу (разбор новой ссылки идёт рядом с
        // загрузкой, и у каждого свой yt-dlp).
        .manage(ytdlp::InUse::new())
        // Один HTTP-клиент контура обновления на всё приложение:
        // соединения и сессии TLS переиспользуются между проверками, а
        // заголовок `User-Agent` задан один раз у агента — забыть его
        // на отдельном запросе нечем (без него API отвечает 403,
        // измерено в TL-55).
        .manage(ytdlp::GithubTransport::new())
        // Состояние контура обновления yt-dlp (E6): статус блока,
        // расписание и запреты в памяти процесса. `Arc` — потому что
        // конвейер живёт отдельной задачей рантайма и переживает
        // возврат из команды, которая его затеяла.
        .manage(Arc::new(ytdlp::UpdateController::new()))
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
            // Очередь загрузок (E4) поднимается первой из всего, что
            // делает `setup`, и поднимается здесь, а не в цепочке
            // `manage` выше: ей нужен каталог данных, а он резолвится
            // только по `AppHandle`. Момент безопасен с обеих сторон —
            // окон ещё нет, то есть команду очереди позвать некому, а
            // замок единственности (TL-20) уже взят, то есть второго
            // писателя снимка не бывает (Р-2, Р-6а).
            app.manage(Arc::new(queue::scheduler::QueueScheduler::new(
                queue_store(app.handle()),
            )));
            // Восстановление — до первого окна и **приостановленным**
            // (Р-3): список прошлого сеанса виден сразу, но ни одного
            // сетевого шага по задачам очереди до явного «Продолжить» не
            // делается (Н-1).
            app.state::<Arc<queue::scheduler::QueueScheduler>>()
                .restore();

            start_ytdlp_preparation(app.handle());
            // Контур самообновления yt-dlp (E6) стартует здесь же и по
            // той же причине, что подготовка: свежесть yt-dlp — условие
            // работоспособности продукта, а не предпочтение
            // пользователя (Р-1). Первое обращение к апстриму —
            // не сразу, а через `STARTUP_CHECK_DELAY`: подготовка
            // первого запуска в этот момент может греть дерево.
            start_ytdlp_update_schedule(app.handle());
            Ok(())
        })
        .build(single_instance::context())
        .unwrap_or_else(|err| {
            eprintln!("error while building tauri application: {err}");
            std::process::exit(1);
        });

    // Один экземпляр приложения (TL-20, решение Р-6 эпика E4). Замок
    // берётся здесь, а не в `setup` ниже, и точка выбрана по порядку в
    // самом Tauri: `build` уже вернул `App`, но окон ещё нет и наш `setup`
    // ещё не выполнялся — они оба ждут `RuntimeRunEvent::Ready` внутри
    // `run`. То есть лишний экземпляр уходит, не мигнув окном и не тронув
    // дерево установок yt-dlp. Подробности и цена решения — в doc-блоке
    // `single_instance`.
    match single_instance::claim_for(app.handle()) {
        // Замок кладётся в состояние приложения, а не в локальную
        // переменную, намеренно: `let _ = …` отпустил бы его немедленно, и
        // единственность пропала бы молча — ровно тот класс правки, что не
        // ловится ни сборкой, ни тестами.
        single_instance::Claim::Sole(lock) => {
            app.manage(lock);
        }
        single_instance::Claim::AlreadyRunning(path) => {
            eprintln!(
                "tube-leak уже запущен: замок {} держит другой процесс. Этот \
                 запуск завершается; окно работающего экземпляра он не \
                 поднимает — это отдельная задача.",
                path.display()
            );
            std::process::exit(0);
        }
        // Fail-open: замок не дали по причине, не связанной с соседом
        // (нет блокировок на сетевом томе, права на каталог). Отказать
        // себе в старте здесь значило бы сделать приложение
        // незапускаемым из-за файловой системы, поэтому запускаемся — но
        // громко.
        single_instance::Claim::Undecided(err) => {
            eprintln!(
                "не удалось проверить, запущен ли уже tube-leak ({err}). Запуск \
                 продолжается; если экземпляр уже работает, они будут мешать \
                 друг другу."
            );
        }
    }

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
