//! Сверка того, что фронтенд вызывает через IPC, с тем, что ему разрешено
//! ACL (TL-24, #25).
//!
//! # Зачем этот тест существует
//!
//! В Tauri 2 доступ webview к core- и plugin-командам закрыт по умолчанию:
//! проходят только те, которые перечислены в `src-tauri/capabilities/`.
//! Собственные команды приложения (`check_sidecar`, `prepare_ytdlp`) через
//! ACL не проходят вообще, пока у приложения нет своего ACL-манифеста, —
//! поэтому отсутствие capabilities не проявляло себя до TL-17, где
//! появилась первая подписка `listen()`. На собранном `.dmg` она молча
//! отклонялась с «Command plugin:event|listen not allowed by ACL», и это
//! нашёл владелец руками на приёмке.
//!
//! Регрессия ровно такого вида откроется снова в E2/E3: события прогресса
//! скачивания пойдут тем же `listen()`, а любая новая обёртка над
//! `@tauri-apps/api/*` — тем же механизмом. Поэтому тест не проверяет
//! фиксированный список команд, а **выводит его из исходников `src/`** и
//! прогоняет через настоящую резолюцию ACL из `tauri::generate_context!()`
//! — ту же самую, что вызывает `Webview::on_message` в рантайме
//! (`RuntimeAuthority::resolve_access`).
//!
//! # Что тест ловит, а что нет
//!
//! Ловит: новую подписку или новый вызов Tauri API во фронтенде без
//! выданного разрешения — в любой форме импорта, включая корневую
//! `from '@tauri-apps/api'` и `import * as`; вызов плагина по
//! литеральному имени; вызов метода объекта окна, за которым нет
//! разрешения; разрешение, за которым нет вызова; capability,
//! открытую наружу (`remote`) или привязанную к окнам по glob;
//! переименование окна в `tauri.conf.json` мимо `windows` в capability;
//! появление у приложения собственного ACL-манифеста, после которого
//! команды `check_sidecar`/`prepare_ytdlp` тоже потребуют разрешений.
//!
//! Не ловит: вызовы, собранные из динамических строк (`invoke(name)` с
//! именем из переменной) — статически их не видно. Такой код во фронтенде
//! запрещён соглашением: обёртки над `invoke`/`listen` держат имена
//! команд и событий в константах модуля (см. `src/composables/`).
//!
//! # Модуль окон разбирается глубже импорта
//!
//! У `@tauri-apps/api/window` команда стоит не за импортом, а за методом:
//! `getCurrentWindow()` сам по себе IPC не делает (читает метку из
//! `window.__TAURI_INTERNALS__.metadata` и строит `Window` с внутренним
//! `skip: true`, минуя `plugin:window|create`), зато у возвращённого
//! объекта 78 методов с командами. Часть из них библиотека зовёт сама:
//! `onCloseRequested` после обработчика, не вызвавшего `preventDefault`,
//! вызывает `destroy()` — вызова, который надо разрешить, в наших
//! исходниках при этом не написано вовсе. Поэтому по окну разбирается не
//! импорт, а употребление: прямая цепочка `getCurrentWindow().m()` и
//! локальная переменная, в которую объект положили. Метод, за которым
//! может стоять оконная команда, обязан быть перечислен в
//! `window_handle_commands` — иначе падение: новое оконное разрешение
//! выдаётся только в capability, то есть решением области core, и
//! проходить это молча ему нельзя.
//!
//! Разбор намеренно не-строгий в одну сторону: всё, что не удалось
//! разобрать однозначно, — это падение с объяснением, а не пропуск.
//! Скан файлов устроен так же: сканируется всё, кроме заведомо
//! неисполняемых расширений.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use tauri::ipc::Origin;
use tauri::utils::acl::APP_ACL_KEY;

/// Корень исходников фронтенда относительно `src-tauri/`.
const FRONTEND_SRC: &str = "../src";

/// Метка окна по умолчанию в Tauri 2, когда `label` в конфиге не задан.
const DEFAULT_WINDOW_LABEL: &str = "main";

/// Что именно вызывает по IPC та или иная привязка из `@tauri-apps/api`.
///
/// `Some(&[])` — привязка существует, но IPC не порождает (тип, константа,
/// либо команда, освобождённая от ACL). `None` — привязка неизвестна:
/// тест обязан упасть, а разработчик — дописать строку сюда и, если нужно,
/// разрешение в capability. Fail-closed здесь принципиален: молчаливое
/// «не знаю — значит можно» вернёт ровно тот дефект, ради которого тест
/// написан.
fn ipc_commands_of(module: &str, binding: &str) -> Option<&'static [&'static str]> {
    match (module, binding) {
        // `invoke` сама по себе ACL не требует: собственные команды
        // приложения не проходят проверку, пока нет app-манифеста (см.
        // `app_commands_stay_outside_the_acl`), а вызовы плагинов по
        // литеральному имени ловятся отдельным сканом строк `plugin:…|…`.
        ("core", "invoke") => Some(NONE),
        ("core", "isTauri") => Some(NONE),
        // Данные `Channel` идут по FETCH_CHANNEL_DATA_COMMAND, который в
        // Tauri 2 намеренно освобождён от проверки ACL.
        ("core", "Channel") => Some(NONE),
        ("core", "transformCallback") => Some(NONE),
        ("event", "listen") => Some(LISTEN),
        ("event", "once") => Some(LISTEN),
        ("event", "emit") => Some(&["plugin:event|emit"]),
        ("event", "emitTo") => Some(&["plugin:event|emit_to"]),
        // Перечисление имён событий `tauri://…`, само по себе IPC нет.
        ("event", "TauriEvent") => Some(NONE),
        // Объект окна: IPC даёт не он, а вызванные на нём методы —
        // их разбирает `window_handle_methods` (см. doc модуля).
        ("window", WINDOW_HANDLE_FACTORY) => Some(NONE),
        _ => None,
    }
}

/// Привязка, дающая фронтенду объект текущего окна.
const WINDOW_HANDLE_FACTORY: &str = "getCurrentWindow";

/// Что вызывает по IPC метод объекта `Window`.
///
/// `None` — метод неизвестен: за ним может стоять любая из 78 оконных
/// команд, и молчаливый проход вернул бы ровно дефект TL-24 — отказ ACL,
/// видимый только на собранном приложении. Перечислены здесь только те
/// методы, состав команд которых сверен с исходником
/// `node_modules/@tauri-apps/api/window.js`.
fn window_handle_commands(method: &str) -> Option<&'static [&'static str]> {
    match method {
        // Все `Window.on*` и `Window.listen/once` идут через тот же
        // `plugin:event|listen`, что и свободная функция из модуля
        // `event`, — с `target: { kind: 'Window' }`. Своего оконного
        // разрешения они не требуют.
        "listen" | "once" | "onResized" | "onMoved" | "onFocusChanged" | "onScaleChanged"
        | "onThemeChanged" | "onDragDropEvent" => Some(LISTEN),
        "emit" => Some(&["plugin:event|emit"]),
        "emitTo" => Some(&["plugin:event|emit_to"]),
        // Единственный метод, чей состав команд шире написанного:
        // подписавшись на `tauri://close-requested`, он в конце
        // обработчика сам зовёт `this.destroy()`, если тот не вызвал
        // `preventDefault()`. Разрешения `plugin:window|destroy` под это
        // достаточно: закрывать окно повторным `close()` нельзя — он
        // снова поднимет то же событие (TL-47, #49).
        "onCloseRequested" => Some(CLOSE_REQUESTED),
        "destroy" => Some(&[DESTROY]),
        _ => None,
    }
}

/// `listen()` возвращает `UnlistenFn`, который дёргает
/// `plugin:event|unlisten`; отписка в `onUnmounted` — обычный путь, а не
/// экзотика, поэтому подписка всегда тянет за собой оба разрешения.
const LISTEN: &[&str] = &["plugin:event|listen", "plugin:event|unlisten"];
const NONE: &[&str] = &[];
const DESTROY: &str = "plugin:window|destroy";
const CLOSE_REQUESTED: &[&str] = &["plugin:event|listen", "plugin:event|unlisten", DESTROY];

/// Разобранный импорт из пакета `@tauri-apps/api`.
#[derive(Debug)]
struct ApiImport {
    /// `None` — импортируется корень пакета (`@tauri-apps/api` без
    /// подмодуля).
    module: Option<String>,
    /// `Err` — привязки не разобраны; строка объясняет, что именно
    /// помешало. Такой импорт обязан превратиться в падение теста, а не в
    /// пустое требование: «не разобрал» и «команд не требуется» — разные
    /// вещи, и путать их означает fail-open.
    bindings: Result<Vec<String>, &'static str>,
}

/// Пара «модуль — привязки» для утверждений в тестах разбора.
type ParsedImport = (Option<String>, Result<Vec<String>, &'static str>);

impl ApiImport {
    /// Спецификатор так, как он написан в исходнике, — для диагностики.
    fn specifier(&self) -> String {
        match &self.module {
            Some(module) => format!("@tauri-apps/api/{module}"),
            None => PACKAGE.to_string(),
        }
    }
}

/// Расширения, за которыми исполняемого кода фронтенда не бывает.
///
/// Правило перевёрнутое по сравнению с очевидным: сканируется всё, кроме
/// перечисленного, а не только `.ts`/`.vue`. Белый список здесь работал бы
/// в неправильную сторону — `src/probe.js` или `.mjs` с новым импортом
/// прошёл бы мимо проверки молча (ревью TL-24).
const NON_CODE_EXTENSIONS: &[&str] = &[
    ".css", ".scss", ".sass", ".less", ".svg", ".png", ".jpg", ".jpeg", ".gif", ".webp", ".avif",
    ".ico", ".json", ".md", ".txt", ".woff", ".woff2", ".ttf", ".otf", ".eot", ".map", ".lock",
];

fn frontend_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|err| panic!("не читается {dir:?}: {err}"));
    for entry in entries {
        let path = entry.expect("элемент каталога").path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if path.is_dir() {
            frontend_files(&path, out);
            continue;
        }
        // Тесты фронтенда мокают Tauri API и в рантайме приложения не
        // участвуют — их импорты не создают требований к ACL.
        let is_test = name.contains(".test.") || name.contains(".spec.");
        // `.d.ts` — только объявления типов, вызовов в них нет.
        let is_declaration = name.ends_with(".d.ts");
        let is_non_code = NON_CODE_EXTENSIONS.iter().any(|ext| name.ends_with(ext));
        if !is_test && !is_declaration && !is_non_code {
            out.push(path);
        }
    }
}

/// Файлы фронтенда, попадающие под скан.
fn scanned_files() -> (PathBuf, Vec<PathBuf>) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(FRONTEND_SRC);
    let mut files = Vec::new();
    frontend_files(&root, &mut files);
    assert!(
        !files.is_empty(),
        "в {root:?} не нашлось ни одного исходника фронтенда — тест бы \
         молча разрешил всё; проверь путь"
    );
    (root, files)
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// Последнее вхождение `token` именно как токена, а не как подстроки.
///
/// `rfind("import")` находил бы `import` внутри слова `important` в
/// комментарии между привязками и обрывал разбор корректного кода
/// (ревью TL-24).
fn rfind_token(haystack: &str, token: &str) -> Option<usize> {
    haystack
        .rmatch_indices(token)
        .find(|(idx, _)| {
            let before = haystack[..*idx].chars().next_back();
            let after = haystack[idx + token.len()..].chars().next();
            !before.is_some_and(is_ident_char) && !after.is_some_and(is_ident_char)
        })
        .map(|(idx, _)| idx)
}

/// Спецификатор пакета без подмодуля.
///
/// Скан идёт именно по нему, а не по `@tauri-apps/api/`: форма
/// `import { event } from '@tauri-apps/api'` (корневой реэкспорт — ровно
/// так написан пример в `index.d.ts` самого пакета) при поиске со слешем
/// не давала ни одного вхождения, требование не записывалось, и тест
/// оставался зелёным на коде, который в собранном приложении отклонит ACL
/// (ревью TL-24).
const PACKAGE: &str = "@tauri-apps/api";

/// Находит импорты из `@tauri-apps/api` без полноценного парсера TS:
/// от каждого вхождения спецификатора идём назад до ближайшего токена
/// `import`/`export` и разбираем список привязок в фигурных скобках.
fn parse_api_imports(content: &str) -> Vec<ApiImport> {
    let mut imports = Vec::new();

    for (idx, _) in content.match_indices(PACKAGE) {
        let module = content[idx + PACKAGE.len()..]
            .strip_prefix('/')
            .map(|rest| {
                rest.chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                    .collect::<String>()
            });

        let head = &content[..idx];
        let stmt_start = rfind_token(head, "import")
            .into_iter()
            .chain(rfind_token(head, "export"))
            .max();

        let bindings = match stmt_start {
            None => Err("спецификатор есть, а оператора импорта перед ним нет"),
            Some(start) => {
                let stmt = &head[start..];
                if stmt.starts_with("import type") || stmt.starts_with("export type") {
                    // `import type { … }` — только типы, IPC за ними нет.
                    Ok(Vec::new())
                } else if module.is_none() {
                    Err(
                        "импортируется корень пакета: в скобках здесь имена модулей \
                         (event, path, window), а не привязки, и какие команды за ними \
                         стоят — статически не видно. Импортируй конкретный модуль, \
                         например '@tauri-apps/api/event'",
                    )
                } else {
                    parse_bindings(stmt)
                }
            }
        };

        imports.push(ApiImport { module, bindings });
    }

    imports
}

/// Список привязок из `{ … }` оператора импорта.
fn parse_bindings(stmt: &str) -> Result<Vec<String>, &'static str> {
    let open = stmt
        .find('{')
        .ok_or("не найден список привязок в фигурных скобках")?;
    let close = stmt
        .rfind('}')
        .ok_or("список привязок открыт, но не закрыт")?;
    if close < open {
        return Err("фигурные скобки не образуют список привязок");
    }
    Ok(stmt[open + 1..close]
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        // `type Event as TauriEvent` — тип, не вызов.
        .filter(|item| !item.starts_with("type "))
        .map(|item| item.split(" as ").next().unwrap_or(item).trim().to_string())
        .collect())
}

/// Вырезает комментарии (`//`, `/* */`, `<!-- -->`), не трогая строковые
/// литералы.
///
/// Нужно ровно для скана литеральных имён команд: имя плагина в тексте
/// комментария («отказ IPC-вызова `plugin:event|listen`» в
/// `useYtDlpPrepare.ts`) — не вызов, и требовать под него разрешение
/// нельзя: тест начал бы подсказывать выдавать доступы, за которыми нет
/// кода. Разбор импортов, наоборот, идёт по сырому тексту — там ошибка
/// в сторону лишнего требования безопасна.
fn strip_comments(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut chars = content.chars().peekable();
    let mut string_delim: Option<char> = None;
    let mut escaped = false;

    while let Some(c) = chars.next() {
        if let Some(delim) = string_delim {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == delim {
                string_delim = None;
            }
            continue;
        }

        match c {
            '\'' | '"' | '`' => {
                string_delim = Some(c);
                out.push(c);
            }
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                let mut prev = ' ';
                for c in chars.by_ref() {
                    if prev == '*' && c == '/' {
                        break;
                    }
                    prev = c;
                }
                out.push(' ');
            }
            // HTML-комментарий в `<template>` .vue-файла.
            '<' if chars.clone().take(3).eq("!--".chars()) => {
                let mut window = String::new();
                for c in chars.by_ref() {
                    window.push(c);
                    if window.ends_with("-->") {
                        break;
                    }
                }
                out.push(' ');
            }
            _ => out.push(c),
        }
    }

    out
}

/// Литеральные имена plugin-команд в исходниках (`invoke('plugin:x|y')`).
fn parse_literal_plugin_commands(content: &str) -> Vec<String> {
    let code = strip_comments(content);
    let mut found = Vec::new();
    for (idx, _) in code.match_indices("plugin:") {
        let quote = code[..idx].chars().last();
        if !matches!(quote, Some('\'') | Some('"') | Some('`')) {
            continue;
        }
        let quote = quote.expect("проверено выше");
        let rest = &code[idx..];
        if let Some(end) = rest.find(quote) {
            let literal = &rest[..end];
            if literal.contains('|') {
                found.push(literal.to_string());
            }
        }
    }
    found
}

/// Обращение к члену объекта сразу за выражением.
///
/// `Some(Some("m"))` — вызов метода `.m(…)` (и `?.m(…)`), `Some(None)` —
/// чтение поля (`.label`), за которым IPC нет, `None` — точки нет вовсе.
fn leading_member(rest: &str) -> Option<Option<String>> {
    let rest = rest.trim_start();
    let rest = rest.strip_prefix("?.").or_else(|| rest.strip_prefix('.'))?;
    let name: String = rest.chars().take_while(|c| is_ident_char(*c)).collect();
    if name.is_empty() {
        return Some(None);
    }
    let is_call = rest[name.len()..].trim_start().starts_with('(');
    Some(is_call.then_some(name))
}

/// Имя локальной переменной, в которую положили объект окна, и позиция
/// этого имени в тексте.
///
/// `head` — всё, что стоит в файле до вызова фабрики.
fn window_alias(head: &str) -> Result<(String, usize), String> {
    let escaped = |what: String| {
        format!(
            "{what}: статически не видно, какие методы на нём вызовут, а за \
             объектом окна стоят все 78 оконных команд при одной \
             разрешённой. Положи окно в локальную переменную этого файла \
             (`const appWindow = {WINDOW_HANDLE_FACTORY}()`) и вызывай \
             методы прямо на ней"
        )
    };

    let decl = head.trim_end().strip_suffix('=').ok_or_else(|| {
        escaped(format!(
            "{WINDOW_HANDLE_FACTORY}() не присваивается переменной"
        ))
    })?;
    if decl.trim_end().ends_with('=') {
        return Err(escaped(format!(
            "{WINDOW_HANDLE_FACTORY}() стоит в сравнении, а не в объявлении"
        )));
    }

    let keyword_at = ["const", "let", "var"]
        .iter()
        .filter_map(|kw| rfind_token(decl, kw))
        .max()
        .ok_or_else(|| {
            escaped(format!(
                "{WINDOW_HANDLE_FACTORY}() присваивается не в объявление \
                 переменной (const/let/var)"
            ))
        })?;

    let tail = &decl[keyword_at..];
    let mut parts = tail.split_whitespace();
    let keyword = parts.next().expect("ключевое слово найдено выше");
    let declared = parts.next().ok_or_else(|| {
        escaped(format!(
            "после {keyword} нет имени переменной перед {WINDOW_HANDLE_FACTORY}()"
        ))
    })?;
    // `const appWindow: Window = …` — аннотацию типа отрезаем.
    let name = declared.split(':').next().unwrap_or(declared);
    if name.is_empty()
        || !name.chars().all(is_ident_char)
        || name.starts_with(|c: char| c.is_ascii_digit())
    {
        return Err(escaped(format!(
            "окно разбирается по частям (`{declared}`), а не кладётся в \
             переменную целиком"
        )));
    }

    let offset = keyword_at
        + keyword.len()
        + tail[keyword.len()..]
            .find(declared)
            .expect("имя взято из этого же куска");
    Ok((name.to_string(), offset))
}

/// Стоит ли имя в списке привязок собственного оператора `import`.
///
/// `head` — текст файла до имени. Между `import` и именем в этом случае
/// бывают только другие привязки: `{`, запятые, `type`, пробелы. Любая
/// скобка вызова или закрытая `}` предыдущего оператора означают, что имя
/// стоит уже не в импорте.
fn is_import_binding(head: &str) -> bool {
    let Some(import_at) = rfind_token(head, "import") else {
        return false;
    };
    if rfind_token(head, "export").is_some_and(|export_at| export_at > import_at) {
        return false;
    }
    head[import_at..]
        .chars()
        .all(|c| is_ident_char(c) || c.is_whitespace() || c == '{' || c == ',')
}

/// Методы, вызванные в файле на объекте текущего окна.
///
/// Разбираются две формы: прямая цепочка `getCurrentWindow().m(…)` и
/// локальная переменная, в которую объект положили. Любая третья — `Err`:
/// см. doc модуля, почему по окну недостаточно разобрать импорт.
fn window_handle_methods(content: &str) -> Result<Vec<String>, String> {
    let code = strip_comments(content);
    let mut methods = Vec::new();
    let mut aliases: Vec<(String, usize)> = Vec::new();

    for (idx, _) in code.match_indices(WINDOW_HANDLE_FACTORY) {
        if code[..idx].chars().next_back().is_some_and(is_ident_char) {
            continue;
        }
        let after = &code[idx + WINDOW_HANDLE_FACTORY.len()..];
        let Some(call) = after.trim_start().strip_prefix('(') else {
            // Имя без вызова законно ровно в одном месте — в списке
            // привязок собственного оператора импорта. Везде ещё это
            // фабрика, отданная кому-то в руки (`useWindow(getCurrentWindow)`,
            // реэкспорт): окно тогда возьмут в другом месте, и по
            // импортам `@tauri-apps/api` его там уже не видно.
            if is_import_binding(&code[..idx]) {
                continue;
            }
            return Err(format!(
                "{WINDOW_HANDLE_FACTORY} используется не как вызов и не в \
                 списке привязок импорта — фабрику окна передают дальше, и \
                 где на окне вызовут методы, статически не видно. Бери окно \
                 там же, где вызываешь его методы"
            ));
        };
        let Some(rest) = call.trim_start().strip_prefix(')') else {
            return Err(format!(
                "{WINDOW_HANDLE_FACTORY}(…) вызывается с аргументами — у фабрики \
                 окна из @tauri-apps/api их нет, и что это за вызов, проверка \
                 ACL не знает"
            ));
        };

        match leading_member(rest) {
            // `getCurrentWindow().m(…)` — метод виден прямо здесь.
            Some(Some(method)) => methods.push(method),
            // `getCurrentWindow().label` — чтение поля, IPC за ним нет.
            Some(None) => {}
            None => aliases.push(window_alias(&code[..idx])?),
        }
    }

    let declarations: BTreeSet<usize> = aliases.iter().map(|(_, at)| *at).collect();
    let names: BTreeSet<&str> = aliases.iter().map(|(name, _)| name.as_str()).collect();
    for name in names {
        for (at, _) in code.match_indices(name) {
            if declarations.contains(&at) {
                continue;
            }
            let rest = &code[at + name.len()..];
            if code[..at].chars().next_back().is_some_and(is_ident_char)
                || rest.chars().next().is_some_and(is_ident_char)
            {
                continue;
            }
            match leading_member(rest) {
                Some(Some(method)) => methods.push(method),
                Some(None) => {}
                None => {
                    return Err(format!(
                        "окно лежит в `{name}`, но `{name}` употреблён не как \
                         вызов метода: статически не видно, какие методы на нём \
                         вызовут, а за объектом окна стоят все 78 оконных \
                         команд при одной разрешённой. Вызывай методы окна в \
                         том же файле, где его взяли"
                    ));
                }
            }
        }
    }

    Ok(methods)
}

/// Полный набор IPC-команд, которые может выдать текущий фронтенд.
///
/// Возвращает также карту «команда → откуда взялась», чтобы падение теста
/// сразу показывало файл, а не только имя команды.
fn frontend_ipc_commands() -> BTreeMap<String, BTreeSet<String>> {
    let (root, files) = scanned_files();

    let mut commands: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut unknown: Vec<String> = Vec::new();

    for file in &files {
        // Через lossy: сканируется всё, кроме заведомо неисполняемых
        // расширений, и наткнуться на не-UTF-8 файл возможно. Паника на
        // нём означала бы «проверка сломалась», а пропуск — «проверка
        // промолчала»; ни то ни другое не нужно.
        let bytes = fs::read(file).expect("исходник фронтенда читается");
        let content = String::from_utf8_lossy(&bytes);
        let shown = file
            .strip_prefix(&root)
            .unwrap_or(file)
            .display()
            .to_string();

        for import in parse_api_imports(&content) {
            let specifier = import.specifier();
            let bindings = match import.bindings {
                Ok(bindings) => bindings,
                Err(reason) => {
                    unknown.push(format!("{shown}: импорт {specifier} — {reason}."));
                    continue;
                }
            };
            let module = import.module.as_deref().unwrap_or_default();
            for binding in bindings {
                match ipc_commands_of(module, &binding) {
                    Some(cmds) => {
                        for cmd in cmds {
                            commands
                                .entry((*cmd).to_string())
                                .or_default()
                                .insert(format!("{shown}: {module}.{binding}"));
                        }
                    }
                    None => unknown.push(format!(
                        "{shown}: {specifier}.{binding} — неизвестная привязка. \
                         Добавь её в `ipc_commands_of` (и, если она делает IPC, \
                         выдай разрешение в src-tauri/capabilities/).",
                    )),
                }
            }
        }

        for literal in parse_literal_plugin_commands(&content) {
            commands
                .entry(literal.clone())
                .or_default()
                .insert(format!("{shown}: литерал \"{literal}\""));
        }

        match window_handle_methods(&content) {
            Ok(methods) => {
                for method in methods {
                    match window_handle_commands(&method) {
                        Some(cmds) => {
                            for cmd in cmds {
                                commands
                                    .entry((*cmd).to_string())
                                    .or_default()
                                    .insert(format!(
                                    "{shown}: окно из {WINDOW_HANDLE_FACTORY}(), метод .{method}()"
                                ));
                            }
                        }
                        None => unknown.push(format!(
                            "{shown}: на объекте окна из {WINDOW_HANDLE_FACTORY}() вызван \
                             .{method}() — метод, о котором проверка ACL ничего не знает. Сверь по \
                             node_modules/@tauri-apps/api/window.js, какие команды он \
                             вызывает, добавь его в `window_handle_commands` и выдай \
                             разрешение в src-tauri/capabilities/: оконные команды \
                             запрещены все, кроме destroy.",
                        )),
                    }
                }
            }
            Err(reason) => unknown.push(format!("{shown}: {reason}.")),
        }
    }

    assert!(
        unknown.is_empty(),
        "фронтенд использует Tauri API, о котором проверка ACL ничего не знает:\n{}",
        unknown.join("\n")
    );

    commands
}

/// Метки окон из `tauri.conf.json` — ровно те, по которым capability
/// сопоставляется с окном в рантайме.
fn configured_window_labels() -> Vec<String> {
    let conf = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json"))
        .expect("tauri.conf.json читается");
    let conf: serde_json::Value =
        serde_json::from_str(&conf).expect("tauri.conf.json — валидный JSON");
    let windows = conf["app"]["windows"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if windows.is_empty() {
        return vec![DEFAULT_WINDOW_LABEL.to_string()];
    }
    windows
        .iter()
        .map(|w| {
            w["label"]
                .as_str()
                .unwrap_or(DEFAULT_WINDOW_LABEL)
                .to_string()
        })
        .collect()
}

/// Контекст приложения — со всей резолюцией ACL, которую `tauri-build`
/// собрал из `capabilities/`.
///
/// `generate_context!` разрешено вызывать в компиляционной единице ровно
/// один раз (макрос встраивает символы вроде `_EMBED_INFO_PLIST`), поэтому
/// точка вызова здесь одна на весь тестовый бинарник.
fn app_context() -> tauri::Context<tauri::Wry> {
    tauri::generate_context!()
}

/// Канарейка: скан всё ещё видит то, что во фронтенде точно есть.
///
/// Без неё вся проверка вырождается молча: сломанный разбор импортов даёт
/// пустой набор команд, и «всё разрешено» становится «нечего проверять».
/// Страховать нужно все три подсистемы скана — обход файлов, разбор
/// импортов, скан литералов, — поэтому здесь и утверждение про `.vue`:
/// подписки живут в `.ts`, и обход `.vue` мог бы сломаться незаметно
/// (ревью TL-24). Если экран подготовки когда-нибудь перестанет
/// подписываться на события — правь этот тест осознанно, вместе с
/// разрешением в capability.
#[test]
fn the_scan_still_finds_the_subscription_the_prepare_screen_makes() {
    let (_, files) = scanned_files();
    assert!(
        files
            .iter()
            .any(|f| f.extension().is_some_and(|ext| ext == "vue")),
        "обход не дал ни одного .vue — компоненты выпали из скана, и вызов \
         Tauri API прямо в компоненте больше не будет замечен"
    );

    let commands = frontend_ipc_commands();
    for expected in ["plugin:event|listen", "plugin:event|unlisten"] {
        assert!(
            commands.contains_key(expected),
            "скан исходников фронтенда не нашёл {expected}, хотя \
             src/composables/useYtDlpPrepare.ts подписывается на \
             `ytdlp://prepare`. Либо сломан разбор импортов (и тогда \
             проверка ACL больше ничего не проверяет), либо подписки \
             действительно не стало."
        );
    }
}

#[test]
fn imports_are_parsed_down_to_the_bindings_that_do_ipc() {
    let imports = parse_api_imports(
        "import { invoke } from '@tauri-apps/api/core'\n\
         import { listen, type Event as TauriEvent, type UnlistenFn } from '@tauri-apps/api/event'\n\
         import type { Something } from '@tauri-apps/api/window'\n",
    );
    let parsed: Vec<ParsedImport> = imports
        .into_iter()
        .map(|i| (i.module, i.bindings))
        .collect();

    assert_eq!(
        parsed,
        vec![
            (Some("core".to_string()), Ok(vec!["invoke".to_string()])),
            (Some("event".to_string()), Ok(vec!["listen".to_string()])),
            // `import type` не порождает вызовов вообще.
            (Some("window".to_string()), Ok(Vec::new())),
        ]
    );
}

/// Комментарий между привязками и спецификатором не ломает разбор.
///
/// `rfind("import")` находил `import` внутри `important` и объявлял
/// корректный оператор неразобранным — красный тест на правильном коде,
/// с диагностикой, уводящей не туда (ревью TL-24).
#[test]
fn a_word_containing_import_inside_the_statement_is_not_mistaken_for_a_keyword() {
    let imports =
        parse_api_imports("import { listen } /* important */ from '@tauri-apps/api/event'\n");
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].bindings, Ok(vec!["listen".to_string()]));
}

/// Импорт корня пакета — дыра, найденная ревью TL-24.
///
/// `import { event } from '@tauri-apps/api'` — форма из примера в
/// `index.d.ts` самого пакета: `event.emit(…)` работает, ACL отклоняет,
/// а скан по `@tauri-apps/api/` не давал ни одного вхождения и оставлял
/// тест зелёным.
#[test]
fn a_root_package_import_is_reported_instead_of_being_silently_skipped() {
    let imports = parse_api_imports("import { event, path } from '@tauri-apps/api'\n");
    assert_eq!(imports.len(), 1);
    assert!(imports[0].module.is_none());
    assert!(
        imports[0].bindings.is_err(),
        "импорт корня пакета обязан превратиться в падение: за именем \
         модуля стоит весь его API, а не одна команда"
    );
}

#[test]
fn a_namespace_import_is_reported_instead_of_being_silently_skipped() {
    let imports = parse_api_imports("import * as event from '@tauri-apps/api/event'\n");
    assert_eq!(imports.len(), 1);
    assert!(
        imports[0].bindings.is_err(),
        "импорт всем модулем сузить до команд нельзя — он обязан \
         превратиться в падение, а не в пустой список"
    );
}

#[test]
fn a_command_name_mentioned_in_a_comment_is_not_a_call() {
    let mentioned = "// отказ IPC-вызова `plugin:event|emit` тут только описан\n\
         const cmd = 'plugin:event|listen'\n";
    assert_eq!(
        parse_literal_plugin_commands(mentioned),
        vec!["plugin:event|listen".to_string()]
    );
}

/// Литерал внутри `<script setup>` виден так же, как в `.ts`.
///
/// Скан литералов — отдельная от разбора импортов подсистема, и на
/// `.vue`-файлах у неё своя специфика: HTML-комментарии в `<template>` не
/// должны давать требований. Апостроф в тексте шаблона разбор строк
/// сбивает, но только в сторону лишних требований (текст после него
/// перестаёт считаться комментарием), пропустить вызов из-за него нельзя
/// — литерал остаётся литералом вместе с кавычками.
#[test]
fn a_literal_command_inside_script_setup_is_found() {
    let sfc = "<script setup lang=\"ts\">\n\
         import { invoke } from '@tauri-apps/api/core'\n\
         void invoke('plugin:updater|check')\n\
         </script>\n\
         <template>\n\
         <!-- в доке это `plugin:updater|download_and_install`, но не вызов -->\n\
         <p>всё готово</p>\n\
         </template>\n";
    assert_eq!(
        parse_literal_plugin_commands(sfc),
        vec!["plugin:updater|check".to_string()]
    );
}

/// Окно, взятое в локальную переменную, прослеживается до методов.
///
/// Форма из TL-46: объект берут один раз на модуль, а закрывают окно
/// потом, из обработчика подтверждения.
#[test]
fn the_window_object_is_traced_from_a_local_variable_to_its_methods() {
    let source = "import { getCurrentWindow } from '@tauri-apps/api/window'\n\
         const appWindow = getCurrentWindow()\n\
         export async function onExit(handler) {\n\
         return appWindow.onCloseRequested(handler)\n\
         }\n\
         export async function exitNow() {\n\
         await appWindow.destroy()\n\
         }\n";
    assert_eq!(
        window_handle_methods(source),
        Ok(vec!["onCloseRequested".to_string(), "destroy".to_string()])
    );
}

/// Прямая цепочка и аннотация типа при объявлении разбираются тоже, а
/// чтение поля команд не требует.
#[test]
fn the_window_object_is_traced_through_a_chain_and_a_typed_declaration() {
    assert_eq!(
        window_handle_methods("await getCurrentWindow()\n  .destroy()\n"),
        Ok(vec!["destroy".to_string()])
    );
    assert_eq!(
        window_handle_methods("const w: Window = getCurrentWindow()\nw?.onCloseRequested(h)\n"),
        Ok(vec!["onCloseRequested".to_string()])
    );
    assert_eq!(
        window_handle_methods("const w = getCurrentWindow()\nconsole.log(w.label)\n"),
        Ok(Vec::new()),
        "`label` — поле объекта, IPC за ним нет"
    );
}

/// Окно, уехавшее из файла, — падение, а не пустой список методов.
///
/// За объектом стоят все оконные команды; разрешена одна. Молчаливый
/// проход здесь — та же дыра, что импорт всем модулем (`import * as`).
#[test]
fn a_window_object_that_leaves_the_file_is_reported() {
    assert!(
        window_handle_methods("export const appWindow = getCurrentWindow()\nsetup(appWindow)\n")
            .is_err(),
        "окно передали в чужую функцию — какие методы позовут там, здесь не видно"
    );
    assert!(
        window_handle_methods("export function useWindow() {\n  return getCurrentWindow()\n}\n")
            .is_err(),
        "окно вернули наружу, не положив в переменную"
    );
    assert!(
        window_handle_methods("const { destroy } = getCurrentWindow()\ndestroy()\n").is_err(),
        "деструктуризация уводит метод из-под привязки к объекту"
    );
}

/// Имя фабрики в операторе импорта за вызов не считается.
#[test]
fn mentioning_the_window_factory_in_an_import_is_not_a_call() {
    assert_eq!(
        window_handle_methods("import { getCurrentWindow } from '@tauri-apps/api/window'\n"),
        Ok(Vec::new())
    );
}

/// Фабрика окна, отданная кому-то в руки, — падение.
///
/// Тот же случай, что окно в переменной, уехавшей из файла, только на
/// шаг раньше: получатель возьмёт окно у себя, а импорта
/// `@tauri-apps/api/window` там уже не будет — искать методы окна станет
/// негде.
#[test]
fn passing_the_window_factory_itself_somewhere_is_reported() {
    assert!(window_handle_methods(
        "import { getCurrentWindow } from '@tauri-apps/api/window'\n\
             export const port = useWindow(getCurrentWindow)\n"
    )
    .is_err());
    assert!(
        window_handle_methods(
            "import { getCurrentWindow } from '@tauri-apps/api/window'\n\
             export { getCurrentWindow }\n"
        )
        .is_err(),
        "реэкспорт уводит окно в файл, который по импортам @tauri-apps/api \
         не найти"
    );
}

/// Разрешена ровно одна оконная команда — и это `destroy`.
///
/// `close()` соседняя по смыслу и на этом месте выглядит естественнее, но
/// он снова поднимает `tauri://close-requested`, то есть возвращает нас в
/// тот же диалог подтверждения; закрывает окно мимо события `destroy` —
/// этим путём идёт и сам `@tauri-apps/api` внутри `onCloseRequested`
/// (TL-47, #49). Тест держит именно это различие: подменить одно другим
/// «чтобы заработало» нельзя молча.
#[test]
fn destroy_is_the_only_window_command_allowed_and_close_is_not() {
    assert_eq!(window_handle_commands("destroy"), Some(&[DESTROY][..]));
    assert!(
        window_handle_commands("close").is_none(),
        "`close()` не должен проходить: разрешения под него нет, и \
         появиться оно может только вместе с разбором цикла \
         close-requested"
    );
    assert!(
        window_handle_commands("onCloseRequested").is_some_and(|cmds| cmds.contains(&DESTROY)),
        "`onCloseRequested` сам зовёт `destroy()`, когда обработчик не \
         вызвал `preventDefault()`: разрешение нужно, хотя вызова в нашем \
         коде не написано"
    );
    let mut context = app_context();
    let authority = context.runtime_authority_mut();
    for label in configured_window_labels() {
        assert!(
            authority
                .resolve_access("plugin:window|close", &label, &label, &Origin::Local)
                .is_none(),
            "ACL пропускает plugin:window|close — соседние оконные команды \
             обязаны оставаться запрещёнными"
        );
    }
}

/// Главный инвариант: всё, что фронтенд может позвать, ACL пропускает.
#[test]
fn every_ipc_command_the_frontend_can_call_is_allowed_by_the_acl() {
    let commands = frontend_ipc_commands();
    let labels = configured_window_labels();
    let mut context = app_context();
    let authority = context.runtime_authority_mut();

    let mut denied = Vec::new();
    for (command, sources) in &commands {
        for label in &labels {
            // Окно и webview в Tauri 2 для окна из конфига имеют одну и ту
            // же метку — так же вызывает `Webview::on_message`.
            if authority
                .resolve_access(command, label, label, &Origin::Local)
                .is_none()
            {
                denied.push(format!(
                    "{command} запрещена для окна «{label}»; вызывается из:\n    {}",
                    sources.iter().cloned().collect::<Vec<_>>().join("\n    ")
                ));
            }
        }
    }

    assert!(
        denied.is_empty(),
        "ACL отклонит эти вызовы в собранном приложении — так и выглядел \
         дефект TL-24. Выдай точечные разрешения в src-tauri/capabilities/ \
         (не `core:default` целиком):\n{}",
        denied.join("\n")
    );
}

/// Разрешения, выданные раньше кода, который их позовёт.
///
/// Обычно так нельзя — «разрешение появляется вместе с вызовом» написано
/// в самой capability, и проверку избытка эта таблица ослабляет.
/// Исключение держится тем, что править capability из области ui нельзя:
/// разрешение и код, который его использует, физически разъезжаются по
/// разным задачам, и в промежутке разрешение стоит без вызова.
///
/// Чтобы аванс не превратился в бессрочный, он гасится не памятью
/// разработчика, а условием: см. `granted_ahead_of_caller`.
const GRANTED_AHEAD_OF_CALLER: &[AdvanceGrant] = &[AdvanceGrant {
    command: DESTROY,
    why: "TL-47 (#49) — под диалог подтверждения выхода из TL-46 (#48): \
          разрешение выдаёт область core, вызывающий его код пишет область ui",
    still_waiting: || !frontend_uses_the_window_module(),
}];

/// Строка таблицы авансов.
struct AdvanceGrant {
    command: &'static str,
    why: &'static str,
    /// Пока это верно, аванс действует; как только перестало — команда
    /// возвращается под обычную проверку избытка.
    still_waiting: fn() -> bool,
}

/// Причина, по которой команда разрешена, хотя вызова за ней ещё нет.
///
/// Аванс гаснет сам: у строки есть условие, и как только оно перестало
/// выполняться, команда возвращается под обычную проверку избытка. Для
/// `destroy` условие — «фронтенд ещё не притронулся к модулю окон»:
/// в тот же коммит, где появится `getCurrentWindow()`, появится и вызов,
/// требующий `destroy`, и подпирать его авансом больше не нужно. Строку
/// после этого можно удалить, но забыть её — не дефект: она уже ничего
/// не разрешает.
fn granted_ahead_of_caller(command: &str) -> Option<&'static str> {
    GRANTED_AHEAD_OF_CALLER
        .iter()
        .find(|grant| grant.command == command && (grant.still_waiting)())
        .map(|grant| grant.why)
}

/// Есть ли во фронтенде хоть один импорт из `@tauri-apps/api/window`.
fn frontend_uses_the_window_module() -> bool {
    let (_, files) = scanned_files();
    files.iter().any(|file| {
        let bytes = fs::read(file).expect("исходник фронтенда читается");
        touches_window_module(&String::from_utf8_lossy(&bytes))
    })
}

fn touches_window_module(content: &str) -> bool {
    parse_api_imports(content)
        .iter()
        .any(|import| import.module.as_deref() == Some("window"))
}

/// Аванс выдан на то, что реально разрешено, и гаснет, когда должен.
///
/// Мёртвая строка (разрешение из capability убрали, а строка осталась)
/// молча выключала бы проверку избытка для этой команды — ни за чем.
/// Погасший аванс, наоборот, обязан перестать действовать сразу.
#[test]
fn a_permission_granted_ahead_of_its_caller_is_actually_granted_and_expires_on_its_own() {
    let labels = configured_window_labels();
    let mut context = app_context();
    let authority = context.runtime_authority_mut();

    for AdvanceGrant {
        command,
        why,
        still_waiting,
    } in GRANTED_AHEAD_OF_CALLER
    {
        for label in &labels {
            assert!(
                authority
                    .resolve_access(command, label, label, &Origin::Local)
                    .is_some(),
                "{command} числится выданной авансом, но ACL её не пропускает \
                 для окна «{label}»: либо разрешение убрали из capability, и \
                 строка осталась мёртвой, либо имя команды написано с \
                 опечаткой ({why})"
            );
        }
        assert_eq!(
            still_waiting(),
            granted_ahead_of_caller(command).is_some(),
            "{command}: условие аванса и его действие разошлись ({why})"
        );
    }

    assert!(
        !frontend_uses_the_window_module() || granted_ahead_of_caller(DESTROY).is_none(),
        "фронтенд начал работать с модулем окон — аванс на {DESTROY} обязан \
         был погаснуть, и дальше разрешение держит обычная проверка избытка"
    );
}

/// Импорт модуля окон виден и в `.ts`, и вместе с другими импортами.
#[test]
fn the_frontend_touching_the_window_module_is_noticed() {
    assert!(touches_window_module(
        "import { getCurrentWindow } from '@tauri-apps/api/window'\n"
    ));
    assert!(
        touches_window_module(
            "import type { CloseRequestedEvent } from '@tauri-apps/api/window'\n"
        ),
        "импорт только типов — тоже работа с модулем окон: код под него \
         пишется в том же файле"
    );
    assert!(!touches_window_module(
        "import { listen } from '@tauri-apps/api/event'\n"
    ));
}

/// Ни одна команда сверх вызываемых фронтендом не разрешена.
///
/// Обратная сторона предыдущего теста: он ловит недостачу, этот — избыток
/// **по оси команд**. `core:default` тянет tray, menu, image, webview и
/// весь `core:window`; `core:event:allow-emit` дал бы фронтенду
/// возможность подделывать события ядра. Команда, за которой нет вызова в
/// `src/`, здесь падает — забрать выданное потом всегда труднее, чем не
/// выдать сейчас. Единственное исключение — `GRANTED_AHEAD_OF_CALLER`, и
/// у него свой тест.
///
/// Две другие оси избытка — origin и метки окон — этим тестом не
/// измеряются: скан идёт с `Origin::Local`, а `windows: ["*"]` даёт для
/// окна `main` тот же ответ, что и точная метка. Их держит
/// `capabilities_stay_local_and_pinned_to_named_windows`.
#[test]
fn no_permission_is_granted_beyond_what_the_frontend_calls() {
    let needed = frontend_ipc_commands();
    let labels = configured_window_labels();
    let mut context = app_context();
    let authority = context.runtime_authority_mut();

    // Прямого перечисления разрешённых команд `RuntimeAuthority` не даёт,
    // поэтому спрашиваем про каждую команду каждого плагина: список берём
    // из ACL-манифестов, сгенерированных tauri-build. Не только core —
    // иначе разрешение стороннего плагина (в E6 придёт updater) осталось
    // бы невидимым (ревью TL-24).
    let mut extra = Vec::new();
    for command in all_plugin_commands() {
        if needed.contains_key(&command) || granted_ahead_of_caller(&command).is_some() {
            continue;
        }
        for label in &labels {
            if authority
                .resolve_access(&command, label, label, &Origin::Local)
                .is_some()
            {
                extra.push(format!("{command} (окно «{label}»)"));
            }
        }
    }

    assert!(
        extra.is_empty(),
        "capability разрешает больше, чем фронтенд вызывает. Либо убери \
         лишнее разрешение, либо, если вызов появился, он должен быть виден \
         в src/:\n{}",
        extra.join("\n")
    );
}

/// Собственные команды приложения ACL не требуют — но ровно до тех пор,
/// пока у приложения нет своего ACL-манифеста.
///
/// Манифест появляется, как только заводится `src-tauri/permissions/`.
/// В этот момент `check_sidecar` и `prepare_ytdlp` начнут отклоняться так
/// же, как отклонялся `plugin:event|listen`, — и опять только в собранном
/// приложении. Тест фиксирует допущение явно, чтобы это выяснилось на
/// `cargo test`, а не на приёмке.
#[test]
fn app_commands_stay_outside_the_acl() {
    let manifests = acl_manifests();
    assert!(
        !manifests.contains_key(APP_ACL_KEY),
        "у приложения появился собственный ACL-манифест ({APP_ACL_KEY}). \
         Теперь команды из generate_handler! тоже проходят проверку ACL: \
         добавь для них разрешения в capability основного окна и внеси их \
         в этот тест."
    );
}

/// ACL-манифесты, сгенерированные `tauri-build` в `gen/schemas/`.
fn acl_manifests() -> BTreeMap<String, serde_json::Value> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("gen/schemas/acl-manifests.json");
    let raw = fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!("{path:?} не читается ({err}); файл генерирует tauri-build при сборке")
    });
    serde_json::from_str(&raw).expect("acl-manifests.json — валидный JSON")
}

/// Все команды всех плагинов в виде `plugin:<плагин>|<команда>` — и
/// core-, и сторонних, когда те появятся.
fn all_plugin_commands() -> BTreeSet<String> {
    let mut commands = BTreeSet::new();
    for (key, manifest) in acl_manifests() {
        if key == APP_ACL_KEY {
            // Команды приложения адресуются без префикса `plugin:`.
            continue;
        }
        // `core:event` → `event`; у стороннего плагина ключ и есть имя.
        let plugin = key.strip_prefix("core:").unwrap_or(&key);
        let permissions = manifest["permissions"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        for permission in permissions.values() {
            let allowed = permission["commands"]["allow"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            for command in allowed {
                if let Some(command) = command.as_str() {
                    commands.insert(format!("plugin:{plugin}|{command}"));
                }
            }
        }
    }
    assert!(
        !commands.is_empty(),
        "в ACL-манифестах не нашлось ни одной команды плагинов — проверка \
         на лишние разрешения превратилась бы в пустую"
    );
    commands
}

/// Capability не открыта наружу и привязана к существующим окнам по имени.
///
/// Мотив отказа от `core:event:allow-emit` — «webview не должен подделывать
/// события ядра»; на этом фоне `remote: { urls: [...] }` опаснее любой
/// лишней команды: он выдаёт весь набор разрешений ещё и стороннему
/// origin, а резолюция с `Origin::Local` этого не видит в принципе.
/// Glob `windows: ["*"]` тем же способом не ловится: для окна `main` он
/// даёт тот же ответ, что точная метка, — но заодно накрывает любое окно,
/// которое заведут потом. Поэтому обе оси проверяются по форме
/// сгенерированного `capabilities.json`, а не через `resolve_access`
/// (ревью TL-24).
#[test]
fn capabilities_stay_local_and_pinned_to_named_windows() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("gen/schemas/capabilities.json");
    let raw = fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!("{path:?} не читается ({err}); файл генерирует tauri-build при сборке")
    });
    let capabilities: BTreeMap<String, serde_json::Value> =
        serde_json::from_str(&raw).expect("capabilities.json — валидный JSON");

    assert!(
        !capabilities.is_empty(),
        "capabilities не сгенерированы — без них webview снова не сможет \
         подписаться на события (дефект TL-24)"
    );

    let labels = configured_window_labels();
    for (identifier, capability) in capabilities {
        assert!(
            !capability
                .get("remote")
                .is_some_and(|remote| !remote.is_null()),
            "capability «{identifier}» открыта для remote-origin. Это \
             раздаёт её разрешения стороннему источнику; в проекте \
             фронтенд только локальный (frontendDist), удалённых окон нет."
        );
        assert_ne!(
            capability.get("local").and_then(serde_json::Value::as_bool),
            Some(false),
            "capability «{identifier}» выключена для локального origin — \
             тогда она либо бесполезна, либо существует ради remote"
        );

        for field in ["windows", "webviews"] {
            let Some(entries) = capability.get(field).and_then(serde_json::Value::as_array) else {
                continue;
            };
            for entry in entries {
                let entry = entry.as_str().unwrap_or_default();
                assert!(
                    !entry.contains('*') && !entry.contains('?'),
                    "capability «{identifier}»: {field} = «{entry}» — glob. \
                     Разрешения обязаны быть привязаны к конкретному окну: \
                     любое окно, заведённое позже, получит их молча."
                );
                assert!(
                    labels.iter().any(|label| label == entry),
                    "capability «{identifier}»: {field} = «{entry}», но окна \
                     с такой меткой в tauri.conf.json нет ({labels:?}) — \
                     разрешения не достанутся никому, и это выяснится \
                     только на собранном приложении"
                );
            }
        }
    }
}

// ───────────────────────── каналы событий (TL-53) ─────────────────────────
//
// Вторая ось того же вопроса «что пересекает границу окна». Первая —
// команды: их ACL действительно проверяет, и тесты выше опираются на
// настоящую резолюцию `RuntimeAuthority`. С каналами так не выйдет, и это
// проверено, а не взято из документации: в ACL-манифесте `core:event`
// поле `global_scope_schema` пустое, а команда `listen` в tauri 2.11.5
// объявлена без `CommandScope` (`src/event/plugin.rs`) — то есть
// `allow-listen` работает по принципу «все каналы или ни одного».
//
// Отсюда разделение труда: разрешение `plugin:event|listen` держат тесты
// выше, а **какие именно каналы** существуют — этот раздел. Список живёт
// в scope у `core:event:allow-listen` в capability основного окна: место
// выбрано потому, что ответ на вопрос «что пересекает границу» должен
// быть в одном файле, а не в двух. Tauri этот scope игнорирует; сторож
// здесь — тест, а не рантайм, и `the_event_permission_still_carries_no_scope_of_its_own`
// покраснеет в тот день, когда это перестанет быть правдой.
//
// Что раздел ловит: канал, который ядро эмитит, а capability не
// объявляет (класс TL-24 — разрешение появилось позже подписчика);
// запись без эмитента (выданное потом трудно забрать); подписку
// фронтенда на канал, которого никто не объявлял, — в том числе опечатку
// в имени, самый тихий из отказов этого рода (подписка на
// `ytdlp://updates` не падает, она просто молчит вечно).
//
// Чего не ловит, и это перечень, а не оговорка. Первое — имя, собранное
// из динамической строки (`format!("{}://{}", …)`, конкатенация): то же
// ограничение, что у скана команд, и та же конвенция против него —
// имена каналов держат в константах. Второе — подписку на канал с
// невиданной схемой: фронтендовый скан считает кандидатами только
// литералы, чья схема уже встречается в ядре, иначе в набор попал бы
// `https://localhost` из теста разбора ссылок. Пробел безвредный: канала
// с новой схемой никто не эмитит, то есть отказ — «событие не приходит»,
// а не «ACL отклонил вызов». Третье — самый первый канал новой схемы,
// если его сразу назвали как адрес (`queue://a/b` при том, что ни одного
// правильного `queue://…` в проекте ещё нет): схема становится канальной
// от первого узнанного имени, и до него отличить такой литерал от адреса
// не по чему.
//
// Чего в этом перечне больше нет — «канал, названный как адрес». Второе
// ревью TL-53 показало, что `"ytdlp://update.v2"` и `"ytdlp://update/done"`
// в ядре без записи в capability дают 26 зелёных тестов; проверка формы
// объявленных каналов туда не дотягивалась по построению — она ходит по
// объявленным, а забытый канал отсутствует в этом множестве ровно
// потому, что его забыли. Теперь адрес со схемой, которая уже известна
// как канальная, — это падение, а не пропуск.
//
// Чего раздел больше не делает — молча пропускать непонятое. Ровно этим
// он и подвёл на ревью TL-53: сегмент `update_done` не разбирался,
// константа исчезала, 24 теста оставались зелёными при двух каналах,
// которых нет ни в одной capability. Теперь всё, что разбор не смог
// отнести ни к каналу, ни к адресу, — падение со списком литералов.

/// Итог разбора одного исходника: найденные каналы и литералы, о которых
/// разбор не может сказать ничего определённого.
///
/// Второе поле существует потому, что первая версия его не имела и молча
/// теряла всё, что не подошло под форму. Ревью TL-53 добавило в ядро
/// `"ytdlp://update_done"` и `"ytdlp://updateDone"` — сегмент с
/// подчёркиванием и с заглавной буквой не разбирался, обе константы
/// пропали, и 24 теста остались зелёными при двух каналах, которых нет ни
/// в одной capability. Это класс TL-24 целиком: отказ ACL, видимый только
/// на собранном `.app`.
#[derive(Default)]
struct ChannelScan {
    channels: BTreeSet<String>,
    /// Литералы, которые не удалось отнести ни к каналу, ни к адресу.
    /// Обязаны стать падением у вызывающего, а не пропуском.
    unparsed: Vec<String>,
    /// Литералы, отнесённые к адресам (`https://i.ytimg.com/vi/abc.jpg`).
    ///
    /// Отдаются наружу, а не выбрасываются на месте, потому что решение
    /// «адрес это или изувеченное имя канала» зависит от схемы, а какие
    /// схемы канальные — известно только после обхода всех файлов. Разбор
    /// одного файла этого знать не может и не притворяется, что может;
    /// взвешивает их [`promote_addresses_with_a_channel_scheme`].
    addresses: Vec<String>,
}

/// Адрес со схемой, которая уже известна как канальная, — это не адрес.
///
/// Последний молчаливый пропуск раздела, найденный на втором ревью
/// TL-53: `"ytdlp://update.v2"` и `"ytdlp://update/done"` в ядре **без**
/// записи в capability давали 26 зелёных тестов. Проверка формы
/// объявленных каналов сюда не дотягивалась по построению — она ходит по
/// объявленным, а забытый канал отсутствует в этом множестве ровно
/// потому, что его забыли.
///
/// Ложных срабатываний не будет: настоящих адресов со схемами `ytdlp` и
/// `download` в проекте нет и быть не может — это наши собственные схемы,
/// а не сетевые. Адрес с чужой схемой (`https`, `ftp`) по-прежнему
/// пропускается: он и есть адрес.
fn promote_addresses_with_a_channel_scheme(
    channels: &BTreeSet<String>,
    addresses: Vec<(String, String)>,
    unparsed: &mut Vec<String>,
) {
    let channel_schemes: BTreeSet<&str> = channels.iter().map(|c| scheme_of(c)).collect();
    // Пара «где» и «что», а не готовая строка: `scheme_of` обязан
    // смотреть на литерал, а не на путь файла перед ним. Первая версия
    // склеивала их раньше времени, схемой оказывался хвост пути, и ветка
    // не срабатывала ни разу — поймано мутацией, не чтением.
    for (whence, literal) in addresses {
        let scheme = scheme_of(&literal);
        if channel_schemes.contains(scheme) {
            unparsed.push(format!(
                "{whence}: {literal} (схема `{scheme}` уже используется \
                 каналами, а имя похоже на адрес)"
            ));
        }
    }
}

/// Разбирает литералы с `://` на каналы и адреса.
///
/// # Что разбор различает и где проходит его настоящая граница
///
/// Кандидат — только содержимое **строкового литерала** в кавычках `"`
/// или `'`. Обратные кавычки не считаются: ими в JSDoc набраны
/// упоминания каналов в прозе, а не подписки.
///
/// Внутри литерала решает остаток после `://`:
///
/// - точка, слеш или двоеточие в остатке — похоже на **адрес** (хост,
///   путь, порт). Окончательное решение принимает не разбор, а
///   вызывающий: адрес с чужой схемой (`https://i.ytimg.com/vi/abc.jpg`,
///   `ftp://example.com/x`) пропускается, а вот такой же по форме
///   литерал с уже канальной схемой (`ytdlp://update.v2`) — это
///   изувеченное имя канала, и он становится падением (см.
///   [`promote_addresses_with_a_channel_scheme`]). Пустой остаток
///   (`"https://"`, фикстура разбора ссылок) — не канал и не адрес;
/// - остаток из `[A-Za-z0-9_-]` при схеме вида `[a-z][a-z0-9]*` — это
///   **канал**, и неважно, как он написан: `update`, `update_done`,
///   `updateDone` разбираются одинаково;
/// - **всё остальное** (пробел, `?`, `%`, экранированная кавычка,
///   не-ASCII) — не пропуск, а запись в [`ChannelScan::unparsed`],
///   то есть падение с объяснением у вызывающего.
///
/// Что осталось за границей, сказано точно: **самый первый канал новой
/// схемы, названный как адрес**. Схема становится канальной оттого, что
/// с ней уже нашёлся хотя бы один правильно названный канал; пока такого
/// нет, `queue://a/b` неотличим от адреса. Как только у схемы появится
/// первое нормальное имя, все её кривые имена начнут падать. Со стороны
/// объявленных каналов та же конвенция закреплена отдельно
/// ([`declared_channels_keep_the_shape_the_scan_can_recognise`]): имя вне
/// формы `[a-z][a-z0-9]*://[A-Za-z0-9_-]+` не сможет тихо появиться в
/// capability.
fn channel_literals(content: &str) -> ChannelScan {
    const SEPARATOR: &str = "://";
    let bytes = content.as_bytes();
    let mut scan = ChannelScan::default();

    for (at, _) in content.match_indices(SEPARATOR) {
        // Схема — слева от разделителя, сразу за открывающей кавычкой.
        let scheme_start = bytes[..at]
            .iter()
            .rposition(|c| !c.is_ascii_alphanumeric())
            .map_or(0, |i| i + 1);
        // Схемы нет вовсе: `scheme + '://' + name` — склейка на месте, а
        // не литерал (ограничение заявлено в шапке раздела).
        if scheme_start == at {
            continue;
        }
        let Some(quote) = scheme_start.checked_sub(1).map(|i| bytes[i]) else {
            continue;
        };
        if quote != b'"' && quote != b'\'' {
            continue;
        }

        // Литерал закрывается той же кавычкой и не переходит на
        // следующую строку.
        let body_start = at + SEPARATOR.len();
        let Some(stop) = bytes[body_start..]
            .iter()
            .position(|c| *c == quote || *c == b'\n')
            .map(|i| body_start + i)
        else {
            scan.unparsed
                .push(format!("{}… (литерал не закрыт)", &content[scheme_start..]));
            continue;
        };
        if bytes[stop] != quote {
            scan.unparsed.push(format!(
                "{} (литерал не закрыт до конца строки)",
                &content[scheme_start..stop]
            ));
            continue;
        }

        let scheme = &content[scheme_start..at];
        let body = &content[body_start..stop];
        let literal = &content[scheme_start..stop];

        // Вырожденный остаток (`"https://"` — фикстура разбора ссылок):
        // ни канал, ни адрес, и сказать о нём нечего.
        if body.is_empty() {
            continue;
        }
        // Похоже на адрес: хост, путь или порт. Окончательное решение —
        // за вызывающим, он один знает канальные схемы.
        if body.contains(['.', '/', ':']) {
            scan.addresses.push(literal.to_string());
            continue;
        }

        let scheme_ok = scheme.starts_with(|c: char| c.is_ascii_lowercase())
            && scheme
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
        let body_ok = body
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');

        if scheme_ok && body_ok {
            scan.channels.insert(literal.to_string());
        } else {
            scan.unparsed.push(literal.to_string());
        }
    }

    scan
}

/// Схема канала — часть до `://`.
fn scheme_of(channel: &str) -> &str {
    channel
        .split_once("://")
        .map_or(channel, |(scheme, _)| scheme)
}

/// Все исходники ядра.
fn core_source_files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
        let entries = fs::read_dir(dir).unwrap_or_else(|err| panic!("{dir:?} не читается ({err})"));
        for entry in entries {
            let path = entry.expect("запись каталога ядра читается").path();
            if path.is_dir() {
                walk(&path, files);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                files.push(path);
            }
        }
    }
    walk(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    assert!(
        !files.is_empty(),
        "в src-tauri/src не нашлось ни одного .rs — скан каналов ядра \
         молча не нашёл бы ничего"
    );
    files
}

/// Каналы, которые эмитит ядро.
fn core_event_channels() -> BTreeSet<String> {
    let mut channels = BTreeSet::new();
    let mut unparsed = Vec::new();
    // Адреса копятся до конца обхода: канальные схемы известны только
    // после того, как найден последний канал.
    let mut addresses = Vec::new();
    for file in core_source_files() {
        let content = fs::read_to_string(&file).expect("исходник ядра читается");
        let scan = channel_literals(&content);
        channels.extend(scan.channels);
        for literal in scan.unparsed {
            unparsed.push(format!("{}: {literal}", file.display()));
        }
        for literal in scan.addresses {
            addresses.push((file.display().to_string(), literal));
        }
    }
    promote_addresses_with_a_channel_scheme(&channels, addresses, &mut unparsed);
    assert!(
        unparsed.is_empty(),
        "в исходниках ядра есть литералы с `://`, которые разбор не смог \
         отнести ни к каналу, ни к адресу:\n{}\nПропустить их нельзя: \
         именно так канал, которого нет в capability, остаётся невидимым \
         до собранного .app (TL-24). Приведите имя к форме \
         `схема://сегмент` либо научите разбор новой форме осознанно.",
        unparsed.join("\n")
    );
    channels
}

/// Каналы, на которые подписывается фронтенд.
///
/// Кандидатами считаются литералы, чья схема уже встречается в ядре:
/// иначе в набор попал бы `https://localhost` из теста разбора ссылок, и
/// сторож требовал бы объявить его каналом. Схемы берутся именно из ядра,
/// а не из capability: возьми мы их оттуда, канал, забытый в capability,
/// перестал бы быть кандидатом ровно потому, что его забыли, — и
/// проверка выключала бы себя сама на том случае, ради которого написана.
fn frontend_event_channels() -> BTreeSet<String> {
    let schemes: BTreeSet<String> = core_event_channels()
        .iter()
        .map(|channel| scheme_of(channel).to_string())
        .collect();
    let (_, files) = scanned_files();
    let mut channels = BTreeSet::new();
    let mut unparsed = Vec::new();
    let mut addresses = Vec::new();
    for file in files {
        let bytes = fs::read(&file).expect("исходник фронтенда читается");
        let scan = channel_literals(&String::from_utf8_lossy(&bytes));
        for channel in scan.channels {
            if schemes.contains(scheme_of(&channel)) {
                channels.insert(channel);
            }
        }
        for literal in scan.unparsed {
            unparsed.push(format!("{}: {literal}", file.display()));
        }
        for literal in scan.addresses {
            addresses.push((file.display().to_string(), literal));
        }
    }
    // Канальные схемы берутся из ядра — те же, по которым отбирались
    // сами каналы: `'ytdlp://prepare/x'` во фронтенде такой же тихий
    // отказ, как в ядре, а `https://…` остаётся адресом.
    let core = core_event_channels();
    promote_addresses_with_a_channel_scheme(&core, addresses, &mut unparsed);
    assert!(
        unparsed.is_empty(),
        "во фронтенде есть литералы с `://`, которые разбор не смог \
         отнести ни к каналу, ни к адресу:\n{}\nЕсли это не про подписку \
         вовсе — литерал всё равно обязан быть разбираемым: молчаливый \
         пропуск здесь означал бы подписку, о которой сторож не знает, а \
         отличить одно от другого разбор не может.",
        unparsed.join("\n")
    );
    channels
}

/// Каналы, объявленные в capability основного окна.
///
/// Читаются из обоих файлов сразу — из исходного `capabilities/main.json`
/// и из `gen/schemas/capabilities.json`, который собирает `tauri-build`.
/// Сверка между ними не педантизм: в приложение уезжает второй, и
/// объявление, застрявшее в первом, было бы объявлением, которого в
/// сборке нет.
fn declared_event_channels() -> BTreeSet<String> {
    fn scope_events(capability: &serde_json::Value) -> BTreeSet<String> {
        let mut events = BTreeSet::new();
        for entry in capability["permissions"].as_array().unwrap_or(&Vec::new()) {
            for allowed in entry["allow"].as_array().unwrap_or(&Vec::new()) {
                if let Some(event) = allowed["event"].as_str() {
                    events.insert(event.to_string());
                }
            }
        }
        events
    }

    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(manifest_dir.join("capabilities/main.json"))
            .expect("capabilities/main.json читается"),
    )
    .expect("capabilities/main.json — валидный JSON");
    let generated: BTreeMap<String, serde_json::Value> = serde_json::from_str(
        &fs::read_to_string(manifest_dir.join("gen/schemas/capabilities.json"))
            .expect("gen/schemas/capabilities.json читается"),
    )
    .expect("capabilities.json — валидный JSON");

    let declared = scope_events(&source);
    let shipped: BTreeSet<String> = generated.values().flat_map(scope_events).collect();
    assert_eq!(
        declared, shipped,
        "capabilities/main.json и сгенерированный gen/schemas/capabilities.json \
         объявляют разные наборы каналов. В приложение уезжает второй; если \
         правку исходника не подхватили — пересоберите (tauri-build \
         перегенерирует его сам)."
    );
    declared
}

/// Ядро и capability говорят об одном и том же наборе каналов.
///
/// Недостача — тот самый TL-24: канал существует, разрешения нет, и в
/// dev-режиме это не проявляется. Избыток — запись без эмитента: она
/// переживёт удаление канала и останется молча разрешать то, чего нет.
#[test]
fn every_event_channel_the_core_emits_is_declared_in_the_capability() {
    let emitted = core_event_channels();
    let declared = declared_event_channels();

    let missing: Vec<&String> = emitted.difference(&declared).collect();
    assert!(
        missing.is_empty(),
        "ядро эмитит каналы, которых нет в capabilities/main.json: {missing:?}\n\
         Объявление обязано появляться в том же изменении, что и канал, и \
         раньше первого подписчика — иначе подписка молча отклоняется ACL \
         только в собранном .app (TL-24)."
    );

    let stale: Vec<&String> = declared.difference(&emitted).collect();
    assert!(
        stale.is_empty(),
        "в capabilities/main.json объявлены каналы, которых ядро не эмитит: \
         {stale:?}\nЛибо канал переименовали и запись осталась мёртвой, либо \
         объявление опередило не только подписчика, но и сам эмитент."
    );
}

/// Фронтенд подписывается только на объявленные каналы.
#[test]
fn the_frontend_listens_only_on_declared_event_channels() {
    let declared = declared_event_channels();
    let undeclared: Vec<String> = frontend_event_channels()
        .into_iter()
        .filter(|channel| !declared.contains(channel))
        .collect();

    assert!(
        undeclared.is_empty(),
        "фронтенд подписывается на каналы, которых никто не объявлял: \
         {undeclared:?}\nТакая подписка не падает — она молчит вечно, и \
         опечатка в имени выясняется только тем, что экран не оживает."
    );
}

/// Скан всё ещё видит каналы, которые в проекте точно есть.
///
/// Без канарейки обе проверки выше вырождаются в «пусто равно пусту»:
/// сломанный разбор литералов дал бы зелёный тест на любом наборе
/// объявлений.
#[test]
fn the_channel_scan_still_finds_the_channels_that_exist() {
    let emitted = core_event_channels();
    for channel in ["ytdlp://prepare", "download://progress", "ytdlp://update"] {
        assert!(
            emitted.contains(channel),
            "скан ядра не нашёл {channel} — разбор литералов сломан, и обе \
             проверки каналов ничего не проверяют"
        );
    }
    assert!(
        frontend_event_channels().contains("ytdlp://prepare"),
        "скан фронтенда не нашёл подписку экрана подготовки, хотя \
         src/composables/useYtDlpPrepare.ts её делает"
    );
}

/// Разбор литерала отличает канал от адреса, от прозы и от непонятного.
///
/// Первые два случая — те самые, на которых ревью TL-53 поймало молчащий
/// сторож: сегмент с подчёркиванием и сегмент с заглавной буквой
/// пропадали, и проверка каналов зеленела при двух необъявленных каналах
/// в ядре.
#[test]
fn a_url_in_the_sources_is_not_mistaken_for_an_event_channel() {
    let found = |source: &str| channel_literals(source).channels;

    for (source, expected) in [
        (
            "const PROGRESS = 'download://progress'\n",
            "download://progress",
        ),
        (
            "pub const E: &str = \"ytdlp://update\";\n",
            "ytdlp://update",
        ),
        (
            "pub const A: &str = \"ytdlp://update_done\";\n",
            "ytdlp://update_done",
        ),
        (
            "pub const B: &str = \"ytdlp://updateDone\";\n",
            "ytdlp://updateDone",
        ),
    ] {
        assert_eq!(
            found(source),
            BTreeSet::from([expected.to_string()]),
            "имя канала обязано находиться в любой форме сегмента: {source}"
        );
    }

    for not_a_channel in [
        "expect(looksLikeUrl('https://i.ytimg.com/vi/abc.jpg')).toBe(true)\n",
        "const partial = 'https://'\n",
        "/// Событие `ytdlp://update` — упоминание в прозе, не литерал\n",
        "const built = scheme + '://' + name\n",
    ] {
        assert!(
            found(not_a_channel).is_empty(),
            "принято за канал: {not_a_channel}"
        );
    }

    assert_eq!(
        found("listen('ytdlp://updates', handler)\n"),
        BTreeSet::from(["ytdlp://updates".to_string()]),
        "опечатка в имени обязана остаться кандидатом — ловить её и есть \
         смысл проверки"
    );
}

/// Непонятная форма — падение с объяснением, а не пропуск.
///
/// Оборотная сторона теста выше и главный урок ревью: пропущенный литерал
/// не оставляет следа, и сторож, который его пропустил, отчитывается
/// зелёным. Всё, что разбор не смог отнести ни к каналу, ни к адресу,
/// обязано доехать до вызывающего списком.
#[test]
fn a_literal_the_scan_cannot_classify_is_reported_instead_of_skipped() {
    for suspicious in [
        "pub const E: &str = \"ytdlp://update done\";\n",
        "pub const E: &str = \"ytdlp://update?force=1\";\n",
        "pub const E: &str = \"ytdlp://обновление\";\n",
        "pub const E: &str = \"Ytdlp://update\";\n",
    ] {
        let scan = channel_literals(suspicious);
        assert!(
            scan.channels.is_empty() && !scan.unparsed.is_empty(),
            "разбор промолчал о литерале, которого не понял: {suspicious}"
        );
    }
}

/// Схема решает, адрес это или изувеченное имя канала.
///
/// Единичный тест на ветку, которой закрыт последний молчаливый пропуск
/// раздела. Обе стороны важны одинаково: наша схема с адресной формой
/// обязана падать, чужая схема с той же формой — оставаться адресом,
/// иначе каждый `https://…` в проекте начал бы ронять проверку каналов.
#[test]
fn an_address_with_our_own_scheme_is_a_mangled_channel_name() {
    let channels = BTreeSet::from([
        "ytdlp://prepare".to_string(),
        "download://progress".to_string(),
    ]);
    let addresses = vec![
        ("ядро".to_string(), "ytdlp://update.v2".to_string()),
        ("ядро".to_string(), "ytdlp://update/done".to_string()),
        (
            "ядро".to_string(),
            "https://i.ytimg.com/vi/abc.jpg".to_string(),
        ),
        ("ядро".to_string(), "ftp://example.com/x".to_string()),
    ];

    let mut unparsed = Vec::new();
    promote_addresses_with_a_channel_scheme(&channels, addresses, &mut unparsed);

    assert_eq!(
        unparsed.len(),
        2,
        "ожидались ровно два повышения — оба с канальной схемой: {unparsed:?}"
    );
    assert!(
        unparsed
            .iter()
            .all(|reported| reported.contains("ytdlp://")),
        "повышен литерал с чужой схемой — каждый адрес в проекте станет \
         падением: {unparsed:?}"
    );
}

/// Объявленные каналы держатся формы, которую разбор умеет узнавать.
///
/// Единственная граница разбора, названная в его doc: канал, названный
/// как адрес (с точкой или слешем в имени), был бы принят за адрес и
/// пропущен. Здесь эта граница и охраняется — имя вне конвенции не
/// сможет тихо появиться в capability, потому что уронит этот тест
/// раньше, чем разойдётся со сканом.
#[test]
fn declared_channels_keep_the_shape_the_scan_can_recognise() {
    let declared = declared_event_channels();
    assert!(
        !declared.is_empty(),
        "в capability не объявлено ни одного канала — проверка формы \
         прошла бы вхолостую"
    );
    for channel in declared {
        let scan = channel_literals(&format!("\"{channel}\""));
        assert_eq!(
            scan.channels,
            BTreeSet::from([channel.clone()]),
            "объявленный канал «{channel}» не разбирается собственным \
             сканом: имя обязано быть вида `схема://сегмент`, где сегмент \
             — из [A-Za-z0-9_-]. Имя с точкой или слешем разбор примет за \
             адрес и пропустит молча, и capability разойдётся с ядром \
             незаметно."
        );
    }
}

/// Растяжка под ногами: список каналов держим мы, а не Tauri.
///
/// Если у `core:event` появится собственная схема scope, наши записи
/// начнёт разбирать чужой валидатор — и совпадение формы (`{"event": …}`)
/// с его схемой станет вопросом, на который нужен ответ, а не догадка.
/// Тот же тест защищает и от обратного прочтения: увидев scope в
/// capability, легко решить, что подписку на посторонний канал отклонит
/// рантайм. Не отклонит.
#[test]
fn the_event_permission_still_carries_no_scope_of_its_own() {
    let manifests = acl_manifests();
    let event = manifests
        .get("core:event")
        .expect("в ACL-манифестах нет core:event");

    assert!(
        event["global_scope_schema"].is_null(),
        "у core:event появилась схема scope — значит Tauri научился сужать \
         события по имени. Сверь форму записей в capabilities/main.json с \
         этой схемой и перепиши доводы здесь: сторожем каналов перестал \
         быть только тест."
    );
    assert!(
        event["permissions"]["allow-listen"]
            .get("scope")
            .is_none_or(serde_json::Value::is_null),
        "у разрешения allow-listen появился собственный scope — см. выше"
    );
}
