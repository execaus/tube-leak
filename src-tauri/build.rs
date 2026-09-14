//! Сборочный скрипт: помимо обычного `tauri_build::build()` готовит
//! onedir-архив yt-dlp к упаковке в бандл (TL-12).
//!
//! # Зачем это здесь
//!
//! ffmpeg остаётся sidecar-бинарником: Tauri сама выбирает
//! `binaries/ffmpeg-<target-triple>` по тройке сборки (`externalBin`).
//! У yt-dlp после TL-12 в бандл едет не исполняемый файл, а **архив**
//! (onedir-сборка PyInstaller), который приложение распаковывает в каталог
//! данных при первом запуске — см. `crate::ytdlp`. Архив едет `resources`,
//! а `resources` в `tauri.conf.json` задаются одним списком на все
//! платформы: механизма «выбери файл по target triple», аналогичного
//! `externalBin`, для них нет.
//!
//! Поэтому выбор тройки делается здесь: скрипт знает `TARGET` и кладёт
//! нужный архив под фиксированным именем `resources/yt-dlp.zip`, на которое
//! и ссылается `tauri.conf.json`. Альтернатива — glob вида
//! `binaries/yt-dlp-*.zip` — упаковала бы в бандл архивы всех троек, что
//! успели скачаться на машине (у разработчика их бывает четыре, это +160 МиБ
//! мусора в `.dmg`), и молча собрала бы бандл под чужую платформу.
//!
//! Копия кладётся ровно в одно место — `resources/yt-dlp.zip` рядом с
//! `tauri.conf.json`. Дальше её разносит сама Tauri: `tauri_build::build()`
//! копирует всё из `bundle.resources` в `target/<profile>/` (функция
//! `copy_resources` в `tauri-build`), откуда их и берёт
//! `BaseDirectory::Resource` в dev-режиме, а бандлер при `tauri build`
//! кладёт их в `Contents/Resources/` бандла.
//!
//! # Почему копирование, а не жёсткая ссылка
//!
//! Соблазн сэкономить 54 МиБ жёсткой ссылкой на файл в `binaries/`
//! проверен и отвергнут: `tauri_build::build()` копирует ресурс поверх
//! своей цели, а копирование в файл, который является ссылкой на тот же
//! inode, обрезает источник. Наблюдалось живьём в TL-12 — архив в
//! `binaries/` становился нулевого размера, и в бандл поехал бы пустой
//! ресурс. Отдельный файл такой связи не имеет, а лишние 54 МиБ на диске
//! разработчика дешевле молча пустого архива в дистрибутиве.
//!
//! # Константы пина
//!
//! Версия и sha256 архива нужны рантайму (имя каталога установки, запись в
//! манифест, обнаружение «в бандле приехал другой yt-dlp — переустановить»).
//! Дублировать их в Rust-константах нельзя: разъедутся с
//! `binaries.lock.json` в первой же смене версии. Поэтому они читаются из
//! самого пина и пробрасываются в код через `cargo:rustc-env`.
//!
//! Тем же способом и по той же причине проброшено имя **апстримного**
//! ассета (`TUBE_LEAK_YTDLP_UPSTREAM_ASSET`, например `yt-dlp_macos.zip`):
//! проверка обновления (TL-55) ищет среди двух десятков ассетов релиза
//! ровно тот, что подходит этой сборке, и знать его имя иначе как из пина
//! ей неоткуда. Альтернатива — `cfg!(target_os)` на рантайме — была бы
//! второй правдой о соответствии «тройка → ассет», расходящейся с пином
//! молча: у macOS обе тройки берут один universal2-архив, а Linux и
//! Windows выбирают свой каждый.
//!
//! # Сверка архива с пином
//!
//! Тем же sha256 архив здесь и проверяется — см. [`place_archive`]. Без
//! сверки в бандл молча уезжает что угодно, лежащее под нужным именем в
//! `binaries/`: заглушка `scripts/ci/stub-binaries.mjs` (её кладут, чтобы
//! быстро прогнать `cargo test` в свежем клоне), недокачанный или битый
//! архив, архив от другой версии пина. Доставку при сборке бандла никто не
//! дёргает — `beforeBuildCommand` в `tauri.conf.json` это только
//! `npm run build`, — так что заметить подмену больше негде: `.dmg`
//! соберётся, а развалится уже у пользователя на распаковке. Ровно этот
//! класс дефектов («видно только на собранном бандле») дважды провалил
//! приёмку E1, поэтому проверка стоит на сборке.
//!
//! # Сверка deno
//!
//! Тем же порядком сверяется распакованный deno (TL-112). Его резолвит и
//! копирует `externalBin` мимо этого скрипта, и без сверки при настоящих
//! yt-dlp и ffmpeg и заглушке на месте deno `.dmg` собрался бы с текстовым
//! файлом на 68 байт в `Contents/MacOS/deno`. У записи deno `sha256` — сумма
//! zip-архива апстрима, из которого бинарник извлекается, поэтому с файлом
//! в `binaries/` сравнивается отдельное поле `binarySha256`: сумма самого
//! бинарника, которую апстрим публикует ассетом `deno-<тройка>.sha256sum`.
//!
//! Решение «совпал / нет / можно ли продолжать» у обоих файлов общее и
//! лежит в `build_support/pinned_file.rs`: билд-скрипт `cargo test` не
//! видит, а тот же файл, подключённый в `tests/build_pinned_file.rs`, —
//! видит. Здесь только проводка: откуда путь и сумма, паника или
//! предупреждение.
//!
//! ffmpeg не сверяется: его сборщики публикуют только суммы архивов, из
//! которых извлекается один файл (см. `scripts/fetch-binaries`), и
//! сравнивать с тем, что лежит в `binaries/`, нечем.
//!
//! У обеих сверок одна форточка: `TUBE_LEAK_ALLOW_STUB_YTDLP=1`. Она
//! нужна затем, что `cargo test` в свежем клоне без настоящего архива не
//! собирается вовсе (`tauri_build::build()` падает на отсутствующем
//! ресурсе), а качать 150 МиБ ассетов ради тестов, которые yt-dlp не
//! запускают, незачем — для этого и есть `scripts/ci/stub-binaries.mjs`,
//! которым пользуется джоб test. С этой переменной несовпадение суммы
//! становится предупреждением. Профиль `release` её игнорирует и падает
//! всё равно: релизный профиль — это `tauri build`, то есть ровно тот
//! случай, ради которого проверка и заводилась, и переменная, забытая в
//! профиле оболочки, не должна его открывать.

#[path = "build_support/pinned_file.rs"]
mod pinned_file;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::{env, fs};

use pinned_file::{PinnedFile, StubPolicy, Verdict, ALLOW_STUB_ENV};

/// Имя, под которым onedir-архив yt-dlp кладётся в ресурсы бандла.
/// Должно совпадать с `bundle.resources` в `tauri.conf.json` и с
/// `crate::ytdlp::BUNDLED_ARCHIVE_RESOURCE`.
const RESOURCE_RELATIVE_PATH: &str = "resources/yt-dlp.zip";

/// Чем оборачивается не тот архив yt-dlp у пользователя — для текста отказа.
const YTDLP_CONSEQUENCE: &str =
    "Собрался бы бандл, который развалится у пользователя на распаковке yt-dlp";

/// Чем оборачивается не тот deno у пользователя — для текста отказа.
const DENO_CONSEQUENCE: &str = "Собрался бы бандл, в котором под именем deno лежит этот файл: \
     yt-dlp не сможет решать JS-челленджи YouTube, и у пользователя пропадёт часть форматов";

fn main() {
    println!("cargo:rerun-if-changed=binaries.lock.json");
    println!("cargo:rerun-if-env-changed={ALLOW_STUB_ENV}");

    let manifest_dir = PathBuf::from(
        env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is always set by cargo"),
    );
    let target = env::var("TARGET").expect("TARGET is always set by cargo for build scripts");

    let pin_path = manifest_dir.join("binaries.lock.json");
    let raw_pin = fs::read_to_string(&pin_path).unwrap_or_else(|err| {
        panic!("failed to read {}: {err}", pin_path.display());
    });
    let pin = YtDlpPin::load(&pin_path, &raw_pin, &target);

    // `PROFILE` — `release` у всех профилей, наследующих release, то есть
    // у `tauri build`; форточка там не действует (см. doc модуля).
    let policy = StubPolicy {
        release: env::var("PROFILE").as_deref() == Ok("release"),
        stub_allowed: env::var(ALLOW_STUB_ENV).as_deref() == Ok("1"),
    };

    println!("cargo:rustc-env=TUBE_LEAK_YTDLP_VERSION={}", pin.version);
    println!("cargo:rustc-env=TUBE_LEAK_YTDLP_SHA256={}", pin.sha256);
    println!(
        "cargo:rustc-env=TUBE_LEAK_YTDLP_ARCHIVE_NAME={}",
        pin.archive_name
    );
    println!(
        "cargo:rustc-env=TUBE_LEAK_YTDLP_UPSTREAM_ASSET={}",
        pin.upstream_asset
    );

    let source = manifest_dir.join("binaries").join(&pin.archive_name);
    println!("cargo:rerun-if-changed={}", source.display());

    // Цель тоже под наблюдением: без этого удалённый или подменённый
    // `resources/yt-dlp.zip` не восстанавливался бы до следующей правки
    // пина или ассета, и в бандл поехало бы то, что лежит по имени
    // ресурса сейчас.
    let destination = manifest_dir.join(RESOURCE_RELATIVE_PATH);
    println!("cargo:rerun-if-changed={}", destination.display());

    place_archive(&source, &destination, &pin.sha256, policy);

    // deno — sidecar из `externalBin`, его кладёт в бандл сама Tauri, так
    // что здесь только сверка, без копирования (см. «Сверка deno»).
    let deno = pinned_file::pinned_binary(&raw_pin, "deno", &target)
        .unwrap_or_else(|err| panic!("{}: {err}", pin_path.display()));
    let deno_path = manifest_dir.join("binaries").join(&deno.binary_name);
    println!("cargo:rerun-if-changed={}", deno_path.display());
    check_pinned_file(
        &PinnedFile {
            tool: "deno",
            path: &deno_path,
            expected_sha256: &deno.binary_sha256,
            consequence: DENO_CONSEQUENCE,
        },
        policy,
    );

    tauri_build::build()
}

/// Проводит решение [`pinned_file::verify_pinned_file`] в сборку: отказ —
/// паника билд-скрипта, предупреждения — `cargo:warning`. Возвращает, есть
/// ли файл, с которым можно работать дальше.
fn check_pinned_file(file: &PinnedFile<'_>, policy: StubPolicy) -> bool {
    match pinned_file::verify_pinned_file(file, policy) {
        Ok(Verdict::Verified) => true,
        Ok(Verdict::StubTolerated { warning }) => {
            println!("cargo:warning={}", warning.replace('\n', " "));
            true
        }
        Ok(Verdict::Missing { warning }) => {
            println!("cargo:warning={}", warning.replace('\n', " "));
            false
        }
        Err(refusal) => panic!("{refusal}"),
    }
}

/// Начало адреса, которым обязан быть пин yt-dlp (Н-1 эпика E6).
///
/// Проверяется здесь, а не только на рантайме, потому что здесь у него
/// первое употребление: из этого адреса берётся имя апстримного ассета,
/// которое рантайм ищет в метаданных релиза
/// (`TUBE_LEAK_YTDLP_UPSTREAM_ASSET`, см. `crate::ytdlp::release`). Пин,
/// уехавший на чужой хост, сломал бы и поставку, и обновление — а заметить
/// это на сборке дешевле, чем у пользователя.
const YTDLP_RELEASE_URL_PREFIX: &str = "https://github.com/yt-dlp/yt-dlp/releases/download/";

/// Данные о вложенном архиве yt-dlp, взятые из пина для текущей тройки.
struct YtDlpPin {
    version: String,
    sha256: String,
    archive_name: String,
    /// Имя ассета **у апстрима** (`yt-dlp_macos.zip` и т. п.) — последний
    /// сегмент пинованного адреса.
    ///
    /// Не то же самое, что [`Self::archive_name`]: то — имя файла в
    /// `binaries/` с суффиксом тройки, наше собственное. Обновлению
    /// (TL-55) нужно апстримное: именно его оно ищет среди двух десятков
    /// ассетов релиза. Выводится из того же пина и той же записи, что
    /// версия и сумма, — второй правды о том, какой ассет нам подходит, в
    /// проекте не заводится, а `cfg!(target_os)` на рантайме был бы ровно
    /// ею.
    upstream_asset: String,
}

impl YtDlpPin {
    fn load(pin_path: &Path, raw: &str, target: &str) -> Self {
        let pin: Pin = serde_json::from_str(raw).unwrap_or_else(|err| {
            panic!("failed to parse {}: {err}", pin_path.display());
        });

        let entry = pin.yt_dlp.targets.get(target).unwrap_or_else(|| {
            panic!(
                "{}: no ytDlp entry for target triple {target} — add it to the pin \
                 (known triples are listed in scripts/fetch-binaries/targets.mjs)",
                pin_path.display()
            );
        });

        // Соответствие «в пине лежит архив, а не исполняемый файл» — не
        // косметика: рантайм безусловно распаковывает этот файл. Если пин
        // когда-нибудь вернут к однофайловой поставке, сборка должна
        // упасть здесь, а не приложение у пользователя.
        assert_eq!(
            entry.kind.as_deref(),
            Some("archive"),
            "{}: ytDlp entry for {target} must declare \"kind\": \"archive\" — \
             приложение распаковывает этот файл на рантайме (см. src/ytdlp)",
            pin_path.display()
        );

        // Адрес пина — единственный источник имени апстримного ассета, и
        // белый список на него стоит по той же причине, по какой стоит
        // проверка kind: обе эти строки описывают не сборку, а то, что
        // приложение потом делает в сети (Н-1) и с диском.
        assert!(
            entry.url.starts_with(YTDLP_RELEASE_URL_PREFIX),
            "{}: url ассета yt-dlp для {target} обязан начинаться с \
             {YTDLP_RELEASE_URL_PREFIX} — контур обновления ходит только к \
             официальным релизам yt-dlp/yt-dlp (Н-1 эпика E6), а имя ассета \
             для поиска в метаданных релиза берётся из этого адреса. \
             Получено: {url}",
            pin_path.display(),
            url = entry.url,
        );

        let upstream_asset = entry.url.rsplit('/').next().unwrap_or_default().to_owned();
        assert!(
            !upstream_asset.is_empty() && upstream_asset.ends_with(".zip"),
            "{}: последний сегмент url ассета yt-dlp для {target} должен быть \
             именем onedir-архива (*.zip), а получилось {upstream_asset:?}",
            pin_path.display(),
        );

        Self {
            version: pin.yt_dlp.version.clone(),
            sha256: entry.sha256.clone(),
            archive_name: entry.binary_name.clone(),
            upstream_asset,
        }
    }
}

#[derive(serde::Deserialize)]
struct Pin {
    #[serde(rename = "ytDlp")]
    yt_dlp: PinSection,
}

#[derive(serde::Deserialize)]
struct PinSection {
    version: String,
    targets: HashMap<String, PinEntry>,
}

#[derive(serde::Deserialize)]
struct PinEntry {
    url: String,
    sha256: String,
    #[serde(rename = "binaryName")]
    binary_name: String,
    kind: Option<String>,
}

/// Кладёт архив по `destination` копией через временный файл.
///
/// Временный файл и `rename` — не педантизм: прерванная копия под именем
/// ресурса означает бандл с обрезанным архивом, который развалится уже у
/// пользователя, а не на сборке. Про отказ от жёстких ссылок — см. doc
/// модуля.
///
/// Отсутствие исходника не обрывает сборку: `binaries/` не в репозитории
/// (см. `.gitignore`), его наполняет `scripts/fetch-binaries`. Ошибку в
/// этом случае осмысленнее получить от бандлера («resource not found» с
/// именем ресурса) или от рантайма, чем панику билд-скрипта на каждом
/// `cargo clippy` в свежем клоне.
///
/// А вот исходник, который есть, но не тот — обрывает: см. «Сверка архива
/// с пином» в doc модуля. Отсутствие файла ещё поймает бандлер, подмену
/// содержимого не поймает никто.
fn place_archive(source: &Path, destination: &Path, expected_sha256: &str, policy: StubPolicy) {
    let _ = fs::remove_file(destination);

    // Сверка с пином заодно закрывает и пустой, и обрезанный архив: ни
    // один из них по сумме не пройдёт, отдельной проверки на непустоту не
    // нужно.
    let archive = PinnedFile {
        tool: "yt-dlp",
        path: source,
        expected_sha256,
        consequence: YTDLP_CONSEQUENCE,
    };
    if !check_pinned_file(&archive, policy) {
        return;
    }
    let source_len = fs::metadata(source)
        .map(|metadata| metadata.len())
        .unwrap_or_else(|err| panic!("failed to stat {}: {err}", source.display()));

    let parent = destination
        .parent()
        .expect("destination always has a parent directory");
    fs::create_dir_all(parent).unwrap_or_else(|err| {
        panic!("failed to create {}: {err}", parent.display());
    });

    let temp = destination.with_extension("zip.tmp");
    let _ = fs::remove_file(&temp);
    fs::copy(source, &temp).unwrap_or_else(|err| {
        let _ = fs::remove_file(&temp);
        panic!(
            "failed to copy {} to {}: {err}",
            source.display(),
            temp.display()
        );
    });
    fs::rename(&temp, destination).unwrap_or_else(|err| {
        let _ = fs::remove_file(&temp);
        panic!("failed to place {}: {err}", destination.display());
    });

    let placed = fs::metadata(destination)
        .map(|metadata| metadata.len())
        .unwrap_or_default();
    assert_eq!(
        placed,
        source_len,
        "{} получился другого размера, чем {} — копирование не состоялось",
        destination.display(),
        source.display()
    );
}
