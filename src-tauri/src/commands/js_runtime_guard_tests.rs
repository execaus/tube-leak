//! Сторож TL-114 (#121): каждый продакшен-запуск yt-dlp получает JS-рантайм,
//! собранный от приложения, а не подставленный.
//!
//! # Что держит конструкция, а что — этот сторож
//!
//! Запускатели разбора (`SidecarLauncher::new`) и скачивания
//! (`SidecarDownloader::new`) принимают `YtDlpJsRuntime`, а у него в
//! продакшене один конструктор — `YtDlpJsRuntime::for_app(&AppHandle)`, резолв
//! внутри. `from_deno` и `disabled` приватны модулю `sidecar::deno`, поэтому
//! мутация ревью TL-109 `from_deno(Err(NotFound))` в `commands/queue.rs` не
//! компилируется ни в какой сборке. Промежуточная переменная, вызов на
//! несколько строк, `Self::new`, псевдоним импорта запускателя — формы того
//! же значения из `for_app`, и сторож на них не смотрит, поэтому и не краснеет.
//!
//! Щель у конструкции одна: тестовая сборка компилирует продакшен-код с
//! `cfg(test)`, и тот видит тестовый конструктор `for_tests`. Первый рубеж
//! здесь — компилятор: полный `cargo test` заодно собирает обычный бинарник
//! без `cfg(test)` (он нужен интеграционным тестам `tests/`), и
//! `cargo build`/`cargo clippy` тоже, так что такой вызов падает с E0599
//! (замер TL-114). Но этот рубеж держится на соседних обстоятельствах — есть
//! ли каталог `tests/`, не сужен ли прогон до `--bin tube-leak`, — поэтому
//! второй рубеж — правила ниже; их мутации доказаны прогоном
//! `cargo test --bin tube-leak`, где компилятор щель не закрывает.
//!
//! # Правила
//!
//! Строки делятся на продакшен и тесты по раскладке rustfmt (`cargo fmt
//! --check` — обязательный прогон): элемент верхнего уровня начинается в
//! колонке 0, и он тестовый, если над ним стоит ровно `#[cfg(test)]`.
//! Файлы `*_tests.rs` — тестовые, если смонтированы под `#[cfg(test)]`.
//!
//! - П-1. Тестовый конструктор в продакшен-строках не упоминается; его
//!   определение одно, в `sidecar/deno.rs`, под `#[cfg(test)]`.
//! - П-2. В тестовом коде он живёт только внутри модулей `…tests`
//!   (встроенных или смонтированных файлов): обёртку из `pub mod testing`
//!   продакшен позвать мог бы, а модуль тестов — только по пути.
//! - П-3. Вне модулей `…tests` путей в них нет: ни `…tests::`, ни `use` со
//!   словом на `tests`, — включая тестовые реэкспорты.
//! - П-4. `impl YtDlpJsRuntime` один, в `sidecar/deno.rs`, со сверенным
//!   списком функций; реализаций трейтов для рантайма нет нигде (иначе
//!   тестовый `impl` дал бы продакшену конструктор); поле `runtime`
//!   заполняется только в `from_deno` и `disabled_with`.
//! - П-5. Тела `DenoLaunch::for_app` и `YtDlpJsRuntime::for_app` сверяются по
//!   тексту: `AppHandle` в тестах нет, и исполнить их тест не может.
//!
//! # Чего сторож не видит
//!
//! - Макрос, склеивающий имя тестового конструктора из частей.
//! - `unsafe` (`transmute`, `zeroed`) вместо конструктора.
//! - `#[cfg(all(test, …))]` и прочие составные условия считаются продакшеном:
//!   это ложный красный, а не слепота.

use std::fs;
use std::path::Path;

/// Имя тестового конструктора — склеено, чтобы файл сторожа не был
/// находкой сам для себя.
const FOR_TESTS: &str = concat!("for_", "tests");

/// Тип рантайма.
const RUNTIME: &str = "YtDlpJsRuntime";

struct Source {
    /// Путь относительно `src/` с `/`.
    rel: String,
    text: String,
}

impl Source {
    fn is_tests_file(&self) -> bool {
        self.rel.ends_with("_tests.rs")
    }

    fn dir(&self) -> &str {
        self.rel.rsplit_once('/').map_or("", |(dir, _)| dir)
    }
}

fn sources() -> Vec<Source> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found = Vec::new();
    let mut stack = vec![src.clone()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("каталог обходится") {
            let path = entry.expect("элемент").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                found.push(Source {
                    rel: path
                        .strip_prefix(&src)
                        .expect("под src")
                        .to_string_lossy()
                        .replace('\\', "/"),
                    text: fs::read_to_string(&path).expect("исходник читается"),
                });
            }
        }
    }
    found.sort_by(|a, b| a.rel.cmp(&b.rel));
    found
}

/// Непустая строка исходника с принадлежностью к элементу верхнего уровня.
struct Line<'a> {
    number: usize,
    indent: usize,
    code: &'a str,
    /// Элемент верхнего уровня стоит под `#[cfg(test)]`.
    gated: bool,
    /// Первая строка элемента верхнего уровня (`mod tests {`, `impl X {`).
    head: &'a str,
}

impl Line<'_> {
    fn is_comment(&self) -> bool {
        self.code.starts_with("//")
    }

    /// Внутри встроенного тестового модуля `…tests { … }`.
    fn in_inline_tests_module(&self) -> bool {
        self.gated
            && self.head.ends_with('{')
            && module_name(self.head).is_some_and(|name| name.ends_with("tests"))
    }
}

fn classify(text: &str) -> Vec<Line<'_>> {
    let mut lines = Vec::new();
    let (mut pending, mut gated, mut head) = (false, false, "");
    for (index, raw) in text.lines().enumerate() {
        let code = raw.trim();
        if code.is_empty() {
            continue;
        }
        let indent = raw.len() - raw.trim_start().len();
        if indent == 0 {
            if code == "#[cfg(test)]" {
                pending = true;
            } else if !(code.starts_with("#[")
                || code.starts_with("//")
                || code.starts_with(['}', ')', ']']))
            {
                gated = pending;
                pending = false;
                head = code;
            }
        }
        lines.push(Line {
            number: index + 1,
            indent,
            code,
            gated,
            head,
        });
    }
    lines
}

fn module_name(head: &str) -> Option<&str> {
    let mut words = head.split_whitespace();
    words.by_ref().find(|word| *word == "mod")?;
    words.next().map(|word| word.trim_end_matches(['{', ';']))
}

fn is_ident(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

/// `word` встречается в `code` не как часть более длинного имени.
fn has_word(code: &str, word: &str) -> bool {
    code.match_indices(word).any(|(at, _)| {
        !code[..at].chars().next_back().is_some_and(is_ident)
            && !code[at + word.len()..].chars().next().is_some_and(is_ident)
    })
}

fn is_use(code: &str) -> bool {
    let rest = code.strip_prefix("pub").map_or(code, |rest| {
        rest.trim_start_matches(|c| c != ' ').trim_start()
    });
    rest.starts_with("use ")
}

/// Строка называет модуль тестов по пути (П-3).
fn names_tests_module(code: &str) -> bool {
    code.contains("tests::")
        || (is_use(code)
            && code
                .split(|c| !is_ident(c))
                .any(|word| word.ends_with("tests")))
}

/// Как смонтирован файл тестов: `(под #[cfg(test)], имя модуля)`.
fn mount_of(sources: &[Source], tests_file: &Source) -> Option<(bool, String)> {
    let name = tests_file.rel.rsplit('/').next()?;
    let stem = name.strip_suffix(".rs")?;
    let by_path = format!("#[path = \"{name}\"]");
    for source in sources.iter().filter(|s| s.dir() == tests_file.dir()) {
        let mounts_by_name = ["mod.rs", "main.rs", "lib.rs"]
            .iter()
            .any(|file| source.rel.rsplit('/').next() == Some(file));
        let lines = classify(&source.text);
        for (at, line) in lines.iter().enumerate() {
            if line.indent != 0 {
                continue;
            }
            if mounts_by_name && line.code.ends_with(';') && module_name(line.code) == Some(stem) {
                return Some((line.gated, stem.to_string()));
            }
            if line.code == by_path {
                let item = lines[at + 1..]
                    .iter()
                    .find(|next| !next.code.starts_with("#[") && !next.is_comment())?;
                return Some((item.gated, module_name(item.code)?.to_string()));
            }
        }
    }
    None
}

#[derive(Default)]
struct Scan {
    scanned: usize,
    violations: Vec<String>,
    /// Определения тестового конструктора.
    definitions: Vec<String>,
    /// Его вызовы в модулях тестов — свидетель, что разметка их видит.
    test_uses: Vec<String>,
    /// `YtDlpJsRuntime::for_app(` в продакшен-строках.
    app_runtimes: Vec<String>,
    /// Строки `impl … YtDlpJsRuntime …` во всём `src/`.
    runtime_impls: Vec<String>,
}

fn scan() -> Scan {
    let sources = sources();
    let mut scan = Scan {
        scanned: sources.len(),
        ..Scan::default()
    };
    let definition = format!("fn {FOR_TESTS}");
    let trait_impl = format!("for {RUNTIME}");
    let runtime_for_app = format!("{RUNTIME}::for_app(");

    for source in &sources {
        let tests_file = source.is_tests_file()
            && match mount_of(&sources, source) {
                Some((true, name)) => name.ends_with("tests"),
                other => {
                    scan.violations.push(format!(
                        "{}: файл тестов смонтирован не под #[cfg(test)] или не найден: {other:?}",
                        source.rel
                    ));
                    false
                }
            };
        let mut previous = "";
        for line in classify(&source.text) {
            if line.is_comment() {
                continue;
            }
            let code = line.code;
            let place = format!("{}:{}", source.rel, line.number);
            let in_tests = tests_file || line.in_inline_tests_module();
            let production = !source.is_tests_file() && !line.gated;
            let mut violation =
                |what: &str| scan.violations.push(format!("{place}: {what}: {code}"));

            // Определение ищется вне модулей тестов: там одноимённая
            // функция достижима только по пути, а путь ловит П-3.
            if !in_tests && has_word(code, &definition) {
                scan.definitions.push(place.clone());
                if previous != "#[cfg(test)]" {
                    violation("П-1, тестовый конструктор без #[cfg(test)]");
                }
            } else if has_word(code, FOR_TESTS) {
                if in_tests {
                    scan.test_uses.push(place.clone());
                } else if production {
                    violation("П-1, продакшен-код зовёт тестовый конструктор");
                } else {
                    violation("П-2, тестовый конструктор вне модуля тестов");
                }
            }
            if !in_tests && names_tests_module(code) {
                violation("П-3, путь в модуль тестов вне модуля тестов");
            }
            if has_word(code, &trait_impl) {
                violation("П-4, реализация трейта для рантайма");
            }
            if code.starts_with("impl") && has_word(code, RUNTIME) {
                scan.runtime_impls.push(place.clone());
            }
            if production && code.contains(&runtime_for_app) {
                scan.app_runtimes.push(source.rel.clone());
            }
            previous = code;
        }
    }
    scan
}

#[test]
fn every_production_ytdlp_launch_gets_the_runtime_built_from_the_app() {
    let scan = scan();

    assert!(
        scan.scanned > 20,
        "сторож не увидел исходников: {}",
        scan.scanned
    );
    assert_eq!(scan.violations, Vec::<String>::new());

    // Не ослеп: известные места видны там, где они есть.
    assert_eq!(scan.definitions.len(), 1, "{:?}", scan.definitions);
    assert!(
        scan.definitions[0].starts_with("sidecar/deno.rs:"),
        "{:?}",
        scan.definitions
    );
    for file in ["probe/orchestrate.rs:", "download/orchestrate_tests.rs:"] {
        assert!(
            scan.test_uses.iter().any(|place| place.starts_with(file)),
            "разметка не видит тестов в {file} {:?}",
            scan.test_uses
        );
    }
    for file in ["commands/probe.rs", "commands/queue.rs"] {
        assert!(
            scan.app_runtimes.iter().any(|seen| seen == file),
            "{file}: {:?}",
            scan.app_runtimes
        );
    }
}

/// Сигнатуры функций верхнего уровня внутри элемента `head` в порядке файла;
/// перед тестовыми — `#[cfg(test)]`.
fn functions(lines: &[Line<'_>], head: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut previous = "";
    for line in lines.iter().filter(|l| l.head == head && !l.is_comment()) {
        if line.indent == 4 && has_word(line.code, "fn") {
            let label = line.code.split('(').next().unwrap_or_default();
            found.push(if previous == "#[cfg(test)]" {
                format!("#[cfg(test)] {label}")
            } else {
                label.to_string()
            });
        }
        previous = line.code;
    }
    found
}

/// Текст функции `label` из элемента `head` одной строкой без комментариев.
fn function_text(lines: &[Line<'_>], head: &str, label: &str) -> String {
    let mut parts = Vec::new();
    for line in lines.iter().filter(|l| l.head == head && !l.is_comment()) {
        let starts = line.indent == 4
            && line
                .code
                .strip_prefix(label)
                .is_some_and(|rest| rest.starts_with('('));
        if starts || !parts.is_empty() {
            parts.push(line.code);
            if line.indent == 4 && line.code == "}" {
                break;
            }
        }
    }
    parts.join(" ")
}

#[test]
fn the_runtime_has_exactly_the_audited_constructors() {
    let scan = scan();
    let deno = include_str!("../sidecar/deno.rs");
    let lines = classify(deno);
    let runtime_impl = format!("impl {RUNTIME} {{");

    assert_eq!(scan.runtime_impls.len(), 1, "{:?}", scan.runtime_impls);
    assert!(
        scan.runtime_impls[0].starts_with("sidecar/deno.rs:"),
        "{:?}",
        scan.runtime_impls
    );

    assert_eq!(
        functions(&lines, &runtime_impl),
        [
            "pub fn for_app",
            "#[cfg(test)] pub fn for_tests",
            "fn from_deno",
            "fn disabled",
            "fn disabled_with",
            "pub fn argv<'a>",
            "pub fn env",
        ],
        "список функций рантайма изменился — новый конструктор сверь с TL-114"
    );

    // Где заполняется поле `runtime` (включая сокращённую запись).
    let mut filled = Vec::new();
    let mut current = String::new();
    for line in lines
        .iter()
        .filter(|l| !l.is_comment() && !l.in_inline_tests_module())
    {
        if line.indent == 4 && has_word(line.code, "fn") {
            current = line.code.split('(').next().unwrap_or_default().to_string();
        }
        let code = line.code;
        if has_word(code, "runtime")
            && (code.contains("runtime:")
                || code.starts_with("runtime,")
                || code.starts_with("runtime }")
                || code.contains("{ runtime"))
        {
            filled.push(if line.head == runtime_impl {
                current.clone()
            } else {
                line.head.to_string()
            });
        }
    }
    assert_eq!(
        filled,
        [
            "pub struct YtDlpJsRuntime {",
            "fn from_deno",
            "fn disabled_with"
        ],
        "поле рантайма заполняется вне сверенных функций"
    );

    assert_eq!(
        function_text(&lines, &runtime_impl, "pub fn for_app"),
        "pub fn for_app(app: &AppHandle) -> Self { Self::from_deno(DenoLaunch::for_app(app)) }"
    );
    assert_eq!(
        function_text(&lines, "impl DenoLaunch {", "pub fn for_app"),
        "pub fn for_app(app: &AppHandle) -> Result<Self, SidecarError> { \
         Self::resolve(app.path().app_data_dir(), super::resolve_sidecar_path) }"
    );
}
