//! `#[tauri::command]` разбора ссылки на ролик (Ф-9 эпика E2) — TL-32.
//!
//! Тонкий слой над [`crate::probe`]: резолвит путь к готовому yt-dlp,
//! собирает продакшен-запускатель и конвертирует доменную
//! [`crate::probe::ProbeFailure`] в контрактный [`crate::types::ProbeError`].
//! Логики разбора здесь нет — порядок вызовов, аргументы запуска, таймаут
//! и вытеснение живут в домене.
//!
//! Экран зовёт две команды и обе — отсюда: [`probe_url`] («начать или
//! заменить разбор») и [`cancel_probe`] («отменить, не начиная нового» —
//! очистка поля ссылки не сопровождается новым адресом, дизайн E2).
//! Новых подписок на события задача не добавляет: разбор отвечает
//! промисом `invoke`, а не событиями, — сторож `frontend_acl` (урок
//! TL-24) в этой задаче неприменим по этой причине, а не по недосмотру.

use tauri::{AppHandle, State};

use super::sidecar::{resolve_deno, resolve_ytdlp_path};
use crate::probe::{probe, validate_url, ProbeFailure, ProbeSession, SidecarLauncher};
use crate::sidecar::{ChildRegistry, YtDlpJsRuntime};
use crate::types::{ProbeError, ProbeErrorDetails, ProbeResult, YtDlpFailureReason};

/// Разбирает ссылку и возвращает карточку ролика либо один из девяти
/// классов Ф-6.
///
/// Возврат — `Result<ProbeResult, ProbeError>` без `Option` в позиции
/// успеха: вытеснение закрыто сторожем по поколениям на фронтенде, а
/// пустой результат текущая карточка не переживает (ревью TL-33). Почему
/// вытесненный вызов при этом реджектится и каким классом — в doc
/// [`crate::probe`].
///
/// `session` и `registry` внедряются Tauri из состояния приложения и
/// частью JS-видимого контракта не являются: фронтенд зовёт
/// `invoke('probe_url', { url })`.
#[tauri::command]
pub async fn probe_url(
    app: AppHandle,
    url: String,
    session: State<'_, ProbeSession>,
    registry: State<'_, ChildRegistry>,
) -> Result<ProbeResult, ProbeError> {
    // Отказ в лог здесь не пишется: каждый путь, ведущий к нему, уже
    // объяснил себя сам — оркестрация пишет класс и время, резолв пути и
    // валидация ввода пишут свою строку ниже. Повторная печать `Display`
    // давала бы на каждое вытеснение правдивую строку «разбор вытеснен» и
    // сразу за ней неправдивую «yt-dlp не смог получить данные», то есть
    // сбивала бы именно там, где по логу разбирают живой дефект.
    probe_now(&app, &url, &session, &registry)
        .await
        .map_err(|failure| failure.to_contract())
}

/// Останавливает идущий разбор, не начиная нового.
///
/// Возвращает `Result`, хотя отменять нечего — это требование Tauri к
/// async-командам, принимающим ссылки (`State<'_, T>`): иначе
/// сгенерированный future не может быть `'static`. На JS-стороне промис
/// всегда резолвится (`invoke<void>`), реджекта у команды нет.
#[tauri::command]
pub async fn cancel_probe(session: State<'_, ProbeSession>) -> Result<(), ()> {
    session.cancel().await;
    Ok(())
}

/// Общий путь [`probe_url`] до конвертации ошибки в контракт.
async fn probe_now(
    app: &AppHandle,
    url: &str,
    session: &ProbeSession,
    registry: &ChildRegistry,
) -> Result<ProbeResult, ProbeFailure> {
    // Ф-2 буквально: пока ввод не признан http(s)-ссылкой, не происходит
    // ничего — ни резолва пути к бинарнику, ни обращения к домену
    // `sidecar`. Домен проверяет ввод и сам (`probe` начинается с той же
    // функции): он не полагается на дисциплину вызывающего, а команда не
    // полагается на то, что домен успеет проверить раньше, чем она
    // потрогает файловую систему.
    validate_url(url).inspect_err(|_| {
        eprintln!("probe: ввод не является http(s)-ссылкой — yt-dlp не запускался");
    })?;

    // Страж занятости держится до конца разбора: пока процесс жив,
    // уборка контура обновления его установку не трогает (Ф-7).
    let (executable, _in_use) = resolve_ytdlp_path(app).map_err(|err| {
        // Практически недостижимо: фронтенд не даёт разбирать ссылки, пока
        // строка yt-dlp на служебном экране не `ok` (Ф-10, С-11). Если всё
        // же случилось — это честный «сбой yt-dlp» (С-12), а не молчание.
        eprintln!("probe: готовой установки yt-dlp нет: {err}");
        ProbeFailure::YtDlpFailure {
            reason: YtDlpFailureReason::Generic,
            details: ProbeErrorDetails {
                stderr_tail: None,
                exit_code: None,
            },
        }
    })?;

    // deno — путь к sidecar и окружение одним значением (TL-109); если
    // запускать его нельзя, yt-dlp получает `--no-js-runtimes`, а причина
    // уходит в лог одной строкой.
    let js_runtime = YtDlpJsRuntime::from_deno(resolve_deno(app));

    probe(
        session,
        &SidecarLauncher::new(executable, js_runtime, registry),
        url,
    )
    .await
}
