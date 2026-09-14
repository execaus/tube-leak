//! Сверка файла в `binaries/` с суммой из пина — общая часть `build.rs`
//! и его теста (TL-112).
//!
//! # Почему отдельный файл
//!
//! Код билд-скрипта `cargo test` не видит: `build.rs` компилируется и
//! запускается, но его `#[cfg(test)]` не собирается никогда. Поэтому
//! логика, которую нужно доказывать тестом, лежит здесь и подключается
//! дважды через `#[path]`: в `build.rs` и в `tests/build_pinned_file.rs`.
//! Модуль не читает окружение и не паникует — решает, а что делать с
//! решением (паника, `cargo:warning`), решает `build.rs`. Иначе тест
//! проверял бы не тот код, что стоит на сборке.
//!
//! # Что сверяется
//!
//! - yt-dlp — архив `binaries/yt-dlp-<тройка>.zip` с `sha256` записи пина
//!   (TL-25): архив кладётся в бандл как есть, и сумма скачанного — это и
//!   есть сумма лежащего.
//! - deno — распакованный `binaries/deno-<тройка>[.exe]` с отдельным полем
//!   `binarySha256` (TL-112). `sha256` записи deno — сумма zip-архива, из
//!   которого бинарник извлекается, и с итоговым файлом её сравнить нечем;
//!   сумму самого бинарника апстрим публикует отдельным ассетом
//!   `deno-<тройка>.sha256sum`.
//! - ffmpeg не сверяется: его сборщики публикуют только суммы архивов.
//!
//! Политика одна на всех: несовпадение — отказ; вне профиля `release`
//! отказ превращается в предупреждение, если задано
//! [`ALLOW_STUB_ENV`]`=1`. Имя переменной историческое (TL-25, тогда
//! сверялся один yt-dlp) и не переименовано намеренно: оно записано в
//! джобе `test` CI и в инструкциях проекта, а вторая переменная ради
//! второго файла означала бы две форточки вместо одной.

use std::fs;
use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

/// Переменная, разрешающая собрать НЕ релизный профиль с файлами, не
/// совпадающими с пином (заглушки `scripts/ci/stub-binaries.mjs`).
pub const ALLOW_STUB_ENV: &str = "TUBE_LEAK_ALLOW_STUB_YTDLP";

/// В каком режиме идёт сборка — ровно то, что политике нужно знать об
/// окружении. Собирается в `build.rs` из `PROFILE` и [`ALLOW_STUB_ENV`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StubPolicy {
    /// Профиль наследует `release` — это `tauri build`.
    pub release: bool,
    /// Задано [`ALLOW_STUB_ENV`]`=1`.
    pub stub_allowed: bool,
}

/// Файл, который сверяется с пином.
pub struct PinnedFile<'a> {
    /// Имя инструмента для сообщений: `yt-dlp`, `deno`.
    pub tool: &'a str,
    pub path: &'a Path,
    pub expected_sha256: &'a str,
    /// Чем обернётся подмена у пользователя — одна фраза без точки.
    pub consequence: &'a str,
}

/// Исход сверки, при котором сборку можно продолжать.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Файл совпал с пином.
    Verified,
    /// Файла нет. Сборку это не обрывает: `binaries/` не в репозитории, а
    /// отсутствие ресурса или `externalBin` ловит сама `tauri_build`.
    Missing { warning: String },
    /// Файл не совпал, но профиль не `release` и форточка открыта.
    StubTolerated { warning: String },
}

/// Сверяет файл с пином. `Err` — текст отказа сборки: называет инструмент,
/// путь, ожидаемую и фактическую суммы и что делать.
pub fn verify_pinned_file(file: &PinnedFile<'_>, policy: StubPolicy) -> Result<Verdict, String> {
    let PinnedFile {
        tool,
        path,
        expected_sha256,
        consequence,
    } = *file;

    let Ok(metadata) = fs::metadata(path) else {
        return Ok(Verdict::Missing {
            warning: format!(
                "{tool}: {} not found — run `npm run fetch-binaries` before building the bundle",
                path.display()
            ),
        });
    };

    // Сверка заодно закрывает пустой, обрезанный и чужой файл: ни один из
    // них по сумме не пройдёт. Каталог под этим именем падает здесь же, на
    // чтении.
    let actual_sha256 = sha256_file(path).map_err(|err| {
        format!(
            "{tool}: failed to read {} for sha256: {err}",
            path.display()
        )
    })?;
    if actual_sha256.eq_ignore_ascii_case(expected_sha256) {
        return Ok(Verdict::Verified);
    }

    if policy.stub_allowed && !policy.release {
        return Ok(Verdict::StubTolerated {
            warning: format!(
                "{tool}: {} не соответствует пину binaries.lock.json ({} байт, sha256 {actual_sha256}); \
                 продолжаю, потому что задано {ALLOW_STUB_ENV}=1. Собранный так бандл нерабочий.",
                path.display(),
                metadata.len(),
            ),
        });
    }

    let ignored_window = if policy.stub_allowed {
        format!("\n{ALLOW_STUB_ENV}=1 задано, но профиль release его не учитывает.")
    } else {
        String::new()
    };

    Err(format!(
        "{tool}: {path} не соответствует пину binaries.lock.json:\n  \
         ожидалось sha256 {expected_sha256}\n  \
         получено   {actual_sha256} ({size} байт)\n\
         Под этим именем лежит не тот файл: заглушка \
         `node scripts/ci/stub-binaries.mjs`, недокачанный файл или файл от \
         другой версии пина. {consequence}, поэтому сборка остановлена.\n\
         Что делать: удалить файл и запустить доставку — \
         `npm run fetch-binaries`. Если это заглушка и нужен только \
         `cargo test`/`cargo clippy` — {ALLOW_STUB_ENV}=1 (в профиле \
         release не действует).{ignored_window}",
        path = path.display(),
        size = metadata.len(),
    ))
}

/// Запись пина, чей итоговый файл в `binaries/` сверяется по
/// `binarySha256`.
#[derive(Debug, PartialEq, Eq)]
pub struct PinnedBinary {
    /// Имя файла в `binaries/` — голое имя, без каталогов.
    pub binary_name: String,
    /// Сумма распакованного бинарника, нижний регистр.
    pub binary_sha256: String,
}

/// Достаёт из пина `binaryName` и `binarySha256` записи
/// `<section>.targets.<target>`. Оба поля обязательны: запись без суммы
/// означала бы release-сборку, которая не отличает бинарник от заглушки.
pub fn pinned_binary(pin_json: &str, section: &str, target: &str) -> Result<PinnedBinary, String> {
    let pin: serde_json::Value =
        serde_json::from_str(pin_json).map_err(|err| format!("pin is not valid JSON: {err}"))?;
    let label = format!("{section}.targets.{target}");
    let entry = pin
        .get(section)
        .and_then(|value| value.get("targets"))
        .and_then(|value| value.get(target))
        .ok_or_else(|| format!("no \"{label}\" entry in the pin"))?;

    let binary_name = entry
        .get("binaryName")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    // Имя уходит в `Path::join`: белый список «голое имя файла», а не
    // перечень опасных подстрок.
    let plain_name = !binary_name.is_empty()
        && binary_name != "."
        && binary_name != ".."
        && binary_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    if !plain_name {
        return Err(format!(
            "\"{label}.binaryName\" must be a plain file name (ASCII letters, digits, '-', '_', '.'), got {binary_name:?}"
        ));
    }

    let binary_sha256 = entry
        .get("binarySha256")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let well_formed = binary_sha256.len() == 64
        && binary_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if !well_formed {
        return Err(format!(
            "\"{label}.binarySha256\" must be the 64-char lowercase hex sha256 of the unpacked binary \
             (upstream publishes it as a separate .sha256sum asset) — without it a release build \
             cannot tell the binary from a stub; got {binary_sha256:?}"
        ));
    }

    Ok(PinnedBinary {
        binary_name: binary_name.to_owned(),
        binary_sha256: binary_sha256.to_owned(),
    })
}

/// Считает sha256 файла потоково.
///
/// Потоково, а не `fs::read`: архив yt-dlp — 54 МиБ, deno — до 93 МиБ, и
/// держать их целиком в памяти билд-скрипта незачем.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];

    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}
