//! TS-зеркало контракта из [`crate::types`]: генерация, сверка и сторожа
//! при них (TL-51).
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
//! # Как это устроено сейчас
//!
//! Зеркало лежит в репозитории (`src/types/generated/`), а сторожем служит
//! сам [`typescript_mirror_matches_the_rust_contract`]: он генерирует
//! эталон во временный каталог и **сравнивает** его с тем, что на диске.
//! Перезаписывает только по явному требованию —
//! `TUBE_LEAK_UPDATE_TS_BINDINGS=1`.
//!
//! Первая версия задачи делала наоборот: тест перезаписывал файлы, а
//! расхождение ловил отдельный шаг CI через `git status`. Так вышло сразу
//! три дыры, и все три закрывает нынешняя схема:
//!
//! 1. **«Шаг не дошёл».** Красный vitest, eslint или сам `cargo test` — и
//!    до сверки прогон не добирался, про расхождение зеркала не говорил
//!    никто. Починить это через `if: always()` нельзя: при недошедшем
//!    `cargo test` каталог не перегенерирован, `git status` чист, и шаг
//!    дал бы **ложно-зелёный** — строго хуже отсутствия сигнала.
//! 2. **Молчаливая запись за пределы `src-tauri/`.** Невинный `cargo test`
//!    оставлял рабочую копию грязной, и разработчик об этом не узнавал:
//!    ровно это и случилось в мутации с новым вариантом enum — пять
//!    зелёных тестов и молча изменённый `ytdlp.ts`. Заодно ушёл
//!    `remove_dir_all` по собранному пути, оставлявший каталог
//!    полупустым, если падение случалось между удалением и записью.
//! 3. **Расхождение видел только CI.** Теперь тот, кто добавил вариант и
//!    прогнал тесты, узнаёт об этом сразу и локально.
//!
//! Почему переменная окружения, а не фича Cargo. Фичу пришлось бы
//! объявить в манифесте, где она видна и релизной сборке, и передавать её
//! `--features` вместе с `--locked`; переменная же инертна, ничего не
//! добавляет в граф и не требует пересборки. Плюс это уже конвенция
//! проекта — рядом живёт `TUBE_LEAK_ALLOW_STUB_YTDLP`.
//!
//! # Почему ts-rs, а не tauri-specta
//!
//! Обоснование целиком — в комментарии над зависимостью в `Cargo.toml`;
//! здесь коротко о том, что проверялось живьём на настоящих типах этого
//! контракта, а не бралось из документации:
//!
//! - **Внутренне-тегированные объединения не разворачиваются в «всё
//!   опционально».** `#[serde(tag = "phase")]` у
//!   [`crate::types::DownloadProgress`] даёт в TS честное размеченное
//!   объединение, а вложенный второй тег
//!   (`DownloadProgress::Downloading(DownloadingState)`) — пересечение
//!   `{ "phase": "downloading" } & DownloadingState`. Отвергнутое состояние
//!   остаётся структурно невыразимым, ради чего форма и выбиралась.
//! - **`skip_serializing_if` даёт `field?: T`, а не `field: T | null`.**
//!   Это не поведение по умолчанию: по умолчанию ts-rs пишет `T | null`, и
//!   именно поэтому у каждого типа стоит `#[ts(optional_fields)]`.
//!
//! Ни один из этих трёх инвариантов (третий — `u64` как `number`, а не
//! `bigint`) не оставлен на доверии к атрибуту: у каждого свой assert
//! ниже.
//!
//! # Чего сверка не покрывает: doc-комментарии вариантов
//!
//! В зеркало едут doc-комментарии **типов и полей структур**, но не
//! вариантов перечислений — ts-rs их отбрасывает (проверено мутацией на
//! TL-53: правка текста у варианта не будит сверку, правка текста у типа
//! будит с указанием строки). Практическое следствие для того, кто пишет
//! фронтенд: у размеченного объединения TS покажет форму
//! (`{ status: "rollbackWaiting", version: string }`) и ни слова о
//! смысле полей — смысл читается в `types.rs`, и сторожа, который следил
//! бы за его переносом, здесь нет. Автору контракта из этого следует
//! обратное: то, без чего вариант можно прочесть неверно, должно стоять
//! в doc **типа**, а не только варианта.
//!
//! # Цена и её границы
//!
//! `ts-rs` — dev-зависимость, `#[derive(TS)]` навешивается через
//! `#[cfg_attr(test, ...)]`. В релизном графе не прибавляется ни одного
//! крейта (замерено `cargo tree -e normal --target <тройка>` по всем трём
//! тройкам), в бандл не попадает ни строки генератора.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use ts_rs::{Config, TS};

use super::*;

/// Переменная, по которой тест перестаёт быть сторожем и становится
/// генератором.
///
/// Без неё на диск не пишется ничего — ни одного байта, ни одного
/// каталога.
const UPDATE_ENV: &str = "TUBE_LEAK_UPDATE_TS_BINDINGS";

/// Единственное значение [`UPDATE_ENV`], включающее перезапись.
///
/// Не «переменная задана», а именно `=1` — так же, как
/// `TUBE_LEAK_ALLOW_STUB_YTDLP` в `build.rs`. Разница не педантская:
/// пустое значение (`TUBE_LEAK_UPDATE_TS_BINDINGS=` в чьём-нибудь
/// `.zshrc` или в окружении раннера) при проверке «задана ли»
/// **выключало бы сторожа навсегда и бесшумно** — расхождение зеркала
/// молча переписывалось бы вместо того, чтобы уронить прогон. Это ровно
/// тот отказ, ради устранения которого задача и делалась, только этажом
/// выше: не контракт разошёлся незаметно, а сам сторож перестал быть
/// сторожем.
const UPDATE_ON: &str = "1";

/// Команда перегенерации в том виде, в каком её показывают в тексте
/// падения. Одно место на всех, кто её печатает.
const UPDATE_HINT: &str =
    "cd src-tauri && TUBE_LEAK_UPDATE_TS_BINDINGS=1 cargo test --locked types::bindings";

/// Каталог, в котором лежит закоммиченное зеркало.
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
/// разошлось бы с проводом на каждом размере, скорости и таймауте; сторож
/// — [`no_field_of_the_contract_is_nullable_or_bigint_in_typescript`].
fn config(out_dir: &Path) -> Config {
    Config::new().with_large_int("number").with_out_dir(out_dir)
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
            // update.ts
            YtDlpUpdatePercent,
            YtDlpUpdateFailure,
            YtDlpUpdateStatus,
            YtDlpUpdateSnapshot,
            YtDlpUpdateCommandErrorKind,
            YtDlpUpdateCommandError,
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
/// `cargo test` — не потому, что её сотрут, а потому, что тест на ней
/// покраснеет.
const HEADER: &str = "\
// Файл СГЕНЕРИРОВАН из src-tauri/src/types.rs. Руками не править: правка
// живёт в Rust, сюда она приезжает перегенерацией.
//
//   перегенерация:  cd src-tauri && TUBE_LEAK_UPDATE_TS_BINDINGS=1 cargo test --locked
//   сторож:         cargo test падает, если этот файл разошёлся с types.rs (TL-51)
//
// Комментарии ниже — те же doc-комментарии, что стоят у типов в types.rs;
// расходиться с ними этот файл не может по построению.
";

/// Столько файлов зеркала производит генератор сейчас: по одному на
/// модуль контракта — `sidecar`, `ytdlp`, `probe`, `download`, `update`.
///
/// Контур самообновления (E6) получил собственный файл, а не дописался в
/// `ytdlp.ts`: тот про подготовку первого запуска, у которой с фоновым
/// обновлением намеренно разные каналы событий и разные экраны.
const MIRROR_FILES: usize = 5;

/// Эталон: что зеркало обязано содержать прямо сейчас.
///
/// ts-rs умеет только писать в файлы (объявления нескольких типов
/// сливаются в один файл его собственной логикой слияния — дедупликация
/// импортов и алфавитный порядок). Повторять эту логику в памяти значило
/// бы завести вторую её копию, которая разойдётся с первой на ближайшем
/// обновлении крейта, поэтому эталон строится **настоящим** ts-rs во
/// временном каталоге, который тут же и умирает.
fn render_expected() -> BTreeMap<String, String> {
    let temp = tempfile::tempdir().expect("не удалось создать временный каталог");
    let cfg = config(temp.path());

    with_contract_types!(export_each, &cfg);

    let mut rendered = BTreeMap::new();
    for entry in fs::read_dir(temp.path()).expect("не удалось прочитать временный каталог")
    {
        let path = entry.expect("не удалось прочитать запись каталога").path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("нечитаемое имя сгенерированного файла")
            .to_owned();
        let body = fs::read_to_string(&path).expect("не удалось прочитать сгенерированный файл");
        // Английская пометка ts-rs (первая строка) заменяется нашей.
        let without_note = body
            .split_once('\n')
            .map(|(_, rest)| rest.to_owned())
            .expect("сгенерированный файл без пометки генератора");
        rendered.insert(name, format!("{HEADER}{without_note}"));
    }

    // Нижняя граница у эталона. Без неё пустая карта прошла бы всю сверку
    // вхолостую: обоим assert'ам о недостающих и лишних файлах сравнивать
    // было бы не с чем, а цикл сравнения не сделал бы ни одной итерации —
    // и тест позеленел бы, не проверив ничего.
    // Сообщение одной строкой намеренно: rustfmt схлопывает литерал с
    // переносами `\`, если результат влезает в ширину, и запекает отступ
    // продолжения в текст — получаются прогоны пробелов посреди фразы.
    assert_eq!(
        rendered.len(),
        MIRROR_FILES,
        "генератор произвёл {} файлов зеркала вместо {MIRROR_FILES}: сравнивать не с чем, и сверка прошла бы вхолостую. Если модули зеркала перегруппировали осознанно — поправьте константу.",
        rendered.len(),
    );

    rendered
}

/// Что лежит в закоммиченном каталоге зеркала.
fn read_committed() -> BTreeMap<String, String> {
    let dir = generated_dir();
    let mut found = BTreeMap::new();
    let Ok(entries) = fs::read_dir(&dir) else {
        return found;
    };
    for entry in entries {
        let path = entry
            .expect("не удалось прочитать запись каталога зеркала")
            .path();
        if path.extension().and_then(|e| e.to_str()) != Some("ts") {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("нечитаемое имя файла зеркала")
            .to_owned();
        let body = fs::read_to_string(&path).expect("не удалось прочитать файл зеркала");
        found.insert(name, body);
    }
    found
}

/// Сторож и генератор в одном лице.
///
/// По умолчанию — сторож: сравнивает эталон с диском и падает с командой
/// перегенерации. С `TUBE_LEAK_UPDATE_TS_BINDINGS=1` — генератор: приводит
/// каталог к эталону, включая удаление файлов, которым больше нечего
/// содержать.
///
/// Один тест, а не тест-на-тип: типы сливаются в четыре общих файла через
/// процесс-глобальную таблицу путей ts-rs, а тесты cargo идут параллельно.
#[test]
fn typescript_mirror_matches_the_rust_contract() {
    let expected = render_expected();
    let dir = generated_dir();

    if std::env::var(UPDATE_ENV).as_deref() == Ok(UPDATE_ON) {
        fs::create_dir_all(&dir).expect("не удалось создать каталог зеркала");
        for (name, body) in &expected {
            fs::write(dir.join(name), body).expect("не удалось записать файл зеркала");
        }
        for name in read_committed().keys() {
            if !expected.contains_key(name) {
                fs::remove_file(dir.join(name)).expect("не удалось убрать лишний файл зеркала");
            }
        }
        return;
    }

    let committed = read_committed();

    let missing: Vec<&String> = expected
        .keys()
        .filter(|name| !committed.contains_key(*name))
        .collect();
    assert!(
        missing.is_empty(),
        "в src/types/generated/ нет файлов зеркала: {missing:?}\n\
         Перегенерировать:\n  {UPDATE_HINT}"
    );

    let extra: Vec<&String> = committed
        .keys()
        .filter(|name| !expected.contains_key(*name))
        .collect();
    assert!(
        extra.is_empty(),
        "в src/types/generated/ лежат файлы, которых генератор больше не \
         производит: {extra:?}\nПерегенерировать:\n  {UPDATE_HINT}"
    );

    for (name, want) in &expected {
        let have = &committed[name];
        if have == want {
            continue;
        }
        panic!(
            "TS-зеркало разошлось с контрактом: src/types/generated/{name}\n\
             Правка контракта живёт в src-tauri/src/types.rs; зеркало приезжает \
             перегенерацией:\n  {UPDATE_HINT}\n\n{}",
            first_difference(have, want)
        );
    }
}

/// Опциональное поле контракта обязано **отсутствовать** в JSON, а не
/// приходить как `null` (правило E1/E2/E3, проверенное сравнением
/// сериализованных значений целиком в `super::tests`). Здесь проверяется
/// вторая половина: что и сгенерированный TS говорит то же самое.
///
/// Заодно — третья мина того же рода: `bigint`. По умолчанию ts-rs
/// отображает `u64`/`i64` в него, а через границу Tauri эти поля едут
/// обычным JSON-числом. Обе проверки здесь, а не в сравнении с
/// закоммиченным текстом, ровно потому, что перегенерация — теперь одна
/// команда: тот, кто снимет `optional_fields` или `with_large_int` и
/// перегенерирует, получил бы зелёный CI и сломанное зеркало.
///
/// Оба поведения — по умолчанию, а не в наказание за ошибку: потерять
/// атрибут у одного типа из тридцати четырёх легко, а заметить — нет.
/// `null` и `bigint` в TS-типе прекрасно проходят и type-check, и сборку,
/// и падают только у пользователя.
#[test]
fn no_field_of_the_contract_is_nullable_or_bigint_in_typescript() {
    let temp = tempfile::tempdir().expect("не удалось создать временный каталог");
    let cfg = config(temp.path());
    let declarations: Vec<(&str, String)> = with_contract_types!(declaration_of_each, &cfg);

    for (name, declaration) in declarations {
        let code = strip_jsdoc(&declaration);
        assert!(
            !code.contains("null"),
            "{name}: в сгенерированном TS появился nullable — опциональное поле \
             контракта обязано отсутствовать в JSON, а не приезжать как null. \
             Скорее всего снят `#[ts(optional_fields)]`.\n{code}"
        );
        assert!(
            !code.contains("bigint"),
            "{name}: в сгенерированном TS появился bigint — через границу Tauri \
             числа едут обычным JSON, и на стороне webview это `number`. \
             Скорее всего снят `Config::with_large_int(\"number\")`.\n{code}"
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
    /// Столько `Option`-полей в контракте сейчас. Число намеренно точное:
    /// нижняя граница «хоть сколько-нибудь» пропустила бы регрессию
    /// разбора, потерявшую треть полей, — а именно ради неё нижняя
    /// граница и заводилась. Меняется вместе с контрактом, одной строкой,
    /// и это осознанная просьба к автору нового поля посмотреть на
    /// сторожа.
    const OPTIONAL_FIELDS: usize = 32;

    let mut checked = 0usize;
    let contract: Vec<&str> = contract_source().lines().collect();

    for (i, line) in contract.iter().enumerate() {
        let Some(field) = option_field(line) else {
            continue;
        };

        assert!(
            attributes_above(&contract[..i])
                .iter()
                .any(|attr| attr.contains(r#"skip_serializing_if = "Option::is_none""#)),
            "поле контракта `{field}` объявлено как Option, но не помечено \
             `skip_serializing_if = \"Option::is_none\"`. Из-за \
             `#[ts(optional_fields)]` зеркало объявит его как `field?: T`, \
             то есть пообещает отсутствие ключа — а ядро пришлёт `null`."
        );
        checked += 1;
    }

    assert_eq!(
        checked, OPTIONAL_FIELDS,
        "сторож нашёл в контракте {checked} Option-полей вместо {OPTIONAL_FIELDS}. \
         Если поле добавили или убрали осознанно — поправьте константу; если нет — \
         сломался разбор `types.rs`, и сторож проверяет не то, что думает."
    );
}

/// Объявление `Option`-поля, если строка им является.
///
/// Отдельная функция с собственным тестом, а не условие внутри цикла,
/// потому что у неё есть **обе** стороны отказа. Пропустить настоящее
/// поле — дыра, ради которой сторож писался. Принять за поле упоминание
/// в прозе — ложное срабатывание: doc-комментарии этого контракта
/// объясняют, почему то или иное поле сделано **не** опциональным, и
/// пишут `: Option<…>` по делу. Первая же такая строка (секция E6)
/// уронила сторожа с обвинением автора в том, чего он не делал, — тот же
/// класс дефекта, что уже чинился в `attributes_above`, и та же цена:
/// сторож, которому не верят, бесполезен.
fn option_field(line: &str) -> Option<&str> {
    let line = line.trim();
    // Проза не объявляет полей: ни doc-комментарий, ни обычный.
    if line.starts_with("//") {
        return None;
    }
    // `-> Option<T>` у метода — возвращаемое значение, а не поле на
    // проводе; `: Option<` в сигнатуре — аргумент.
    if !line.contains(": Option<") || line.contains("fn ") {
        return None;
    }
    Some(line)
}

/// Разбор строки контракта различает поле и рассказ о поле.
#[test]
fn a_mention_of_option_in_prose_is_not_a_field_declaration() {
    assert_eq!(
        option_field("    pub reason: Option<LaunchFailedReason>,"),
        Some("pub reason: Option<LaunchFailedReason>,")
    );
    assert_eq!(
        option_field("/// Почему объединение, а не `kind` + `version: Option<String>`."),
        None,
        "doc-комментарий, объясняющий отказ от опционального поля, полем не является"
    );
    assert_eq!(
        option_field("    pub fn version(&self) -> Option<&str> {"),
        None,
        "возвращаемое значение метода на провод не уходит"
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
        .filter_map(|line| declaration_name(line))
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
/// Не дублирование сверки с диском: та ловит «поменяли Rust и не
/// перегенерировали», а этот — «перегенерировали, закоммитили, и форма
/// молча поехала». Обновление ts-rs — ровно тот случай; каждая из строк
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
///    появилась. Цена размена названа прямо: `-D warnings` это
///    предупреждение не ловит, и оно останется шумом на каждой тестовой
///    сборке.
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
    let temp = tempfile::tempdir().expect("не удалось создать временный каталог");
    let cfg = config(temp.path());

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

/// Формы, на которых стоит контракт E6, проверенные поимённо.
///
/// Тот же довод, что у соседнего теста про E2/E3: сверка с диском ловит
/// «поменяли Rust и не перегенерировали», а этот — «перегенерировали,
/// закоммитили, и форма молча поехала». Каждое утверждение здесь — про
/// свойство, ради которого форма и выбиралась.
#[test]
fn the_serde_shapes_the_update_contract_stands_on_survive_generation() {
    let temp = tempfile::tempdir().expect("не удалось создать временный каталог");
    let cfg = config(temp.path());

    let percent = YtDlpUpdatePercent::export_to_string(&cfg).expect("объявление процента");
    assert!(
        strip_jsdoc(&percent).contains("export type YtDlpUpdatePercent = number;"),
        "transparent-newtype перестал быть голым числом:\n{percent}"
    );

    let status = YtDlpUpdateStatus::export_to_string(&cfg).expect("объявление состояния");
    let status = strip_jsdoc(&status);
    assert!(
        status.contains(
            r#"{ "status": "downloading", version: string, percent: YtDlpUpdatePercent, }"#
        ),
        "процент перестал быть обязательным полем ровно одного варианта — \
         вся форма выбиралась ради этого:\n{status}"
    );
    assert!(
        !status.contains(r#""status": "preparing", version: string, percent"#)
            && !status.contains(r#""status": "checking", "#),
        "у состояния без процента появились чужие поля:\n{status}"
    );

    let failure = YtDlpUpdateFailure::export_to_string(&cfg).expect("объявление отказа");
    let failure = strip_jsdoc(&failure);
    for (kind, version_is_named) in [
        ("networkUnavailable", false),
        ("sourceUnavailable", false),
        ("archiveCorrupted", true),
        ("notEnoughSpace", true),
        ("smokeCheckFailed", true),
    ] {
        let expected = if version_is_named {
            format!(r#"{{ "kind": "{kind}", version: string, message: string, }}"#)
        } else {
            format!(r#"{{ "kind": "{kind}", message: string, }}"#)
        };
        assert!(
            failure.contains(&expected),
            "класс отказа Ф-9 `{kind}` потерял форму `{expected}`:\n{failure}"
        );
    }

    let snapshot = YtDlpUpdateSnapshot::export_to_string(&cfg).expect("объявление снимка");
    let snapshot = strip_jsdoc(&snapshot);
    assert!(
        snapshot.contains("rollbackTarget?: string")
            && snapshot.contains("busy: boolean")
            && snapshot.contains(r#"} & ({ "status": "neverChecked" }"#),
        "flatten перестал класть статус рядом с целью отката, либо \
         обязательность полей снимка поехала:\n{snapshot}"
    );
}

/// Исходник `types.rs` до объявления этого модуля — то есть ровно
/// объявления контракта, без тестов.
///
/// # Две ловушки, обе найденные падением, а не рассуждением
///
/// **Резать по первому `#[cfg(test)]` нельзя:** такая же строка стоит у
/// импорта `ts_rs::TS` в самом верху файла, и оба сторожа выше мгновенно
/// стали смотреть в пустоту. Поймали они это сами — один по числу
/// проверенных полей, другой по сверке длин списков.
///
/// **Хвост после маркера обязан быть пуст.** Тип, дописанный ПОСЛЕ
/// `mod bindings;`, был бы невидим всем сторожам сразу: список полноты
/// его не хватится, сторож опциональных полей его не осмотрит, зеркала у
/// него не будет — и всё это молча. А «дописать новый тип в конец файла»
/// — самое вероятное движение руки, тем более в E6, где типов границы
/// прибавится больше всего. Отсюда проверка хвоста прямо здесь, у самого
/// разреза: она защищает всех, кто этой функцией пользуется, а не одного
/// вызывающего.
fn contract_source() -> &'static str {
    const MARKER: &str = "\nmod bindings;";
    let source = include_str!("../types.rs");
    let end = source
        .find(MARKER)
        .expect("в types.rs не нашлось объявления `mod bindings;` — маркер конца контракта");

    let (contract, tail) = source.split_at(end);
    let hidden: Vec<&str> = tail.lines().filter_map(declaration_name).collect();
    assert!(
        hidden.is_empty(),
        "в types.rs объявлены типы ПОСЛЕ `mod bindings;`: {hidden:?}\n\
         Для всех сторожей зеркала этот хвост невидим: полноты списка им не \
         хватятся, опциональные поля не осмотрят, TS-зеркала не сгенерируют — \
         и всё молча. Объявления контракта должны стоять выше маркера."
    );

    contract
}

/// Имя типа, если строка — объявление на верхнем уровне файла.
///
/// Отступ значим: `pub struct` внутри `mod tests` — не тип границы, а
/// фикстура. Форма видимости берётся шире, чем `pub`: `pub(crate)`-тип в
/// `types.rs` тоже пересекал бы границу через любую команду, которая его
/// возвращает.
fn declaration_name(line: &str) -> Option<&str> {
    // Видимость снимается любая, а не перечислением форм. Перечисление
    // уже подводило: `pub` и `pub(crate)` были, а `pub(super)` и
    // `pub(in crate::…)` проходили мимо — при том, что `pub(super)` у
    // модуля уровня корня виден соседнему `crate::commands`, то есть
    // такой тип спокойно становится возвращаемым значением команды и
    // пересекает границу. Следующая форма, которую забыли бы перечислить,
    // была бы такой же тихой.
    let rest = match line.strip_prefix("pub") {
        Some(after_pub) => {
            let after_group = match after_pub.strip_prefix('(') {
                // Скобка не закрыта на этой строке — не объявление.
                Some(group) => &group[group.find(')')? + 1..],
                None => after_pub,
            };
            after_group.trim_start()
        }
        None => line,
    };
    let rest = rest
        .strip_prefix("struct ")
        .or_else(|| rest.strip_prefix("enum "))
        .or_else(|| rest.strip_prefix("union "))?;
    // Отступ значим и здесь: строка с отступом не начинается ни с `pub`,
    // ни с одного из трёх ключевых слов, поэтому до сюда не доходит —
    // `pub struct` внутри `mod tests` остаётся фикстурой, а не контрактом.
    let name = rest
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .next()
        .unwrap_or(rest);
    (!name.is_empty()).then_some(name)
}

/// Все атрибуты, стоящие непосредственно над строкой.
///
/// Смотреть только ближайшую непустую строку было ошибкой: корректное
/// поле с двумя атрибутами (`skip_serializing_if` плюс `rename`) роняло
/// сторожа, обвиняя автора в том, чего он не делал. Падение было в
/// безопасную сторону, но ложное срабатывание — тоже отказ: сторож,
/// которому не верят, бесполезен.
fn attributes_above<'a>(before: &[&'a str]) -> Vec<&'a str> {
    let mut attributes = Vec::new();
    for line in before.iter().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        if trimmed.starts_with("#[") {
            attributes.push(trimmed);
            continue;
        }
        break;
    }
    attributes
}

/// Первое расхождение двух версий файла, в виде, пригодном для чтения в
/// логе CI.
///
/// Целиком diff здесь не нужен и вреден: файлы зеркала — сотни строк с
/// длинными объявлениями, и в них тонет ровно та строка, ради которой
/// прогон покраснел.
fn first_difference(have: &str, want: &str) -> String {
    let mut have_lines = have.lines();
    let mut want_lines = want.lines();
    let mut number = 0usize;
    loop {
        number += 1;
        match (have_lines.next(), want_lines.next()) {
            (None, None) => return "файлы совпали построчно, но различаются хвостом".to_owned(),
            (h, w) if h == w => continue,
            (h, w) => {
                return format!(
                    "Первое расхождение — строка {number}:\n  \
                     на диске: {}\n  из Rust: {}",
                    h.unwrap_or("<конец файла>"),
                    w.unwrap_or("<конец файла>"),
                )
            }
        }
    }
}

/// Убирает JSDoc-блоки из объявления, чтобы сторожа смотрели на код, а не
/// на прозу: doc-комментарии контракта по-русски, и слова `null` и
/// `bigint` в них встречаются по делу (например, у `QualitySize` —
/// «отдельный вариант, а не `0`/`null`»).
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
