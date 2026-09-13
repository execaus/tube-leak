//! Тесты команд настроек (TL-91) — уровень команд, без Tauri-рантайма.
//!
//! Тела команд зовутся напрямую (`get_in`, `set_in`, `preview_on`) с
//! настоящим [`SettingsState`] во временном каталоге; обёртки
//! `#[tauri::command]` только достают состояние и резолвер «Загрузок». Сети и
//! процессов нет.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use tempfile::{tempdir, TempDir};

use super::*;
use crate::storage::settings::{NAME_TEMPLATE_MAX_CHARS, SETTINGS_FILE_NAME};
use crate::types::{DestinationFolder, FolderProblem, Settings};

/// Сколько рандеву ждёт соседа, прежде чем признать рантайм заблокированным.
/// Потолок провала, а не замер: при исправном коде сосед откликается сразу.
const RENDEZVOUS_CEILING: Duration = Duration::from_secs(10);

struct Scene {
    data: TempDir,
    downloads: TempDir,
    state: Arc<SettingsState>,
}

/// Сцена с `settings.json`, если он задан, — файл кладётся до открытия.
fn scene_with(file: Option<&str>) -> Scene {
    let data = tempdir().expect("временный каталог данных");
    let downloads = tempdir().expect("временная системная «Загрузки»");
    if let Some(bytes) = file {
        fs::write(data.path().join(SETTINGS_FILE_NAME), bytes).expect("фикстура settings.json");
    }
    let state = Arc::new(SettingsState::open_isolated(Ok(data.path().to_path_buf())));
    Scene {
        data,
        downloads,
        state,
    }
}

fn scene() -> Scene {
    scene_with(None)
}

impl Scene {
    fn file_bytes(&self) -> Option<Vec<u8>> {
        fs::read(self.data.path().join(SETTINGS_FILE_NAME)).ok()
    }

    fn downloads(&self) -> impl FnOnce() -> Option<PathBuf> + Send + 'static {
        let path = self.downloads.path().to_path_buf();
        move || Some(path)
    }

    async fn get(&self) -> SettingsView {
        get_in(Arc::clone(&self.state), self.downloads()).await
    }

    async fn set(&self, patch: SettingsPatch) -> Result<SettingsView, SettingsCommandError> {
        set_in(
            Arc::clone(&self.state),
            patch,
            self.downloads(),
            SettingsStore::set,
        )
        .await
    }
}

fn custom(path: &Path) -> SettingsPatch {
    SettingsPatch::DestinationFolder(DestinationFolder::Custom {
        path: path.to_string_lossy().into_owned(),
    })
}

fn sample_date() -> TemplateDate {
    TemplateDate::new(2026, 9, 14).expect("дата")
}

fn defaults() -> Settings {
    Settings {
        destination_folder: DestinationFolder::System,
        name_template: "{title}".to_owned(),
        max_attempts: 8,
    }
}

// ---------------------------------------------------------------------------
// settings_get
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_fresh_data_dir_reads_defaults_and_creates_nothing() {
    let scene = scene();
    let view = scene.get().await;

    assert_eq!(view.settings, defaults());
    assert_eq!(view.defaults, defaults());
    assert!(view.reset_fields.is_empty());
    assert!(!view.whole_file_reset);
    assert!(view.destination_folder_exists);
    assert_eq!(
        fs::read_dir(scene.data.path()).expect("каталог").count(),
        0,
        "чтение создало файл"
    );
}

/// `destinationFolderExists` — проверка на каждый запрос, у системной и у
/// своей папки, в обе стороны.
#[tokio::test]
async fn destination_folder_exists_is_checked_on_every_get() {
    let scene = scene();
    let state = || Arc::clone(&scene.state);

    assert!(scene.get().await.destination_folder_exists);
    assert!(
        !get_in(state(), || None).await.destination_folder_exists,
        "ОС не дала «Загрузки» — папки нет"
    );
    let gone = scene.data.path().join("нет такой");
    assert!(
        !get_in(state(), move || Some(gone))
            .await
            .destination_folder_exists,
        "системная «Загрузки» не существует"
    );

    let folder = tempdir().expect("своя папка");
    let saved = scene.set(custom(folder.path())).await.expect("сохранение");
    assert!(saved.destination_folder_exists);
    assert!(scene.get().await.destination_folder_exists);

    let canonical = fs::canonicalize(folder.path()).expect("канон");
    folder.close().expect("папка удалена");
    let view = scene.get().await;
    assert!(
        !view.destination_folder_exists,
        "удалённая папка названа существующей"
    );
    assert_eq!(
        view.settings.destination_folder,
        DestinationFolder::Custom {
            path: canonical.to_string_lossy().into_owned()
        },
        "отсутствие папки не сбрасывает выбор (Ф-11)"
    );
}

/// Круг чтение → запись → чтение; пометка сброса гаснет после первого
/// успешного сохранения **другого** поля.
#[tokio::test]
async fn a_saved_value_is_read_back_and_clears_the_reset_marks() {
    let scene = scene_with(Some(r#"{"version":1,"maxAttempts":0}"#));
    let before = scene.get().await;
    assert_eq!(before.reset_fields, vec![SettingsField::MaxAttempts]);
    assert_eq!(before.settings.max_attempts, 8);

    let saved = scene
        .set(SettingsPatch::NameTemplate("{id} — {title}".to_owned()))
        .await
        .expect("сохранение");
    assert_eq!(saved.settings.name_template, "{id} — {title}");
    assert!(saved.reset_fields.is_empty());
    assert!(!saved.whole_file_reset);
    assert_eq!(saved.defaults, defaults());

    let after = scene.get().await;
    assert_eq!(after.settings, saved.settings);
    assert!(after.reset_fields.is_empty());

    let file: serde_json::Value =
        serde_json::from_slice(&scene.file_bytes().expect("файл записан")).expect("JSON");
    assert_eq!(file["nameTemplate"], "{id} — {title}");
}

// ---------------------------------------------------------------------------
// settings_set: каждый класс отказа
// ---------------------------------------------------------------------------

#[tokio::test]
async fn every_rejected_folder_is_not_a_directory_with_its_problem() {
    let scene = scene();
    scene
        .set(SettingsPatch::MaxAttempts(3))
        .await
        .expect("файл до отказов");
    let before = scene.file_bytes();
    let file = scene.data.path().join("файл.txt");
    fs::write(&file, b"x").expect("файл");

    let cases = [
        (String::new(), FolderProblem::NotAbsolute),
        ("Movies".to_owned(), FolderProblem::NotAbsolute),
        (
            scene.data.path().join("нет").to_string_lossy().into_owned(),
            FolderProblem::NotFound,
        ),
        (
            file.to_string_lossy().into_owned(),
            FolderProblem::NotADirectory,
        ),
    ];
    for (path, problem) in cases {
        let refused = scene
            .set(SettingsPatch::DestinationFolder(
                DestinationFolder::Custom { path: path.clone() },
            ))
            .await
            .expect_err("папка не принята");
        assert_eq!(
            refused.kind,
            SettingsCommandErrorKind::NotADirectory { problem },
            "{path:?}"
        );
        assert_eq!(scene.file_bytes(), before, "отказ тронул файл: {path:?}");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn a_folder_behind_a_closed_parent_is_not_a_directory_no_access() {
    use std::os::unix::fs::PermissionsExt;

    let scene = scene();
    let closed = scene.data.path().join("закрыт");
    fs::create_dir_all(closed.join("папка")).expect("папки");
    fs::set_permissions(&closed, fs::Permissions::from_mode(0o000)).expect("права");

    let refused = scene.set(custom(&closed.join("папка"))).await;
    fs::set_permissions(&closed, fs::Permissions::from_mode(0o755)).expect("права назад");

    assert_eq!(
        refused.expect_err("нет доступа").kind,
        SettingsCommandErrorKind::NotADirectory {
            problem: FolderProblem::NoAccess
        }
    );
    assert_eq!(scene.file_bytes(), None, "отказ записал файл");
}

/// Критерий #99: один и тот же неверный шаблон даёт в `settings_set` и в
/// `preview_name_template` один и тот же отказ целиком — включая `tooLong`
/// и `{{title}}` (N9).
#[tokio::test]
async fn a_bad_template_is_refused_alike_by_save_and_preview() {
    let scene = scene();
    // `{title}` — 7 символов; кириллица — чтобы предел считался в символах,
    // а не в байтах.
    let too_long = format!("{{title}}{}", "я".repeat(NAME_TEMPLATE_MAX_CHARS + 1 - 7));
    assert_eq!(too_long.chars().count(), NAME_TEMPLATE_MAX_CHARS + 1);

    let cases = [
        (
            "{channel}".to_owned(),
            TemplateProblem::UnknownVariable {
                position: 1,
                name: "channel".to_owned(),
            },
        ),
        (
            "{{title}}".to_owned(),
            TemplateProblem::UnclosedBrace { position: 1 },
        ),
        (
            "{title}}".to_owned(),
            TemplateProblem::StrayClosingBrace { position: 8 },
        ),
        ("видео".to_owned(), TemplateProblem::NoVariables),
        (String::new(), TemplateProblem::NoVariables),
        (too_long, TemplateProblem::TooLong { max: 200 }),
    ];
    for (template, problem) in cases {
        let saved = scene
            .set(SettingsPatch::NameTemplate(template.clone()))
            .await
            .expect_err("шаблон не принят");
        assert_eq!(
            saved.kind,
            SettingsCommandErrorKind::InvalidTemplate {
                problem: problem.clone()
            },
            "{template:?}"
        );
        let previewed = preview_on(&template, sample_date()).expect_err("пример не строится");
        assert_eq!(previewed, saved, "{template:?}");
        assert_eq!(scene.file_bytes(), None, "отказ записал файл: {template:?}");
    }
}

/// Граница предела: ровно 200 символов принимают обе команды.
#[tokio::test]
async fn a_template_at_the_limit_is_accepted_by_save_and_preview() {
    let scene = scene();
    let exactly = format!("{{title}}{}", "я".repeat(NAME_TEMPLATE_MAX_CHARS - 7));
    assert_eq!(exactly.chars().count(), NAME_TEMPLATE_MAX_CHARS);

    preview_on(&exactly, sample_date()).expect("пример строится");
    let saved = scene
        .set(SettingsPatch::NameTemplate(exactly.clone()))
        .await
        .expect("сохранение");
    assert_eq!(saved.settings.name_template, exactly);
}

#[tokio::test]
async fn attempts_out_of_range_are_invalid_value_with_the_bounds() {
    let scene = scene();
    for attempts in [0_i64, 21, -1, i64::MAX, i64::MIN] {
        let refused = scene
            .set(SettingsPatch::MaxAttempts(attempts))
            .await
            .expect_err("вне 1…20");
        assert_eq!(
            refused.kind,
            SettingsCommandErrorKind::InvalidValue { min: 1, max: 20 },
            "{attempts}"
        );
    }
    assert_eq!(scene.file_bytes(), None, "отказ записал файл");

    for attempts in [1_i64, 20] {
        let saved = scene
            .set(SettingsPatch::MaxAttempts(attempts))
            .await
            .expect("граница принимается");
        assert_eq!(i64::from(saved.settings.max_attempts), attempts);
    }
}

/// Отказ записи — `writeFailed`, значение в памяти прежнее.
#[cfg(unix)]
#[tokio::test]
async fn a_failing_write_is_write_failed() {
    use std::os::unix::fs::PermissionsExt;

    let scene = scene();
    let data = scene.data.path();
    fs::set_permissions(data, fs::Permissions::from_mode(0o500)).expect("каталог только на чтение");
    let refused = scene.set(SettingsPatch::MaxAttempts(3)).await;
    fs::set_permissions(data, fs::Permissions::from_mode(0o700)).expect("права назад");

    assert_eq!(
        refused.expect_err("временный файл не создаётся").kind,
        SettingsCommandErrorKind::WriteFailed
    );
    assert_eq!(scene.get().await.settings.max_attempts, 8);
}

/// Хранилища нет (каталог данных не определился): чтение — умолчания,
/// сохранение — `writeFailed` с причиной в `message`.
#[tokio::test]
async fn without_a_store_get_answers_defaults_and_set_answers_write_failed() {
    let state = Arc::new(SettingsState::open_isolated(Err(
        "нет домашнего каталога".to_owned()
    )));

    let view = get_in(Arc::clone(&state), || None).await;
    assert_eq!(view.settings, defaults());
    assert!(view.reset_fields.is_empty());
    assert!(!view.whole_file_reset);

    let refused = set_in(
        state,
        SettingsPatch::MaxAttempts(3),
        || None,
        SettingsStore::set,
    )
    .await
    .expect_err("сохранять некуда");
    assert_eq!(refused.kind, SettingsCommandErrorKind::WriteFailed);
    assert!(
        refused.message.contains("нет домашнего каталога"),
        "{}",
        refused.message
    );
}

/// Паника в блокирующем пуле — типизированный отказ, промис `invoke` не
/// повисает.
#[tokio::test]
async fn a_panicking_save_is_write_failed() {
    let scene = scene();
    let refused = set_in(
        Arc::clone(&scene.state),
        SettingsPatch::MaxAttempts(3),
        || None,
        |_, _| panic!("сохранение запаниковало (ожидаемо в тесте)"),
    )
    .await
    .expect_err("паника — отказ");
    assert_eq!(refused.kind, SettingsCommandErrorKind::WriteFailed);
}

// ---------------------------------------------------------------------------
// preview_name_template
// ---------------------------------------------------------------------------

/// Пример дизайна и каждая переменная на фиксированном образце. `{date}`
/// один допустим (N5).
#[test]
fn the_preview_builds_names_on_the_fixed_sample() {
    for (template, expected) in [
        ("{id} — {title}", "dQw4w9WgXcQ — Как приручить дракона"),
        (
            "{title} [{quality}] {date}",
            "Как приручить дракона [1080p] 2026-09-14",
        ),
        ("{date}", "2026-09-14"),
    ] {
        assert_eq!(
            preview_on(template, sample_date()).expect("пример строится"),
            TemplatePreview {
                result: expected.to_owned()
            },
            "{template}"
        );
    }
}

/// Команда целиком: дата образца — сегодняшняя по `clock::today_utc`, диска
/// команда не касается (у неё нет ни состояния, ни каталога данных).
#[tokio::test]
async fn the_preview_command_uses_today_and_touches_no_disk() {
    let scene = scene();
    let (year, month, day) = clock::today_utc();

    let preview = preview_name_template("{date}".to_owned())
        .await
        .expect("пример строится");

    assert_eq!(preview.result, format!("{year:04}-{month:02}-{day:02}"));
    assert_eq!(fs::read_dir(scene.data.path()).expect("каталог").count(), 0);
}

/// N10: одиночный суррогат в JSON отклоняет разбор тела, до типа `String`
/// аргумента. Тест проверяет посылку doc команды на `serde_json`, которым
/// Tauri разбирает тело `invoke`; сам путь Tauri здесь не запускается.
#[test]
fn a_lone_surrogate_is_refused_by_json_parsing_before_the_command() {
    let lone = serde_json::from_str::<serde_json::Value>(r#"{"template":"\ud800{title}"}"#);
    assert!(lone.is_err(), "{lone:?}");

    let paired: serde_json::Value =
        serde_json::from_str(r#"{"template":"😀{title}"}"#).expect("пара суррогатов");
    assert_eq!(paired["template"], "\u{1F600}{title}");
}

// ---------------------------------------------------------------------------
// Н-3: блокирующая работа не держит рантайм
// ---------------------------------------------------------------------------

/// Рандеву с соседней задачей того же однопоточного рантайма (приём
/// `commands::history`).
fn rendezvous() -> (
    tokio::task::JoinHandle<()>,
    impl FnOnce() -> bool + Send + 'static,
) {
    let (tx, rx) = mpsc::channel::<()>();
    let neighbour = tokio::spawn(async move {
        let _ = tx.send(());
    });
    let wait = move || rx.recv_timeout(RENDEZVOUS_CEILING).is_ok();
    (neighbour, wait)
}

/// Сохранение папки целиком — `canonicalize`, запись, `rename` — вне потока
/// рантайма. Подменяется не `canonicalize`, а весь доменный вызов (doc
/// модуля, «Ограничение»).
#[tokio::test(flavor = "current_thread")]
async fn saving_does_not_block_the_runtime() {
    let scene = scene();
    let folder = tempdir().expect("своя папка");
    let (neighbour, wait) = rendezvous();

    let saved = set_in(
        Arc::clone(&scene.state),
        custom(folder.path()),
        scene.downloads(),
        move |store, patch| {
            assert!(
                wait(),
                "сохранение настроек выполнялось на потоке рантайма: соседняя задача не получила хода"
            );
            store.set(patch)
        },
    )
    .await
    .expect("сохранение");
    assert!(saved.destination_folder_exists);
    neighbour.await.expect("сосед завершился");
}

#[tokio::test(flavor = "current_thread")]
async fn checking_the_folder_after_saving_does_not_block_the_runtime() {
    let scene = scene();
    let (neighbour, wait) = rendezvous();
    let downloads = scene.downloads.path().to_path_buf();

    let saved = set_in(
        Arc::clone(&scene.state),
        SettingsPatch::MaxAttempts(3),
        move || {
            assert!(
                wait(),
                "резолв «Загрузок» при сохранении выполнялся на потоке рантайма"
            );
            Some(downloads)
        },
        SettingsStore::set,
    )
    .await
    .expect("сохранение");
    assert!(
        saved.destination_folder_exists,
        "резолвер не дошёл до ответа"
    );
    neighbour.await.expect("сосед завершился");
}

#[tokio::test(flavor = "current_thread")]
async fn checking_the_folder_on_get_does_not_block_the_runtime() {
    let scene = scene();
    let (neighbour, wait) = rendezvous();
    let downloads = scene.downloads.path().to_path_buf();

    let view = get_in(Arc::clone(&scene.state), move || {
        assert!(
            wait(),
            "резолв «Загрузок» при чтении выполнялся на потоке рантайма"
        );
        Some(downloads)
    })
    .await;
    assert!(
        view.destination_folder_exists,
        "резолвер не дошёл до ответа"
    );
    neighbour.await.expect("сосед завершился");
}

// ---------------------------------------------------------------------------
// Единственность открытия
// ---------------------------------------------------------------------------

/// Флаг — главная защита: второе открытие за процесс не читает, не
/// откладывает и не удаляет ничего, в том числе испорченный файл, который
/// открытие иначе отложило бы под `.broken-`.
///
/// Первый вызов процесса мог сделать и другой тест, поэтому его исход не
/// проверяется: после него флаг взят наверняка. Остальные тесты флага не
/// трогают (`open_isolated`, `open_with`).
#[test]
fn a_second_open_in_the_process_touches_no_disk_and_has_no_store() {
    let first = tempdir().expect("каталог первого открытия");
    let second = tempdir().expect("каталог второго открытия");
    let garbage = second.path().join(SETTINGS_FILE_NAME);
    fs::write(&garbage, "не JSON").expect("мусор");

    drop(SettingsState::open(Ok(first.path().to_path_buf())));

    let again = SettingsState::open(Ok(second.path().to_path_buf()));
    let refused = again.store().expect_err("второе открытие отклонено");
    assert!(refused.contains("уже открыто"), "{refused}");
    let names = || {
        fs::read_dir(second.path())
            .expect("каталог")
            .map(|entry| entry.expect("элемент").file_name())
            .collect::<Vec<_>>()
    };
    assert_eq!(names(), vec![std::ffi::OsString::from(SETTINGS_FILE_NAME)]);
    assert_eq!(
        fs::read(&garbage).expect("мусор на месте"),
        "не JSON".as_bytes()
    );

    // Мимо состояния — тем же флагом.
    let bypass = SettingsStore::open(second.path());
    assert!(
        matches!(bypass, Err(SettingsOpenError::AlreadyOpen)),
        "{bypass:?}"
    );
    assert_eq!(names(), vec![std::ffi::OsString::from(SETTINGS_FILE_NAME)]);
}

/// Сторож по исходникам: токен `SettingsStore::open` в продакшен-коде стоит
/// ровно в `SettingsState::open`. Правила сканера и его слепые зоны — те же,
/// что у истории (`commands::history`, тест
/// `the_store_is_opened_in_exactly_one_production_place`).
#[test]
fn the_settings_store_is_opened_in_exactly_one_production_place() {
    let places = crate::commands::source_guard::production_places("SettingsStore");
    assert!(
        places.scanned > 20,
        "сторож не увидел исходников: {}",
        places.scanned
    );
    assert_eq!(
        places.aliases,
        Vec::<String>::new(),
        "переименованный импорт или псевдоним типа SettingsStore"
    );
    let line = crate::commands::source_guard::first_line_in_impl(
        include_str!("settings.rs"),
        "impl SettingsState",
        "SettingsStore::open",
    );
    assert_eq!(
        places.calls,
        vec![format!("commands/settings.rs:{line}")],
        "SettingsStore::open стоит не ровно в SettingsState::open"
    );
}
