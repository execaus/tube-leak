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
//! Ловит: новую подписку/новый вызов Tauri API во фронтенде без выданного
//! разрешения; переименование окна в `tauri.conf.json` мимо `windows`
//! в capability; появление у приложения собственного ACL-манифеста, после
//! которого команды `check_sidecar`/`prepare_ytdlp` тоже потребуют
//! разрешений.
//!
//! Не ловит: вызовы, собранные из динамических строк (`invoke(name)` с
//! именем из переменной) — статически их не видно. Такой код во фронтенде
//! запрещён соглашением: обёртки над `invoke`/`listen` держат имена
//! команд и событий в константах модуля (см. `src/composables/`).

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

/// Разобранный импорт из `@tauri-apps/api/<module>`.
#[derive(Debug)]
struct ApiImport {
    module: String,
    /// `None` — импорт не разобран (namespace/default/динамический):
    /// сузить его до конкретных команд статически нельзя.
    bindings: Option<Vec<String>>,
}

fn frontend_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|err| panic!("не читается {dir:?}: {err}"));
    for entry in entries {
        let path = entry.expect("элемент каталога").path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if path.is_dir() {
            frontend_files(&path, out);
            continue;
        }
        let is_source = name.ends_with(".ts") || name.ends_with(".vue");
        // Тесты фронтенда мокают Tauri API и в рантайме приложения не
        // участвуют — их импорты не создают требований к ACL.
        let is_test = name.ends_with(".test.ts") || name.ends_with(".spec.ts");
        if is_source && !is_test {
            out.push(path);
        }
    }
}

/// Находит импорты из `@tauri-apps/api/*` без полноценного парсера TS:
/// от каждого вхождения спецификатора модуля идём назад до ближайшего
/// `import`/`export` и разбираем список привязок в фигурных скобках.
fn parse_api_imports(content: &str) -> Vec<ApiImport> {
    const PREFIX: &str = "@tauri-apps/api/";
    let mut imports = Vec::new();

    for (idx, _) in content.match_indices(PREFIX) {
        let module: String = content[idx + PREFIX.len()..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();

        let head = &content[..idx];
        let stmt_start = head
            .rfind("import")
            .into_iter()
            .chain(head.rfind("export"))
            .max();
        let bindings = stmt_start.and_then(|start| {
            let stmt = &head[start..];
            // `import type { … }` — только типы, IPC за ними нет.
            if stmt.starts_with("import type") || stmt.starts_with("export type") {
                return Some(Vec::new());
            }
            let open = stmt.find('{')?;
            let close = stmt.rfind('}')?;
            if close < open {
                return None;
            }
            Some(
                stmt[open + 1..close]
                    .split(',')
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    // `type Event as TauriEvent` — тип, не вызов.
                    .filter(|item| !item.starts_with("type "))
                    .map(|item| item.split(" as ").next().unwrap_or(item).trim().to_string())
                    .collect(),
            )
        });

        imports.push(ApiImport { module, bindings });
    }

    imports
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
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(FRONTEND_SRC);
    let mut files = Vec::new();
    frontend_files(&root, &mut files);
    assert!(
        !files.is_empty(),
        "в {root:?} не нашлось ни одного исходника фронтенда — тест бы \
         молча разрешил всё; проверь путь"
    );

    let mut commands: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut unknown: Vec<String> = Vec::new();

    for file in &files {
        let content = fs::read_to_string(file).expect("исходник фронтенда читается");
        let shown = file
            .strip_prefix(&root)
            .unwrap_or(file)
            .display()
            .to_string();

        for import in parse_api_imports(&content) {
            let Some(bindings) = import.bindings else {
                unknown.push(format!(
                    "{shown}: импорт из @tauri-apps/api/{} разобрать не удалось \
                     (namespace/default/динамический импорт). Фронтенд обязан \
                     импортировать именованные привязки — иначе набор команд \
                     статически не виден.",
                    import.module
                ));
                continue;
            };
            for binding in bindings {
                match ipc_commands_of(&import.module, &binding) {
                    Some(cmds) => {
                        for cmd in cmds {
                            commands
                                .entry((*cmd).to_string())
                                .or_default()
                                .insert(format!("{shown}: {}.{binding}", import.module));
                        }
                    }
                    None => unknown.push(format!(
                        "{shown}: @tauri-apps/api/{}.{binding} — неизвестная привязка. \
                         Добавь её в `ipc_commands_of` (и, если она делает IPC, \
                         выдай разрешение в src-tauri/capabilities/).",
                        import.module
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

/// Канарейка: скан всё ещё видит подписку, которая во фронтенде точно
/// есть.
///
/// Без неё вся проверка вырождается молча: сломанный разбор импортов даёт
/// пустой набор команд, и «всё разрешено» становится «нечего проверять».
/// Если экран подготовки когда-нибудь перестанет подписываться на события
/// — правь этот тест осознанно, вместе с разрешением в capability.
#[test]
fn the_scan_still_finds_the_subscription_the_prepare_screen_makes() {
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
    let by_module: Vec<(String, Option<Vec<String>>)> = imports
        .into_iter()
        .map(|i| (i.module, i.bindings))
        .collect();

    assert_eq!(
        by_module,
        vec![
            ("core".to_string(), Some(vec!["invoke".to_string()])),
            ("event".to_string(), Some(vec!["listen".to_string()])),
            // `import type` не порождает вызовов вообще.
            ("window".to_string(), Some(Vec::new())),
        ]
    );
}

#[test]
fn a_namespace_import_is_reported_instead_of_being_silently_skipped() {
    let imports = parse_api_imports("import * as event from '@tauri-apps/api/event'\n");
    assert_eq!(imports.len(), 1);
    assert!(
        imports[0].bindings.is_none(),
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

/// Разрешения выданы под конкретный вызов, а не «на вырост».
///
/// Обратная сторона предыдущего теста: он ловит недостачу, этот — избыток.
/// `core:default` тянет tray, menu, image, webview и весь `core:window`;
/// `core:event:allow-emit` дал бы фронтенду возможность подделывать
/// события ядра. Разрешение, за которым нет вызова в `src/`, здесь падает
/// — забрать выданное потом всегда труднее, чем не выдать сейчас.
#[test]
fn no_permission_is_granted_beyond_what_the_frontend_calls() {
    let needed = frontend_ipc_commands();
    let labels = configured_window_labels();
    let mut context = app_context();
    let authority = context.runtime_authority_mut();

    // Прямого перечисления разрешённых команд `RuntimeAuthority` не даёт,
    // поэтому спрашиваем про каждую команду всех core-плагинов: список
    // берём из ACL-манифестов, сгенерированных tauri-build.
    let mut extra = Vec::new();
    for command in all_core_plugin_commands() {
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

/// Все команды всех core-плагинов в виде `plugin:<плагин>|<команда>`.
fn all_core_plugin_commands() -> BTreeSet<String> {
    let mut commands = BTreeSet::new();
    for (key, manifest) in acl_manifests() {
        let Some(plugin) = key.strip_prefix("core:") else {
            continue;
        };
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
        "в ACL-манифестах не нашлось ни одной core-команды — проверка на \
         лишние разрешения превратилась бы в пустую"
    );
    commands
}
