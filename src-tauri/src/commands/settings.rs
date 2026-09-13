//! Три команды настроек (контракт TL-83, эпик E5).
//!
//! Тонкий слой над доменом, как и остальные команды. Типы ответов и отказов
//! — секция E5 в [`crate::types`].
//!
//! # Что здесь настоящее, а что ждёт реализации
//!
//! Тела подставит TL-91 поверх хранилища настроек TL-87 и шаблона TL-86.
//! До них:
//!
//! - `settings_get` отвечает **правдой**: настроек ещё не существует, и
//!   каждая задача качается по умолчаниям — в системную «Загрузки», с
//!   основой имени из названия, с пределом попыток E3;
//! - `settings_set` и `preview_name_template` честно ответить не могут:
//!   сохранить некуда, отрендерить шаблон нечем. Здесь `unimplemented!`, как
//!   допускает задача. Правдоподобный успех был бы хуже: интерфейс показал бы
//!   сохранённым то, что не сохранено. Фронтенд эти команды до TL-91 не
//!   зовёт — экран настроек (TL-94) строится на замоканном `invoke`.

use tauri::{AppHandle, Manager};

use crate::types::{
    DestinationFolder, QualityKind, SelectedQuality, Settings, SettingsCommandError, SettingsPatch,
    SettingsView, TemplatePreview,
};

/// Образец данных предпросмотра шаблона: название ролика.
///
/// Решение неясности дизайна (сноска ¹ в пункте 3): образец один и тот же
/// везде, где строится пример имени. Полный состав и правило для даты — в
/// doc [`TemplatePreview`].
#[allow(dead_code)]
pub const PREVIEW_SAMPLE_TITLE: &str = "Как приручить дракона";

/// Образец данных предпросмотра шаблона: канонический id ролика (11 знаков,
/// форма TL-72).
#[allow(dead_code)]
pub const PREVIEW_SAMPLE_VIDEO_ID: &str = "dQw4w9WgXcQ";

/// Образец данных предпросмотра шаблона: пункт качества, `{quality}` даёт
/// `1080p`.
#[allow(dead_code)]
pub const PREVIEW_SAMPLE_QUALITY: SelectedQuality = SelectedQuality {
    kind: QualityKind::Standard,
    height_px: Some(1080),
};

/// Умолчание шаблона имени (Ф-10): основа — название ролика, байт в байт
/// как в E3.
const DEFAULT_NAME_TEMPLATE: &str = "{title}";

/// Настройки по умолчанию (Ф-10, Р-5). До TL-87 — они же действующие.
fn defaults() -> Settings {
    Settings {
        destination_folder: DestinationFolder::System,
        name_template: DEFAULT_NAME_TEMPLATE.to_string(),
        max_attempts: crate::download::retry::MAX_ATTEMPTS,
    }
}

/// Действующие настройки с пометками сброса (Ф-9).
///
/// Без отказов: нечитаемый файл — это умолчания с `wholeFileReset`, а не
/// ошибка команды.
#[tauri::command]
pub async fn settings_get(app: AppHandle) -> SettingsView {
    // Тот же резолв системной «Загрузки», что у воркера очереди
    // (`commands::queue::run`): существует ли папка, в которую пойдёт
    // следующая задача.
    let destination_folder_exists = app.path().download_dir().is_ok_and(|dir| dir.is_dir());

    SettingsView {
        settings: defaults(),
        defaults: defaults(),
        reset_fields: Vec::new(),
        whole_file_reset: false,
        destination_folder_exists,
    }
}

/// Сохранить **одно** поле настроек (Ф-9, Ф-11…Ф-13).
#[tauri::command]
pub async fn settings_set(patch: SettingsPatch) -> Result<SettingsView, SettingsCommandError> {
    let _ = patch;
    unimplemented!("settings_set: тело подставит TL-91 поверх хранилища настроек TL-87")
}

/// Пример основы имени по черновому шаблону на фиксированном образце
/// ([`TemplatePreview`]). Файл не пишет, папки назначения не требует.
#[tauri::command]
pub async fn preview_name_template(
    template: String,
) -> Result<TemplatePreview, SettingsCommandError> {
    let _ = template;
    unimplemented!("preview_name_template: тело подставит TL-91 поверх шаблона TL-86")
}
