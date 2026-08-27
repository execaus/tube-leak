//! Генерация TS-зеркала контракта из [`crate::types`] и сторожа при ней
//! (TL-51).
//!
//! # Зачем модуль существует
//!
//! CLAUDE.md обещает: типы границы объявляются один раз в Rust, а
//! расхождение ловится **на сборке, а не в рантайме**. До TL-51 вторая
//! половина обещания не выполнялась: зеркало в `src/types/*.ts` писалось
//! руками, и когда в Rust появился новый класс ошибки подготовки yt-dlp, а
//! в TS его не завели, `npm run type-check` остался зелёным. Белый список
//! `KNOWN_ERROR_KINDS` в композабле превратил незнакомый `kind` в заглушку
//! и **потерял текст сообщения** — то есть пользователь получил совет хуже,
//! чем до появления класса. Дыру нашёл человек на ревью.
//!
//! Теперь зеркало генерируется, лежит в репозитории (`src/types/generated/`)
//! и сверяется в CI: джоб `test` прогоняет `cargo test`, который и есть
//! генератор, а затем падает, если перегенерация дала diff.
//!
//! # Почему ts-rs, а не tauri-specta
//!
//! Обоснование целиком — в комментарии над зависимостью в `Cargo.toml` и в
//! отчёте задачи; здесь коротко о том, что проверялось живьём на настоящих
//! типах этого контракта, а не бралось из документации:
//!
//! - **Внутренне-тегированные объединения не разворачиваются в «всё
//!   опционально».** `#[serde(tag = "phase")]` у [`crate::types::DownloadProgress`]
//!   даёт в TS честное размеченное объединение, а вложенный второй тег
//!   (`DownloadProgress::Downloading(DownloadingState)`) — пересечение
//!   `{ "phase": "downloading" } & DownloadingState`. Отвергнутое состояние
//!   остаётся структурно невыразимым, ради чего форма и выбиралась.
//! - **`skip_serializing_if` даёт `field?: T`, а не `field: T | null`.**
//!   Это не поведение по умолчанию: по умолчанию ts-rs пишет `T | null`, и
//!   именно поэтому у каждого типа стоит `#[ts(optional_fields)]`. Оба
//!   инварианта — что поля отсутствуют и что nullable не появился —
//!   сторожатся тестами ниже, а не доверием к атрибуту.
//!
//! # Цена и её границы
//!
//! `ts-rs` — dev-зависимость, `#[derive(TS)]` навешивается через
//! `#[cfg_attr(test, ...)]`. В релизном графе не прибавляется ни одного
//! крейта (замерено `cargo tree -e normal --target <тройка>` по всем трём
//! тройкам), в бандл не попадает ни строки генератора.
//!
//! Побочный эффект, который стоит назвать прямо: `cargo test` **пишет в
//! `src/types/generated/`**, то есть за пределы `src-tauri/`. Это цена
//! выбранной схемы — генератор запускается тем же прогоном, что и тесты,
//! и разработчику не нужно помнить отдельную команду.

use std::{fs, path::PathBuf};

use ts_rs::{Config, TS};

use super::*;

/// Каталог, в который уезжает сгенерированное зеркало.
///
/// Считается от `CARGO_MANIFEST_DIR`, а не от текущего каталога процесса:
/// `cargo test` запускает тестовый бинарник с cwd = корень пакета, но
/// полагаться на это незачем — путь известен на компиляции.
fn generated_dir() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../src/types/generated"
    ))
}

/// Настройки генерации.
///
/// `with_large_int("number")` обязателен и не косметика: по умолчанию
/// ts-rs отображает `u64`/`i64` в `bigint`, а через границу Tauri числа
/// едут обычным JSON — на стороне webview это `number`. С `bigint` зеркало
/// разошлось бы с проводом на каждом размере, скорости и таймауте.
fn config() -> Config {
    Config::new()
        .with_large_int("number")
        .with_out_dir(generated_dir())
}

/// Единственный список типов границы. Всё, что ниже, ходит по нему.
///
/// Порядок здесь ни на что не влияет (ts-rs раскладывает объявления в
/// файле по алфавиту), а вот полнота влияет: тип, не попавший в список, в
/// зеркале не появится и разойтись с ним не сможет — потому что его там
/// нет. Сторож полноты — [`the_list_covers_every_contract_type`].
macro_rules! with_contract_types {
    ($macro_name:ident $(, $arg:expr)?) => {
        $macro_name![
            $($arg,)?
            // sidecar.ts
            SidecarStatus,
            LaunchFailedReason,
            SidecarCheckResult,
            SidecarCheckReport,
            // ytdlp.ts
            YtDlpPrepareStage,
            YtDlpPrepareErrorKind,
            YtDlpPrepareError,
            YtDlpPrepareEvent,
            YtDlpPrepared,
            // probe.ts
            QualityKind,
            QualitySize,
            QualityStreams,
            QualityItem,
            ProbeResult,
            ProbeErrorKind,
            YtDlpFailureReason,
            ProbeErrorDetails,
            ProbeError,
            // download.ts
            DownloadPhase,
            DownloadPlan,
            DownloadStream,
            DownloadPercent,
            DownloadAttempt,
            PartialData,
            DownloadErrorKind,
            DownloadErrorDetails,
            DownloadError,
            DownloadingState,
            DownloadProgress,
            DownloadProgressEvent,
            StartDownloadRequest,
            DownloadStarted,
            DownloadCommandErrorKind,
            DownloadCommandError,
        ]
    };
}

macro_rules! export_each {
    ($cfg:expr, $($ty:ty),+ $(,)?) => {
        $(
            <$ty as TS>::export($cfg)
                .unwrap_or_else(|e| panic!("не удалось выгрузить {}: {e}", stringify!($ty)));
        )+
    };
}

macro_rules! declaration_of_each {
    ($cfg:expr, $($ty:ty),+ $(,)?) => {
        vec![$((
            stringify!($ty),
            <$ty as TS>::export_to_string($cfg)
                .unwrap_or_else(|e| panic!("не удалось объявить {}: {e}", stringify!($ty))),
        )),+]
    };
}

macro_rules! name_of_each {
    ($($ty:ty),+ $(,)?) => {
        vec![$(stringify!($ty)),+]
    };
}

/// Шапка, которой заменяется английская пометка ts-rs.
///
/// Пометка генератора остаётся первой строкой не случайно: файл открывают
/// в первую очередь те, кто пришёл править контракт «по-быстрому», и они
/// должны сразу видеть, что правка отсюда не переживёт ближайший
/// `cargo test`.
const HEADER: &str = "\
// Файл СГЕНЕРИРОВАН из src-tauri/src/types.rs. Руками не править: правка
// живёт в Rust, сюда она приезжает перегенерацией.
//
//   перегенерация:  cd src-tauri && cargo test --locked
//   сторож:         джоб test падает, если перегенерация даёт diff (TL-51)
//
// Комментарии ниже — те же doc-комментарии, что стоят у типов в types.rs;
// расходиться с ними этот файл не может по построению.
";

/// Собственно генерация: один тест, потому что все типы пишутся в четыре
/// общих файла.
///
/// Разбить его на тест-на-тип нельзя: ts-rs сливает объявления в один
/// файл через процесс-глобальную таблицу уже записанных путей, а тесты
/// cargo идут параллельно — при фильтрации (`cargo test some_filter`)
/// файл оказался бы обрезан до подмножества типов. Один тест снимает и
/// параллельность, и частичность разом.
#[test]
fn typescript_mirror_is_generated_from_the_rust_contract() {
    let cfg = config();
    let dir = generated_dir();

    // Каталог чистится целиком: файл, оставшийся от типа, который из
    // контракта убрали, иначе жил бы вечно и продолжал бы казаться
    // сгенерированным.
    if dir.exists() {
        fs::remove_dir_all(&dir).expect("не удалось очистить каталог зеркала");
    }
    fs::create_dir_all(&dir).expect("не удалось создать каталог зеркала");

    with_contract_types!(export_each, &cfg);

    for entry in fs::read_dir(&dir).expect("не удалось прочитать каталог зеркала")
    {
        let path = entry.expect("не удалось прочитать запись каталога").path();
        let body = fs::read_to_string(&path).expect("не удалось прочитать сгенерированный файл");
        let without_note = body
            .split_once('\n')
            .map(|(_, rest)| rest.to_owned())
            .expect("сгенерированный файл без пометки генератора");
        fs::write(&path, format!("{HEADER}{without_note}"))
            .expect("не удалось переписать шапку сгенерированного файла");
    }
}

/// Опциональное поле контракта обязано **отсутствовать** в JSON, а не
/// приходить как `null` (это правило E1/E2/E3, проверенное сравнением
/// сериализованных значений целиком в `super::tests`). Здесь проверяется
/// вторая половина: что и сгенерированный TS говорит то же самое.
///
/// Сторож нужен потому, что поведение по умолчанию у ts-rs
/// противоположное: `Option<T>` без `#[ts(optional_fields)]` становится
/// `T | null`. Потерять атрибут у одного типа из тридцати четырёх легко, а
/// заметить — нет: `null` в TS-типе прекрасно проходит и type-check, и
/// сборку, и падает только у пользователя.
#[test]
fn no_field_of_the_contract_is_nullable_in_typescript() {
    let cfg = config();
    let declarations: Vec<(&str, String)> = with_contract_types!(declaration_of_each, &cfg);

    for (name, declaration) in declarations {
        let code = strip_jsdoc(&declaration);
        assert!(
            !code.contains("null"),
            "{name}: в сгенерированном TS появился nullable — опциональное поле \
             контракта обязано отсутствовать в JSON, а не приезжать как null.\n{code}"
        );
    }
}

/// Оборотная сторона предыдущего сторожа.
///
/// `#[ts(optional_fields)]` стоит на каждом типе контракта и делает
/// `field?: T` из **любого** `Option<T>` — в том числе из такого, который
/// на проводе честно приезжал бы как `null`. Пара «`Option<T>` +
/// `skip_serializing_if`» держится этим тестом, а не памятью автора
/// следующего поля: без `skip_serializing_if` зеркало обещало бы
/// отсутствие ключа там, где ядро шлёт `null`, — и это расхождение не
/// поймал бы ни один из тестов сериализации, потому что они смотрят на
/// значения, а не на TS.
#[test]
fn every_optional_field_of_the_contract_is_omitted_when_absent() {
    let mut checked = 0usize;
    let contract: Vec<&str> = contract_source().lines().collect();

    for (i, line) in contract.iter().enumerate() {
        let field = line.trim();
        if !field.contains(": Option<") || field.contains("fn ") {
            continue;
        }

        let attribute = contract[..i]
            .iter()
            .rev()
            .map(|l| l.trim())
            .find(|l| !l.is_empty() && !l.starts_with("//"))
            .unwrap_or("");

        assert!(
            attribute.contains(r#"skip_serializing_if = "Option::is_none""#),
            "поле контракта `{field}` объявлено как Option, но не помечено \
             `skip_serializing_if = \"Option::is_none\"`. Из-за \
             `#[ts(optional_fields)]` зеркало объявит его как `field?: T`, \
             то есть пообещает отсутствие ключа — а ядро пришлёт `null`."
        );
        checked += 1;
    }

    // Сам сторож тоже может протухнуть — например, если разбор строк
    // перестанет узнавать поля. Число берётся с запасом снизу: важно, что
    // тест что-то действительно смотрел, а не прошёл по пустому списку.
    assert!(
        checked >= 20,
        "сторож не нашёл полей Option в контракте — разбор `types.rs` сломался \
         (найдено {checked})"
    );
}

/// Полнота списка: в контракте не осталось типа, который зеркалу не
/// известен.
///
/// Проверяется по исходнику `types.rs`, а не по списку самому по себе —
/// иначе тест сверял бы список с собой. Пропущенный тип — самый тихий из
/// возможных отказов этой задачи: расхождения не будет, потому что
/// сравнивать будет не с чем.
#[test]
fn the_list_covers_every_contract_type() {
    let listed: Vec<&str> = with_contract_types!(name_of_each);

    let declared: Vec<&str> = contract_source()
        .lines()
        .filter_map(|line| {
            let rest = line
                .strip_prefix("pub struct ")
                .or_else(|| line.strip_prefix("pub enum "))?;
            Some(
                rest.split(|c: char| !c.is_alphanumeric() && c != '_')
                    .next()
                    .unwrap_or(rest),
            )
        })
        .collect();

    for name in &declared {
        assert!(
            listed.contains(name),
            "тип границы `{name}` объявлен в types.rs, но не попал в список \
             генерации — TS-зеркала у него не будет, и разойтись с ним он не \
             сможет за отсутствием второй стороны"
        );
    }
    assert_eq!(
        declared.len(),
        listed.len(),
        "список генерации и объявления в types.rs разошлись числом:\n\
         объявлено: {declared:?}\nв списке: {listed:?}"
    );
}

/// Три конструкции, на которых стоят контракты E2 и E3, проверенные
/// поимённо.
///
/// Не дублирование diff-сторожа: тот ловит «перегенерировали и не
/// закоммитили», а этот — «перегенерировали, закоммитили, и форма молча
/// поехала». Обновление ts-rs — ровно тот случай; каждая из трёх строк
/// держится на поведении, которое проверялось живьём, а не на обещании
/// документации.
///
/// 1. **`#[serde(transparent)]`** ts-rs не понимает и печатает об этом
///    предупреждение при сборке тестов («failed to parse serde attribute
///    | transparent»). Игнорирование безвредно ровно потому, что
///    newtype-структуру ts-rs и без того печатает как её внутренний тип, —
///    но это совпадение, а не гарантия, и держит его этот assert.
///    Глушить предупреждение фичей `no-serde-warnings` было бы хуже: она
///    скрывает такие же сообщения обо ВСЕХ будущих атрибутах, то есть
///    заводит ровно тот молчаливый отказ, из-за которого задача и
///    появилась.
/// 2. **Вложенный второй тег.** `DownloadProgress::Downloading` —
///    newtype-вариант внутренне-тегированного объединения, обёртка над
///    другим внутренне-тегированным объединением. В TS это пересечение;
///    генератор, развернувший бы его в «всё опционально», уничтожил бы
///    смысл формы: отвергнутое состояние обязано быть структурно
///    невыразимым.
/// 3. **`#[serde(flatten)]`** у события прогресса: `phase` лежит рядом с
///    `taskId`, а не вложенным объектом.
#[test]
fn the_serde_shapes_the_contract_stands_on_survive_generation() {
    let cfg = config();

    let percent = DownloadPercent::export_to_string(&cfg).expect("объявление процента");
    assert!(
        strip_jsdoc(&percent).contains("export type DownloadPercent = number;"),
        "transparent-newtype перестал быть голым числом:\n{percent}"
    );

    let progress = DownloadProgress::export_to_string(&cfg).expect("объявление прогресса");
    assert!(
        progress.contains(r#"{ "phase": "downloading" } & DownloadingState"#),
        "вложенный второй тег перестал быть пересечением:\n{progress}"
    );

    let event = DownloadProgressEvent::export_to_string(&cfg).expect("объявление события");
    assert!(
        strip_jsdoc(&event).contains("export type DownloadProgressEvent = { taskId: string, } &"),
        "flatten перестал класть фазу рядом с taskId:\n{event}"
    );

    let running = DownloadingState::export_to_string(&cfg).expect("объявление состояния");
    assert!(
        running.contains(r#"{ "state": "running""#)
            && running.contains("percent?: DownloadPercent"),
        "внутренний тег или опциональность поля состояния поехали:\n{running}"
    );
}

/// Исходник `types.rs` до объявления этого модуля — то есть ровно
/// объявления контракта, без тестов.
///
/// Резать по первому `#[cfg(test)]` нельзя, и это выяснилось падением, а
/// не рассуждением: такая же строка стоит у импорта `ts_rs::TS` в самом
/// верху файла, и оба сторожа ниже мгновенно стали смотреть в пустоту.
/// Поймали они это сами — один по нижней границе числа проверенных полей,
/// другой по сверке длин списков; если бы не эти два assert'а, сторожа
/// остались бы зелёными, не проверяя ничего.
///
/// Маркер `mod bindings;` — единственная строка такого вида до тестов, и
/// её отсутствие роняет тест сразу, а не превращает его в пустой.
fn contract_source() -> &'static str {
    const MARKER: &str = "\nmod bindings;";
    let source = include_str!("../types.rs");
    let end = source
        .find(MARKER)
        .expect("в types.rs не нашлось объявления `mod bindings;` — маркер конца контракта");
    &source[..end]
}

/// Убирает JSDoc-блоки из объявления, чтобы сторож nullable смотрел на
/// код, а не на прозу: doc-комментарии контракта по-русски и слово `null`
/// в них встречается по делу (например, у `QualitySize` — «отдельный
/// вариант, а не `0`/`null`»).
fn strip_jsdoc(declaration: &str) -> String {
    let mut out = String::with_capacity(declaration.len());
    let mut rest = declaration;
    while let Some(start) = rest.find("/**") {
        out.push_str(&rest[..start]);
        match rest[start..].find("*/") {
            Some(end) => rest = &rest[start + end + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}
