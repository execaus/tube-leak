//! Сторож политики содержимого (CSP, TL-28).
//!
//! # Зачем этот тест существует
//!
//! Политика содержимого — ровно тот же класс риска, что ACL в TL-24
//! (см. `frontend_acl.rs`): ограничение живёт между webview и
//! конфигурацией, не видно ни тестам фронтенда, ни логам ядра, и
//! проявляется только на собранном приложении. Мало того, в `tauri dev`
//! заголовок не навешивается вовсе — не потому, что в dev нет политики
//! (`AppManager::csp` и там отдаёт ту же самую), а потому, что html в dev
//! отдаёт Vite, а не Tauri, и вешать заголовок некуда. Значит цикл
//! «поправил — проверил» для политики стоит целой сборки бандла, и без
//! теста следующая правка попадёт к владельцу непроверенной.
//!
//! Поводов тронуть политику в следующих эпиках хватает: веб-шрифт,
//! первый ленивый экран, `useHttpsScheme` у окна, «расширить источники
//! ради апдейтера». Каждый из них ломает что-нибудь молча — вызовы
//! сползают на запасной транспорт, картинка не грузится, — и ни один не
//! даёт сигнала до сборки. Этот тест такой сигнал даёт.
//!
//! # Что проверяется и чего тест не умеет
//!
//! Проверяется: политика вообще задана и разбирается; ни одна директива
//! не выдаёт послаблений `unsafe-*`; транспорт вызовов разрешён в **обеих**
//! формах адреса; схема `https` у окна не включена в обход политики;
//! набор хостов превью ровно тот, что сверен на живой выдаче yt-dlp;
//! директивы, которые не наследуются от `default-src`, выписаны явно; сам
//! `default-src` остался узким (иначе рассуждение «остальное наследуется»
//! разваливается).
//!
//! Не проверяется: что политика **применилась** в webview. Это
//! принципиально недостижимо отсюда — заголовок появляется только на
//! собранном приложении, и проверяется он запуском `.app` со
//! смонтированного `.dmg` (как в TL-28). Тест сторожит намерение, а не
//! факт применения.
//!
//! # Почему конфиг читается файлом, а не через `generate_context!`
//!
//! Макрос втягивает в тестовый бинарник всю сборку фронтенда ради одной
//! строки конфигурации. Разбор идёт через тот же тип `Config` и ту же
//! конверсию `Csp` → карта директив, которой пользуется рантайм
//! (`tauri::manager::set_csp`), так что форма записи политики — строкой
//! или объектом — на проверку не влияет. Единственное, что теряется, —
//! слияние с платформенными `tauri.<платформа>.conf.json`; на этот случай
//! есть отдельное утверждение, что таких файлов в проекте нет.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use tauri::utils::config::{Config, CspDirectiveSources};

/// Где лежит обоснование политики. Любое падение обязано привести сюда:
/// повод править политику звучит как «картинка не грузится», и такой
/// человек открывает `tauri.conf.json`, где обратной ссылки нет.
const REASONING: &str = "src-tauri/src/main.rs (doc-блок «Политика содержимого»)";

/// Хосты превью, разрешённые в `img-src`, — ровно те, что сверены на
/// живой выдаче yt-dlp (TL-28).
const EXPECTED_IMG_SOURCES: &[&str] = &["'self'", "https://i.ytimg.com"];

/// Формы адреса IPC: `invoke` в Tauri 2 — это `fetch` на `ipc://localhost`
/// (macOS, Linux) либо на `http://ipc.localhost` (Windows). Политика одна
/// на все платформы, поэтому нужны обе.
const IPC_SOURCES: &[&str] = &["ipc:", "http://ipc.localhost"];

/// Директивы, которые **не** наследуются от `default-src`: не выписал —
/// значит не ограничил.
const NON_INHERITING_DIRECTIVES: &[&str] = &["base-uri", "form-action", "frame-ancestors"];

fn manifest_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// Конфигурация приложения, разобранная теми же типами, что и в рантайме.
fn config() -> Config {
    let path = manifest_dir().join("tauri.conf.json");
    let raw = fs::read_to_string(&path).unwrap_or_else(|err| panic!("{path:?} не читается: {err}"));
    serde_json::from_str(&raw).unwrap_or_else(|err| {
        panic!(
            "{path:?} не разбирается типом tauri::utils::config::Config: {err}. \
             Приложение с такой конфигурацией не соберётся; см. {REASONING}"
        )
    })
}

/// Политика в виде «директива → источники», через ту же конверсию
/// `From<Csp>`, которой пользуется `set_csp` в рантайме.
///
/// Имена директив и источники приводятся к нижнему регистру: CSP
/// регистронезависим и в именах директив, и в хостах, а сравнивать удобнее
/// нормализованное.
fn policy() -> HashMap<String, BTreeSet<String>> {
    let csp = config().app.security.csp.unwrap_or_else(|| {
        panic!(
            "`app.security.csp` в tauri.conf.json не задан: у webview нет политики \
             содержимого вообще. Так было до TL-28 — и это не «разрешено всё по \
             умолчанию» в безобидном смысле: страница могла бы обратиться куда \
             угодно. См. {REASONING}"
        )
    });

    let map: HashMap<String, CspDirectiveSources> = csp.into();
    let policy: HashMap<String, BTreeSet<String>> = map
        .into_iter()
        .map(|(directive, sources)| {
            let sources: Vec<String> = sources.into();
            (
                directive.to_ascii_lowercase(),
                sources
                    .into_iter()
                    .filter(|source| !source.trim().is_empty())
                    .map(|source| source.trim().to_ascii_lowercase())
                    .collect(),
            )
        })
        .collect();

    // Канарейка: разбор мог выродиться в пустую карту (например, политику
    // записали так, что конверсия не нашла ни одной директивы), и тогда
    // все проверки ниже стали бы зелёными, ничего не проверяя.
    assert!(
        policy.len() >= NON_INHERITING_DIRECTIVES.len(),
        "из политики разобралось всего {} директив ({:?}) — на настоящую \
         политику это не похоже, а проверки ниже на такой карте вырождаются \
         в «всё хорошо». Проверь запись политики; см. {REASONING}",
        policy.len(),
        policy.keys().collect::<BTreeSet<_>>()
    );

    policy
}

/// Источники директивы; `None` — директивы в политике нет.
fn sources_of(
    policy: &HashMap<String, BTreeSet<String>>,
    directive: &str,
) -> Option<BTreeSet<String>> {
    policy.get(directive).cloned()
}

#[test]
fn the_policy_is_set_and_parses_into_directives() {
    let policy = policy();
    for required in [
        "default-src",
        "img-src",
        "script-src",
        "style-src",
        "connect-src",
    ] {
        assert!(
            policy.contains_key(required),
            "в политике нет директивы {required}, хотя приложение на неё \
             опирается. Наследование от `default-src` здесь не спасает: \
             оно узкое ('self'), а превью и вызовы команд ходят наружу. \
             См. {REASONING}"
        );
    }
}

/// Послаблений `unsafe-*` в политике нет — и в первую очередь их нет в
/// `style-src`, хотя `:style` и `v-show` во фронтенде есть.
#[test]
fn no_directive_grants_an_unsafe_source() {
    let policy = policy();
    let granted: Vec<String> = policy
        .iter()
        .flat_map(|(directive, sources)| {
            sources
                .iter()
                .filter(|source| source.contains("unsafe-"))
                .map(move |source| format!("{directive} {source}"))
        })
        .collect();

    assert!(
        granted.is_empty(),
        "политика выдала послабления: {granted:?}.\n\
         Если это `'unsafe-inline'` в `style-src` ради `:style`/`v-show` — \
         оно не нужно: CSP запрещает разбор атрибута `style`, а Vue ставит \
         стили через CSSOM, и `transformStyle` в @vue/compiler-dom \
         превращает даже литеральный `style=\"…\"` в привязку. Проверено \
         на бандле в TL-28: полоса прогресса и плейсхолдер превью работают \
         без послабления. Если причина другая — она должна быть записана \
         в {REASONING} рядом с директивой."
    );
}

/// Транспорт вызовов разрешён в обеих формах адреса.
///
/// Отказ здесь молчаливый: `invoke` не падает, а сползает на запасной
/// `postMessage` (см. `ipc-protocol.js` в крейте tauri) — ровно тот класс,
/// что стоил приёмки в TL-24.
#[test]
fn the_ipc_transport_is_allowed_in_both_of_its_address_forms() {
    let policy = policy();
    let connect = sources_of(&policy, "connect-src").unwrap_or_else(|| {
        panic!(
            "в политике нет `connect-src`, а он наследуется от `default-src 'self'` \
             — адрес IPC под 'self' не подходит ни на одной платформе. См. {REASONING}"
        )
    });

    for source in IPC_SOURCES {
        assert!(
            connect.contains(*source),
            "`connect-src` не содержит {source}; сейчас там {connect:?}. \
             Нужны обе формы: `ipc://localhost` на macOS и Linux, \
             `http://ipc.localhost` на Windows — политика одна на все \
             платформы, и проверить её живьём можно только на той, где \
             собираешь. Без источника вызовы команд не падают, а тихо \
             уходят на запасной транспорт. См. {REASONING}"
        );
    }
}

/// Ловушка `useHttpsScheme`: она молча переводит адрес вызовов на
/// `https://ipc.localhost`, которого в политике нет.
#[test]
fn turning_on_the_https_scheme_requires_widening_connect_src_first() {
    let policy = policy();
    let connect = sources_of(&policy, "connect-src").unwrap_or_default();
    let https_allowed = connect.contains("https://ipc.localhost");

    for window in &config().app.windows {
        assert!(
            !window.use_https_scheme || https_allowed,
            "у окна {:?} включён `useHttpsScheme`, а `connect-src` \
             разрешает только {connect:?}. Схема меняет адрес IPC на \
             `https://ipc.localhost`, и вызовы уедут на запасной транспорт \
             молча — ни ошибки, ни лога. Сначала источник, потом схема. \
             См. {REASONING}",
            window.label
        );
    }
}

/// Хосты превью — ровно те, что сверены на живой выдаче yt-dlp.
#[test]
fn the_preview_hosts_are_exactly_the_ones_verified_against_yt_dlp() {
    let policy = policy();
    let img = sources_of(&policy, "img-src").unwrap_or_else(|| {
        panic!(
            "в политике нет `img-src`, значит картинки наследуют \
             `default-src 'self'` и превью ролика (Р-2) не загрузится вовсе. \
             См. {REASONING}"
        )
    });
    let expected: BTreeSet<String> = EXPECTED_IMG_SOURCES.iter().map(|s| s.to_string()).collect();

    assert_eq!(
        img, expected,
        "набор источников `img-src` разошёлся с проверенным.\n\
         Расширение — это не строчка в конфиге: хост нужно увидеть в живой \
         выдаче yt-dlp (поле `thumbnail`), а не предположить. Отказ здесь \
         мягкий — на неразрешённой ссылке VideoThumbnail показывает \
         плейсхолдер и карточку не ломает, — поэтому «на всякий случай» \
         добавлять нечего. Сужение — тем более осознанная правка. \
         Обнови вместе с {REASONING}."
    );
}

/// Директивы, которые не наследуются от `default-src`, выписаны явно.
#[test]
fn directives_that_never_fall_back_to_default_src_are_spelled_out() {
    let policy = policy();
    for directive in NON_INHERITING_DIRECTIVES {
        assert!(
            policy.contains_key(*directive),
            "в политике нет `{directive}` — эта директива не наследуется от \
             `default-src` ни при каких условиях, то есть не выписана = не \
             ограничена. См. {REASONING}"
        );
    }
}

/// `default-src` остался узким.
///
/// На нём держится всё рассуждение «директивы, которых в политике нет
/// (`font-src`, `media-src`, `worker-src`, `manifest-src`), наследуются и
/// потому безопасны». Расширили `default-src` — рассуждение развалилось
/// молча, и вместе с ним все ненаписанные директивы.
#[test]
fn default_src_stays_the_narrow_fallback_the_unwritten_directives_inherit() {
    let policy = policy();
    let default_src = sources_of(&policy, "default-src").unwrap_or_default();
    let expected: BTreeSet<String> = ["'self'".to_string()].into_iter().collect();

    assert_eq!(
        default_src, expected,
        "`default-src` перестал быть узким. От него наследуются все \
         директивы, которых в политике нет (`font-src`, `media-src`, \
         `worker-src`, `manifest-src`), — расширение открывает их все \
         сразу и незаметно. Если наружу нужен конкретный ресурс, выпиши \
         его директиву отдельно. См. {REASONING}"
    );
}

/// `devCsp` не задан.
///
/// Соблазн понятен: в dev политика не видна, и кажется, что дело в
/// отдельной dev-политике. Дело не в ней — в dev html отдаёт Vite, а не
/// Tauri, и навесить заголовок негде. `devCsp` завёл бы вторую политику,
/// которую никто не проверяет и которая всё равно не применяется.
#[test]
fn there_is_exactly_one_policy_and_no_dev_only_second_one() {
    assert!(
        config().app.security.dev_csp.is_none(),
        "задан `app.security.devCsp` — вторая политика, которую не видит ни \
         этот тест, ни webview: в dev заголовок не навешивается независимо \
         от того, что в конфиге. Видимости в dev это не вернёт, а расхождение \
         двух политик заведёт. См. {REASONING}"
    );
}

/// Платформенных конфигураций в проекте нет.
///
/// Fail-closed: `tauri.<платформа>.conf.json` сливается с основным
/// конфигом на сборке, но не при чтении файла здесь. Появится такой файл —
/// тест обязан упасть и потребовать пересмотра, а не молча проверять
/// половину правды.
#[test]
fn no_platform_specific_config_can_override_the_policy_behind_the_tests_back() {
    let dir = manifest_dir();
    let overrides: Vec<String> = fs::read_dir(&dir)
        .unwrap_or_else(|err| panic!("{dir:?} не читается: {err}"))
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| {
            let name = name.to_ascii_lowercase();
            name.starts_with("tauri.") && name.contains(".conf.") && name != "tauri.conf.json"
        })
        .collect();

    assert!(
        overrides.is_empty(),
        "в src-tauri/ появились платформенные конфигурации {overrides:?}. \
         Они сливаются с основной на сборке, но этот тест читает только \
         tauri.conf.json — то есть политику под соответствующую платформу \
         больше никто не сторожит. Либо перенеси политику обратно в общий \
         конфиг, либо научи тест слиянию. См. {REASONING}"
    );
}
