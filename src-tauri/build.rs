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

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::{env, fs};

/// Имя, под которым onedir-архив yt-dlp кладётся в ресурсы бандла.
/// Должно совпадать с `bundle.resources` в `tauri.conf.json` и с
/// `crate::ytdlp::BUNDLED_ARCHIVE_RESOURCE`.
const RESOURCE_RELATIVE_PATH: &str = "resources/yt-dlp.zip";

fn main() {
    println!("cargo:rerun-if-changed=binaries.lock.json");

    let manifest_dir = PathBuf::from(
        env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is always set by cargo"),
    );
    let target = env::var("TARGET").expect("TARGET is always set by cargo for build scripts");

    let pin = YtDlpPin::load(&manifest_dir.join("binaries.lock.json"), &target);

    println!("cargo:rustc-env=TUBE_LEAK_YTDLP_VERSION={}", pin.version);
    println!("cargo:rustc-env=TUBE_LEAK_YTDLP_SHA256={}", pin.sha256);
    println!(
        "cargo:rustc-env=TUBE_LEAK_YTDLP_ARCHIVE_NAME={}",
        pin.archive_name
    );

    let source = manifest_dir.join("binaries").join(&pin.archive_name);
    println!("cargo:rerun-if-changed={}", source.display());

    // Цель тоже под наблюдением: без этого удалённый или подменённый
    // `resources/yt-dlp.zip` не восстанавливался бы до следующей правки
    // пина или ассета, и в бандл поехало бы то, что лежит по имени
    // ресурса сейчас.
    let destination = manifest_dir.join(RESOURCE_RELATIVE_PATH);
    println!("cargo:rerun-if-changed={}", destination.display());

    place_archive(&source, &destination);

    tauri_build::build()
}

/// Данные о вложенном архиве yt-dlp, взятые из пина для текущей тройки.
struct YtDlpPin {
    version: String,
    sha256: String,
    archive_name: String,
}

impl YtDlpPin {
    fn load(pin_path: &Path, target: &str) -> Self {
        let raw = fs::read_to_string(pin_path).unwrap_or_else(|err| {
            panic!("failed to read {}: {err}", pin_path.display());
        });
        let pin: Pin = serde_json::from_str(&raw).unwrap_or_else(|err| {
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

        Self {
            version: pin.yt_dlp.version.clone(),
            sha256: entry.sha256.clone(),
            archive_name: entry.binary_name.clone(),
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
fn place_archive(source: &Path, destination: &Path) {
    let _ = fs::remove_file(destination);

    let Ok(source_metadata) = fs::metadata(source) else {
        println!(
            "cargo:warning=yt-dlp archive {} not found — run `npm run fetch-binaries` before building the bundle",
            source.display()
        );
        return;
    };

    // Пустой архив — это не «почти готово», а гарантированно нерабочий
    // дистрибутив: приложение не сможет распаковать yt-dlp и не запустится
    // дальше экрана подготовки. Такое ловится здесь, а не у пользователя.
    assert!(
        source_metadata.len() > 0,
        "{} пуст — доставка sidecar-ассетов сломана, перезапустите `npm run fetch-binaries`",
        source.display()
    );

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
        source_metadata.len(),
        "{} получился другого размера, чем {} — копирование не состоялось",
        destination.display(),
        source.display()
    );
}
