//! Тесты «Показать в папке» (TL-88).
//!
//! Таблицы `argv` проверяют все три ОС на macOS: ОС — параметр
//! [`build_plan`]. Сравнивается структура `argv`, а не склеенная строка.
//! Исключение — Windows: там строка и есть контракт с explorer.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::*;

const NFD_E: &str = "e\u{301}";

fn argv(command: &RevealCommand) -> &[OsString] {
    match &command.args {
        CommandArgs::Argv(args) => args,
        CommandArgs::WindowsCommandLine(line) => {
            panic!("ожидался argv, а не строка Windows {line:?}")
        }
    }
}

fn windows_line(command: &RevealCommand) -> &str {
    match &command.args {
        CommandArgs::WindowsCommandLine(line) => line,
        CommandArgs::Argv(args) => panic!("ожидалась строка Windows, а не argv {args:?}"),
    }
}

fn strings(items: &[&str]) -> Vec<OsString> {
    items.iter().map(OsString::from).collect()
}

fn dbus_send() -> DbusTool {
    DbusTool {
        flavor: DbusFlavor::DbusSend,
        program: PathBuf::from("/usr/bin/dbus-send"),
    }
}

fn gdbus() -> DbusTool {
    DbusTool {
        flavor: DbusFlavor::Gdbus,
        program: PathBuf::from("/usr/bin/gdbus"),
    }
}

/// Путь Unix и его URI. Эталон — Python 3.9 `urllib.parse.quote_from_bytes(
/// путь, safe="/")`: безопасны латиница, цифры, `_.-~` и `/`. Команда:
///
/// ```text
/// python3 -c 'from urllib.parse import quote_from_bytes as q; print(q(b"...", safe="/"))'
/// ```
fn unix_table() -> Vec<(String, String)> {
    [
        ("/home/u/clip.mp4", "file:///home/u/clip.mp4"),
        ("/home/u/a b.mp4", "file:///home/u/a%20b.mp4"),
        (
            "/home/u/Видео/ролик.mp4",
            "file:///home/u/%D0%92%D0%B8%D0%B4%D0%B5%D0%BE/%D1%80%D0%BE%D0%BB%D0%B8%D0%BA.mp4",
        ),
        ("/home/u/-rf.mp4", "file:///home/u/-rf.mp4"),
        ("/home/u/a,b.mp4", "file:///home/u/a%2Cb.mp4"),
        (
            "/home/u/it's \"q\".mp4",
            "file:///home/u/it%27s%20%22q%22.mp4",
        ),
        (
            "/home/u/100% #1?.mp4",
            "file:///home/u/100%25%20%231%3F.mp4",
        ),
        ("/home/u/~a_b-c.d", "file:///home/u/~a_b-c.d"),
        (
            "/home/u/[x];(y)&=+$!@*:\\z",
            "file:///home/u/%5Bx%5D%3B%28y%29%26%3D%2B%24%21%40%2A%3A%5Cz",
        ),
    ]
    .into_iter()
    .map(|(path, uri)| (path.to_string(), uri.to_string()))
    .chain([(
        format!("/home/u/{NFD_E}.mp4"),
        "file:///home/u/e%CC%81.mp4".to_string(),
    )])
    .collect()
}

// ---------------------------------------------------------------------------
// macOS
// ---------------------------------------------------------------------------

#[test]
fn macos_reveals_file_and_folder_with_open_r_double_dash() {
    for (path, _) in unix_table() {
        let plan = build_plan(
            TargetOs::MacOs,
            RevealTarget::SelectFile,
            Path::new(&path),
            None,
        )
        .unwrap_or_else(|err| panic!("{path:?}: {err}"));
        assert_eq!(
            plan.first.program,
            OsString::from("/usr/bin/open"),
            "{path:?}"
        );
        assert_eq!(argv(&plan.first), strings(&["-R", "--", &path]), "{path:?}");
        assert_eq!(plan.first.kind, LauncherKind::Finder);
        assert_eq!(plan.fallback, None);

        // Папка — тоже `-R`, не `open <папка>` (пакет `.app` запустился бы).
        let folder = Path::new(&path)
            .parent()
            .expect("у пути таблицы есть папка");
        let plan = build_plan(TargetOs::MacOs, RevealTarget::OpenFolder, folder, None)
            .unwrap_or_else(|err| panic!("{folder:?}: {err}"));
        assert_eq!(
            argv(&plan.first),
            vec![
                OsString::from("-R"),
                OsString::from("--"),
                folder.as_os_str().to_os_string()
            ],
        );
    }
}

#[test]
fn macos_puts_double_dash_immediately_before_a_leading_dash_path() {
    for path in [
        "/-rf.mp4",
        "/Users/u/--help",
        "/Users/u/-R",
        "/Users/u/-a Finder.mp4",
    ] {
        let plan = build_plan(
            TargetOs::MacOs,
            RevealTarget::SelectFile,
            Path::new(path),
            None,
        )
        .expect("абсолютный путь годится");
        let args = argv(&plan.first);
        let at = args
            .iter()
            .position(|arg| arg == OsStr::new(path))
            .unwrap_or_else(|| panic!("путь {path:?} не дошёл до argv: {args:?}"));
        assert!(
            at > 0 && args[at - 1] == "--",
            "перед {path:?} нет `--`: {args:?}"
        );
        assert_eq!(
            at,
            args.len() - 1,
            "после пути ничего быть не должно: {args:?}"
        );
    }
}

#[test]
fn unix_branches_reject_relative_paths_so_a_dash_cannot_lead() {
    for os in [TargetOs::MacOs, TargetOs::Linux] {
        for target in [RevealTarget::SelectFile, RevealTarget::OpenFolder] {
            for path in [
                "-rf.mp4",
                "--help",
                "-R",
                "relative/clip.mp4",
                "",
                " /x.mp4",
            ] {
                assert_eq!(
                    build_plan(os, target, Path::new(path), Some(&dbus_send())),
                    Err(PathRejection::NotAbsolute),
                    "{os:?} {target:?} {path:?}",
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

#[test]
fn windows_select_line_quotes_the_whole_path_after_the_comma() {
    let nfd = format!(r"C:\Users\u\{NFD_E}.mp4");
    let long_name = "я".repeat(120);
    let long = format!(r"C:\Users\u\{long_name}\{long_name}\{long_name}.mp4");
    let table: Vec<(String, String)> = [
        (
            r"C:\Users\u\Videos\clip.mp4",
            r#"/select,"C:\Users\u\Videos\clip.mp4""#,
        ),
        (
            r"C:\Users\u\My Videos\a b.mp4",
            r#"/select,"C:\Users\u\My Videos\a b.mp4""#,
        ),
        (
            r"C:\Users\u\a,b\c, d.mp4",
            r#"/select,"C:\Users\u\a,b\c, d.mp4""#,
        ),
        (
            r"C:\Users\u\Видео\ролик.mp4",
            r#"/select,"C:\Users\u\Видео\ролик.mp4""#,
        ),
        (r"C:\Users\u\-rf.mp4", r#"/select,"C:\Users\u\-rf.mp4""#),
        (
            r"C:\Users\u\100% #1 ^&;'.mp4",
            r#"/select,"C:\Users\u\100% #1 ^&;'.mp4""#,
        ),
        (r"d:/Media/x y.mp4", r#"/select,"d:\Media\x y.mp4""#),
        (r"\\?\C:\Users\u\x.mp4", r#"/select,"C:\Users\u\x.mp4""#),
        (
            r"\\?\UNC\srv\share\a b.mp4",
            r#"/select,"\\srv\share\a b.mp4""#,
        ),
        (r"\\srv\share\a,b.mp4", r#"/select,"\\srv\share\a,b.mp4""#),
    ]
    .into_iter()
    .map(|(path, line)| (path.to_string(), line.to_string()))
    .chain([
        (nfd.clone(), format!("/select,\"{nfd}\"")),
        (long.clone(), format!("/select,\"{long}\"")),
    ])
    .collect();

    for (path, expected) in table {
        let plan = build_plan(
            TargetOs::Windows,
            RevealTarget::SelectFile,
            Path::new(&path),
            None,
        )
        .unwrap_or_else(|err| panic!("{path:?}: {err}"));
        assert_eq!(plan.first.program, OsString::from("explorer.exe"));
        assert_eq!(plan.first.kind, LauncherKind::Explorer);
        assert_eq!(plan.fallback, None);
        assert_eq!(windows_line(&plan.first), expected, "{path:?}");
    }
}

#[test]
fn windows_folder_line_never_puts_a_backslash_before_the_closing_quote() {
    for (folder, expected) in [
        (r"C:\Users\u\Downloads", r#""C:\Users\u\Downloads""#),
        (r"C:\Users\u\My Downloads\", r#""C:\Users\u\My Downloads""#),
        (r"C:\Users\u\a,b\\", r#""C:\Users\u\a,b""#),
        (r"C:\", r"C:\"),
        (r"d:/", r"d:\"),
        (r"\\?\C:\", r"C:\"),
        (r"\\srv\share\", r#""\\srv\share""#),
    ] {
        let plan = build_plan(
            TargetOs::Windows,
            RevealTarget::OpenFolder,
            Path::new(folder),
            None,
        )
        .unwrap_or_else(|err| panic!("{folder:?}: {err}"));
        assert_eq!(windows_line(&plan.first), expected, "{folder:?}");
    }
}

/// Свойство, ради которого выбрана форма (doc модуля, «Windows»): строка
/// читается одинаково при правилах MSVCRT и при простых кавычках. Для этого
/// в ней нет `\"`, а кавычек — ровно пара вокруг пути или ни одной.
#[test]
fn windows_lines_are_unambiguous_under_both_quote_readings() {
    let folders = [r"C:\a b\", r"C:\a\\\", r"\\srv\s\", r"C:\", r"C:\x"];
    let files = [r"C:\a b\c.mp4", r"C:\a\b\", r"\\srv\share\x,y", r"C:\-x"];
    let lines = folders
        .iter()
        .map(|p| (RevealTarget::OpenFolder, p))
        .chain(files.iter().map(|p| (RevealTarget::SelectFile, p)))
        .map(|(target, path)| {
            let plan = build_plan(TargetOs::Windows, target, Path::new(path), None)
                .unwrap_or_else(|err| panic!("{path:?}: {err}"));
            windows_line(&plan.first).to_string()
        });
    for line in lines {
        assert!(!line.contains("\\\""), "`\\\"` в {line:?}");
        let quotes = line.matches('"').count();
        assert!(quotes == 0 || quotes == 2, "кавычек {quotes} в {line:?}");
        if quotes == 2 {
            assert!(line.ends_with('"'), "путь не последний в {line:?}");
        }
    }
}

#[test]
fn windows_rejects_paths_it_cannot_quote_or_that_are_not_absolute() {
    for (path, rejection) in [
        (r#"C:\Users\u\"q".mp4"#, PathRejection::ForbiddenChar),
        ("C:\\Users\\u\\a\nb.mp4", PathRejection::ForbiddenChar),
        ("C:\\Users\\u\\a\u{0}.mp4", PathRejection::ForbiddenChar),
        ("-rf.mp4", PathRejection::NotAbsolute),
        (r"relative\x.mp4", PathRejection::NotAbsolute),
        (r"/select,C:\x.mp4", PathRejection::NotAbsolute),
        (r"C:x.mp4", PathRejection::NotAbsolute),
        (r"\\.\PhysicalDrive0", PathRejection::NotAbsolute),
        (r"\\?\GLOBALROOT\x", PathRejection::NotAbsolute),
        (r"\\\x", PathRejection::NotAbsolute),
        ("", PathRejection::NotAbsolute),
    ] {
        for target in [RevealTarget::SelectFile, RevealTarget::OpenFolder] {
            assert_eq!(
                build_plan(TargetOs::Windows, target, Path::new(path), None),
                Err(rejection),
                "{path:?} {target:?}",
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn windows_rejects_non_unicode_paths() {
    use std::os::unix::ffi::OsStrExt;
    let path = Path::new(OsStr::from_bytes(b"C:\\a\xff.mp4"));
    assert_eq!(
        build_plan(TargetOs::Windows, RevealTarget::SelectFile, path, None),
        Err(PathRejection::NotUnicode),
    );
}

// ---------------------------------------------------------------------------
// Linux
// ---------------------------------------------------------------------------

#[test]
fn linux_without_dbus_opens_the_parent_folder_with_xdg_open() {
    for (path, _) in unix_table() {
        let folder = Path::new(&path)
            .parent()
            .expect("у пути таблицы есть папка");
        let plan = build_plan(
            TargetOs::Linux,
            RevealTarget::SelectFile,
            Path::new(&path),
            None,
        )
        .unwrap_or_else(|err| panic!("{path:?}: {err}"));
        assert_eq!(plan.first.program, OsString::from("xdg-open"));
        assert_eq!(plan.first.kind, LauncherKind::XdgOpen);
        assert_eq!(argv(&plan.first), vec![folder.as_os_str().to_os_string()]);
        assert_eq!(plan.fallback, None);
    }
}

#[test]
fn linux_folder_is_opened_with_xdg_open_even_when_dbus_is_there() {
    let folder = Path::new("/home/u/-dir, with space");
    let plan = build_plan(
        TargetOs::Linux,
        RevealTarget::OpenFolder,
        folder,
        Some(&dbus_send()),
    )
    .expect("абсолютная папка годится");
    assert_eq!(plan.first.program, OsString::from("xdg-open"));
    assert_eq!(argv(&plan.first), strings(&["/home/u/-dir, with space"]));
    assert_eq!(plan.fallback, None);
}

#[test]
fn linux_with_dbus_send_calls_show_items_then_falls_back_to_xdg_open() {
    for (path, uri) in unix_table() {
        let folder = Path::new(&path)
            .parent()
            .expect("у пути таблицы есть папка");
        let plan = build_plan(
            TargetOs::Linux,
            RevealTarget::SelectFile,
            Path::new(&path),
            Some(&dbus_send()),
        )
        .unwrap_or_else(|err| panic!("{path:?}: {err}"));

        assert_eq!(plan.first.program, OsString::from("/usr/bin/dbus-send"));
        assert_eq!(plan.first.kind, LauncherKind::DbusShowItems);
        assert_eq!(
            argv(&plan.first),
            strings(&[
                "--session",
                "--print-reply",
                "--dest=org.freedesktop.FileManager1",
                "--type=method_call",
                "--reply-timeout=4000",
                "/org/freedesktop/FileManager1",
                "org.freedesktop.FileManager1.ShowItems",
                &format!("array:string:{uri}"),
                "string:",
            ]),
            "{path:?}",
        );

        let fallback = plan.fallback.expect("у D-Bus есть запасной путь");
        assert_eq!(fallback.program, OsString::from("xdg-open"));
        assert_eq!(argv(&fallback), vec![folder.as_os_str().to_os_string()]);
    }
}

#[test]
fn linux_with_gdbus_passes_gvariant_text() {
    for (path, uri) in unix_table() {
        let plan = build_plan(
            TargetOs::Linux,
            RevealTarget::SelectFile,
            Path::new(&path),
            Some(&gdbus()),
        )
        .unwrap_or_else(|err| panic!("{path:?}: {err}"));
        assert_eq!(plan.first.program, OsString::from("/usr/bin/gdbus"));
        assert_eq!(
            argv(&plan.first),
            strings(&[
                "call",
                "--session",
                "--dest",
                "org.freedesktop.FileManager1",
                "--object-path",
                "/org/freedesktop/FileManager1",
                "--method",
                "org.freedesktop.FileManager1.ShowItems",
                "--timeout",
                "4",
                &format!("['{uri}']"),
                "''",
            ]),
            "{path:?}",
        );
        assert!(plan.fallback.is_some());
    }
}

#[test]
fn dbus_reply_timeout_lets_the_tool_give_up_before_the_ceiling_kills_it() {
    assert!(DBUS_REPLY_TIMEOUT < Ceilings::PRODUCTION.exit);
    // gdbus принимает секунды: доля секунды молча потерялась бы.
    assert_eq!(DBUS_REPLY_TIMEOUT.subsec_nanos(), 0);
}

#[test]
fn linux_root_file_has_no_parent_to_fall_back_to() {
    assert_eq!(
        build_plan(
            TargetOs::Linux,
            RevealTarget::SelectFile,
            Path::new("/"),
            None
        ),
        Err(PathRejection::NoParent),
    );
}

// ---------------------------------------------------------------------------
// URI
// ---------------------------------------------------------------------------

fn decode_uri_path(uri: &str) -> Vec<u8> {
    let rest = uri
        .strip_prefix("file://")
        .expect("URI начинается с file://");
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let hex = std::str::from_utf8(&bytes[at + 1..at + 3]).expect("две цифры после %");
            out.push(u8::from_str_radix(hex, 16).expect("шестнадцатеричный байт"));
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    out
}

#[test]
fn file_uri_matches_the_python_reference() {
    for (path, uri) in unix_table() {
        assert_eq!(
            file_uri(Path::new(&path)).as_deref(),
            Ok(uri.as_str()),
            "{path:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn file_uri_encodes_non_utf8_bytes_as_they_are() {
    use std::os::unix::ffi::OsStrExt;
    let path = Path::new(OsStr::from_bytes(b"/home/u/\xff\xfe.mp4"));
    assert_eq!(file_uri(path).as_deref(), Ok("file:///home/u/%FF%FE.mp4"));
}

#[test]
fn file_uri_leaves_nothing_dbus_send_or_gvariant_would_split_on() {
    let uri = file_uri(Path::new("/h/a,b 'c' \"d\" \\e #f ?g %h [i] ;j"))
        .expect("абсолютный путь годится");
    for forbidden in [',', '\'', '"', '\\', ' ', '#', '?', '[', ']', ';'] {
        assert!(!uri.contains(forbidden), "{forbidden:?} в {uri}");
    }
    let body = uri.strip_prefix("file://").expect("префикс");
    assert!(
        body.bytes()
            .all(|byte| is_uri_path_safe(byte) || byte == b'%' || byte.is_ascii_hexdigit()),
        "{uri}",
    );
}

#[test]
fn file_uri_rejects_relative_paths() {
    assert_eq!(
        file_uri(Path::new("-rf.mp4")),
        Err(PathRejection::NotAbsolute)
    );
    assert_eq!(file_uri(Path::new("")), Err(PathRejection::NotAbsolute));
}

/// Обратимость на всех 256 байтах и на ста тысячах псевдослучайных путей:
/// раскодированный URI — ровно исходные байты.
#[cfg(unix)]
#[test]
fn file_uri_round_trips_every_byte() {
    use std::os::unix::ffi::OsStrExt;

    let mut every_byte: Vec<u8> = b"/".to_vec();
    every_byte.extend((1_u8..=255).filter(|byte| *byte != b'/'));
    let mut corpus = vec![every_byte];

    // xorshift64: воспроизводимо и без крейтов.
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..100_000 {
        let len = usize::try_from(next() % 64).expect("u64 % 64 влезает в usize");
        let mut path = vec![b'/'];
        path.extend(
            (0..len)
                .map(|_| next().to_le_bytes()[0])
                .filter(|byte| *byte != 0),
        );
        corpus.push(path);
    }

    for bytes in corpus {
        let uri = file_uri(Path::new(OsStr::from_bytes(&bytes))).expect("путь абсолютный");
        let body = uri.strip_prefix("file://").expect("префикс");
        assert!(
            body.bytes()
                .all(|b| is_uri_path_safe(b) || b == b'%' || b.is_ascii_hexdigit()),
            "{uri}",
        );
        assert_eq!(decode_uri_path(&uri), bytes, "{uri}");
    }
}

#[test]
fn very_long_path_reaches_argv_and_uri_whole() {
    let component = "я".repeat(127); // 254 байта — у предела имени ext4.
    let path = format!(
        "/home/u/{}/clip, 1.mp4",
        vec![component.as_str(); 16].join("/")
    );
    assert!(path.len() > 4096, "путь длиннее PATH_MAX: {}", path.len());

    let mac = build_plan(
        TargetOs::MacOs,
        RevealTarget::SelectFile,
        Path::new(&path),
        None,
    )
    .expect("длинный путь годится");
    assert_eq!(argv(&mac.first)[2], OsString::from(&path));

    let uri = file_uri(Path::new(&path)).expect("длинный путь годится");
    assert_eq!(decode_uri_path(&uri), path.as_bytes());
}

// ---------------------------------------------------------------------------
// Поиск D-Bus в PATH
// ---------------------------------------------------------------------------

#[cfg(unix)]
fn put_file(dir: &Path, name: &str, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    fs::write(&path, "#!/bin/sh\nexit 0\n").expect("запись заглушки");
    fs::set_permissions(&path, fs::Permissions::from_mode(mode)).expect("chmod заглушки");
}

#[cfg(unix)]
#[test]
fn find_dbus_tool_prefers_dbus_send_and_skips_non_executables() {
    let first = tempfile::tempdir().expect("временный каталог");
    let second = tempfile::tempdir().expect("временный каталог");
    let path_var = std::env::join_paths([first.path(), second.path()]).expect("PATH");

    assert_eq!(find_dbus_tool(None), None);
    assert_eq!(find_dbus_tool(Some(&path_var)), None);

    // Не исполняемый файл и каталог с тем же именем — не инструмент.
    put_file(first.path(), "dbus-send", 0o644);
    fs::create_dir(second.path().join("gdbus")).expect("каталог-обманка");
    assert_eq!(find_dbus_tool(Some(&path_var)), None);

    put_file(first.path(), "gdbus", 0o755);
    assert_eq!(
        find_dbus_tool(Some(&path_var)),
        Some(DbusTool {
            flavor: DbusFlavor::Gdbus,
            program: first.path().join("gdbus"),
        }),
    );

    // gdbus раньше в PATH, но dbus-send предпочтительнее.
    put_file(second.path(), "dbus-send", 0o755);
    assert_eq!(
        find_dbus_tool(Some(&path_var)),
        Some(DbusTool {
            flavor: DbusFlavor::DbusSend,
            program: second.path().join("dbus-send"),
        }),
    );
}

#[cfg(unix)]
#[test]
fn find_dbus_tool_ignores_relative_path_entries() {
    let dir = tempfile::tempdir().expect("временный каталог");
    put_file(dir.path(), "dbus-send", 0o755);
    // Относительный элемент, указывающий в тот же каталог от текущего.
    let cwd = std::env::current_dir().expect("текущий каталог");
    let relative: PathBuf = cwd
        .components()
        .skip(1)
        .map(|_| "..")
        .collect::<PathBuf>()
        .join(
            dir.path()
                .strip_prefix("/")
                .expect("временный каталог абсолютный"),
        );
    assert!(relative.is_relative());
    assert!(
        relative.join("dbus-send").is_file(),
        "{relative:?} не ведёт к заглушке"
    );

    let path_var = std::env::join_paths([relative.as_path(), Path::new("")]).expect("PATH");
    assert_eq!(find_dbus_tool(Some(&path_var)), None);
}

// ---------------------------------------------------------------------------
// Три случая Ф-8 — подменённый запускатель
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Recording {
    calls: RefCell<Vec<RevealCommand>>,
    outcomes: RefCell<VecDeque<Result<(), LauncherFailure>>>,
}

impl Recording {
    fn answering(outcomes: impl IntoIterator<Item = Result<(), LauncherFailure>>) -> Self {
        Self {
            calls: RefCell::default(),
            outcomes: RefCell::new(outcomes.into_iter().collect()),
        }
    }

    fn calls(&self) -> Vec<RevealCommand> {
        self.calls.borrow().clone()
    }
}

impl Launcher for Recording {
    fn launch(&self, command: &RevealCommand) -> Result<(), LauncherFailure> {
        self.calls.borrow_mut().push(command.clone());
        self.outcomes.borrow_mut().pop_front().unwrap_or(Ok(()))
    }
}

fn failure(code: i32) -> LauncherFailure {
    LauncherFailure {
        program: "tool".into(),
        cause: LaunchCause::ExitedWithError,
        exit_code: Some(code),
        stderr_tail: Some(format!("stderr {code}")),
    }
}

struct Scene {
    _root: tempfile::TempDir,
    folder: PathBuf,
    file: PathBuf,
}

fn scene(file_present: bool, folder_present: bool) -> Scene {
    let root = tempfile::tempdir().expect("временный каталог");
    let folder = root.path().join("-Загрузки, #1");
    let file = folder.join("-rf ролик.mp4");
    if folder_present {
        fs::create_dir(&folder).expect("папка");
    }
    if file_present {
        fs::write(&file, b"video").expect("файл");
    }
    Scene {
        _root: root,
        folder,
        file,
    }
}

#[test]
fn file_present_is_revealed_selected() {
    let scene = scene(true, true);
    let launcher = Recording::default();
    assert_eq!(
        reveal_with(TargetOs::MacOs, &scene.file, None, &launcher),
        Ok(())
    );
    assert_eq!(
        launcher.calls(),
        vec![
            build_plan(TargetOs::MacOs, RevealTarget::SelectFile, &scene.file, None)
                .expect("план")
                .first
        ],
    );
}

#[test]
fn file_missing_folder_present_opens_folder_then_reports_file_missing() {
    for os in [TargetOs::MacOs, TargetOs::Linux] {
        let scene = scene(false, true);
        let launcher = Recording::default();
        assert_eq!(
            reveal_with(os, &scene.file, Some(&dbus_send()), &launcher),
            Err(RevealError::FileMissing),
            "{os:?}",
        );
        let calls = launcher.calls();
        assert_eq!(calls.len(), 1, "{os:?}: {calls:?}");
        let expected_folder = scene.folder.as_os_str().to_os_string();
        assert_eq!(argv(&calls[0]).last(), Some(&expected_folder), "{os:?}");
        if os == TargetOs::Linux {
            assert_eq!(
                calls[0].kind,
                LauncherKind::XdgOpen,
                "папку D-Bus не выделяет"
            );
        }
    }
}

#[test]
fn folder_missing_launches_nothing() {
    for os in [TargetOs::MacOs, TargetOs::Linux] {
        let scene = scene(false, false);
        let launcher = Recording::default();
        assert_eq!(
            reveal_with(os, &scene.file, Some(&dbus_send()), &launcher),
            Err(RevealError::FolderMissing),
            "{os:?}",
        );
        assert_eq!(launcher.calls(), Vec::new(), "{os:?}");
    }
}

#[test]
fn directory_in_place_of_the_file_counts_as_missing_file() {
    let scene = scene(false, true);
    fs::create_dir(&scene.file).expect("каталог вместо файла");
    let launcher = Recording::default();
    assert_eq!(
        reveal_with(TargetOs::MacOs, &scene.file, None, &launcher),
        Err(RevealError::FileMissing),
    );
    assert_eq!(launcher.calls().len(), 1);
}

#[test]
fn failed_folder_open_is_a_launcher_failure_not_file_missing() {
    let scene = scene(false, true);
    let launcher = Recording::answering([Err(failure(1))]);
    assert_eq!(
        reveal_with(TargetOs::MacOs, &scene.file, None, &launcher),
        Err(RevealError::LauncherFailed(failure(1))),
    );
}

#[test]
fn failed_select_is_a_launcher_failure() {
    let scene = scene(true, true);
    let launcher = Recording::answering([Err(failure(2))]);
    assert_eq!(
        reveal_with(TargetOs::MacOs, &scene.file, None, &launcher),
        Err(RevealError::LauncherFailed(failure(2))),
    );
}

#[test]
fn linux_dbus_failure_falls_back_to_xdg_open_and_succeeds() {
    let scene = scene(true, true);
    let launcher = Recording::answering([Err(failure(1)), Ok(())]);
    assert_eq!(
        reveal_with(TargetOs::Linux, &scene.file, Some(&dbus_send()), &launcher),
        Ok(()),
    );
    let kinds: Vec<_> = launcher.calls().iter().map(|call| call.kind).collect();
    assert_eq!(kinds, [LauncherKind::DbusShowItems, LauncherKind::XdgOpen]);
}

#[test]
fn linux_dbus_success_does_not_open_a_second_window() {
    let scene = scene(true, true);
    let launcher = Recording::default();
    assert_eq!(
        reveal_with(TargetOs::Linux, &scene.file, Some(&gdbus()), &launcher),
        Ok(()),
    );
    let kinds: Vec<_> = launcher.calls().iter().map(|call| call.kind).collect();
    assert_eq!(kinds, [LauncherKind::DbusShowItems]);
}

#[test]
fn linux_both_attempts_failing_reports_the_last_failure() {
    let scene = scene(true, true);
    let launcher = Recording::answering([Err(failure(1)), Err(failure(3))]);
    assert_eq!(
        reveal_with(TargetOs::Linux, &scene.file, Some(&dbus_send()), &launcher),
        Err(RevealError::LauncherFailed(failure(3))),
    );
}

#[test]
fn rejected_path_touches_neither_disk_nor_processes() {
    let launcher = Recording::default();
    assert_eq!(
        reveal_with(TargetOs::MacOs, Path::new("-rf.mp4"), None, &launcher),
        Err(RevealError::Rejected(PathRejection::NotAbsolute)),
    );
    assert_eq!(launcher.calls(), Vec::new());
}

// ---------------------------------------------------------------------------
// Настоящий запуск — заглушки-скрипты
// ---------------------------------------------------------------------------

#[cfg(unix)]
mod system {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("запись скрипта");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod скрипта");
        path
    }

    fn command(program: &Path, args: &[&str], kind: LauncherKind) -> RevealCommand {
        RevealCommand {
            program: program.as_os_str().to_os_string(),
            args: CommandArgs::Argv(strings(args)),
            kind,
        }
    }

    /// Для проверок кода возврата: потолок заведомо не наступает, тест
    /// кончается, когда кончается скрипт. Секундомер ничего не решает.
    fn patient() -> SystemLauncher {
        SystemLauncher {
            ceilings: Ceilings {
                exit: Duration::from_secs(30),
                linger: Duration::from_secs(30),
            },
        }
    }

    /// Для проверок потолка. Три секунды — с запасом на запуск `sh` под
    /// нагрузкой параллельного прогона: при 300 мс скрипт не успевал
    /// стартовать. Скрипт спит 30 с, так что наступление потолка не зависит
    /// от скорости машины.
    const STALL_CEILING: Duration = Duration::from_secs(3);

    fn stalling() -> SystemLauncher {
        SystemLauncher {
            ceilings: Ceilings {
                exit: STALL_CEILING,
                linger: STALL_CEILING,
            },
        }
    }

    fn alive(pid: &str) -> bool {
        std::process::Command::new("kill")
            .args(["-0", pid])
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    fn wait_for_file(path: &Path) -> String {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Ok(text) = fs::read_to_string(path) {
                if text.ends_with('\n') {
                    return text;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("{path:?} так и не появился");
    }

    #[test]
    fn missing_program_is_not_started_rather_than_a_panic() {
        let dir = tempfile::tempdir().expect("временный каталог");
        let missing = dir.path().join("no-such-tool-tl88");
        let result =
            SystemLauncher::default().launch(&command(&missing, &[], LauncherKind::XdgOpen));
        assert_eq!(
            result,
            Err(LauncherFailure {
                program: "no-such-tool-tl88".into(),
                cause: LaunchCause::NotStarted(io::ErrorKind::NotFound),
                exit_code: None,
                stderr_tail: None,
            }),
        );
    }

    #[test]
    fn nonzero_exit_carries_code_and_stderr_tail() {
        let dir = tempfile::tempdir().expect("временный каталог");
        let tool = script(
            dir.path(),
            "open",
            "echo 'The file does not exist.' >&2\nexit 3",
        );
        let result = patient().launch(&command(&tool, &[], LauncherKind::Finder));
        assert_eq!(
            result,
            Err(LauncherFailure {
                program: "open".into(),
                cause: LaunchCause::ExitedWithError,
                exit_code: Some(3),
                stderr_tail: Some("The file does not exist.".into()),
            }),
        );
    }

    #[test]
    fn zero_exit_is_success() {
        let dir = tempfile::tempdir().expect("временный каталог");
        let tool = script(dir.path(), "open", "exit 0");
        assert_eq!(
            patient().launch(&command(&tool, &[], LauncherKind::Finder)),
            Ok(())
        );
    }

    #[test]
    fn explorer_exit_code_is_not_a_failure() {
        let dir = tempfile::tempdir().expect("временный каталог");
        let tool = script(dir.path(), "explorer.exe", "exit 1");
        assert_eq!(
            patient().launch(&command(&tool, &[], LauncherKind::Explorer)),
            Ok(())
        );
    }

    #[test]
    fn xdg_open_exit_code_is_a_failure_without_stderr() {
        let dir = tempfile::tempdir().expect("временный каталог");
        let tool = script(dir.path(), "xdg-open", "echo 'no method' >&2\nexit 3");
        let result = patient().launch(&command(&tool, &[], LauncherKind::XdgOpen));
        assert_eq!(
            result,
            Err(LauncherFailure {
                program: "xdg-open".into(),
                cause: LaunchCause::ExitedWithError,
                exit_code: Some(3),
                // stderr отпускаемой утилиты не собирается (doc модуля).
                stderr_tail: None,
            }),
        );
    }

    /// Аргументы доходят байт в байт и не проходят через shell: подстановка
    /// команды в аргументе не исполняется.
    #[test]
    fn arguments_reach_the_program_verbatim_without_a_shell() {
        let dir = tempfile::tempdir().expect("временный каталог");
        let out = dir.path().join("argv.txt");
        let pwned = dir.path().join("pwned");
        let tool = script(
            dir.path(),
            "open",
            &format!(
                "for a in \"$@\"; do printf '[%s]' \"$a\"; done > '{}'\necho >> '{}'",
                out.display(),
                out.display()
            ),
        );
        let hostile = format!(
            "/tmp/-rf $(touch {}) `touch {}`; a,b 'c' \"d\" %e #f",
            pwned.display(),
            pwned.display()
        );
        let plan = build_plan(
            TargetOs::MacOs,
            RevealTarget::SelectFile,
            Path::new(&hostile),
            None,
        )
        .expect("абсолютный путь годится");
        let mut launched = plan.first.clone();
        launched.program = tool.into_os_string();

        assert_eq!(patient().launch(&launched), Ok(()));
        assert_eq!(wait_for_file(&out), format!("[-R][--][{hostile}]\n"));
        assert!(!pwned.exists(), "аргумент исполнился как команда");
    }

    #[test]
    fn hanging_launcher_is_killed_at_the_ceiling() {
        let dir = tempfile::tempdir().expect("временный каталог");
        let pid_file = dir.path().join("pid");
        let tool = script(
            dir.path(),
            "dbus-send",
            &format!(
                "echo $$ > '{}'\necho 'waiting' >&2\nexec sleep 30",
                pid_file.display()
            ),
        );
        let started = Instant::now();
        let result = stalling().launch(&command(&tool, &[], LauncherKind::DbusShowItems));
        let elapsed = started.elapsed();

        assert_eq!(
            result,
            Err(LauncherFailure {
                program: "dbus-send".into(),
                cause: LaunchCause::TimedOut(STALL_CEILING),
                exit_code: None,
                stderr_tail: Some("waiting".into()),
            }),
        );
        // Меньше `sleep 30`: процесс не дождались, а убили.
        assert!(elapsed < Duration::from_secs(20), "ждали {elapsed:?}");
        let pid = wait_for_file(&pid_file);
        assert!(!alive(pid.trim()), "зависшая утилита пережила потолок");
    }

    #[test]
    fn lingering_handler_is_released_alive_not_killed() {
        let dir = tempfile::tempdir().expect("временный каталог");
        let pid_file = dir.path().join("pid");
        let tool = script(
            dir.path(),
            "xdg-open",
            &format!("echo $$ > '{}'\nexec sleep 30", pid_file.display()),
        );
        let started = Instant::now();
        let result = stalling().launch(&command(&tool, &[], LauncherKind::XdgOpen));

        assert_eq!(result, Ok(()));
        assert!(started.elapsed() < Duration::from_secs(20));
        let pid = wait_for_file(&pid_file);
        let pid = pid.trim();
        assert!(alive(pid), "отпускаемый обработчик убит");
        let _ = std::process::Command::new("kill")
            .args(["-9", pid])
            .status();
    }

    #[cfg(not(windows))]
    #[test]
    fn windows_command_line_is_refused_elsewhere() {
        let launched = RevealCommand {
            program: "explorer.exe".into(),
            args: CommandArgs::WindowsCommandLine(r#"/select,"C:\x.mp4""#.into()),
            kind: LauncherKind::Explorer,
        };
        let result = SystemLauncher::default().launch(&launched);
        assert_eq!(
            result.map_err(|failure| failure.cause),
            Err(LaunchCause::Unsupported)
        );
    }

    /// Живой `open -R` на macOS (К-3, визуальная часть — Finder откроется).
    ///
    /// ```text
    /// cd src-tauri && cargo test --locked os_reveal::tests::system::live_finder_reveals_a_real_file -- --ignored --nocapture
    /// ```
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "открывает Finder; команда запуска в doc теста"]
    fn live_finder_reveals_a_real_file() {
        let dir = tempfile::tempdir().expect("временный каталог");
        let file = dir.path().join("-rf TL-88, #1 %20 ролик.mp4");
        fs::write(&file, b"video").expect("файл");

        let started = Instant::now();
        let result = reveal(&file);
        eprintln!("reveal({file:?}) = {result:?} за {:?}", started.elapsed());
        assert_eq!(result, Ok(()), "open -R вернул не 0");
        // Finder успевает прочитать путь до удаления каталога.
        std::thread::sleep(Duration::from_secs(2));
    }
}
