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
//! - `settings_set` и `preview_name_template` успехом ответить не могут:
//!   сохранить некуда, отрендерить шаблон нечем. Обе отвечают `writeFailed`
//!   — единственным классом, который не утверждает ложного о введённом
//!   значении (doc [`SettingsCommandErrorKind::WriteFailed`]). Правдоподобный
//!   успех был бы хуже: интерфейс показал бы сохранённым то, что не
//!   сохранено. Фронтенд эти команды до TL-91 не зовёт — экран настроек
//!   (TL-94) строится на замоканном `invoke`.
//!
//! **Паники в заглушках нет намеренно.** Паника внутри асинхронной команды
//! Tauri не доходит до промиса ни ответом, ни отказом, и `invoke` на стороне
//! окна повис бы навсегда. Метка заглушек для поиска перед сборкой —
//! [`STUB_UNTIL_TL91`].

use tauri::{AppHandle, Manager};

use crate::types::{
    DestinationFolder, QualityKind, SelectedQuality, Settings, SettingsCommandError,
    SettingsCommandErrorKind, SettingsPatch, SettingsView, TemplatePreview,
};

/// Метка ответа заглушки в `message`. Уходит вместе с заглушками в TL-91:
/// `grep -rn STUB_UNTIL_TL91 src-tauri/src` перед финальной сборкой обязан
/// быть пуст.
const STUB_UNTIL_TL91: &str = "заглушка до TL-91";

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
///
/// Заглушка до TL-91: всегда `writeFailed`, ничего не сохраняет. В границах
/// класса это правда — файл настроек не записан, действующие значения
/// прежние.
#[tauri::command]
pub async fn settings_set(patch: SettingsPatch) -> Result<SettingsView, SettingsCommandError> {
    Err(SettingsCommandError {
        kind: SettingsCommandErrorKind::WriteFailed,
        message: format!(
            "settings_set: {patch:?} не сохранён, хранилища настроек нет ({STUB_UNTIL_TL91})"
        ),
    })
}

/// Пример основы имени по черновому шаблону на фиксированном образце
/// ([`TemplatePreview`]). Файл не пишет, папки назначения не требует.
///
/// Заглушка до TL-91: всегда `writeFailed`, предпросмотр не построен.
/// `invalidTemplate` здесь был бы ложью о шаблоне, которого никто не
/// проверял, а успех с выдуманным результатом — ложью о примере имени.
#[tauri::command]
pub async fn preview_name_template(
    template: String,
) -> Result<TemplatePreview, SettingsCommandError> {
    Err(SettingsCommandError {
        kind: SettingsCommandErrorKind::WriteFailed,
        message: format!(
            "preview_name_template: {template:?} не отрендерен, шаблона нет ({STUB_UNTIL_TL91})"
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Заглушки отвечают типизированным отказом, а не паникой: промис
    /// `invoke` обязан завершиться. Тест зовёт тела команд напрямую — их
    /// сигнатура от `#[tauri::command]` не меняется.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_stubs_answer_write_failed_instead_of_panicking() {
        let set = settings_set(SettingsPatch::MaxAttempts(3))
            .await
            .expect_err("до TL-91 сохранять некуда");
        assert_eq!(set.kind, SettingsCommandErrorKind::WriteFailed);
        assert!(set.message.contains(STUB_UNTIL_TL91), "{}", set.message);

        let preview = preview_name_template("{id} — {title}".to_string())
            .await
            .expect_err("до TL-91 рендерить нечем");
        assert_eq!(preview.kind, SettingsCommandErrorKind::WriteFailed);
        assert!(
            preview.message.contains(STUB_UNTIL_TL91),
            "{}",
            preview.message
        );
    }
}
