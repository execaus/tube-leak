//! Сторож сверки sidecar-файлов с пином на сборке (TL-112).
//!
//! `build.rs` отказывает сборке, если под именем deno, ffmpeg или архива
//! yt-dlp в `binaries/` лежит не то, что закреплено в `binaries.lock.json`, и в
//! профиле `release` не принимает для этого никаких форточек. Сам
//! билд-скрипт `cargo test` не видит, поэтому решение вынесено в
//! `build_support/pinned_file.rs` и подключено сюда тем же `#[path]`, что и
//! в `build.rs`: тест проверяет ровно тот код, что стоит на сборке.
//!
//! Решение о политике и записях пина вынесено туда же
//! (`plan_build_checks`, TL-114): `build.rs` только читает `PROFILE` и
//! форточку и передаёт значения как есть, а политику несёт каждая сверка.
//! Проводку — что `build.rs` зовёт обе сверки из плана и политику не
//! трогает — тест не исполняет, а сверяет по исходнику
//! (`build_rs_wires_every_check_from_the_plan_and_never_touches_the_policy`).
//! Проводку целиком по-прежнему доказывает только живая сборка в
//! release-профиле с заглушкой на месте deno (отчёт TL-112).

#[path = "../build_support/pinned_file.rs"]
mod pinned_file;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use pinned_file::{
    pinned_binary, plan_build_checks, sha256_file, verify_pinned_file, BuildChecks, PinnedBinary,
    PinnedFile, StubPolicy, Verdict, ALLOW_STUB_ENV,
};

/// Содержимое заглушки `scripts/ci/stub-binaries.mjs`. Сумма посчитана
/// `shasum -a 256`, а не кодом под тестом.
const STUB_CONTENT: &str = "tube-leak CI stub, not a real binary (scripts/ci/stub-binaries.mjs)\n";
const STUB_SHA256: &str = "16f9c25e72a528d4667009d2d87d057d11aa2ae0d99e4b70748a73830cb1692b";

/// Сумма настоящего `deno-aarch64-apple-darwin` 2.9.6: снята `shasum -a 256`
/// с доставленного бинарника и совпала с апстримным
/// `deno-aarch64-apple-darwin.sha256sum` релиза v2.9.6.
const DENO_AARCH64_DARWIN_SHA256: &str =
    "b3ac3bd206e48c26026cadd80c1367e96c149f9c66130952382a642b09fa8a71";

/// Сумма настоящего `ffmpeg-aarch64-apple-darwin` 9.0.1 (TL-134): снята
/// `shasum -a 256` с файла, который положила доставка, а не кодом под
/// тестом. Подтвердить её апстримом нельзя — сборщики ffmpeg сумм
/// распакованного не публикуют (см. `ffmpeg._note` в пине); это наш замер.
const FFMPEG_AARCH64_DARWIN_SHA256: &str =
    "393e4c395020a1cb7cbd77fbe00599ce69d1c6466fee0dbd59d13f86a81a1611";

/// Опубликованные векторы SHA-256 (FIPS 180-2): «abc» и миллион «a». Второй
/// длиннее буфера потокового чтения, то есть проходит через его границы.
const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
const MILLION_A_SHA256: &str = "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0";

/// Тройки из `scripts/fetch-binaries/targets.mjs`. Копия списка: пин обязан
/// покрывать их все, и тест называет, какую именно потерял.
const KNOWN_TARGETS: [&str; 4] = [
    "x86_64-pc-windows-msvc",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-unknown-linux-gnu",
];

const RELEASE: StubPolicy = StubPolicy {
    release: true,
    stub_allowed: false,
};
const RELEASE_WITH_WINDOW: StubPolicy = StubPolicy {
    release: true,
    stub_allowed: true,
};
const DEV: StubPolicy = StubPolicy {
    release: false,
    stub_allowed: false,
};
const DEV_WITH_WINDOW: StubPolicy = StubPolicy {
    release: false,
    stub_allowed: true,
};

fn repo_pin() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("binaries.lock.json");
    fs::read_to_string(&path).unwrap_or_else(|err| panic!("reading {}: {err}", path.display()))
}

fn deno_file<'a>(path: &'a Path, expected_sha256: &'a str) -> PinnedFile<'a> {
    PinnedFile {
        tool: "deno",
        path,
        expected_sha256,
        consequence: "Собрался бы бандл с заглушкой вместо deno",
    }
}

fn stub_in(dir: &Path) -> PathBuf {
    let path = dir.join("deno-aarch64-apple-darwin");
    fs::write(&path, STUB_CONTENT).expect("write stub");
    path
}

fn assert_contains(haystack: &str, needles: &[&str]) {
    for needle in needles {
        assert!(
            haystack.contains(needle),
            "expected {needle:?} in:\n{haystack}"
        );
    }
}

#[test]
fn release_refuses_the_deno_stub_naming_deno_the_path_and_both_sums() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = stub_in(dir.path());
    // Ожидаемая сумма — из настоящего пина, как на сборке.
    let pinned = pinned_binary(&repo_pin(), "deno", "aarch64-apple-darwin").expect("deno pin");

    let refusal = verify_pinned_file(&deno_file(&path, &pinned.binary_sha256), RELEASE)
        .expect_err("release build must refuse the stub");

    assert!(refusal.starts_with("deno: "), "{refusal}");
    assert_contains(
        &refusal,
        &[
            &path.display().to_string(),
            &format!("ожидалось sha256 {DENO_AARCH64_DARWIN_SHA256}"),
            &format!("получено   {STUB_SHA256} (68 байт)"),
            "Собрался бы бандл с заглушкой вместо deno",
            "npm run fetch-binaries",
        ],
    );
}

#[test]
fn release_ignores_the_stub_window_and_says_so() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = stub_in(dir.path());

    let refusal = verify_pinned_file(
        &deno_file(&path, DENO_AARCH64_DARWIN_SHA256),
        RELEASE_WITH_WINDOW,
    )
    .expect_err("the stub window must not open a release build");

    assert_contains(
        &refusal,
        &[
            "deno: ",
            &format!("{ALLOW_STUB_ENV}=1 задано, но профиль release его не учитывает"),
        ],
    );
}

#[test]
fn outside_release_the_stub_is_refused_unless_the_window_is_open() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = stub_in(dir.path());
    let file = deno_file(&path, DENO_AARCH64_DARWIN_SHA256);

    let refusal = verify_pinned_file(&file, DEV).expect_err("closed window refuses the stub");
    assert_contains(&refusal, &["deno: ", STUB_SHA256]);
    assert!(
        !refusal.contains("профиль release его не учитывает"),
        "{refusal}"
    );

    match verify_pinned_file(&file, DEV_WITH_WINDOW) {
        Ok(Verdict::StubTolerated { warning }) => assert_contains(
            &warning,
            &[
                "deno: ",
                &path.display().to_string(),
                STUB_SHA256,
                ALLOW_STUB_ENV,
            ],
        ),
        other => panic!("expected a tolerated stub, got {other:?}"),
    }
}

#[test]
fn a_file_matching_the_pin_is_verified_in_every_mode() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("deno-aarch64-apple-darwin");
    fs::write(&path, b"abc").expect("write");

    for policy in [RELEASE, RELEASE_WITH_WINDOW, DEV, DEV_WITH_WINDOW] {
        assert_eq!(
            verify_pinned_file(&deno_file(&path, ABC_SHA256), policy),
            Ok(Verdict::Verified),
            "{policy:?}"
        );
    }
    // Апстрим deno для Windows публикует сумму в верхнем регистре.
    let upper = ABC_SHA256.to_ascii_uppercase();
    assert_eq!(
        verify_pinned_file(&deno_file(&path, &upper), RELEASE),
        Ok(Verdict::Verified)
    );
}

#[test]
fn hashing_streams_across_buffer_boundaries() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("million-a");
    fs::write(&path, vec![b'a'; 1_000_000]).expect("write");

    assert_eq!(sha256_file(&path).expect("hash"), MILLION_A_SHA256);
}

#[test]
fn a_missing_file_is_a_warning_not_a_refusal() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("deno-aarch64-apple-darwin");

    match verify_pinned_file(&deno_file(&path, DENO_AARCH64_DARWIN_SHA256), RELEASE) {
        Ok(Verdict::Missing { warning }) => {
            assert_contains(&warning, &["deno: ", &path.display().to_string()]);
        }
        other => panic!("expected a missing-file warning, got {other:?}"),
    }
}

#[test]
fn a_directory_under_the_binary_name_is_refused_naming_the_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("deno-aarch64-apple-darwin");
    fs::create_dir(&path).expect("mkdir");

    let refusal = verify_pinned_file(
        &deno_file(&path, DENO_AARCH64_DARWIN_SHA256),
        DEV_WITH_WINDOW,
    )
    .expect_err("a directory is not a binary, window or not");
    assert_contains(&refusal, &["deno: ", &path.display().to_string()]);
}

#[test]
fn the_repository_pin_carries_the_unpacked_deno_sum_for_every_target() {
    let raw = repo_pin();
    let json: serde_json::Value = serde_json::from_str(&raw).expect("pin is JSON");
    let mut sums = HashSet::new();

    for target in KNOWN_TARGETS {
        let PinnedBinary {
            binary_name,
            binary_sha256,
        } = pinned_binary(&raw, "deno", target).unwrap_or_else(|err| panic!("{target}: {err}"));

        let exe = if target.contains("-windows-") {
            ".exe"
        } else {
            ""
        };
        assert_eq!(binary_name, format!("deno-{target}{exe}"));

        // Сумма бинарника — не сумма архива: перепутанное поле пропустило бы
        // любой файл, кроме настоящего.
        let archive_sha256 = json["deno"]["targets"][target]["sha256"]
            .as_str()
            .expect("archive sha256");
        assert_ne!(binary_sha256, archive_sha256, "{target}");
        assert!(
            sums.insert(binary_sha256),
            "{target}: duplicate binarySha256"
        );
    }

    assert_eq!(
        pinned_binary(&raw, "deno", "aarch64-apple-darwin")
            .expect("aarch64 deno")
            .binary_sha256,
        DENO_AARCH64_DARWIN_SHA256
    );
}

/// TL-134 (#141): у ffmpeg сумма распакованного тоже есть, и она своя на
/// каждую тройку. До этой задачи поля не было вовсе — подмену файла в
/// `binaries/` между доставкой и сборкой не ловил никто, тогда как у
/// соседнего deno ловил `build.rs`.
#[test]
fn the_repository_pin_carries_the_unpacked_ffmpeg_sum_for_every_target() {
    let raw = repo_pin();
    let json: serde_json::Value = serde_json::from_str(&raw).expect("pin is JSON");
    let mut sums = HashSet::new();

    for target in KNOWN_TARGETS {
        let PinnedBinary {
            binary_name,
            binary_sha256,
        } = pinned_binary(&raw, "ffmpeg", target).unwrap_or_else(|err| panic!("{target}: {err}"));

        let exe = if target.contains("-windows-") {
            ".exe"
        } else {
            ""
        };
        assert_eq!(binary_name, format!("ffmpeg-{target}{exe}"));

        // Сумма распакованного — не сумма архива: перепутанное поле
        // пропустило бы любой файл, кроме настоящего.
        let archive_sha256 = json["ffmpeg"]["targets"][target]["sha256"]
            .as_str()
            .expect("archive sha256");
        assert_ne!(binary_sha256, archive_sha256, "{target}");
        // И не сумма соседней тройки: четыре разные сборки.
        assert!(
            sums.insert(binary_sha256),
            "{target}: duplicate binarySha256"
        );
    }

    assert_eq!(
        pinned_binary(&raw, "ffmpeg", "aarch64-apple-darwin")
            .expect("aarch64 ffmpeg")
            .binary_sha256,
        FFMPEG_AARCH64_DARWIN_SHA256
    );
}

#[test]
fn pinned_binary_refuses_an_entry_it_could_not_check() {
    let entry = |binary_name: &str, binary_sha256: Option<&str>| {
        let mut target = serde_json::json!({ "binaryName": binary_name });
        if let Some(sum) = binary_sha256 {
            target["binarySha256"] = serde_json::json!(sum);
        }
        serde_json::json!({ "deno": { "targets": { "aarch64-apple-darwin": target } } }).to_string()
    };
    let upper = DENO_AARCH64_DARWIN_SHA256.to_ascii_uppercase();

    let cases = [
        (entry("deno-aarch64-apple-darwin", None), "binarySha256"),
        (
            entry("deno-aarch64-apple-darwin", Some(&upper)),
            "binarySha256",
        ),
        (
            entry("deno-aarch64-apple-darwin", Some("b3ac3bd2")),
            "binarySha256",
        ),
        (
            entry("../deno", Some(DENO_AARCH64_DARWIN_SHA256)),
            "binaryName",
        ),
        (
            entry("sub/deno", Some(DENO_AARCH64_DARWIN_SHA256)),
            "binaryName",
        ),
        (entry("..", Some(DENO_AARCH64_DARWIN_SHA256)), "binaryName"),
        (entry("", Some(DENO_AARCH64_DARWIN_SHA256)), "binaryName"),
    ];
    for (pin, field) in &cases {
        let err = pinned_binary(pin, "deno", "aarch64-apple-darwin")
            .expect_err("malformed entry must be refused");
        assert_contains(
            &err,
            &[&format!("deno.targets.aarch64-apple-darwin.{field}")],
        );
    }

    let ok = entry(
        "deno-aarch64-apple-darwin",
        Some(DENO_AARCH64_DARWIN_SHA256),
    );
    assert!(pinned_binary(&ok, "deno", "aarch64-apple-darwin").is_ok());
    let err = pinned_binary(&ok, "deno", "x86_64-apple-darwin").expect_err("no entry");
    assert_contains(&err, &["deno.targets.x86_64-apple-darwin"]);
}

// ─────────────── Решение сборки и его проводка (TL-114) ───────────────

/// Тройка, на которой тесты плана читают настоящий пин.
const PLAN_TARGET: &str = "aarch64-apple-darwin";

/// Замечание 1 ревью TL-112: при `PROFILE=release` форточка `=1` не
/// открывает ни одну из сверок, и все берут записи своей тройки.
#[test]
fn the_release_profile_keeps_every_check_closed_even_with_the_stub_window() {
    let raw = repo_pin();
    let json: serde_json::Value = serde_json::from_str(&raw).expect("pin is JSON");
    let BuildChecks {
        yt_dlp_archive,
        deno,
        ffmpeg,
    } = plan_build_checks(&raw, PLAN_TARGET, Some("release"), Some("1")).expect("plan");

    assert_eq!(yt_dlp_archive.policy, RELEASE_WITH_WINDOW, "yt-dlp");
    assert_eq!(deno.policy, RELEASE_WITH_WINDOW, "deno");
    assert_eq!(ffmpeg.policy, RELEASE_WITH_WINDOW, "ffmpeg");

    assert_eq!(yt_dlp_archive.tool, "yt-dlp");
    assert_eq!(
        yt_dlp_archive.file_name,
        format!("yt-dlp-{PLAN_TARGET}.zip")
    );
    assert_eq!(
        Some(yt_dlp_archive.expected_sha256.as_str()),
        json["ytDlp"]["targets"][PLAN_TARGET]["sha256"].as_str()
    );
    assert_eq!(deno.tool, "deno");
    assert_eq!(deno.file_name, format!("deno-{PLAN_TARGET}"));
    assert_eq!(deno.expected_sha256, DENO_AARCH64_DARWIN_SHA256);
    assert_eq!(ffmpeg.tool, "ffmpeg");
    assert_eq!(ffmpeg.file_name, format!("ffmpeg-{PLAN_TARGET}"));
    assert_eq!(ffmpeg.expected_sha256, FFMPEG_AARCH64_DARWIN_SHA256);
    // Сверяется сумма РАСПАКОВАННОГО, а не архива: взятое не из того поля
    // приняло бы в `binaries/` что угодно, кроме настоящего бинарника.
    assert_ne!(
        Some(ffmpeg.expected_sha256.as_str()),
        json["ffmpeg"]["targets"][PLAN_TARGET]["sha256"].as_str()
    );

    // Поведением, а не только полем: заглушка под именем каждого файла —
    // отказ, и отказ называет проигнорированную форточку.
    let dir = tempfile::tempdir().expect("tempdir");
    for check in [&yt_dlp_archive, &deno, &ffmpeg] {
        let path = check.path_in(dir.path());
        fs::write(&path, STUB_CONTENT).expect("write stub");
        let refusal = check
            .verify(dir.path(), "Собрался бы нерабочий бандл")
            .expect_err("release refuses the stub whatever the window says");
        assert!(
            refusal.starts_with(&format!("{}: ", check.tool)),
            "{refusal}"
        );
        assert_contains(
            &refusal,
            &[
                &path.display().to_string(),
                &format!("{ALLOW_STUB_ENV}=1 задано, но профиль release его не учитывает"),
            ],
        );
    }
}

#[test]
fn the_policy_is_taken_from_exact_environment_values() {
    let raw = repo_pin();
    let cases = [
        (Some("release"), None, RELEASE),
        (Some("release"), Some("1"), RELEASE_WITH_WINDOW),
        (Some("debug"), None, DEV),
        (Some("debug"), Some("1"), DEV_WITH_WINDOW),
        (None, Some("1"), DEV_WITH_WINDOW),
        (None, None, DEV),
        // Форточка открыта ровно значением `1`, профиль релизный ровно `release`.
        (Some("debug"), Some("true"), DEV),
        (Some("debug"), Some("0"), DEV),
        (Some("debug"), Some(""), DEV),
        (Some("Release"), None, DEV),
        (Some("release "), Some("1"), DEV_WITH_WINDOW),
    ];
    for (profile, window, expected) in cases {
        let checks = plan_build_checks(&raw, PLAN_TARGET, profile, window).expect("plan");
        assert_eq!(
            (
                checks.yt_dlp_archive.policy,
                checks.deno.policy,
                checks.ffmpeg.policy
            ),
            (expected, expected, expected),
            "PROFILE={profile:?} {ALLOW_STUB_ENV}={window:?}"
        );
    }
}

#[test]
fn the_plan_refuses_a_pin_it_could_not_check() {
    let raw = repo_pin();
    let json: serde_json::Value = serde_json::from_str(&raw).expect("pin is JSON");

    for section in ["ytDlp", "deno", "ffmpeg"] {
        let mut broken = json.clone();
        broken[section]["targets"]
            .as_object_mut()
            .expect("targets")
            .remove(PLAN_TARGET);
        let err = plan_build_checks(&broken.to_string(), PLAN_TARGET, Some("release"), None)
            .expect_err("entry missing");
        assert_contains(&err, &[&format!("{section}.targets.{PLAN_TARGET}")]);
    }

    // Сумма архива yt-dlp проверяется так же, как сумма deno.
    let mut upper = json.clone();
    let sum = upper["ytDlp"]["targets"][PLAN_TARGET]["sha256"]
        .as_str()
        .expect("archive sha256")
        .to_ascii_uppercase();
    upper["ytDlp"]["targets"][PLAN_TARGET]["sha256"] = serde_json::json!(sum);
    let err = plan_build_checks(&upper.to_string(), PLAN_TARGET, Some("release"), None)
        .expect_err("malformed archive sum");
    assert_contains(&err, &[&format!("ytDlp.targets.{PLAN_TARGET}.sha256")]);
}

/// Проводка `build.rs` по исходнику (TL-114): билд-скрипт тест не
/// исполняет, поэтому сверяется текст.
///
/// - Политику `build.rs` не собирает, не читает и не меняет: в коде нет
///   слов `StubPolicy`, `policy`, `release`, `stub_allowed`, построения
///   `PlannedCheck { … }`, `..` поверх сверки и сверки в обход плана
///   (`verify_pinned_file`, `PinnedFile`, `pinned_binary`), нет и `#[cfg`.
/// - Окружение читается ровно раз и уходит в план как есть.
/// - Все три сверки зовутся ровно раз на верхнем уровне `main` (отступ 4 —
///   не под `if`), в `main` нет `return` и `exit(`, отказ сверки — паника.
///
/// Не видит: выход из `main` паникой или бесконечным циклом до вызовов и
/// вызов, собранный макросом.
#[test]
fn build_rs_wires_every_check_from_the_plan_and_never_touches_the_policy() {
    let code: Vec<(usize, &str)> = include_str!("../build.rs")
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with("//")
        })
        .map(|line| (line.len() - line.trim_start().len(), line.trim()))
        .collect();
    let is_ident = |c: char| c == '_' || c.is_alphanumeric();
    let has_word = |line: &str, word: &str| {
        line.match_indices(word).any(|(at, _)| {
            !line[..at].chars().next_back().is_some_and(is_ident)
                && !line[at + word.len()..].chars().next().is_some_and(is_ident)
        })
    };
    let containing = |needle: &str| {
        code.iter()
            .filter(|(_, line)| line.contains(needle))
            .count()
    };

    for word in [
        "StubPolicy",
        "policy",
        "release",
        "stub_allowed",
        "PinnedFile",
        "verify_pinned_file",
        "pinned_binary",
    ] {
        let found: Vec<_> = code
            .iter()
            .filter(|(_, line)| has_word(line, word))
            .collect();
        assert!(found.is_empty(), "build.rs: {word} в коде: {found:?}");
    }
    for fragment in ["PlannedCheck {", "..checks", "..check", "..*check", "#[cfg"] {
        assert_eq!(containing(fragment), 0, "build.rs: {fragment:?} в коде");
    }

    for (indent, text) in [
        (4, "let checks = pinned_file::plan_build_checks("),
        (8, "env::var(\"PROFILE\").ok().as_deref(),"),
        (8, "env::var(ALLOW_STUB_ENV).ok().as_deref(),"),
        (
            4,
            "place_archive(&binaries, &destination, &checks.yt_dlp_archive);",
        ),
        (
            4,
            "check_pinned_file(&binaries, &checks.deno, DENO_CONSEQUENCE);",
        ),
        (
            4,
            "check_pinned_file(&binaries, &checks.ffmpeg, FFMPEG_CONSEQUENCE);",
        ),
        (
            4,
            "if !check_pinned_file(binaries, check, YTDLP_CONSEQUENCE) {",
        ),
        (4, "match check.verify(binaries, consequence) {"),
        (8, "Err(refusal) => panic!(\"{refusal}\"),"),
    ] {
        let found = code
            .iter()
            .filter(|(at, line)| *at == indent && *line == text)
            .count();
        assert_eq!(found, 1, "build.rs: строка {text:?} с отступом {indent}");
    }
    for (needle, expected) in [
        ("env::var(\"PROFILE\")", 1),
        ("env::var(ALLOW_STUB_ENV)", 1),
        ("plan_build_checks(", 1),
        ("checks.deno", 1),
        ("checks.ffmpeg", 1),
        ("check_pinned_file(", 4),
        ("place_archive(", 2),
    ] {
        assert_eq!(containing(needle), expected, "build.rs: {needle:?}");
    }

    let main: Vec<&str> = code
        .iter()
        .skip_while(|(_, line)| *line != "fn main() {")
        .take_while(|(at, line)| !(*at == 0 && *line == "}"))
        .map(|(_, line)| *line)
        .collect();
    assert!(main.len() > 10, "main не найден: {main:?}");
    for line in &main {
        assert!(
            !has_word(line, "return") && !line.contains("exit("),
            "build.rs: ранний выход из main: {line}"
        );
    }
}
