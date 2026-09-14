//! Сторож сверки sidecar-файлов с пином на сборке (TL-112).
//!
//! `build.rs` отказывает сборке, если под именем deno или архива yt-dlp в
//! `binaries/` лежит не то, что закреплено в `binaries.lock.json`, и в
//! профиле `release` не принимает для этого никаких форточек. Сам
//! билд-скрипт `cargo test` не видит, поэтому решение вынесено в
//! `build_support/pinned_file.rs` и подключено сюда тем же `#[path]`, что и
//! в `build.rs`: тест проверяет ровно тот код, что стоит на сборке.
//!
//! Чего тест не видит: проводку — что `build.rs` вообще зовёт сверку для
//! deno и берёт путь из пина. Она доказывается живой сборкой в
//! release-профиле с заглушкой на месте deno (отчёт TL-112).

#[path = "../build_support/pinned_file.rs"]
mod pinned_file;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use pinned_file::{
    pinned_binary, sha256_file, verify_pinned_file, PinnedBinary, PinnedFile, StubPolicy, Verdict,
    ALLOW_STUB_ENV,
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
