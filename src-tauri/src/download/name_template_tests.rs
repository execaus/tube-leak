//! Тесты шаблона имени (TL-86): белый список, позиции отказов, тождество
//! умолчания с E3, корпус инъекций с обходом ФС, генерация.

use std::path::{Component, Path, PathBuf};

use super::*;
use crate::commands::{PREVIEW_SAMPLE_QUALITY, PREVIEW_SAMPLE_TITLE, PREVIEW_SAMPLE_VIDEO_ID};
use crate::download::filename::{
    candidate_name, finalize_in_dir, sanitized_stem, MAX_FILE_NAME_BYTES, MAX_STEM_BYTES,
};

fn date() -> TemplateDate {
    TemplateDate::new(2026, 9, 14).expect("дата существует")
}

fn ctx<'a>(title: &'a str, video_id: &'a str) -> TemplateContext<'a> {
    TemplateContext {
        title,
        video_id,
        quality: PREVIEW_SAMPLE_QUALITY,
        date: date(),
    }
}

fn sample() -> TemplateContext<'static> {
    ctx(PREVIEW_SAMPLE_TITLE, PREVIEW_SAMPLE_VIDEO_ID)
}

// --- Разбор -----------------------------------------------------------------

#[test]
fn the_design_examples_render_as_the_design_shows() {
    // Дизайн E5, пункт 3: живой пример на образце предпросмотра.
    let cases = [
        ("{id} — {title}", "dQw4w9WgXcQ — Как приручить дракона"),
        ("{title} [{quality}]", "Как приручить дракона [1080p]"),
        ("{title}", "Как приручить дракона"),
        ("{date} {title}", "2026-09-14 Как приручить дракона"),
    ];
    for (template, expected) in cases {
        assert_eq!(
            validate_for_save(template, &sample()).as_deref(),
            Ok(expected),
            "шаблон {template}"
        );
    }
}

#[test]
fn the_whitelist_is_exactly_the_four_variables_of_f12() {
    // Ф-12 буквально. Расширение списка — отдельное решение, и этот тест
    // обязан покраснеть вместе с ним.
    let names: Vec<&str> = Variable::ALL.iter().map(|v| v.name()).collect();
    assert_eq!(names, ["title", "id", "quality", "date"]);

    for variable in Variable::ALL {
        let template = format!("{{{}}}", variable.name());
        let parsed = NameTemplate::parse(&template).expect("имя из белого списка");
        assert_eq!(parsed.segments(), [Segment::Variable(variable)]);
        assert_eq!(parsed.as_str(), template);
    }
}

#[test]
fn every_name_outside_the_whitelist_is_an_unknown_variable() {
    // Правдоподобные соседи: переменные yt-dlp, регистр, пробелы, пути,
    // окружение. Ни одна не должна пройти как «похожая».
    let rejected = [
        "",
        "channel",
        "ext",
        "upload_date",
        "duration",
        "uploader",
        "playlist",
        "format_id",
        "Title",
        "TITLE",
        " title",
        "title ",
        "titles",
        "tit le",
        "id/..",
        "../",
        "$HOME",
        "%(title)s",
        "\u{202e}eltit",
        "title\u{0}",
        "ид",
    ];
    for name in rejected {
        let template = format!("до {{{name}}} после");
        assert_eq!(
            NameTemplate::parse(&template),
            Err(TemplateProblem::UnknownVariable {
                position: 4,
                name: name.to_string()
            }),
            "имя {name:?}"
        );
    }
}

#[test]
fn problems_carry_the_position_in_unicode_characters() {
    use TemplateProblem::{StrayClosingBrace, UnclosedBrace, UnknownVariable};

    let unknown = |position, name: &str| UnknownVariable {
        position,
        name: name.to_string(),
    };
    let cases = [
        ("{channel}", unknown(1, "channel")),
        // Кириллица — два байта на символ, позиция всё равно в символах.
        ("Видео {channel}", unknown(7, "channel")),
        // Эмодзи — четыре байта UTF-8 и две единицы UTF-16, но один символ.
        ("🎬 {ext}", unknown(3, "ext")),
        ("{}", unknown(1, "")),
        ("{title", UnclosedBrace { position: 1 }),
        ("{title} {id", UnclosedBrace { position: 9 }),
        ("{ti{tle}", UnclosedBrace { position: 1 }),
        ("{{title}}", UnclosedBrace { position: 1 }),
        ("видео {", UnclosedBrace { position: 7 }),
        ("title}", StrayClosingBrace { position: 6 }),
        ("{title}}", StrayClosingBrace { position: 8 }),
        ("}{title}", StrayClosingBrace { position: 1 }),
        // Первая проблема слева, а не «самая важная».
        ("{channel} {", unknown(1, "channel")),
        ("{title} } {channel}", StrayClosingBrace { position: 9 }),
        // Синтаксис раньше «нет переменных».
        ("{channel} видео", unknown(1, "channel")),
    ];
    for (template, expected) in cases {
        assert_eq!(
            NameTemplate::parse(template),
            Err(expected),
            "шаблон {template:?}"
        );
    }
}

#[test]
fn a_template_without_variables_is_rejected() {
    // С-9: все файлы получили бы одно имя. Сюда же — литералы, которые
    // выглядят как путь: отказ раньше, чем до них дойдёт санитизация.
    for template in [
        "",
        "видео",
        "   ",
        "../../etc/passwd",
        "/",
        "C:\\Windows",
        "CON",
        "%(ext)s",
    ] {
        assert_eq!(
            NameTemplate::parse(template),
            Err(TemplateProblem::NoVariables),
            "шаблон {template:?}"
        );
        assert_eq!(
            validate_for_save(template, &sample()),
            Err(TemplateProblem::NoVariables)
        );
    }
}

#[test]
fn a_huge_position_saturates_instead_of_wrapping() {
    assert_eq!(position_of(0), 1);
    assert_eq!(position_of(usize::MAX), u32::MAX);
}

#[test]
fn the_default_template_is_title_and_equals_its_parse() {
    assert_eq!(DEFAULT_TEMPLATE, "{title}");
    assert_eq!(
        NameTemplate::parse(DEFAULT_TEMPLATE).as_ref(),
        Ok(&NameTemplate::default())
    );
}

// --- Подстановка ------------------------------------------------------------

#[test]
fn a_value_is_never_parsed_as_a_template() {
    // Данные не проходят разбор второй раз: скобки в названии и id — текст.
    let template = NameTemplate::parse("{title} {id}").expect("шаблон верный");
    assert_eq!(
        template.file_stem(&ctx("{id} и {date}", "{title}")),
        "{id} и {date} {title}"
    );
    // И незакрытая скобка в значении — не ошибка, ошибок у подстановки нет.
    assert_eq!(template.file_stem(&ctx("}{", "{")), "}{ {");
}

#[test]
fn the_quality_label_has_one_form_per_kind() {
    let label = |kind, height_px| quality_label(SelectedQuality { kind, height_px });
    assert_eq!(label(QualityKind::Standard, Some(1080)), "1080p");
    assert_eq!(label(QualityKind::Standard, Some(2160)), "2160p");
    assert_eq!(label(QualityKind::MaxAvailable, Some(480)), "480p");
    assert_eq!(label(QualityKind::AudioOnly, None), "audio");
    assert_eq!(label(QualityKind::AudioOnly, Some(1080)), "audio");
    assert_eq!(label(QualityKind::Standard, None), "");
    assert_eq!(label(QualityKind::MaxAvailable, None), "");
}

#[test]
fn the_date_is_a_real_zero_padded_calendar_day() {
    let formatted = |y, m, d| TemplateDate::new(y, m, d).map(|date| date.to_string());
    assert_eq!(formatted(2026, 1, 5).as_deref(), Some("2026-01-05"));
    assert_eq!(formatted(987, 12, 31).as_deref(), Some("0987-12-31"));
    assert_eq!(formatted(2024, 2, 29).as_deref(), Some("2024-02-29"));
    assert_eq!(formatted(2000, 2, 29).as_deref(), Some("2000-02-29"));
    for (y, m, d) in [
        (2026, 2, 29),
        (1900, 2, 29),
        (2026, 4, 31),
        (2026, 0, 1),
        (2026, 13, 1),
        (2026, 1, 0),
        (2026, 1, 32),
        (0, 1, 1),
        (10_000, 1, 1),
    ] {
        assert_eq!(formatted(y, m, d), None, "{y}-{m}-{d}");
    }
}

#[test]
fn an_empty_stem_falls_back_to_the_video_id() {
    // Ф-12: пустая основа — не ошибка шаблона, а запасное имя E3.
    let no_height = SelectedQuality {
        kind: QualityKind::Standard,
        height_px: None,
    };
    let quality_only = NameTemplate::parse("{quality}").expect("шаблон верный");
    let with_empty_quality = TemplateContext {
        quality: no_height,
        ..sample()
    };
    assert_eq!(
        quality_only.file_stem(&with_empty_quality),
        "video-dQw4w9WgXcQ"
    );

    let dotted = NameTemplate::parse("  ..{quality}..  ").expect("шаблон верный");
    assert_eq!(dotted.file_stem(&with_empty_quality), "video-dQw4w9WgXcQ");

    let title = NameTemplate::default();
    assert_eq!(
        title.file_stem(&ctx("???", "dQw4w9WgXcQ")),
        "video-dQw4w9WgXcQ"
    );
    assert_eq!(title.file_stem(&ctx("", "")), "video");
}

// --- Тождество умолчания с E3 -------------------------------------------------

/// Все значения `"title"` из JSON-фикстур `tests/fixtures/`, на любой
/// глубине. Порог в тесте ловит пропажу набора.
fn fixture_titles() -> Vec<String> {
    fn walk(dir: &Path, out: &mut Vec<String>) {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
            .expect("каталог фикстур читается")
            .map(|entry| entry.expect("запись каталога").path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "json") {
                let text = std::fs::read_to_string(&path).expect("фикстура читается");
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                    collect(&value, out);
                }
            }
        }
    }
    fn collect(value: &serde_json::Value, out: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, inner) in map {
                    if let (true, Some(title)) = (key == "title", inner.as_str()) {
                        out.push(title.to_string());
                    }
                    collect(inner, out);
                }
            }
            serde_json::Value::Array(items) => items.iter().for_each(|item| collect(item, out)),
            _ => {}
        }
    }

    let mut titles = Vec::new();
    walk(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"),
        &mut titles,
    );
    titles.sort();
    titles.dedup();
    titles
}

/// Корпус названий E3 целиком: безобидные и враждебные названия санитизации
/// (TL-40), названия, раскрывавшиеся шаблоном yt-dlp (ревью TL-44), и живые
/// названия из фикстур.
fn e3_title_corpus() -> Vec<String> {
    use crate::download::filename::tests::{HOSTILE_TITLES, ORDINARY_TITLES};
    use crate::download::orchestrate::tests::HOSTILE_TITLES as TEMPLATE_HOSTILE_TITLES;

    ORDINARY_TITLES
        .iter()
        .chain(HOSTILE_TITLES.iter())
        .chain(TEMPLATE_HOSTILE_TITLES.iter())
        .map(ToString::to_string)
        .chain(fixture_titles())
        .collect()
}

#[test]
fn the_default_template_names_files_exactly_as_e3_did() {
    // К-5, Ф-12: пользователь без настроек получает байт в байт прежние имена.
    let fixtures = fixture_titles();
    assert!(
        fixtures.len() >= 20,
        "из фикстур пропали названия: {} вместо 20+",
        fixtures.len()
    );

    let corpus: Vec<String> = e3_title_corpus()
        .into_iter()
        .chain(injection_values())
        .collect();
    let template = NameTemplate::default();

    for title in &corpus {
        for id in INJECTION_IDS {
            assert_eq!(
                template.file_stem(&ctx(title, id)),
                sanitized_stem(title, id),
                "название {title:?}, id {id:?}"
            );
        }
    }
}

// --- Корпус инъекций и обход ФС ---------------------------------------------

/// Id — тоже непроверенный ввод: он идёт и в `{id}`, и в запасное имя.
const INJECTION_IDS: [&str; 3] = ["dQw4w9WgXcQ", "", "../../etc"];

/// Значения переменных, каждое из которых ломает наивную склейку пути.
fn injection_values() -> Vec<String> {
    let fixed = [
        "",
        "../",
        "..\\",
        "..",
        ".",
        "../../../etc/passwd",
        "..\\..\\Windows\\System32",
        "/etc/passwd",
        "/",
        "\\",
        "C:\\Windows\\System32\\drivers",
        "C:",
        "\\\\server\\share",
        ":",
        "a:b",
        "имя\u{0}с\u{0}нулями",
        "\u{1}\u{1f}\u{7f}\u{9b}",
        "две\nстроки\r\tи таб",
        "CON",
        "con",
        "NUL.txt",
        "COM1",
        "LPT9.mp4",
        "CONIN$",
        "точка в конце.",
        "пробел в конце ",
        "...",
        ". . .",
        "отчёт\u{202e}gpj.exe",
        "\u{202e}",
        "\u{feff}\u{200b}\u{ad}",
        "{",
        "}",
        "{title}",
        "{id}{",
        "$HOME",
        "${HOME}",
        "~/",
        "~",
        "%(ext)s",
        "%HOME%",
    ];
    fixed
        .iter()
        .map(ToString::to_string)
        .chain([
            "я".repeat(400),
            "a".repeat(1000),
            "🎬".repeat(300),
            ".".repeat(500),
            "/".repeat(300),
            "../".repeat(200),
        ])
        .collect()
}

/// Шаблоны с переменными, чьи литералы сами несут инъекцию.
fn injection_templates() -> Vec<String> {
    let fixed = [
        "{title}",
        "{id} — {title}",
        "{title} [{quality}]",
        "{date} {title}",
        "../{title}",
        "..\\{title}",
        "{title}/../../x",
        "/etc/{title}",
        "/{title}",
        "{title}/",
        "C:\\Windows\\{title}",
        "\\\\server\\share\\{id}",
        "{title}\\{id}",
        "{title}:{id}",
        "a\u{0}{title}",
        "{title}\u{1}\u{7f}",
        "CON{title}",
        "COM1.{id}",
        "NUL.txt{title}",
        "{title}.",
        "{title} ",
        "{title}. . .",
        "..{title}..",
        "\u{202e}{title}",
        "$HOME/{title}",
        "$${title}",
        "~/{title}",
        "%(ext)s{title}",
        "{title}%(title)s",
        "{quality}/{date}/{id}",
    ];
    fixed
        .iter()
        .map(ToString::to_string)
        .chain([format!("{}{{title}}", "x".repeat(500))])
        .collect()
}

/// Основа — один инертный компонент пути, и E3 ничего бы в ней не поменял.
///
/// Последняя проверка (неподвижная точка `candidate_name`) — та, что ловит
/// обход санитизации: без неё финализация почистила бы имя сама, и файл на
/// диске не выдал бы, что основа пришла сырой.
fn assert_inert_stem(stem: &str, context: &str) {
    assert!(!stem.is_empty(), "{context}: пустая основа");
    assert!(!stem.contains('/'), "{context}: {stem:?}");
    assert!(!stem.contains('\\'), "{context}: {stem:?}");
    assert!(!stem.contains(".."), "{context}: {stem:?}");
    assert!(!stem.starts_with('.'), "{context}: {stem:?}");
    assert!(!stem.ends_with(['.', ' ']), "{context}: {stem:?}");
    assert!(!stem.chars().any(char::is_control), "{context}: {stem:?}");
    assert!(
        stem.len() <= MAX_STEM_BYTES,
        "{context}: {} байт",
        stem.len()
    );
    let components: Vec<Component<'_>> = Path::new(stem).components().collect();
    assert!(
        matches!(components.as_slice(), [Component::Normal(_)]),
        "{context}: {stem:?} → {components:?}"
    );
    assert_eq!(
        candidate_name(stem, 1, ""),
        stem,
        "{context}: основа не прошла конвейер E3"
    );
}

/// Всё под `dir` рекурсивно, без перехода по ссылкам.
fn walk_tree(dir: &Path, out: &mut Vec<(PathBuf, std::fs::FileType)>) {
    for entry in std::fs::read_dir(dir).expect("каталог читается") {
        let entry = entry.expect("запись каталога");
        let kind = entry.file_type().expect("тип записи");
        let path = entry.path();
        out.push((path.clone(), kind));
        if kind.is_dir() {
            walk_tree(&path, out);
        }
    }
}

#[test]
fn rejected_injection_templates_fail_with_their_problem() {
    let unknown = |position, name: &str| TemplateProblem::UnknownVariable {
        position,
        name: name.to_string(),
    };
    let cases = [
        ("../../etc/passwd", TemplateProblem::NoVariables),
        ("C:\\x\\y", TemplateProblem::NoVariables),
        ("NUL.txt", TemplateProblem::NoVariables),
        ("%(ext)s", TemplateProblem::NoVariables),
        ("{../}", unknown(1, "../")),
        ("{title/..}", unknown(1, "title/..")),
        ("{$HOME}", unknown(1, "$HOME")),
        ("${HOME}", unknown(2, "HOME")),
        ("{\u{0}}", unknown(1, "\u{0}")),
        ("{title", TemplateProblem::UnclosedBrace { position: 1 }),
        ("../{title", TemplateProblem::UnclosedBrace { position: 4 }),
        (
            "{title}}/",
            TemplateProblem::StrayClosingBrace { position: 8 },
        ),
    ];
    for (template, expected) in cases {
        assert_eq!(NameTemplate::parse(template), Err(expected), "{template:?}");
    }
}

#[test]
fn no_injected_template_or_value_leaves_the_destination() {
    // К-5: проверяется не строкой, а файловой системой. Каждая основа
    // становится настоящим файлом через финализацию E3, после чего дерево
    // обходится целиком: вне своих папок назначения ничего нет, подпапок
    // нет, файлов ровно столько, сколько финализаций.
    let root = tempfile::tempdir().expect("временный каталог");
    let sandbox = root.path().join("sandbox");
    std::fs::create_dir(&sandbox).expect("песочница");

    let templates = injection_templates();
    let values = injection_values();
    let mut finalized = 0usize;

    for (t_index, source) in templates.iter().enumerate() {
        let template = NameTemplate::parse(source)
            .unwrap_or_else(|problem| panic!("шаблон {source:?} обязан быть верным: {problem:?}"));
        let dest = sandbox.join(format!("dest-{t_index}"));
        std::fs::create_dir(&dest).expect("папка назначения");

        for (v_index, value) in values.iter().enumerate() {
            // Значение идёт и названием (с враждебным id по кругу), и id (с
            // обычным названием) — через разные переменные одного шаблона.
            // Не полное произведение: полное — пять секунд на обычном прогоне,
            // а произведение целиком закрывает генерация.
            let rotating_id = INJECTION_IDS[v_index % INJECTION_IDS.len()];
            for (title, video_id) in [
                (value.as_str(), rotating_id),
                (PREVIEW_SAMPLE_TITLE, value.as_str()),
            ] {
                let context = format!("шаблон {source:?}, название {title:?}, id {video_id:?}");
                let stem = template.file_stem(&ctx(title, video_id));
                assert_inert_stem(&stem, &context);

                let ready = dest.join(format!("ready-{v_index}.tl-merging.mp4"));
                std::fs::write(&ready, context.as_bytes()).expect("рабочий файл");
                let name = finalize_in_dir(&dest, &ready, &stem, "mp4")
                    .unwrap_or_else(|error| panic!("{context}: {error}"));
                finalized += 1;

                // На диск легла ровно та основа, что вычислена: без
                // суффикса или с « (N)», но не почищенная заново.
                let rest = name
                    .strip_prefix(stem.as_str())
                    .and_then(|rest| rest.strip_suffix(".mp4"))
                    .unwrap_or_else(|| panic!("{context}: имя {name:?} не из основы {stem:?}"));
                assert!(
                    rest.is_empty()
                        || rest
                            .strip_prefix(" (")
                            .and_then(|n| n.strip_suffix(')'))
                            .is_some_and(|n| n.parse::<u32>().is_ok()),
                    "{context}: хвост {rest:?}"
                );
                assert!(dest.join(&name).is_file(), "{context}: файла нет");
            }
        }
    }

    let mut root_entries = Vec::new();
    walk_tree(root.path(), &mut root_entries);

    let mut files = 0usize;
    let mut dirs = 0usize;
    for (path, kind) in &root_entries {
        let relative = path.strip_prefix(root.path()).expect("внутри корня");
        let parts: Vec<Component<'_>> = relative.components().collect();
        if kind.is_dir() {
            dirs += 1;
            let is_sandbox = parts.len() == 1 && path == &sandbox;
            let is_dest = parts.len() == 2 && path.parent() == Some(sandbox.as_path());
            assert!(is_sandbox || is_dest, "лишний каталог: {relative:?}");
        } else {
            assert!(kind.is_file(), "не обычный файл: {relative:?}");
            assert_eq!(parts.len(), 3, "файл вне папки назначения: {relative:?}");
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            assert!(
                !name.contains(".tl-merging"),
                "рабочий файл не финализирован: {relative:?}"
            );
            files += 1;
        }
    }
    assert_eq!(dirs, templates.len() + 1, "подпапки появились или пропали");
    assert_eq!(files, finalized, "файлов не столько, сколько финализаций");
    assert!(finalized >= 2_500, "корпус выродился: {finalized}");
}

// --- Генерация ----------------------------------------------------------------

/// SplitMix64: детерминированный генератор без зависимостей. Сид в
/// сообщении об отказе воспроизводит пару.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        let index = usize::try_from(self.below(items.len() as u64)).unwrap_or(0);
        &items[index]
    }
}

/// Куски, из которых собираются и литералы шаблона, и значения.
const FRAGMENTS: &[&str] = &[
    "/",
    "\\",
    ".",
    "..",
    "../",
    "..\\",
    " ",
    ":",
    "\u{0}",
    "\n",
    "\t",
    "\u{7f}",
    "\u{9b}",
    "\u{202e}",
    "\u{200b}",
    "\u{feff}",
    "\u{ad}",
    "CON",
    "nul",
    "COM1",
    "LPT¹",
    "CONIN$",
    "<",
    ">",
    "|",
    "?",
    "*",
    "\"",
    "~",
    "$HOME",
    "${HOME}",
    "%(ext)s",
    "%",
    "C:",
    "\\\\",
    "…",
    "_",
    "-",
    "Ролик",
    "video",
    "🎬",
    "👨\u{200d}👩",
    "日本",
    "a",
    "Z",
    "0",
    "(2)",
];

fn random_char(rng: &mut SplitMix64) -> char {
    let code = match rng.below(5) {
        0 => rng.below(0x80),
        1 => rng.below(0x800),
        2 => 0x2000 + rng.below(0x80),
        3 => rng.below(0x1_0000),
        _ => 0x1_0000 + rng.below(0x1_0000),
    };
    char::from_u32(u32::try_from(code).unwrap_or(0)).unwrap_or('\u{fffd}')
}

fn random_text(rng: &mut SplitMix64, pieces: u64) -> String {
    let mut out = String::new();
    for _ in 0..rng.below(pieces + 1) {
        match rng.below(10) {
            0..=5 => out.push_str(rng.pick(FRAGMENTS)),
            6..=8 => out.push(random_char(rng)),
            _ => {
                let piece = *rng.pick(FRAGMENTS);
                let times = usize::try_from(1 + rng.below(120)).unwrap_or(1);
                out.push_str(&piece.repeat(times));
            }
        }
    }
    out
}

fn random_template(rng: &mut SplitMix64) -> String {
    let mut out = String::new();
    for _ in 0..rng.below(7) {
        match rng.below(20) {
            0..=6 => {
                out.push('{');
                out.push_str(rng.pick(&Variable::ALL).name());
                out.push('}');
            }
            7 => out.push_str(rng.pick(&["{", "}", "{}", "{x}", "{title", "title}"])),
            _ => out.push_str(&random_text(rng, 3)),
        }
    }
    out
}

fn random_context_parts(rng: &mut SplitMix64) -> (String, String, SelectedQuality, TemplateDate) {
    let title = random_text(rng, 8);
    let id = random_text(rng, 3);
    let kind = *rng.pick(&[
        QualityKind::Standard,
        QualityKind::MaxAvailable,
        QualityKind::AudioOnly,
    ]);
    let height_px = match rng.below(4) {
        0 => None,
        _ => Some(u32::try_from(rng.below(5000)).unwrap_or(0)),
    };
    let date = TemplateDate::new(
        u16::try_from(1 + rng.below(9999)).unwrap_or(1),
        u8::try_from(1 + rng.below(12)).unwrap_or(1),
        u8::try_from(1 + rng.below(28)).unwrap_or(1),
    )
    .expect("до 28-го числа любой месяц годится");
    (title, id, SelectedQuality { kind, height_px }, date)
}

/// Счётчики прогона: сколько пар и сколько из них действительно несли то,
/// от чего конвейер защищает. Без них генератор, разучившийся порождать
/// опасное, проходил бы вечно зелёным.
#[derive(Debug, Default)]
struct GenerationStats {
    pairs: u64,
    valid: u64,
    raw_with_separator: u64,
    raw_with_dot_dot: u64,
    fallbacks: u64,
}

fn run_generated_pairs(seed: u64, pairs: u64) -> GenerationStats {
    let mut rng = SplitMix64(seed);
    let mut stats = GenerationStats::default();

    for iteration in 0..pairs {
        stats.pairs += 1;
        let source = random_template(&mut rng);
        let (title, id, quality, date) = random_context_parts(&mut rng);
        let context = TemplateContext {
            title: &title,
            video_id: &id,
            quality,
            date,
        };
        let where_ = || {
            format!(
                "сид {seed}, пара {iteration}: шаблон {source:?}, название {title:?}, id {id:?}"
            )
        };

        let template = match NameTemplate::parse(&source) {
            Ok(template) => template,
            Err(problem) => {
                let length = source.chars().count();
                match problem {
                    TemplateProblem::UnknownVariable { position, .. }
                    | TemplateProblem::UnclosedBrace { position }
                    | TemplateProblem::StrayClosingBrace { position } => {
                        let at = usize::try_from(position).unwrap_or(usize::MAX);
                        assert!((1..=length).contains(&at), "{}: {problem:?}", where_());
                    }
                    TemplateProblem::NoVariables => {
                        assert!(!source.contains('{'), "{}: {problem:?}", where_());
                    }
                    // Предел длины проверяет хранилище настроек до разбора
                    // (TL-87), сам разбор длину не ограничивает.
                    TemplateProblem::TooLong { .. } => {
                        panic!("{}: разбор вернул {problem:?}", where_());
                    }
                }
                continue;
            }
        };
        stats.valid += 1;

        let raw = template.substituted(&context);
        stats.raw_with_separator += u64::from(raw.contains(['/', '\\']));
        stats.raw_with_dot_dot += u64::from(raw.contains(".."));

        let stem = template.file_stem(&context);
        stats.fallbacks += u64::from(stem.starts_with("video"));

        for (index, extension) in [(1, "mp4"), (LAST_INDEX, "webm")] {
            let name = candidate_name(&stem, index, extension);
            let separator_free = !name.contains(['/', '\\']) && !stem.contains(['/', '\\']);
            let no_dot_dot = !name.contains("..") && !stem.contains("..");
            let one_component = matches!(
                Path::new(&name).components().collect::<Vec<_>>().as_slice(),
                [Component::Normal(_)]
            );
            if !(separator_free
                && no_dot_dot
                && one_component
                && !stem.is_empty()
                && stem.len() <= MAX_STEM_BYTES
                && name.len() <= MAX_FILE_NAME_BYTES
                && candidate_name(&stem, 1, "") == stem)
            {
                panic!("{}: основа {stem:?}, имя {name:?}", where_());
            }
        }
    }
    stats
}

const LAST_INDEX: u32 = crate::download::filename::LAST_COLLISION_INDEX;

fn assert_generator_reaches_the_danger(stats: &GenerationStats) {
    assert!(
        stats.valid * 3 >= stats.pairs,
        "мало верных шаблонов: {stats:?}"
    );
    assert!(
        stats.raw_with_separator * 5 >= stats.valid,
        "мало разделителей: {stats:?}"
    );
    assert!(
        stats.raw_with_dot_dot * 10 >= stats.valid,
        "мало `..`: {stats:?}"
    );
    assert!(
        stats.fallbacks > 0,
        "запасное имя не встретилось: {stats:?}"
    );
}

#[test]
fn generated_templates_and_values_never_make_a_path() {
    // Репрезентативная выборка для обычного прогона. Миллионы — в
    // `millions_of_generated_pairs_never_make_a_path` под `#[ignore]`.
    let stats = run_generated_pairs(0x7486_0001, 20_000);
    assert_generator_reaches_the_danger(&stats);
}

/// Большой прогон К-5. Запуск:
///
/// ```text
/// cd src-tauri && cargo test --locked name_template::tests::millions -- --ignored --nocapture
/// ```
///
/// Число пар — `TUBE_LEAK_TEMPLATE_PAIRS` (умолчание 5 000 000), делится
/// между потоками с разными сидами.
#[test]
#[ignore = "миллионы пар, минуты работы; команда запуска в doc теста"]
fn millions_of_generated_pairs_never_make_a_path() {
    let total: u64 = std::env::var("TUBE_LEAK_TEMPLATE_PAIRS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(5_000_000);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get() as u64);
    let per_thread = total.div_ceil(threads);
    let started = std::time::Instant::now();

    let stats: Vec<GenerationStats> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|thread| {
                scope.spawn(move || run_generated_pairs(0x7486_1000 + thread, per_thread))
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("поток генерации"))
            .collect()
    });

    let mut sum = GenerationStats::default();
    for part in &stats {
        sum.pairs += part.pairs;
        sum.valid += part.valid;
        sum.raw_with_separator += part.raw_with_separator;
        sum.raw_with_dot_dot += part.raw_with_dot_dot;
        sum.fallbacks += part.fallbacks;
    }
    eprintln!(
        "шаблон имени: {sum:?}, потоков {threads}, за {:.1} с",
        started.elapsed().as_secs_f64()
    );
    assert_generator_reaches_the_danger(&sum);
}

#[test]
fn a_civil_date_from_the_clock_becomes_a_template_date_only_within_its_range() {
    // Одно преобразование тройки `clock::today_utc` на предпросмотр (TL-91)
    // и оркестрацию (TL-89).
    assert_eq!(
        TemplateDate::from_civil((2026, 9, 14)).map(|date| date.to_string()),
        Some("2026-09-14".to_string())
    );
    assert_eq!(
        TemplateDate::from_civil((2028, 2, 29)).map(|date| date.to_string()),
        Some("2028-02-29".to_string())
    );
    for outside in [
        (10_000, 1, 1),
        (0, 1, 1),
        (-1, 1, 1),
        (2026, 13, 1),
        (2026, 2, 30),
    ] {
        assert_eq!(TemplateDate::from_civil(outside), None, "{outside:?}");
    }
    assert_eq!(TemplateDate::UNIX_EPOCH.to_string(), "1970-01-01");
}
