//! Тесты хранилища настроек (TL-87). Сети и процессов нет; диск — только
//! временные каталоги.

use super::*;
use crate::download::name_template::validate_for_save;
use crate::types::{QualityKind, SelectedQuality};
use serde_json::json;
use tempfile::{tempdir, TempDir};

/// Метка отложенной копии в тестах — постоянная, чтобы занятость имени
/// проверялась без гонки с секундной стрелкой.
const LABEL: &str = "1700000000";

fn fixed_label() -> String {
    LABEL.to_owned()
}

fn real_rename(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)
}

fn failing_rename(_from: &Path, _to: &Path) -> io::Result<()> {
    Err(io::Error::other("подменённый отказ rename"))
}

fn open(dir: &Path) -> SettingsStore {
    SettingsStore::open_with(dir, real_rename, fixed_label)
}

fn settings_path(dir: &TempDir) -> PathBuf {
    dir.path().join(SETTINGS_FILE_NAME)
}

fn broken_path(dir: &TempDir, suffix: &str) -> PathBuf {
    dir.path().join(format!(
        "{SETTINGS_FILE_NAME}{BROKEN_MARKER}{LABEL}{suffix}"
    ))
}

fn write_file(dir: &TempDir, bytes: &[u8]) {
    fs::write(settings_path(dir), bytes).expect("фикстура settings.json");
}

/// Все имена в каталоге, отсортированные — для сверки «ничего лишнего».
fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("каталог обходится")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Абсолютный путь, которого на диске нет: правило чтения диск не смотрит.
fn absent_absolute() -> String {
    if cfg!(windows) {
        r"C:\нет-такой-папки\tl-87".to_owned()
    } else {
        "/нет-такой-папки/tl-87".to_owned()
    }
}

/// Файл версии 1, где все три поля не по умолчанию и верны.
fn valid_object() -> serde_json::Map<String, Value> {
    let Value::Object(object) = json!({
        "version": 1,
        "destinationFolder": { "kind": "custom", "path": absent_absolute() },
        "nameTemplate": "{id} — {title}",
        "maxAttempts": 3,
    }) else {
        unreachable!("литерал — объект")
    };
    object
}

fn write_object(dir: &TempDir, object: &serde_json::Map<String, Value>) {
    write_file(
        dir,
        &serde_json::to_vec(&Value::Object(object.clone())).expect("фикстура сериализуется"),
    );
}

fn custom(path: &str) -> Destination {
    Destination::Custom(FolderPath(path.to_owned()))
}

fn assert_defaults_whole_reset(store: &SettingsStore) {
    let readout = store.readout();
    assert_eq!(readout.settings, Settings::default());
    assert!(
        readout.whole_file_reset,
        "пометка файла целиком не выставлена"
    );
    assert_eq!(readout.reset_fields, Vec::new());
}

// ───────────────────────────── таблица чтения ─────────────────────────────

#[test]
fn a_missing_file_gives_defaults_without_marks_and_creates_nothing() {
    let dir = tempdir().expect("временный каталог");
    let store = open(dir.path());

    let readout = store.readout();
    assert_eq!(readout.settings, Settings::default());
    assert!(!readout.whole_file_reset);
    assert_eq!(readout.reset_fields, Vec::new());
    assert_eq!(store.open_problem(), None);
    assert_eq!(
        names(dir.path()),
        Vec::<String>::new(),
        "чтение создало файл"
    );
}

#[test]
fn a_missing_data_directory_gives_defaults_too() {
    let dir = tempdir().expect("временный каталог");
    let store = open(&dir.path().join("не-создан"));

    assert_eq!(store.current(), Settings::default());
    assert!(!store.readout().whole_file_reset);
}

#[test]
fn defaults_are_system_folder_title_template_and_eight_attempts() {
    let defaults = Settings::default();
    assert_eq!(defaults.destination(), &Destination::System);
    assert_eq!(defaults.name_template(), "{title}");
    assert_eq!(defaults.max_attempts(), 8);
}

/// Пустой, не JSON, массив, объект без версии, версия 0 или строкой —
/// одна строка таблицы: файл целиком не читается и откладывается.
#[test]
fn every_unreadable_shape_is_set_aside_byte_for_byte_with_defaults() {
    let cases: [(&str, &[u8]); 8] = [
        ("пустой файл", b""),
        ("не JSON", b"\x00\xffnot json {"),
        ("обрезанный JSON", br#"{"version":1,"maxAttempts":"#),
        ("JSON-массив", br#"[{"version":1}]"#),
        ("JSON-строка", br#""version""#),
        ("объект без версии", br#"{"maxAttempts":3}"#),
        ("версия 0", br#"{"version":0,"maxAttempts":3}"#),
        ("версия строкой", br#"{"version":"1","maxAttempts":3}"#),
    ];

    for (name, bytes) in cases {
        let dir = tempdir().expect("временный каталог");
        write_file(&dir, bytes);

        let store = open(dir.path());

        assert_defaults_whole_reset(&store);
        assert!(
            !settings_path(&dir).exists(),
            "{name}: испорченный файл остался под рабочим именем"
        );
        assert_eq!(
            fs::read(broken_path(&dir, ""))
                .unwrap_or_else(|err| panic!("{name}: копии нет: {err}")),
            bytes,
            "{name}: отложенная копия отличается от исходных байт"
        );
        assert!(
            matches!(
                store.open_problem(),
                Some(WholeFileProblem::Malformed {
                    set_aside: Ok(_),
                    ..
                })
            ),
            "{name}: {:?}",
            store.open_problem()
        );
    }
}

/// Откладывание не затирает прежнюю отложенную копию.
#[test]
fn setting_aside_never_overwrites_an_earlier_broken_copy() {
    let dir = tempdir().expect("временный каталог");
    let earlier = b"earlier garbage".as_slice();
    let second = b"second garbage, the same second".as_slice();
    fs::write(broken_path(&dir, ""), earlier).expect("прежняя копия");
    fs::write(broken_path(&dir, "-1"), b"and another").expect("ещё одна копия");
    write_file(&dir, second);

    let store = open(dir.path());

    assert_defaults_whole_reset(&store);
    assert_eq!(fs::read(broken_path(&dir, "")).expect("прежняя"), earlier);
    assert_eq!(
        fs::read(broken_path(&dir, "-1")).expect("вторая"),
        b"and another"
    );
    assert_eq!(fs::read(broken_path(&dir, "-2")).expect("новая"), second);
}

/// Отложенный файл не возвращает пометку на следующем запуске: под рабочим
/// именем его больше нет.
#[test]
fn a_set_aside_file_does_not_mark_the_next_launch_again() {
    let dir = tempdir().expect("временный каталог");
    write_file(&dir, b"garbage");
    assert!(open(dir.path()).readout().whole_file_reset);

    let second_launch = open(dir.path());

    assert!(!second_launch.readout().whole_file_reset);
    assert_eq!(second_launch.open_problem(), None);
}

/// Будущая версия: умолчания с пометкой, файл не тронут, копии нет.
#[test]
fn a_newer_version_is_left_untouched_and_marked_on_every_launch() {
    let dir = tempdir().expect("временный каталог");
    let bytes = br#"{"version":2,"destinationFolder":"shape from the future"}"#;
    write_file(&dir, bytes);

    for launch in 1..=2 {
        let store = open(dir.path());
        assert_defaults_whole_reset(&store);
        assert_eq!(
            store.open_problem(),
            Some(&WholeFileProblem::NewerVersion {
                path: settings_path(&dir),
                found: 2,
                supported: SETTINGS_FORMAT_VERSION,
            }),
            "запуск {launch}"
        );
        assert_eq!(fs::read(settings_path(&dir)).expect("файл"), bytes);
        assert_eq!(names(dir.path()), vec![SETTINGS_FILE_NAME.to_owned()]);
    }
}

/// Первое сохранение поверх будущей версии не теряет её байты.
#[test]
fn the_first_save_over_a_newer_version_keeps_its_bytes_aside() {
    let dir = tempdir().expect("временный каталог");
    let bytes = br#"{"version":7,"whatever":true}"#;
    write_file(&dir, bytes);
    let store = open(dir.path());

    store
        .set(&SettingsPatch::MaxAttempts(5))
        .expect("сохранение");

    assert_eq!(fs::read(broken_path(&dir, "")).expect("копия"), bytes);
    let reopened = open(dir.path());
    assert_eq!(reopened.current().max_attempts(), 5);
    assert!(!reopened.readout().whole_file_reset);
}

/// Файл, который не читается вовсе: пометка, файл не тронут.
#[test]
fn an_unreadable_file_is_marked_and_left_in_place() {
    let dir = tempdir().expect("временный каталог");
    fs::create_dir(settings_path(&dir)).expect("каталог на месте файла");

    let store = open(dir.path());

    assert_defaults_whole_reset(&store);
    assert!(matches!(
        store.open_problem(),
        Some(WholeFileProblem::Unreadable { .. })
    ));
    assert!(settings_path(&dir).is_dir());
}

#[test]
fn a_valid_file_is_read_as_is() {
    let dir = tempdir().expect("временный каталог");
    write_object(&dir, &valid_object());

    let readout = open(dir.path()).readout();

    assert_eq!(readout.settings.destination(), &custom(&absent_absolute()));
    assert_eq!(readout.settings.name_template(), "{id} — {title}");
    assert_eq!(readout.settings.max_attempts(), 3);
    assert_eq!(readout.reset_fields, Vec::new());
    assert!(!readout.whole_file_reset);
}

/// Отсутствующее поле — умолчание без пометки: это не «вне правил», а
/// файл, в котором поля ещё не было.
#[test]
fn missing_fields_take_defaults_silently() {
    let dir = tempdir().expect("временный каталог");
    write_file(&dir, br#"{"version":1,"maxAttempts":12}"#);

    let readout = open(dir.path()).readout();

    assert_eq!(readout.settings.destination(), &Destination::System);
    assert_eq!(readout.settings.name_template(), "{title}");
    assert_eq!(readout.settings.max_attempts(), 12);
    assert_eq!(readout.reset_fields, Vec::new());
}

/// Каждое поле вне правил по отдельности: сброшено только оно, два других
/// читаются как есть.
#[test]
fn each_field_out_of_rules_resets_only_itself() {
    let too_long = format!("{{title}}{}", "я".repeat(NAME_TEMPLATE_MAX_CHARS - 6));
    assert_eq!(too_long.chars().count(), NAME_TEMPLATE_MAX_CHARS + 1);
    assert!(
        NameTemplate::parse(&too_long).is_ok(),
        "фикстура обязана разбираться: сбросить её должна длина, а не разбор"
    );

    let cases: Vec<(&str, &str, Value, SettingsField)> = vec![
        (
            "попыток 0",
            KEY_ATTEMPTS,
            json!(0),
            SettingsField::MaxAttempts,
        ),
        (
            "попыток 21",
            KEY_ATTEMPTS,
            json!(21),
            SettingsField::MaxAttempts,
        ),
        (
            "попыток -1",
            KEY_ATTEMPTS,
            json!(-1),
            SettingsField::MaxAttempts,
        ),
        (
            "попыток 8.5",
            KEY_ATTEMPTS,
            json!(8.5),
            SettingsField::MaxAttempts,
        ),
        (
            "попыток 8.0",
            KEY_ATTEMPTS,
            json!(8.0),
            SettingsField::MaxAttempts,
        ),
        (
            "попыток строкой",
            KEY_ATTEMPTS,
            json!("8"),
            SettingsField::MaxAttempts,
        ),
        (
            "попыток null",
            KEY_ATTEMPTS,
            Value::Null,
            SettingsField::MaxAttempts,
        ),
        (
            "попыток 2^32+3",
            KEY_ATTEMPTS,
            json!(4_294_967_299_u64),
            SettingsField::MaxAttempts,
        ),
        (
            "шаблон с неизвестной переменной",
            KEY_TEMPLATE,
            json!("{channel}"),
            SettingsField::NameTemplate,
        ),
        (
            "шаблон с незакрытой скобкой",
            KEY_TEMPLATE,
            json!("{title"),
            SettingsField::NameTemplate,
        ),
        (
            "шаблон без переменных",
            KEY_TEMPLATE,
            json!("видео"),
            SettingsField::NameTemplate,
        ),
        (
            "пустой шаблон",
            KEY_TEMPLATE,
            json!(""),
            SettingsField::NameTemplate,
        ),
        (
            "шаблон длиннее 200",
            KEY_TEMPLATE,
            json!(too_long),
            SettingsField::NameTemplate,
        ),
        (
            "шаблон числом",
            KEY_TEMPLATE,
            json!(42),
            SettingsField::NameTemplate,
        ),
        (
            "относительный путь",
            KEY_DESTINATION,
            json!({"kind":"custom","path":"Movies/YouTube"}),
            SettingsField::DestinationFolder,
        ),
        (
            "пустой путь",
            KEY_DESTINATION,
            json!({"kind":"custom","path":""}),
            SettingsField::DestinationFolder,
        ),
        (
            "путь с ..",
            KEY_DESTINATION,
            json!({"kind":"custom","path": format!("{}/../x", absent_absolute())}),
            SettingsField::DestinationFolder,
        ),
        (
            "custom без пути",
            KEY_DESTINATION,
            json!({"kind":"custom"}),
            SettingsField::DestinationFolder,
        ),
        (
            "путь числом",
            KEY_DESTINATION,
            json!({"kind":"custom","path":7}),
            SettingsField::DestinationFolder,
        ),
        (
            "неизвестный вид папки",
            KEY_DESTINATION,
            json!({"kind":"desktop"}),
            SettingsField::DestinationFolder,
        ),
        (
            "папка строкой",
            KEY_DESTINATION,
            json!("system"),
            SettingsField::DestinationFolder,
        ),
    ];

    for (name, key, bad, field) in cases {
        let dir = tempdir().expect("временный каталог");
        let mut object = valid_object();
        object.insert(key.to_owned(), bad);
        write_object(&dir, &object);

        let store = open(dir.path());
        let readout = store.readout();

        assert_eq!(readout.reset_fields, vec![field], "{name}");
        assert!(!readout.whole_file_reset, "{name}");
        let defaults = Settings::default();
        let settings = &readout.settings;
        let expect_destination = if field == SettingsField::DestinationFolder {
            defaults.destination().clone()
        } else {
            custom(&absent_absolute())
        };
        let expect_template = if field == SettingsField::NameTemplate {
            defaults.name_template()
        } else {
            "{id} — {title}"
        };
        let expect_attempts = if field == SettingsField::MaxAttempts {
            defaults.max_attempts()
        } else {
            3
        };
        assert_eq!(settings.destination(), &expect_destination, "{name}");
        assert_eq!(settings.name_template(), expect_template, "{name}");
        assert_eq!(settings.max_attempts(), expect_attempts, "{name}");
        assert!(settings_path(&dir).exists(), "{name}: файл отложен целиком");
    }
}

/// Границы включены: 1 и 20 попыток, шаблон ровно в 200 символов.
#[test]
fn the_boundaries_themselves_are_within_the_rules() {
    let exactly = format!("{{title}}{}", "я".repeat(NAME_TEMPLATE_MAX_CHARS - 7));
    assert_eq!(exactly.chars().count(), NAME_TEMPLATE_MAX_CHARS);

    for (attempts, template) in [(1, "{id}"), (20, exactly.as_str())] {
        let dir = tempdir().expect("временный каталог");
        let mut object = valid_object();
        object.insert(KEY_ATTEMPTS.to_owned(), json!(attempts));
        object.insert(KEY_TEMPLATE.to_owned(), json!(template));
        write_object(&dir, &object);

        let readout = open(dir.path()).readout();

        assert_eq!(readout.reset_fields, Vec::new(), "{attempts}, {template}");
        assert_eq!(readout.settings.max_attempts(), attempts);
        assert_eq!(readout.settings.name_template(), template);
    }
}

#[test]
fn two_fields_out_of_rules_reset_both_and_keep_the_third() {
    let dir = tempdir().expect("временный каталог");
    let mut object = valid_object();
    object.insert(KEY_ATTEMPTS.to_owned(), json!(0));
    object.insert(
        KEY_DESTINATION.to_owned(),
        json!({"kind":"custom","path":"relative"}),
    );
    write_object(&dir, &object);

    let readout = open(dir.path()).readout();

    assert_eq!(
        readout.reset_fields,
        vec![SettingsField::DestinationFolder, SettingsField::MaxAttempts]
    );
    assert_eq!(readout.settings.destination(), &Destination::System);
    assert_eq!(readout.settings.max_attempts(), 8);
    assert_eq!(readout.settings.name_template(), "{id} — {title}");
}

/// Неизвестный ключ игнорируется при чтении и переживает запись.
#[test]
fn an_unknown_key_is_ignored_and_survives_a_save() {
    let dir = tempdir().expect("временный каталог");
    let mut object = valid_object();
    object.insert("theme".to_owned(), json!({ "mode": "dark" }));
    write_object(&dir, &object);

    let store = open(dir.path());
    assert_eq!(store.readout().reset_fields, Vec::new());
    assert_eq!(store.current().max_attempts(), 3);

    store
        .set(&SettingsPatch::MaxAttempts(4))
        .expect("сохранение");

    let on_disk: Value =
        serde_json::from_slice(&fs::read(settings_path(&dir)).expect("файл")).expect("JSON");
    assert_eq!(on_disk["theme"], json!({ "mode": "dark" }));
    assert_eq!(on_disk["maxAttempts"], json!(4));
}

/// Огрызок убитой записи убирается при чтении и не мешает файлу.
#[test]
fn a_leftover_temporary_file_is_removed_on_open() {
    let dir = tempdir().expect("временный каталог");
    write_object(&dir, &valid_object());
    fs::write(dir.path().join(SETTINGS_TEMP_FILE_NAME), b"{\"version\":1,").expect("огрызок");

    let store = open(dir.path());

    assert!(!store.temp_path().exists());
    assert_eq!(store.current().max_attempts(), 3);
}

// ───────────────────────────── запись ─────────────────────────────

/// Форма файла — та, что описана в шапке модуля.
#[test]
fn a_saved_file_has_the_documented_shape() {
    let dir = tempdir().expect("временный каталог");
    let store = open(dir.path());
    let folder = tempdir().expect("папка назначения");

    store
        .set(&SettingsPatch::DestinationFolder(
            DestinationFolder::Custom {
                path: folder.path().to_string_lossy().into_owned(),
            },
        ))
        .expect("сохранение");

    let canonical = fs::canonicalize(folder.path()).expect("канон");
    let on_disk: Value =
        serde_json::from_slice(&fs::read(settings_path(&dir)).expect("файл")).expect("JSON");
    assert_eq!(
        on_disk,
        json!({
            "version": 1,
            "destinationFolder": { "kind": "custom", "path": canonical.to_string_lossy() },
            "nameTemplate": "{title}",
            "maxAttempts": 8,
        })
    );
}

#[test]
fn saving_creates_a_missing_data_directory() {
    let dir = tempdir().expect("временный каталог");
    let data = dir.path().join("данные").join("tube-leak");
    let store = open(&data);

    store
        .set(&SettingsPatch::NameTemplate("{id}".to_owned()))
        .expect("сохранение");

    assert_eq!(open(&data).current().name_template(), "{id}");
}

/// Сохранение одного поля не трогает два других — ни на диске, ни в памяти.
#[test]
fn saving_one_field_leaves_the_other_two_as_they_were() {
    let patches = [
        SettingsPatch::DestinationFolder(DestinationFolder::System),
        SettingsPatch::NameTemplate("{title} [{quality}]".to_owned()),
        SettingsPatch::MaxAttempts(17),
    ];

    for patch in patches {
        let dir = tempdir().expect("временный каталог");
        write_object(&dir, &valid_object());
        let store = open(dir.path());

        let saved = store.set(&patch).expect("сохранение");

        for settings in [saved, store.current(), open(dir.path()).current()] {
            let (dest, template, attempts) = match &patch {
                SettingsPatch::DestinationFolder(_) => (Destination::System, "{id} — {title}", 3),
                SettingsPatch::NameTemplate(_) => {
                    (custom(&absent_absolute()), "{title} [{quality}]", 3)
                }
                SettingsPatch::MaxAttempts(_) => (custom(&absent_absolute()), "{id} — {title}", 17),
            };
            assert_eq!(settings.destination(), &dest, "{patch:?}");
            assert_eq!(settings.name_template(), template, "{patch:?}");
            assert_eq!(settings.max_attempts(), attempts, "{patch:?}");
        }
    }
}

/// Отказ проверки значения — до диска: файл байт в байт прежний.
#[test]
fn a_rejected_value_does_not_touch_the_file() {
    let dir = tempdir().expect("временный каталог");
    write_object(&dir, &valid_object());
    let before = fs::read(settings_path(&dir)).expect("файл");
    let store = open(dir.path());

    let rejected = [
        SettingsPatch::MaxAttempts(0),
        SettingsPatch::NameTemplate("{channel}".to_owned()),
        SettingsPatch::NameTemplate("{title}".repeat(40)),
        SettingsPatch::DestinationFolder(DestinationFolder::Custom {
            path: "relative".to_owned(),
        }),
    ];
    for patch in rejected {
        assert!(store.set(&patch).is_err(), "{patch:?} принят");
        assert_eq!(
            fs::read(settings_path(&dir)).expect("файл"),
            before,
            "{patch:?}"
        );
    }
    assert_eq!(names(dir.path()), vec![SETTINGS_FILE_NAME.to_owned()]);
}

#[test]
fn attempts_outside_one_to_twenty_are_rejected_and_the_edges_accepted() {
    let dir = tempdir().expect("временный каталог");
    let store = open(dir.path());

    for value in [0, 21, -1, i64::MIN, i64::MAX, 4_294_967_297] {
        assert_eq!(
            store.set(&SettingsPatch::MaxAttempts(value)),
            Err(SettingsSetError::InvalidAttempts {
                value,
                min: 1,
                max: 20
            })
        );
    }
    for value in [1, 20] {
        let saved = store
            .set(&SettingsPatch::MaxAttempts(value))
            .expect("граница");
        assert_eq!(i64::from(saved.max_attempts()), value);
    }
}

/// Шаблон при сохранении отклоняется той же проблемой, что даёт
/// предпросмотр (`validate_for_save`, TL-91), — один валидатор, не два.
#[test]
fn a_template_is_rejected_with_the_same_problem_as_the_preview() {
    let dir = tempdir().expect("временный каталог");
    let store = open(dir.path());
    let sample = TemplateContext {
        title: "Как приручить дракона",
        video_id: "dQw4w9WgXcQ",
        quality: SelectedQuality {
            kind: QualityKind::Standard,
            height_px: Some(1080),
        },
        date: TemplateDate::new(2026, 9, 14).expect("дата"),
    };

    for template in ["{title", "{channel} {title}", "видео", "", "}{title}"] {
        let preview = validate_for_save(template, &sample).expect_err(template);
        assert_eq!(
            store.set(&SettingsPatch::NameTemplate(template.to_owned())),
            Err(SettingsSetError::InvalidTemplate(preview)),
            "{template}"
        );
    }
}

/// Предел 200 символов действует и при сохранении, иначе сохранённое
/// значение молча сбросилось бы на следующем запуске.
#[test]
fn a_template_longer_than_the_limit_is_rejected_on_save() {
    let dir = tempdir().expect("временный каталог");
    let store = open(dir.path());
    let exactly = format!("{{title}}{}", "я".repeat(NAME_TEMPLATE_MAX_CHARS - 7));
    let longer = format!("{exactly}я");

    assert_eq!(
        store.set(&SettingsPatch::NameTemplate(longer)),
        Err(SettingsSetError::TemplateTooLong {
            found: NAME_TEMPLATE_MAX_CHARS + 1,
            max: NAME_TEMPLATE_MAX_CHARS,
        })
    );
    store
        .set(&SettingsPatch::NameTemplate(exactly.clone()))
        .expect("ровно предел");
    let reopened = open(dir.path()).readout();
    assert_eq!(reopened.settings.name_template(), exactly);
    assert_eq!(reopened.reset_fields, Vec::new());
}

// ───────────────────────────── папка ─────────────────────────────

#[test]
fn a_custom_folder_is_stored_canonical_through_dots_and_symlinks() {
    let dir = tempdir().expect("временный каталог");
    let target = tempdir().expect("папка назначения");
    fs::create_dir(target.path().join("inner")).expect("вложенная");
    let dotted = target.path().join("inner").join("..");
    let store = open(dir.path());

    let saved = store
        .set(&SettingsPatch::DestinationFolder(
            DestinationFolder::Custom {
                path: dotted.to_string_lossy().into_owned(),
            },
        ))
        .expect("сохранение");

    let canonical = fs::canonicalize(target.path()).expect("канон");
    assert_eq!(
        saved.destination(),
        &custom(canonical.to_str().expect("UTF-8"))
    );

    #[cfg(unix)]
    {
        let link = dir.path().join("ссылка");
        std::os::unix::fs::symlink(target.path(), &link).expect("символическая ссылка");
        let saved = store
            .set(&SettingsPatch::DestinationFolder(
                DestinationFolder::Custom {
                    path: link.to_string_lossy().into_owned(),
                },
            ))
            .expect("сохранение ссылки");
        assert_eq!(
            saved.destination(),
            &custom(canonical.to_str().expect("UTF-8"))
        );
    }
}

#[test]
fn a_bad_folder_is_rejected_with_its_problem() {
    let dir = tempdir().expect("временный каталог");
    let store = open(dir.path());
    let file = dir.path().join("файл.txt");
    fs::write(&file, b"x").expect("файл");

    let cases = [
        (String::new(), FolderProblem::NotAbsolute),
        ("Movies".to_owned(), FolderProblem::NotAbsolute),
        (
            dir.path().join("нет").to_string_lossy().into_owned(),
            FolderProblem::NotFound,
        ),
        (
            file.join("внутри").to_string_lossy().into_owned(),
            FolderProblem::NotFound,
        ),
        (
            file.to_string_lossy().into_owned(),
            FolderProblem::NotADirectory,
        ),
    ];
    for (path, problem) in cases {
        match store.set(&SettingsPatch::DestinationFolder(
            DestinationFolder::Custom { path: path.clone() },
        )) {
            Err(SettingsSetError::Folder { problem: got, .. }) => {
                assert_eq!(got, problem, "{path:?}");
            }
            other => panic!("{path:?}: {other:?}"),
        }
    }
    assert!(!settings_path(&dir).exists(), "отказ записал файл");
}

#[cfg(unix)]
#[test]
fn a_folder_behind_a_closed_parent_is_no_access() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempdir().expect("временный каталог");
    let closed = dir.path().join("закрыт");
    fs::create_dir_all(closed.join("папка")).expect("папки");
    fs::set_permissions(&closed, fs::Permissions::from_mode(0o000)).expect("права");

    let result = check_folder(closed.join("папка").to_str().expect("UTF-8"));
    fs::set_permissions(&closed, fs::Permissions::from_mode(0o755)).expect("права назад");

    assert_eq!(
        result.map_err(|err| err.problem),
        Err(FolderProblem::NoAccess)
    );
}

#[test]
fn destination_existence_is_checked_on_request_not_on_read() {
    let dir = tempdir().expect("временный каталог");
    let folder = dir.path().join("папка");
    fs::create_dir(&folder).expect("папка");
    let path = folder.to_str().expect("UTF-8").to_owned();
    write_file(
        &dir,
        &serde_json::to_vec(&json!({
            "version": 1,
            "destinationFolder": { "kind": "custom", "path": path },
        }))
        .expect("фикстура"),
    );
    fs::remove_dir(&folder).expect("папку убрали до чтения");

    let store = open(dir.path());
    let destination = store.current().destination().clone();

    assert_eq!(
        destination,
        custom(&path),
        "выбор забыт из-за отсутствия папки"
    );
    assert_eq!(store.readout().reset_fields, Vec::new());
    assert!(!destination_exists(&destination, None));
    fs::create_dir(&folder).expect("папка вернулась");
    assert!(destination_exists(&destination, None));

    assert!(destination_exists(&Destination::System, Some(dir.path())));
    assert!(!destination_exists(
        &Destination::System,
        Some(&dir.path().join("нет"))
    ));
    assert!(!destination_exists(&Destination::System, None));
}

// ───────────────────────────── атомарность ─────────────────────────────

/// Отказ `rename` на последнем шаге: прежний файл байт в байт, временного
/// нет, память и пометки прежние.
#[test]
fn a_failed_rename_keeps_the_previous_file_byte_for_byte() {
    let dir = tempdir().expect("временный каталог");
    let mut object = valid_object();
    object.insert(KEY_ATTEMPTS.to_owned(), json!(0));
    write_object(&dir, &object);
    let before = fs::read(settings_path(&dir)).expect("файл");
    let store = SettingsStore::open_with(dir.path(), failing_rename, fixed_label);

    let result = store.set(&SettingsPatch::NameTemplate("{id}".to_owned()));

    assert!(
        matches!(result, Err(SettingsSetError::WriteFailed { .. })),
        "{result:?}"
    );
    assert_eq!(fs::read(settings_path(&dir)).expect("файл"), before);
    assert_eq!(names(dir.path()), vec![SETTINGS_FILE_NAME.to_owned()]);
    assert_eq!(store.current().name_template(), "{id} — {title}");
    assert_eq!(
        store.readout().reset_fields,
        vec![SettingsField::MaxAttempts]
    );
}

/// Каталог только на чтение: ни временный файл, ни `rename` невозможны, а
/// прямая запись в уже существующий файл — возможна. Потому этот тест и
/// ловит «писать сразу в целевой файл».
#[cfg(unix)]
#[test]
fn a_read_only_directory_keeps_the_previous_file_byte_for_byte() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempdir().expect("временный каталог");
    write_object(&dir, &valid_object());
    let before = fs::read(settings_path(&dir)).expect("файл");
    let store = open(dir.path());

    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o555)).expect("только чтение");
    let result = store.set(&SettingsPatch::MaxAttempts(9));
    let after = fs::read(settings_path(&dir));
    let listing = names(dir.path());
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).expect("права назад");

    assert!(
        matches!(result, Err(SettingsSetError::WriteFailed { .. })),
        "{result:?}"
    );
    assert_eq!(after.expect("файл"), before);
    assert_eq!(listing, vec![SETTINGS_FILE_NAME.to_owned()]);
    assert_eq!(store.current().max_attempts(), 3);
}

/// Отказ записи поверх неотложенного мусора: копия сделана, повтор не
/// плодит вторую, удачный повтор пишет файл.
#[cfg(unix)]
#[test]
fn garbage_that_could_not_be_set_aside_is_copied_once_before_the_save() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempdir().expect("временный каталог");
    let garbage = b"not json at all".as_slice();
    write_file(&dir, garbage);

    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o555)).expect("только чтение");
    let store = open(dir.path());
    let problem = store.open_problem().cloned();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).expect("права назад");

    assert!(
        matches!(
            problem,
            Some(WholeFileProblem::Malformed {
                set_aside: Err(_),
                ..
            })
        ),
        "{problem:?}"
    );
    assert_eq!(
        fs::read(settings_path(&dir)).expect("мусор на месте"),
        garbage
    );

    let failing = SettingsStore {
        rename: failing_rename,
        ..store
    };
    assert!(failing.set(&SettingsPatch::MaxAttempts(2)).is_err());
    let store = SettingsStore {
        rename: real_rename,
        ..failing
    };
    store.set(&SettingsPatch::MaxAttempts(2)).expect("повтор");

    assert_eq!(fs::read(broken_path(&dir, "")).expect("копия"), garbage);
    assert!(
        !broken_path(&dir, "-1").exists(),
        "повтор сделал вторую копию"
    );
    assert_eq!(open(dir.path()).current().max_attempts(), 2);
}

#[test]
fn a_directory_in_place_of_the_file_fails_the_save_without_leftovers() {
    let dir = tempdir().expect("временный каталог");
    fs::create_dir(settings_path(&dir)).expect("каталог на месте файла");
    let store = open(dir.path());

    let result = store.set(&SettingsPatch::MaxAttempts(2));

    assert!(
        matches!(result, Err(SettingsSetError::WriteFailed { .. })),
        "{result:?}"
    );
    assert!(!store.temp_path().exists());
    assert!(settings_path(&dir).is_dir());
}

// ───────────────────────────── пометки ─────────────────────────────

/// Пометки выставляет чтение, чтение их не гасит, гасит первое удачное
/// сохранение любого поля — и больше они не возвращаются.
#[test]
fn reset_marks_live_until_the_first_successful_save_of_any_field() {
    let dir = tempdir().expect("временный каталог");
    let mut object = valid_object();
    object.insert(KEY_TEMPLATE.to_owned(), json!("{channel}"));
    write_object(&dir, &object);
    let store = SettingsStore::open_with(dir.path(), failing_rename, fixed_label);

    for _ in 0..2 {
        assert_eq!(
            store.readout().reset_fields,
            vec![SettingsField::NameTemplate]
        );
    }
    assert!(store.set(&SettingsPatch::MaxAttempts(0)).is_err());
    assert!(store.set(&SettingsPatch::MaxAttempts(4)).is_err());
    assert_eq!(
        store.readout().reset_fields,
        vec![SettingsField::NameTemplate],
        "неудачное сохранение сняло пометку"
    );

    let store = SettingsStore {
        rename: real_rename,
        ..store
    };
    store
        .set(&SettingsPatch::MaxAttempts(4))
        .expect("сохранение другого поля");

    for _ in 0..2 {
        let readout = store.readout();
        assert_eq!(readout.reset_fields, Vec::new());
        assert!(!readout.whole_file_reset);
    }
    let reopened = open(dir.path()).readout();
    assert_eq!(reopened.reset_fields, Vec::new());
    assert_eq!(reopened.settings.name_template(), "{title}");
}

#[test]
fn the_whole_file_mark_is_cleared_by_the_first_successful_save() {
    let dir = tempdir().expect("временный каталог");
    write_file(&dir, b"garbage");
    let store = open(dir.path());
    assert!(store.readout().whole_file_reset);

    store
        .set(&SettingsPatch::DestinationFolder(DestinationFolder::System))
        .expect("сохранение");

    assert!(!store.readout().whole_file_reset);
    assert!(
        store.open_problem().is_some(),
        "факт открытия не для экрана"
    );
}

// ───────────────────────────── потребители ─────────────────────────────

#[test]
fn the_contract_form_carries_the_same_values() {
    let dir = tempdir().expect("временный каталог");
    write_object(&dir, &valid_object());

    assert_eq!(
        open(dir.path()).current().to_contract(),
        ContractSettings {
            destination_folder: DestinationFolder::Custom {
                path: absent_absolute()
            },
            name_template: "{id} — {title}".to_owned(),
            max_attempts: 3,
        }
    );
    assert_eq!(
        Settings::default().to_contract().destination_folder,
        DestinationFolder::System
    );
}

/// Основа имени по сохранённому шаблону — тот же конвейер, что у модуля
/// шаблона; `current` видит сохранённое сразу.
#[test]
fn the_file_stem_follows_the_saved_template() {
    let dir = tempdir().expect("временный каталог");
    let store = open(dir.path());
    let ctx = TemplateContext {
        title: "Ролик: часть 1/2",
        video_id: "dQw4w9WgXcQ",
        quality: SelectedQuality {
            kind: QualityKind::Standard,
            height_px: Some(720),
        },
        date: TemplateDate::new(2026, 9, 14).expect("дата"),
    };

    assert_eq!(
        store.current().file_stem(&ctx),
        crate::download::filename::sanitized_stem(ctx.title, ctx.video_id)
    );

    store
        .set(&SettingsPatch::NameTemplate(
            "{id} [{quality}] {date}".to_owned(),
        ))
        .expect("сохранение");

    assert_eq!(
        store.current().file_stem(&ctx),
        "dQw4w9WgXcQ [720p] 2026-09-14"
    );
}
