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
//! литеральному имени; разрешение, за которым нет вызова; capability,
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
    const NONE: &[&str] = &[];
    // `listen()` возвращает `UnlistenFn`, который дёргает
    // `plugin:event|unlisten`; отписка в `onUnmounted` — обычный путь, а не
    // экзотика, поэтому подписка всегда тянет за собой оба разрешения.
    const LISTEN: &[&str] = &["plugin:event|listen", "plugin:event|unlisten"];

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
        _ => None,
    }
}

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

/// Ни одна команда сверх вызываемых фронтендом не разрешена.
///
/// Обратная сторона предыдущего теста: он ловит недостачу, этот — избыток
/// **по оси команд**. `core:default` тянет tray, menu, image, webview и
/// весь `core:window`; `core:event:allow-emit` дал бы фронтенду
/// возможность подделывать события ядра. Команда, за которой нет вызова в
/// `src/`, здесь падает — забрать выданное потом всегда труднее, чем не
/// выдать сейчас.
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
        if needed.contains_key(&command) {
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
