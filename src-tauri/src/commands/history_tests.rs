//! Тесты команд истории (TL-90) — уровень команд, без Tauri-рантайма.
//!
//! Тела команд зовутся напрямую (`page_in`, `delete_in`, `clear_in`,
//! `show_in`) с настоящим [`HistoryState`] во временном каталоге. Обёртки
//! `#[tauri::command]` только достают `State` и зовут их. Сети и процессов
//! нет: запускатель показа подменён.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::Connection;
use tempfile::{tempdir, TempDir};

use super::*;
use crate::os_reveal::{
    reveal_with, CommandArgs, LaunchCause, Launcher, LauncherFailure, LinuxTools, RevealCommand,
    TargetOs,
};
use crate::storage::history::{NewHistoryRecord, HISTORY_FILE_NAME};
use crate::types::{
    HistoryFileStatus, HistoryNotice, HistoryWriteFailure, QualityKind, SelectedQuality,
};

/// Сколько рандеву ждёт соседа, прежде чем признать рантайм заблокированным.
/// Это не замер, а только потолок провала: при исправном коде сосед
/// откликается сразу, и тест проходит без ожидания.
const RENDEZVOUS_CEILING: Duration = Duration::from_secs(10);

struct Scene {
    data: TempDir,
    downloads: TempDir,
    state: Arc<HistoryState>,
}

fn scene() -> Scene {
    let data = tempdir().expect("временный каталог данных");
    let downloads = tempdir().expect("временная папка назначения");
    let state = Arc::new(HistoryState::open(Ok(data.path().to_path_buf())));
    Scene {
        data,
        downloads,
        state,
    }
}

fn store(state: &HistoryState) -> &HistoryStore {
    state.store().expect("история открыта")
}

fn record(folder: &Path, n: u64) -> NewHistoryRecord {
    NewHistoryRecord {
        video_id: format!("vid{n:08}"),
        url: format!("https://www.youtube.com/watch?v=vid{n:08}"),
        title: format!("Ролик №{n}"),
        quality: SelectedQuality {
            kind: QualityKind::Standard,
            height_px: Some(1080),
        },
        file_name: format!("Ролик №{n}.mp4"),
        folder: folder.to_path_buf(),
        size_bytes: 1_000 + n,
        finished_at_unix_secs: 1_000 + n,
    }
}

/// Вставляет запись и кладёт её файл на диск.
fn insert_with_file(state: &HistoryState, folder: &Path, n: u64) -> String {
    let new = record(folder, n);
    fs::write(folder.join(&new.file_name), format!("содержимое {n}")).expect("файл записи");
    store(state).insert(&new).expect("вставка").to_string()
}

async fn first_page(state: &Arc<HistoryState>) -> HistoryPage {
    page_in(Arc::clone(state), None, None, HistoryStore::page)
        .await
        .expect("страница читается")
}

/// Обход ФС: относительный путь → содержимое файла (`None` у каталога).
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("каталог обходится") {
            let path = entry.expect("элемент каталога").path();
            let relative = path.strip_prefix(root).expect("внутри корня").to_path_buf();
            if path.is_dir() {
                out.insert(relative, None);
                stack.push(path);
            } else {
                out.insert(relative, Some(fs::read(&path).expect("файл читается")));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// history_page
// ---------------------------------------------------------------------------

/// Добавка к TL-90: пометки только в ответе без курсора, обе сразу; запрос с
/// курсором, пришедший первым, их не выдаёт и не гасит.
#[tokio::test]
async fn notices_wait_through_a_cursor_request_and_arrive_both_on_the_first_page() {
    let data = tempdir().expect("каталог данных");
    let downloads = tempdir().expect("папка назначения");
    fs::write(data.path().join(HISTORY_FILE_NAME), "не база ".repeat(40)).expect("мусор");
    let state = Arc::new(HistoryState::open(Ok(data.path().to_path_buf())));
    let id = insert_with_file(&state, downloads.path(), 1);
    store(&state).record_write_failure(HistoryWriteFailure::DiskFull);

    let issued_cursor = HistoryCursor {
        finished_at_unix_secs: 5_000,
        id: "7".to_owned(),
    };
    let with_cursor = page_in(
        Arc::clone(&state),
        Some(issued_cursor),
        None,
        HistoryStore::page,
    )
    .await
    .expect("страница с курсором");
    assert!(with_cursor.notices.is_empty(), "{:?}", with_cursor.notices);
    assert_eq!(with_cursor.entries.len(), 1, "курсор старше записи");

    let first = first_page(&state).await;
    assert_eq!(
        first.notices,
        vec![
            HistoryNotice::BaseRecreated,
            HistoryNotice::LastWriteFailed {
                cause: HistoryWriteFailure::DiskFull
            },
        ]
    );
    assert_eq!(first.entries.len(), 1);
    let entry = &first.entries[0];
    assert_eq!(entry.id, id);
    assert_eq!(entry.file_status, HistoryFileStatus::Present);
    assert_eq!(entry.title, "Ролик №1");
    assert!(first.next_cursor.is_none());

    assert!(
        first_page(&state).await.notices.is_empty(),
        "пометки выданы дважды"
    );
}

/// Контракт `HistoryCursor`: курсор с нецелым `id` — пустая страница без
/// `nextCursor`, не отказ, и пометки он не гасит. Строка лога называет класс.
#[tokio::test]
async fn a_foreign_cursor_gives_an_empty_page_and_is_logged_with_its_class() {
    let scene = scene();
    insert_with_file(&scene.state, scene.downloads.path(), 1);
    store(&scene.state).record_write_failure(HistoryWriteFailure::NoAccess);

    for id in ["чужой", "1.5", "007", ""] {
        let cursor = HistoryCursor {
            finished_at_unix_secs: 5_000,
            id: id.to_owned(),
        };
        let line = foreign_cursor_log(&cursor).expect("курсор чужой");
        assert!(line.contains("foreignCursor"), "{line}");

        let page = page_in(
            Arc::clone(&scene.state),
            Some(cursor),
            None,
            HistoryStore::page,
        )
        .await
        .expect("пустая страница, а не отказ");
        assert_eq!(page.entries, Vec::new(), "{id:?}");
        assert_eq!(page.next_cursor, None, "{id:?}");
        assert_eq!(page.notices, Vec::new(), "{id:?}");
    }

    let issued = HistoryCursor {
        finished_at_unix_secs: 5_000,
        id: "12".to_owned(),
    };
    assert_eq!(foreign_cursor_log(&issued), None);
    let long = HistoryCursor {
        finished_at_unix_secs: 1,
        id: "x".repeat(10_000),
    };
    let line = foreign_cursor_log(&long).expect("курсор чужой");
    assert!(
        line.chars().count() < 300,
        "id в логе не обрезан: {} знаков",
        line.len()
    );

    assert_eq!(
        first_page(&scene.state).await.notices,
        vec![HistoryNotice::LastWriteFailed {
            cause: HistoryWriteFailure::NoAccess
        }],
        "чужой курсор погасил пометку"
    );
}

#[test]
fn folder_display_names_the_system_downloads_only_by_equal_path() {
    let downloads = Path::new("/Users/u/Downloads");
    assert_eq!(
        folder_display(Path::new("/Users/u/Downloads/"), Some(downloads)),
        FolderDisplay::SystemDownloads
    );
    assert_eq!(
        folder_display(Path::new("/Users/u/Downloads/sub"), Some(downloads)),
        FolderDisplay::Custom {
            path: "/Users/u/Downloads/sub".to_owned()
        }
    );
    assert_eq!(
        folder_display(downloads, None),
        FolderDisplay::Custom {
            path: "/Users/u/Downloads".to_owned()
        }
    );
}

#[tokio::test]
async fn entries_carry_the_folder_display_by_the_resolved_downloads() {
    let scene = scene();
    let other = tempdir().expect("своя папка");
    insert_with_file(&scene.state, scene.downloads.path(), 1);
    insert_with_file(&scene.state, other.path(), 2);

    let page = page_in(
        Arc::clone(&scene.state),
        None,
        Some(scene.downloads.path().to_path_buf()),
        HistoryStore::page,
    )
    .await
    .expect("страница");
    let displays: Vec<_> = page
        .entries
        .iter()
        .map(|e| e.folder_display.clone())
        .collect();
    assert_eq!(
        displays,
        vec![
            FolderDisplay::Custom {
                path: other.path().to_string_lossy().into_owned()
            },
            FolderDisplay::SystemDownloads,
        ]
    );
}

/// Паника в блокирующем пуле — типизированный отказ, промис `invoke` не
/// повисает.
#[tokio::test]
async fn a_panicking_read_is_a_typed_refusal() {
    let scene = scene();
    let refused = page_in(Arc::clone(&scene.state), None, None, |_, _| {
        panic!("чтение запаниковало (ожидаемо в тесте)")
    })
    .await
    .expect_err("паника — отказ");
    assert_eq!(refused.reason, HistoryUnavailableReason::NoAccess);
}

// ---------------------------------------------------------------------------
// Недоступная история
// ---------------------------------------------------------------------------

/// Ф-1 (в): база новее — каждая из четырёх команд отвечает причиной
/// открытия, а файл не тронут.
#[tokio::test]
async fn a_newer_base_makes_every_command_answer_unavailable() {
    let data = tempdir().expect("каталог данных");
    let path = data.path().join(HISTORY_FILE_NAME);
    {
        let conn = Connection::open(&path).expect("фикстура");
        conn.execute_batch("CREATE TABLE t (x INTEGER);")
            .expect("фикстура-схема");
        conn.pragma_update(None, "user_version", 99)
            .expect("фикстура-версия");
    }
    let before = fs::read(&path).expect("байты фикстуры");
    let state = Arc::new(HistoryState::open(Ok(data.path().to_path_buf())));
    let unavailable = HistoryUnavailableReason::NewerVersion;

    let page = page_in(Arc::clone(&state), None, None, HistoryStore::page)
        .await
        .expect_err("история недоступна");
    assert_eq!(page.reason, unavailable);
    assert!(!page.message.is_empty());

    let delete = delete_in(Arc::clone(&state), "1".to_owned())
        .await
        .expect_err("история недоступна");
    assert_eq!(
        delete.kind,
        HistoryCommandErrorKind::Unavailable {
            reason: unavailable
        }
    );
    let clear = clear_in(Arc::clone(&state))
        .await
        .expect_err("история недоступна");
    assert_eq!(
        clear.kind,
        HistoryCommandErrorKind::Unavailable {
            reason: unavailable
        }
    );
    let recorder = Recorder::default();
    let show = show_in(
        Arc::clone(&state),
        "1".to_owned(),
        recorder.reveal_on(TargetOs::MacOs),
    )
    .await
    .expect_err("история недоступна");
    assert_eq!(
        show.kind,
        ShowInFolderErrorKind::Unavailable {
            reason: unavailable
        }
    );
    assert!(recorder.calls().is_empty());

    assert_eq!(
        fs::read(&path).expect("байты"),
        before,
        "база новее тронута"
    );
}

#[tokio::test]
async fn an_unresolved_data_dir_is_no_access() {
    let state = Arc::new(HistoryState::open(Err("нет домашнего каталога".to_owned())));
    let page = page_in(state, None, None, HistoryStore::page)
        .await
        .expect_err("история недоступна");
    assert_eq!(page.reason, HistoryUnavailableReason::NoAccess);
    assert!(
        page.message.contains("нет домашнего каталога"),
        "{}",
        page.message
    );
}

// ---------------------------------------------------------------------------
// delete_history_record, clear_history
// ---------------------------------------------------------------------------

#[tokio::test]
async fn deleting_an_unknown_record_is_unknown_record() {
    let scene = scene();
    let id = insert_with_file(&scene.state, scene.downloads.path(), 1);

    for unknown in ["999", "не id", ""] {
        let refused = delete_in(Arc::clone(&scene.state), unknown.to_owned())
            .await
            .expect_err("записи нет");
        assert_eq!(
            refused.kind,
            HistoryCommandErrorKind::UnknownRecord,
            "{unknown:?}"
        );
    }

    delete_in(Arc::clone(&scene.state), id.clone())
        .await
        .expect("удаление");
    let again = delete_in(Arc::clone(&scene.state), id)
        .await
        .expect_err("уже удалена");
    assert_eq!(again.kind, HistoryCommandErrorKind::UnknownRecord);
    assert!(first_page(&scene.state).await.entries.is_empty());
}

/// Отказ диска посреди удаления — `writeFailed` (база только на чтение).
#[cfg(unix)]
#[tokio::test]
async fn a_failing_write_is_write_failed() {
    use std::os::unix::fs::PermissionsExt;

    let scene = scene();
    let id = insert_with_file(&scene.state, scene.downloads.path(), 1);
    let data = scene.data.path();
    fs::set_permissions(data, fs::Permissions::from_mode(0o500)).expect("каталог только на чтение");

    let delete = delete_in(Arc::clone(&scene.state), id).await;
    let clear = clear_in(Arc::clone(&scene.state)).await;
    fs::set_permissions(data, fs::Permissions::from_mode(0o700)).expect("права назад");

    assert_eq!(
        delete.expect_err("журнал не создаётся").kind,
        HistoryCommandErrorKind::WriteFailed
    );
    assert_eq!(
        clear.expect_err("журнал не создаётся").kind,
        HistoryCommandErrorKind::WriteFailed
    );
}

/// К-9: очистка и удаление не трогают файлы (обход ФС до и после).
#[tokio::test]
async fn clear_and_delete_touch_no_files_on_disk() {
    let scene = scene();
    let downloads = scene.downloads.path();
    let nested = downloads.join("вложенная");
    fs::create_dir(&nested).expect("папка");
    let first = insert_with_file(&scene.state, downloads, 1);
    insert_with_file(&scene.state, downloads, 2);
    insert_with_file(&scene.state, &nested, 3);
    fs::write(downloads.join("чужой.txt"), b"not ours").expect("посторонний файл");
    let before = snapshot(downloads);
    assert_eq!(before.len(), 5, "{before:?}");

    delete_in(Arc::clone(&scene.state), first)
        .await
        .expect("удаление");
    assert_eq!(snapshot(downloads), before, "удаление тронуло файлы");

    clear_in(Arc::clone(&scene.state)).await.expect("очистка");
    assert_eq!(snapshot(downloads), before, "очистка тронула файлы");
    assert!(first_page(&scene.state).await.entries.is_empty());

    clear_in(Arc::clone(&scene.state))
        .await
        .expect("повторная очистка пустой истории — успех");
}

// ---------------------------------------------------------------------------
// show_in_folder
// ---------------------------------------------------------------------------

/// Запускатель, который ничего не запускает, а запоминает команды.
#[derive(Clone, Default)]
struct Recorder {
    calls: Arc<Mutex<Vec<RevealCommand>>>,
    failure: Option<LauncherFailure>,
}

impl Recorder {
    fn failing(failure: LauncherFailure) -> Self {
        Self {
            failure: Some(failure),
            ..Self::default()
        }
    }

    fn calls(&self) -> Vec<RevealCommand> {
        self.calls.lock().expect("мьютекс").clone()
    }

    /// Показ для `show_in`: настоящие три случая Ф-8 (`reveal_with`) с этим
    /// запускателем.
    fn reveal_on(
        &self,
        os: TargetOs,
    ) -> impl FnOnce(&Path) -> Result<(), RevealError> + Send + 'static {
        let launcher = self.clone();
        move |file| reveal_with(os, file, &LinuxTools::default(), &launcher)
    }
}

impl Launcher for Recorder {
    fn launch(&self, command: &RevealCommand) -> Result<(), LauncherFailure> {
        self.calls.lock().expect("мьютекс").push(command.clone());
        self.failure.clone().map_or(Ok(()), Err)
    }
}

fn last_argv(command: &RevealCommand) -> Option<OsString> {
    match &command.args {
        CommandArgs::Argv(args) => args.last().cloned(),
        CommandArgs::WindowsCommandLine(_) => None,
    }
}

#[tokio::test]
async fn a_present_file_is_revealed_by_its_record_path() {
    let scene = scene();
    let id = insert_with_file(&scene.state, scene.downloads.path(), 1);
    let recorder = Recorder::default();

    show_in(
        Arc::clone(&scene.state),
        id,
        recorder.reveal_on(TargetOs::MacOs),
    )
    .await
    .expect("файл показан");

    let calls = recorder.calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(
        last_argv(&calls[0]),
        Some(scene.downloads.path().join("Ролик №1.mp4").into_os_string())
    );
}

#[tokio::test]
async fn a_missing_file_opens_its_folder_and_answers_file_missing() {
    let scene = scene();
    let id = insert_with_file(&scene.state, scene.downloads.path(), 1);
    fs::remove_file(scene.downloads.path().join("Ролик №1.mp4")).expect("файл удалён");
    let recorder = Recorder::default();

    let refused = show_in(
        Arc::clone(&scene.state),
        id,
        recorder.reveal_on(TargetOs::MacOs),
    )
    .await
    .expect_err("файла нет");
    assert_eq!(refused.kind, ShowInFolderErrorKind::FileMissing);
    let calls = recorder.calls();
    assert_eq!(calls.len(), 1, "папка открыта: {calls:?}");
    assert_eq!(
        last_argv(&calls[0]),
        Some(scene.downloads.path().as_os_str().to_os_string())
    );
}

#[tokio::test]
async fn a_missing_folder_launches_nothing_and_answers_folder_missing() {
    let scene = scene();
    let gone = scene.downloads.path().join("исчезнет");
    fs::create_dir(&gone).expect("папка");
    let id = insert_with_file(&scene.state, &gone, 1);
    fs::remove_dir_all(&gone).expect("папка удалена");
    let recorder = Recorder::default();

    let refused = show_in(
        Arc::clone(&scene.state),
        id,
        recorder.reveal_on(TargetOs::MacOs),
    )
    .await
    .expect_err("нет ни файла, ни папки");
    assert_eq!(refused.kind, ShowInFolderErrorKind::FolderMissing);
    assert!(recorder.calls().is_empty());
}

#[tokio::test]
async fn a_failing_launcher_answers_launcher_failed_with_details() {
    let scene = scene();
    let id = insert_with_file(&scene.state, scene.downloads.path(), 1);
    let recorder = Recorder::failing(LauncherFailure {
        program: "open".to_owned(),
        cause: LaunchCause::ExitedWithError,
        exit_code: Some(3),
        stderr_tail: Some("хвост stderr".to_owned()),
    });

    let refused = show_in(
        Arc::clone(&scene.state),
        id,
        recorder.reveal_on(TargetOs::MacOs),
    )
    .await
    .expect_err("запускатель отказал");
    assert_eq!(
        refused.kind,
        ShowInFolderErrorKind::LauncherFailed {
            details: LauncherFailureDetails {
                exit_code: Some(3),
                stderr_tail: Some("хвост stderr".to_owned()),
            }
        }
    );
}

/// `Rejected` — `launcherFailed` без деталей (doc `RevealError`). Путь Unix
/// для Windows не абсолютный, и план отклоняется до диска и процессов.
#[tokio::test]
async fn a_rejected_path_answers_launcher_failed_without_details() {
    let scene = scene();
    let id = insert_with_file(&scene.state, scene.downloads.path(), 1);
    let recorder = Recorder::default();

    let refused = show_in(
        Arc::clone(&scene.state),
        id,
        recorder.reveal_on(TargetOs::Windows),
    )
    .await
    .expect_err("путь отклонён");
    assert_eq!(refused.kind, launcher_failed_without_details());
    assert!(
        refused.message.contains("не годится"),
        "{}",
        refused.message
    );
    assert!(recorder.calls().is_empty());
}

#[tokio::test]
async fn show_in_folder_of_an_unknown_record_is_unknown_record() {
    let scene = scene();
    let recorder = Recorder::default();
    for unknown in ["42", "не id"] {
        let refused = show_in(
            Arc::clone(&scene.state),
            unknown.to_owned(),
            recorder.reveal_on(TargetOs::MacOs),
        )
        .await
        .expect_err("записи нет");
        assert_eq!(refused.kind, ShowInFolderErrorKind::UnknownRecord);
    }
    assert!(recorder.calls().is_empty());
}

/// Запись, изменённая вне приложения так, что `file_path()` пути не строит,
/// до показа не доходит: путь берётся только из `file_path()`.
#[tokio::test]
async fn a_tampered_record_never_reaches_the_launcher() {
    let scene = scene();
    let id = insert_with_file(&scene.state, scene.downloads.path(), 1);
    fs::write(scene.downloads.path().join("x.mp4"), b"x").expect("соседний файл");
    let conn = Connection::open(scene.data.path().join(HISTORY_FILE_NAME)).expect("соединение");
    conn.execute("UPDATE history SET file_name = '../x.mp4'", [])
        .expect("правка вне приложения");
    drop(conn);
    let recorder = Recorder::default();

    let refused = show_in(
        Arc::clone(&scene.state),
        id,
        recorder.reveal_on(TargetOs::MacOs),
    )
    .await
    .expect_err("путь не строится");
    assert_eq!(refused.kind, launcher_failed_without_details());
    assert!(recorder.calls().is_empty(), "{:?}", recorder.calls());
}

// ---------------------------------------------------------------------------
// Н-3: блокирующая работа не держит рантайм
// ---------------------------------------------------------------------------

/// Рандеву: работа ждёт сигнала от соседней задачи того же
/// однопоточного рантайма. Если работа идёт на потоке рантайма, сосед не
/// получит хода, и рандеву не состоится.
fn rendezvous() -> (
    tokio::task::JoinHandle<()>,
    impl FnOnce() -> bool + Send + 'static,
) {
    let (tx, rx) = mpsc::channel::<()>();
    let neighbour = tokio::spawn(async move {
        let _ = tx.send(());
    });
    let wait = move || rx.recv_timeout(RENDEZVOUS_CEILING).is_ok();
    (neighbour, wait)
}

#[tokio::test(flavor = "current_thread")]
async fn reading_a_page_does_not_block_the_runtime() {
    let scene = scene();
    insert_with_file(&scene.state, scene.downloads.path(), 1);
    let (neighbour, wait) = rendezvous();

    let page = page_in(
        Arc::clone(&scene.state),
        None,
        None,
        move |store, cursor| {
            assert!(
                wait(),
                "чтение истории выполнялось на потоке рантайма: соседняя задача не получила хода"
            );
            store.page(cursor)
        },
    )
    .await
    .expect("страница");
    assert_eq!(page.entries.len(), 1);
    neighbour.await.expect("сосед завершился");
}

#[tokio::test(flavor = "current_thread")]
async fn showing_in_folder_does_not_block_the_runtime() {
    let scene = scene();
    let id = insert_with_file(&scene.state, scene.downloads.path(), 1);
    let (neighbour, wait) = rendezvous();
    let recorder = Recorder::default();
    let reveal = recorder.reveal_on(TargetOs::MacOs);

    show_in(Arc::clone(&scene.state), id, move |file: &Path| {
        assert!(
            wait(),
            "показ в папке выполнялся на потоке рантайма: соседняя задача не получила хода"
        );
        reveal(file)
    })
    .await
    .expect("файл показан");
    assert_eq!(recorder.calls().len(), 1);
    neighbour.await.expect("сосед завершился");
}

// ---------------------------------------------------------------------------
// Единственность открытия
// ---------------------------------------------------------------------------

/// Поведенческий сторож: файл базы удалён из-под открытого хранилища. Уже
/// открытое соединение продолжает видеть запись, а повторное открытие
/// создало бы на том же месте новый пустой `history.sqlite`. Ни одна из
/// четырёх команд его не создаёт.
#[cfg(unix)]
#[tokio::test]
async fn no_command_opens_the_store_a_second_time() {
    let scene = scene();
    let id = insert_with_file(&scene.state, scene.downloads.path(), 1);
    let db = scene.data.path().join(HISTORY_FILE_NAME);
    fs::remove_file(&db).expect("файл базы удалён из-под хранилища");

    let page = first_page(&scene.state).await;
    assert_eq!(
        page.entries
            .iter()
            .map(|e| e.id.clone())
            .collect::<Vec<_>>(),
        vec![id.clone()],
        "history_page читала не из открытого хранилища"
    );
    assert!(!db.exists(), "history_page открыла базу заново");

    let recorder = Recorder::default();
    let _ = show_in(
        Arc::clone(&scene.state),
        id.clone(),
        recorder.reveal_on(TargetOs::MacOs),
    )
    .await;
    assert_eq!(
        recorder.calls().len(),
        1,
        "show_in_folder читала не из открытого хранилища"
    );
    assert!(!db.exists(), "show_in_folder открыла базу заново");

    let _ = delete_in(Arc::clone(&scene.state), id).await;
    assert!(!db.exists(), "delete_history_record открыла базу заново");

    let _ = clear_in(Arc::clone(&scene.state)).await;
    assert!(!db.exists(), "clear_history открыла базу заново");
}

/// Сторож по исходникам: `HistoryStore::open(` в продакшен-коде зовётся
/// ровно в одном месте — в `HistoryState::open`. Второе открытие в `main.rs`
/// или в оркестрации поведенческий сторож не увидел бы.
///
/// Границы: смотрятся все `.rs` под `src/`, кроме файлов `*_tests.rs`;
/// строки-комментарии (`//`, `///`, `//!`) пропускаются. Вызов через
/// переименованный импорт (`use … HistoryStore as Store`) сторож не видит —
/// поэтому такой импорт он тоже запрещает.
#[test]
fn the_store_is_opened_in_exactly_one_production_place() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut calls = Vec::new();
    let mut aliases = Vec::new();
    let mut stack = vec![src.clone()];
    let mut scanned = 0;
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("каталог обходится") {
            let path = entry.expect("элемент").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            if !name.ends_with(".rs") || name.ends_with("_tests.rs") {
                continue;
            }
            scanned += 1;
            let text = fs::read_to_string(&path).expect("исходник читается");
            for (number, line) in text.lines().enumerate() {
                let code = line.trim_start();
                if code.starts_with("//") {
                    continue;
                }
                let place = format!(
                    "{}:{}",
                    path.strip_prefix(&src).expect("под src").display(),
                    number + 1
                );
                calls.extend(code.matches("HistoryStore::open(").map(|_| place.clone()));
                if code.contains("HistoryStore as ") {
                    aliases.push(place);
                }
            }
        }
    }
    assert!(scanned > 20, "сторож не увидел исходников: {scanned}");
    assert_eq!(
        aliases,
        Vec::<String>::new(),
        "переименованный импорт HistoryStore"
    );
    assert_eq!(
        calls,
        vec!["commands/history.rs".to_owned() + ":" + &open_call_line().to_string()],
        "HistoryStore::open зовётся не ровно в HistoryState::open"
    );
}

/// Номер строки единственного вызова — из самого `history.rs`, чтобы сторож
/// сверял место, а не только количество.
fn open_call_line() -> usize {
    let text = include_str!("history.rs");
    let start = text.find("impl HistoryState").expect("impl HistoryState");
    let offset = start
        + text[start..]
            .find("HistoryStore::open(")
            .expect("вызов внутри impl HistoryState");
    text[..offset].lines().count()
}
