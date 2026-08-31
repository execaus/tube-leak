//! `#[tauri::command]` управления задачей загрузки (Ф-2, Ф-4, Ф-5 эпика
//! E4) — TL-44, переведены на очередь в TL-73.
//!
//! Тонкий слой над [`crate::queue::scheduler`]: конвертирует доменный
//! отказ [`crate::download::DownloadCommandRejection`] в контрактный
//! [`DownloadCommandError`] и собирает боевое окружение очереди
//! ([`app_queue`]). Решения о порядке, слоте и повторах принимает
//! планировщик, автомат фаз и подчистку — домен скачивания.
//!
//! Экран зовёт три команды и все три — отсюда. Новых подписок задача не
//! добавляет: событие `download://progress` едет тем же `listen()`, на
//! который разрешение уже выдано (E1), а сторож `tests/frontend_acl.rs`
//! (урок TL-24) проверяет это сам.
//!
//! # Что изменилось с приходом очереди
//!
//! Имена и формы команд те же (TL-70), а поведение — другое, и разница
//! видна прямо здесь: `start_download` больше не занимает слот, а ставит
//! задачу в хвост очереди (Ф-2); `cancel_download` работает и для
//! ожидающей задачи (Ф-4); `retry_download` возвращает упавшую задачу в
//! очередь тем же id (Ф-5). Папка назначения по-прежнему не параметр
//! команды: она системная (Р-1 E3) и станет настройкой в E5, не меняя
//! формы вызова.

use std::sync::Arc;

use tauri::{AppHandle, State};

use super::queue::app_queue;
use crate::queue::scheduler::{self, QueueScheduler};
use crate::types::{DownloadCommandError, DownloadStarted, StartDownloadRequest};

/// Ставит выбранный пункт карточки в очередь (Ф-2).
///
/// Возвращается быстро — идентификатором задачи и фазой, а не
/// результатом: ждать в промисе часовую работу нечего, всё идёт
/// событиями. Отказ у постановки остался ровно один — дубль (Ф-8).
#[tauri::command]
pub async fn start_download(
    app: AppHandle,
    request: StartDownloadRequest,
    scheduler: State<'_, Arc<QueueScheduler>>,
) -> Result<DownloadStarted, DownloadCommandError> {
    scheduler::start_download(&scheduler, request, &app_queue(&app)).map_err(|rejection| {
        eprintln!("download: старт отклонён: {rejection}");
        rejection.to_contract()
    })
}

/// Отменяет задачу в любой нетерминальной фазе — активную или ожидающую
/// (Ф-4).
#[tauri::command]
pub async fn cancel_download(
    app: AppHandle,
    task_id: String,
    scheduler: State<'_, Arc<QueueScheduler>>,
) -> Result<(), DownloadCommandError> {
    scheduler::cancel_download(&scheduler, &task_id, &app_queue(&app))
        .await
        .map_err(|rejection| {
            eprintln!("download: отмена отклонена: {rejection}");
            rejection.to_contract()
        })
}

/// Возвращает упавшую задачу в очередь — в хвост, с тем же `taskId`
/// (Ф-5).
#[tauri::command]
pub async fn retry_download(
    app: AppHandle,
    task_id: String,
    scheduler: State<'_, Arc<QueueScheduler>>,
) -> Result<(), DownloadCommandError> {
    scheduler::retry_download(&scheduler, &task_id, &app_queue(&app)).map_err(|rejection| {
        eprintln!("download: повтор отклонён: {rejection}");
        rejection.to_contract()
    })
}
