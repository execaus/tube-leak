//! Тесты истории загрузок (TL-85, Ф-18 в части истории).
//!
//! Все — во временных каталогах, без сети и без процессов. Проверяется
//! наблюдаемое: байты файла, содержимое каталога, ответ независимого
//! соединения SQLite, а не внутреннее состояние хранилища.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OpenFlags};
use tempfile::{tempdir, TempDir};

use super::*;
use crate::types::{
    HistoryCursor, HistoryFileStatus, HistoryNotice, HistoryUnavailableReason, HistoryWriteFailure,
    QualityKind, SelectedQuality, HISTORY_PAGE_SIZE,
};

/// Каталог данных и отдельная «папка назначения» для файлов записей.
struct Dirs {
    data: TempDir,
    downloads: TempDir,
}

fn dirs() -> Dirs {
    Dirs {
        data: tempdir().expect("временный каталог данных"),
        downloads: tempdir().expect("временная папка назначения"),
    }
}

fn open(data: &Path) -> HistoryStore {
    HistoryStore::open_isolated(data).expect("история открывается")
}

fn db_path(data: &Path) -> PathBuf {
    data.join(HISTORY_FILE_NAME)
}

fn record(folder: &Path, n: u64, finished: u64) -> NewHistoryRecord {
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
        finished_at_unix_secs: finished,
    }
}

/// Версия схемы глазами независимого соединения только на чтение.
fn read_user_version(path: &Path) -> i64 {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("база открывается на чтение");
    conn.pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("версия читается")
}

fn count_schema_objects(path: &Path, name: &str) -> i64 {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("база открывается на чтение");
    conn.query_row(
        "SELECT count(*) FROM sqlite_schema WHERE name = ?1",
        [name],
        |row| row.get(0),
    )
    .expect("схема читается")
}

/// Настоящая база SQLite с произвольной `user_version`.
fn make_sqlite_with_version(path: &Path, version: i64) {
    let conn = Connection::open(path).expect("фикстура-база создаётся");
    conn.execute_batch("CREATE TABLE t (x INTEGER); INSERT INTO t VALUES (1);")
        .expect("фикстура-схема");
    conn.pragma_update(None, "user_version", version)
        .expect("фикстура-версия");
}

fn ids(page: &HistoryRecordsPage) -> Vec<i64> {
    page.records.iter().map(|record| record.id.get()).collect()
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

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("каталог обходится")
        .map(|entry| {
            entry
                .expect("элемент каталога")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

/// Отложенные копии самой базы (без спутников).
fn broken_copies(data: &Path) -> Vec<PathBuf> {
    let prefix = format!("{HISTORY_FILE_NAME}{BROKEN_MARKER}");
    let mut copies: Vec<PathBuf> = names_in(data)
        .into_iter()
        .filter(|name| name.starts_with(&prefix))
        .filter(|name| {
            !SIDE_FILE_SUFFIXES
                .iter()
                .any(|suffix| name.ends_with(suffix))
        })
        .map(|name| data.join(name))
        .collect();
    copies.sort();
    copies
}

/// Постраничный обход до конца: id в порядке выдачи.
fn walk(store: &HistoryStore, size: usize) -> Vec<i64> {
    let mut seen = Vec::new();
    let mut cursor: Option<HistoryCursor> = None;
    for _ in 0..100_000 {
        let page = store
            .page_sized(cursor.as_ref(), size)
            .expect("страница читается");
        assert!(page.records.len() <= size, "страница больше своего размера");
        seen.extend(ids(&page));
        match page.next_cursor {
            Some(next) => {
                assert!(!page.records.is_empty(), "курсор у пустой страницы");
                cursor = Some(next);
            }
            None => return seen,
        }
    }
    panic!("постраничный обход не закончился");
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("права меняются");
}

/// Детерминированный генератор: без новой зависимости ради тестов.
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

// ───────────────────────────── миграции и открытие ─────────────────────────────

/// Ф-1 (а): каталога и базы нет — оба появляются, схема версии 1.
#[test]
fn a_fresh_directory_gets_schema_version_1() {
    let dirs = dirs();
    let data = dirs.data.path().join("ещё-не-создан");

    let store = open(&data);
    drop(store);

    assert_eq!(SCHEMA_VERSION, 1);
    assert_eq!(read_user_version(&db_path(&data)), 1);
    assert_eq!(count_schema_objects(&db_path(&data), "history"), 1);
    assert_eq!(
        count_schema_objects(&db_path(&data), "history_newest_first"),
        1
    );
}

/// Повторное открытие идемпотентно: записи на месте, первой идёт новая,
/// ложной пометки о пересоздании нет, а сам файл открытием и чтением не
/// переписан (К-2, часть «новое открытие того же файла отдаёт её первой»).
#[test]
fn reopening_is_idempotent_and_keeps_records_newest_first() {
    let dirs = dirs();
    let data = dirs.data.path();
    let folder = dirs.downloads.path();

    let store = open(data);
    let first = store.insert(&record(folder, 1, 10)).expect("вставка");
    let second = store.insert(&record(folder, 2, 20)).expect("вставка");
    let third = store.insert(&record(folder, 3, 15)).expect("вставка");
    drop(store);
    let bytes_before = fs::read(db_path(data)).expect("база читается");

    let store = open(data);
    let page = store.page(None).expect("страница");
    drop(store);

    assert_eq!(ids(&page), vec![second.get(), third.get(), first.get()]);
    assert!(
        page.notices.is_empty(),
        "пометки без события: {:?}",
        page.notices
    );
    assert_eq!(read_user_version(&db_path(data)), 1);
    assert_eq!(
        fs::read(db_path(data)).expect("база читается"),
        bytes_before,
        "повторное открытие переписало базу"
    );
}

/// Заголовок разбирается так же, как версию видит сам SQLite, — иначе
/// проверка «до SQLite» проверяла бы не то.
#[test]
fn the_header_version_matches_what_sqlite_reports() {
    let dirs = dirs();
    let data = dirs.data.path();

    for version in [0, 1, 99, i64::from(i32::MAX), -5] {
        let path = data.join(format!("v{version}.sqlite"));
        make_sqlite_with_version(&path, version);
        assert_eq!(read_user_version(&path), version);
        assert_eq!(
            inspect_header(&path).expect("заголовок читается"),
            Header::Sqlite {
                user_version: version,
                format: FileFormat::JOURNAL,
            }
        );
    }

    assert_eq!(
        inspect_header(&data.join("нет-такого")).expect("отсутствие — не ошибка"),
        Header::Absent
    );
    let empty = data.join("пустой");
    fs::write(&empty, b"").expect("фикстура");
    assert_eq!(inspect_header(&empty).expect("читается"), Header::Absent);
    let short = data.join("короткий");
    fs::write(&short, SQLITE_MAGIC).expect("фикстура");
    assert_eq!(inspect_header(&short).expect("читается"), Header::Foreign);
    let text = data.join("текст");
    fs::write(&text, "не база ".repeat(40)).expect("фикстура");
    assert_eq!(inspect_header(&text).expect("читается"), Header::Foreign);
}

/// Ф-1 (в), К-1: база новее приложения — отказ `NewerVersion`, и каталог
/// данных после попытки байт в байт тот же: ни файл, ни спутники, ни
/// отложенные копии не появились.
///
/// Мутация, которую тест обязан ловить: «открыть и переписать» — принять
/// чужую версию и довести её до своей.
#[test]
fn a_newer_base_is_refused_and_left_byte_for_byte() {
    for version in [2, 99, -1] {
        let dirs = dirs();
        let data = dirs.data.path();
        make_sqlite_with_version(&db_path(data), version);
        let before = snapshot(data);

        let result = HistoryStore::open_isolated(data);

        match &result {
            Err(
                err @ HistoryOpenError::NewerVersion {
                    found,
                    supported,
                    cause,
                    ..
                },
            ) => {
                assert_eq!(*found, version);
                assert_eq!(*supported, SCHEMA_VERSION);
                assert_eq!(
                    *cause,
                    NewerCause::SchemaVersion,
                    "отказ по версии назван другой причиной"
                );
                assert_eq!(err.reason(), HistoryUnavailableReason::NewerVersion);
            }
            other => panic!("база версии {version} открыта как {other:?}"),
        }
        assert_eq!(
            snapshot(data),
            before,
            "каталог данных изменился после отказа по версии {version}"
        );
    }
}

/// Ф-1 (г), К-8: текстовый мусор под именем базы отложен (не удалён, байты
/// те же), новая база версии 1 создана, пометка `baseRecreated` выдана
/// один раз.
///
/// Мутация, которую тест обязан ловить: удалить мусор вместо того, чтобы
/// отложить.
#[test]
fn text_garbage_is_set_aside_and_a_new_base_is_created() {
    let garbages: [Vec<u8>; 2] = [
        "это не база, а заметки пользователя\n"
            .repeat(20)
            .into_bytes(),
        b"junk\n".to_vec(),
    ];
    for garbage in garbages {
        let dirs = dirs();
        let data = dirs.data.path();
        fs::write(db_path(data), &garbage).expect("фикстура");

        let store = open(data);

        let copies = broken_copies(data);
        assert_eq!(copies.len(), 1, "отложенных копий: {copies:?}");
        assert_eq!(
            fs::read(&copies[0]).expect("отложенная копия читается"),
            garbage,
            "отложенная копия — не исходный мусор"
        );
        assert_eq!(read_user_version(&db_path(data)), 1);

        let first = store.page(None).expect("страница");
        assert_eq!(first.notices, vec![HistoryNotice::BaseRecreated]);
        assert!(first.records.is_empty());
        let second = store.page(None).expect("страница");
        assert!(second.notices.is_empty(), "пометка выдана дважды");
    }
}

/// Файл, отвергнутый по заголовку (не SQLite), SQLite не открывал: он и
/// его `-journal` уезжают байт в байт под новым именем. Журнал, оставшийся
/// рядом с новой пустой базой, SQLite удалил бы.
///
/// Про SQLite-базу с порчей страниц тест ничего не утверждает: там горячий
/// журнал применяет сам SQLite ещё до откладывания (шапка модуля).
#[test]
fn a_journal_beside_a_base_rejected_by_header_travels_byte_for_byte() {
    let dirs = dirs();
    let data = dirs.data.path();
    let journal = with_suffix(&db_path(data), "-journal");
    let garbage = "не база ".repeat(40);
    fs::write(db_path(data), &garbage).expect("фикстура");
    fs::write(&journal, b"journal bytes").expect("фикстура");

    let _store = open(data);

    let copies = broken_copies(data);
    assert_eq!(copies.len(), 1, "отложенных копий: {copies:?}");
    assert_eq!(
        fs::read(&copies[0]).expect("копия читается"),
        garbage.as_bytes()
    );
    assert_eq!(
        fs::read(with_suffix(&copies[0], "-journal")).expect("журнал уехал с базой"),
        b"journal bytes"
    );
    assert!(
        !journal.exists(),
        "старый журнал остался рядом с новой базой"
    );
}

/// Порча внутри настоящей базы SQLite (заголовок цел, страницы — нет)
/// ловится `integrity_check` и обрабатывается так же, как мусор.
#[test]
fn a_base_with_corrupted_pages_is_set_aside() {
    let dirs = dirs();
    let data = dirs.data.path();
    let folder = dirs.downloads.path();

    let store = open(data);
    let many: Vec<NewHistoryRecord> = (0..400)
        .map(|n| NewHistoryRecord {
            title: "длинное название ".repeat(12),
            ..record(folder, n, n)
        })
        .collect();
    store.insert_all(&many);
    drop(store);

    let mut bytes = fs::read(db_path(data)).expect("база читается");
    assert!(bytes.len() > 4096 * 4, "база слишком мала для фикстуры");
    for byte in &mut bytes[4096..] {
        *byte = 0xA5;
    }
    fs::write(db_path(data), &bytes).expect("порча записана");
    assert!(matches!(
        inspect_header(&db_path(data)),
        Ok(Header::Sqlite {
            user_version: 1,
            format: FileFormat::JOURNAL,
        })
    ));

    let store = open(data);

    let copies = broken_copies(data);
    assert_eq!(copies.len(), 1, "отложенных копий: {copies:?}");
    assert_eq!(fs::read(&copies[0]).expect("копия читается"), bytes);
    let page = store.page(None).expect("страница новой базы");
    assert!(page.records.is_empty());
    assert_eq!(page.notices, vec![HistoryNotice::BaseRecreated]);
}

/// Строки ответа `PRAGMA <pragma>` у независимого соединения только на чтение.
fn pragma_rows(path: &Path, pragma: &str) -> Vec<String> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("база открывается на чтение");
    let mut statement = conn
        .prepare(&format!("PRAGMA {pragma}"))
        .expect("прагма готовится");
    let rows = statement
        .query_map([], |row| row.get(0))
        .expect("прагма выполняется")
        .collect::<Result<Vec<String>, _>>()
        .expect("строки прагмы");
    rows
}

fn index_sql(path: &Path) -> String {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("база открывается на чтение");
    conn.query_row(
        "SELECT sql FROM sqlite_schema WHERE name = 'history_newest_first'",
        [],
        |row| row.get(0),
    )
    .expect("текст индекса читается")
}

/// Переписывает текст объявления индекса в `sqlite_schema`, не трогая его
/// дерево. Новое соединение разбирает схему заново и видит новый текст.
fn rewrite_index_sql(path: &Path, sql: &str) {
    let conn = Connection::open(path).expect("база открывается");
    conn.execute_batch("PRAGMA writable_schema = ON")
        .expect("схема открыта на запись");
    let changed = conn
        .execute(
            "UPDATE sqlite_schema SET sql = ?1 WHERE name = 'history_newest_first'",
            [sql],
        )
        .expect("текст индекса переписан");
    assert_eq!(changed, 1, "объявление индекса не найдено");
}

/// Наша база, у которой индекс `history_newest_first` разошёлся с таблицей,
/// а каждая страница цела. Возвращает число строк таблицы.
///
/// Способ — штатные операции SQLite, без правки байтов. На время одного
/// `UPDATE` схема утверждает, что индекс построен по `size_bytes`: SQLite
/// обновляет только индексы, в которых есть изменённый столбец, и новое
/// время в дерево индекса не попадает. Затем текст объявления возвращается
/// дословно. Итог: схема та же, что пишет [`SCHEMA_V1`], дерево индекса
/// упорядочено и структурно цело, но его ключи — старые времена.
fn make_index_out_of_sync_with_table(data: &Path, folder: &Path) -> usize {
    let rows = 50;
    let store = open(data);
    let records: Vec<NewHistoryRecord> = (1..=rows).map(|n| record(folder, n, n)).collect();
    store.insert_all(&records);
    drop(store);

    let path = db_path(data);
    let original = index_sql(&path);
    rewrite_index_sql(
        &path,
        "CREATE INDEX history_newest_first ON history (size_bytes DESC, id DESC)",
    );
    let conn = Connection::open(&path).expect("база открывается");
    let changed = conn
        .execute(
            "UPDATE history SET finished_at_unix_secs = finished_at_unix_secs + 1000",
            [],
        )
        .expect("время строк сдвинуто");
    assert_eq!(changed, 50);
    drop(conn);
    rewrite_index_sql(&path, &original);
    assert_eq!(
        index_sql(&path),
        original,
        "объявление индекса не вернулось"
    );
    usize::try_from(rows).expect("число строк")
}

/// Решение ревью TL-85 (issue #92, п. 4; TL-99): проверка открытия —
/// `integrity_check`, а не `quick_check`. Индекс, разошедшийся с таблицей
/// при целых страницах, — порча: база отложена, заведена новая, пометка
/// `baseRecreated`.
///
/// Фикстура сама доказывает, что различает прагмы: `quick_check` отвечает
/// `ok`, `integrity_check` называет индекс. Без этого тест мог бы краснеть
/// на мутации по другой причине или не краснеть вовсе.
///
/// Мутация, которую тест обязан ловить: `integrity_check` → `quick_check` в
/// [`integrity_check`].
#[test]
fn a_base_whose_index_disagrees_with_the_table_is_set_aside() {
    let dirs = dirs();
    let data = dirs.data.path();
    let rows = make_index_out_of_sync_with_table(data, dirs.downloads.path());

    let path = db_path(data);
    assert_eq!(
        pragma_rows(&path, "quick_check"),
        vec!["ok".to_owned()],
        "фикстура не та: quick_check видит порчу, тест не отличит прагмы"
    );
    let verdicts = pragma_rows(&path, "integrity_check");
    assert!(
        verdicts != ["ok"] && verdicts.iter().all(|v| v.contains("history_newest_first")),
        "фикстура не та: integrity_check должен назвать только индекс: {verdicts:?}"
    );
    assert_eq!(
        inspect_header(&path).expect("заголовок читается"),
        Header::Sqlite {
            user_version: 1,
            format: FileFormat::JOURNAL,
        }
    );
    let bytes = fs::read(&path).expect("база читается");
    assert_eq!(
        names_in(data),
        vec![HISTORY_FILE_NAME.to_owned()],
        "спутники у фикстуры"
    );

    let store = open(data);

    let copies = broken_copies(data);
    assert_eq!(copies.len(), 1, "отложенных копий: {copies:?}");
    assert_eq!(
        fs::read(&copies[0]).expect("копия читается"),
        bytes,
        "отложенная копия — не та база"
    );
    let copied_rows: i64 =
        Connection::open_with_flags(&copies[0], OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("копия открывается на чтение")
            .query_row("SELECT count(*) FROM history", [], |row| row.get(0))
            .expect("строки копии считаются");
    assert_eq!(
        usize::try_from(copied_rows).ok(),
        Some(rows),
        "в отложенной копии не те записи"
    );
    let page = store.page(None).expect("страница новой базы");
    assert!(page.records.is_empty(), "новая база унаследовала записи");
    assert_eq!(page.notices, vec![HistoryNotice::BaseRecreated]);
}

/// Пара «главный файл — `-wal`» с непримененными кадрами.
///
/// Главный файл в режиме WAL хранит в заголовке `header_version`, а кадры
/// в `-wal` поднимают её до `wal_version` и дописывают строку. Байты
/// снимаются, пока соединение открыто: закрытие перенесло бы кадры в файл
/// и удалило бы `-wal`.
fn wal_pair(scratch: &Path, header_version: i64, wal_version: i64) -> (Vec<u8>, Vec<u8>) {
    let path = scratch.join("wal-source.sqlite");
    let conn = Connection::open(&path).expect("фикстура-база создаётся");
    let mode: String = conn
        .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
        .expect("режим WAL");
    assert_eq!(mode, "wal");
    conn.execute_batch("CREATE TABLE foreign_t (x INTEGER); INSERT INTO foreign_t VALUES (1);")
        .expect("фикстура-схема");
    conn.pragma_update(None, "user_version", header_version)
        .expect("версия заголовка");
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
        .expect("перенос в файл");
    conn.query_row("PRAGMA wal_autocheckpoint = 0", [], |_| Ok(()))
        .expect("автоперенос выключен");
    conn.execute_batch("INSERT INTO foreign_t VALUES (2);")
        .expect("кадр в WAL");
    conn.pragma_update(None, "user_version", wal_version)
        .expect("версия в WAL");

    let main = fs::read(&path).expect("главный файл читается");
    let wal = fs::read(with_suffix(&path, WAL_SUFFIX)).expect("-wal читается");
    assert!(!wal.is_empty(), "кадров в -wal нет");
    drop(conn);
    (main, wal)
}

/// Отказ из-за WAL: `NewerVersion` с причиной-журналом, каталог данных
/// байт в байт тот же, включая `-wal`.
///
/// Текст для лога называет причину точно (TL-99): рядом журнал `-wal`,
/// заголовок в режиме WAL или оба. Экран при этом говорит «база новее» —
/// причина в контракте одна.
fn assert_refused_as_wal_and_untouched(data: &Path, expected_found: i64, expected: WalSign) {
    let before = snapshot(data);

    let result = HistoryStore::open_isolated(data);

    match &result {
        Err(
            err @ HistoryOpenError::NewerVersion {
                found,
                supported,
                cause,
                ..
            },
        ) => {
            assert_eq!(*found, expected_found);
            assert_eq!(*supported, SCHEMA_VERSION);
            assert_eq!(*cause, NewerCause::Wal(expected));
            assert_eq!(err.reason(), HistoryUnavailableReason::NewerVersion);
            let message = err.to_string();
            let names_file = message.contains("history.sqlite-wal");
            let names_header = message.contains("заголовок файла объявляет режим");
            let expected_names = match expected {
                WalSign::File => (true, false),
                WalSign::Header => (false, true),
                WalSign::FileAndHeader => (true, true),
            };
            assert_eq!(
                (names_file, names_header),
                expected_names,
                "текст для лога не называет причину WAL точно: {message}"
            );
            assert!(
                !message.contains("чужой версии схемы"),
                "отказ по WAL назван отказом по версии: {message}"
            );
        }
        other => panic!("база с признаком WAL открыта как {other:?}"),
    }
    drop(result);
    assert_eq!(
        snapshot(data),
        before,
        "каталог данных изменился после отказа по WAL"
    );
}

/// Сценарий ревью (а): заголовок объявляет версию 1, а в `-wal` лежит
/// версия 2. SQLite применил бы кадры при первом чтении, и отказ по
/// `PRAGMA user_version` пришёл бы после изменения файла.
#[test]
fn a_version_1_header_with_a_version_2_wal_is_refused_untouched() {
    let dirs = dirs();
    let data = dirs.data.path();
    let (main, wal) = wal_pair(dirs.downloads.path(), 1, 2);
    fs::write(db_path(data), &main).expect("фикстура");
    fs::write(with_suffix(&db_path(data), WAL_SUFFIX), &wal).expect("фикстура");
    assert_eq!(
        inspect_header(&db_path(data)).expect("заголовок читается"),
        Header::Sqlite {
            user_version: 1,
            format: FileFormat { write: 2, read: 2 },
        },
        "фикстура не та: в заголовке должна быть версия 1 в режиме WAL"
    );

    assert_refused_as_wal_and_untouched(data, 1, WalSign::FileAndHeader);
}

/// Сценарий ревью (б): наша база в режиме DELETE с записями и чужой `-wal`
/// рядом. Заголовок чист, ловит только проверка существования `-wal`.
/// Без неё `open` успешен, `page` отдаёт `no such table: history`, а после
/// закрытия история уничтожена без `.broken`-копии.
///
/// Мутация, которую тест обязан ловить: убрать проверку `-wal`.
#[test]
fn a_delete_mode_base_with_a_foreign_wal_is_refused_untouched() {
    let dirs = dirs();
    let data = dirs.data.path();
    let folder = dirs.downloads.path();
    let store = open(data);
    for n in 1..=3 {
        store.insert(&record(folder, n, 100 + n)).expect("вставка");
    }
    drop(store);
    let scratch = tempdir().expect("временный каталог");
    let (_, foreign_wal) = wal_pair(scratch.path(), 1, 7);
    fs::write(with_suffix(&db_path(data), WAL_SUFFIX), &foreign_wal).expect("фикстура");
    assert_eq!(
        inspect_header(&db_path(data)).expect("заголовок читается"),
        Header::Sqlite {
            user_version: 1,
            format: FileFormat::JOURNAL,
        },
        "фикстура не та: заголовок должен быть в режиме DELETE"
    );

    assert_refused_as_wal_and_untouched(data, 1, WalSign::File);
}

/// Заголовок объявляет WAL, а `-wal` рядом нет (журнал был перенесён и
/// удалён). Ловит только проверка байтов 18–19: иначе SQLite открыл бы
/// базу в режиме WAL.
///
/// Мутация, которую тест обязан ловить: убрать проверку байтов заголовка.
#[test]
fn a_wal_mode_header_without_a_wal_file_is_refused_untouched() {
    let dirs = dirs();
    let data = dirs.data.path();
    let (main, _) = wal_pair(dirs.downloads.path(), 1, 1);
    fs::write(db_path(data), &main).expect("фикстура");

    assert_refused_as_wal_and_untouched(data, 1, WalSign::Header);
}

/// TL-99: байт 18 или 19 заголовка больше 2 — формат записи или чтения
/// новее того, что знает эта сборка. Отказ `NewerVersion` до SQLite, каталог
/// данных байт в байт тот же.
///
/// Без проверки SQLite открывает базу с байтом 18 > 2 только на чтение,
/// `open` проходит, а отказ случается на первой записи (`noAccess` и пометка).
/// Базу с байтом 19 > 2 SQLite не читает вовсе (`SQLITE_NOTADB`), и она
/// отложилась бы как порча с пометкой `baseRecreated`.
///
/// Мутация, которую тест обязан ловить: убрать проверку формата в заголовке.
#[test]
fn a_newer_file_format_in_the_header_is_refused_untouched() {
    let mut failures = Vec::new();
    for (offset, value) in [(18_usize, 3_u8), (18, 255), (19, 3), (19, 255)] {
        let dirs = dirs();
        let data = dirs.data.path();
        let store = open(data);
        store
            .insert(&record(dirs.downloads.path(), 1, 1))
            .expect("вставка");
        drop(store);
        let mut bytes = fs::read(db_path(data)).expect("база читается");
        assert_eq!(bytes[18..20], [1, 1], "фикстура не в режиме DELETE");
        bytes[offset] = value;
        fs::write(db_path(data), &bytes).expect("фикстура");
        let (write, read) = (bytes[18], bytes[19]);
        let before = snapshot(data);

        let result = HistoryStore::open_isolated(data);

        let refused_as_newer_format = match &result {
            Err(
                err @ HistoryOpenError::NewerVersion {
                    found: 1,
                    supported,
                    cause,
                    ..
                },
            ) => {
                *supported == SCHEMA_VERSION
                    && *cause == NewerCause::FileFormat { write, read }
                    && err.reason() == HistoryUnavailableReason::NewerVersion
                    && err.to_string().contains("формат файла SQLite новее")
            }
            _ => false,
        };
        if !refused_as_newer_format {
            failures.push(format!("байт {offset} = {value}: открыто как {result:?}"));
        }
        drop(result);
        if snapshot(data) != before {
            failures.push(format!("байт {offset} = {value}: каталог данных изменился"));
        }
    }
    assert!(
        failures.is_empty(),
        "новый формат не отвергнут до SQLite:\n{}",
        failures.join("\n")
    );
}

/// Чужой `-wal` рядом с пустым или отсутствующим файлом SQLite удалил бы.
#[test]
fn a_wal_beside_a_missing_base_is_refused_untouched() {
    let dirs = dirs();
    let data = dirs.data.path();
    let (_, wal) = wal_pair(dirs.downloads.path(), 1, 2);
    fs::write(with_suffix(&db_path(data), WAL_SUFFIX), &wal).expect("фикстура");

    assert_refused_as_wal_and_untouched(data, 0, WalSign::File);
}

/// Вторая порча с той же меткой не затирает первую отложенную копию.
#[test]
fn set_aside_never_overwrites_an_earlier_broken_copy() {
    let dirs = dirs();
    let data = dirs.data.path();
    let path = db_path(data);

    fs::write(&path, b"first").expect("фикстура");
    let first = set_aside(&path, "42").expect("первая откладывается");
    fs::write(&path, b"second").expect("фикстура");
    let second = set_aside(&path, "42").expect("вторая откладывается");

    assert_ne!(first, second);
    assert_eq!(fs::read(&first).expect("первая цела"), b"first");
    assert_eq!(fs::read(&second).expect("вторая цела"), b"second");
    assert!(!path.exists());
}

/// Ф-1 (д): каталог данных только на чтение — `NoAccess`, и в нём ничего
/// не создано, в том числе недописанной базы.
#[cfg(unix)]
#[test]
fn a_read_only_directory_is_no_access_and_nothing_is_created() {
    let dirs = dirs();
    let data = dirs.data.path();
    set_mode(data, 0o555);

    let result = HistoryStore::open_isolated(data);
    let names = names_in(data);
    set_mode(data, 0o755);

    match &result {
        Err(err @ HistoryOpenError::NoAccess { .. }) => {
            assert_eq!(err.reason(), HistoryUnavailableReason::NoAccess);
        }
        other => panic!("каталог только на чтение открыт как {other:?}"),
    }
    assert_eq!(names, Vec::<String>::new(), "в каталоге что-то создано");
}

/// Файл базы без права записи SQLite открыл бы молча на чтение; хранилище
/// называет это сразу и файл не трогает.
#[cfg(unix)]
#[test]
fn a_read_only_base_file_is_no_access_and_left_alone() {
    let dirs = dirs();
    let data = dirs.data.path();
    let store = open(data);
    store
        .insert(&record(dirs.downloads.path(), 1, 1))
        .expect("вставка");
    drop(store);
    let before = fs::read(db_path(data)).expect("база читается");
    set_mode(&db_path(data), 0o444);

    let result = HistoryStore::open_isolated(data);
    set_mode(&db_path(data), 0o644);

    assert!(
        matches!(result, Err(HistoryOpenError::NoAccess { .. })),
        "база только на чтение открыта как {result:?}"
    );
    assert_eq!(fs::read(db_path(data)).expect("база читается"), before);
}

const BROKEN_V2: &str = "
CREATE TABLE half_done (x INTEGER);
INSERT INTO no_such_table VALUES (1);
";

/// Ф-1 (б), Н-4: миграция, падающая посередине, откатывается целиком —
/// таблицы, созданной её первым оператором, нет, версия прежняя, записи
/// целы, и та же база открывается сборкой, знающей только версию 1.
#[test]
fn a_migration_failing_midway_rolls_back_to_the_previous_version() {
    let dirs = dirs();
    let data = dirs.data.path();
    let store = HistoryStore::open_with(data, &[SCHEMA_V1]).expect("версия 1");
    let id = store
        .insert(&record(dirs.downloads.path(), 1, 1))
        .expect("вставка");
    drop(store);

    let result = HistoryStore::open_with(data, &[SCHEMA_V1, BROKEN_V2]);

    match &result {
        Err(err @ HistoryOpenError::MigrationFailed { from: 1, to: 2, .. }) => {
            assert_eq!(err.reason(), HistoryUnavailableReason::MigrationFailed);
        }
        other => panic!("падающая миграция дала {other:?}"),
    }
    assert_eq!(read_user_version(&db_path(data)), 1);
    assert_eq!(
        count_schema_objects(&db_path(data), "half_done"),
        0,
        "первый оператор упавшей миграции не откатился"
    );
    let store = open(data);
    assert_eq!(ids(&store.page(None).expect("страница")), vec![id.get()]);
}

/// Каждая миграция — своя транзакция: с нуля до отказа второй база
/// остаётся на версии 1, а не на 0 и не на промежуточной.
#[test]
fn a_failing_second_step_from_scratch_keeps_the_first_step() {
    let dirs = dirs();
    let data = dirs.data.path();

    let result = HistoryStore::open_with(data, &[SCHEMA_V1, BROKEN_V2]);

    assert!(
        matches!(
            result,
            Err(HistoryOpenError::MigrationFailed { from: 1, to: 2, .. })
        ),
        "{result:?}"
    );
    assert_eq!(read_user_version(&db_path(data)), 1);
    assert_eq!(count_schema_objects(&db_path(data), "half_done"), 0);
}

/// Ф-1 (б): база старой версии доводится до текущей, записи переживают.
#[test]
fn a_migration_from_an_older_version_keeps_records() {
    let dirs = dirs();
    let data = dirs.data.path();
    let store = HistoryStore::open_with(data, &[SCHEMA_V1]).expect("версия 1");
    let id = store
        .insert(&record(dirs.downloads.path(), 7, 70))
        .expect("вставка");
    drop(store);

    let store = HistoryStore::open_with(
        data,
        &[SCHEMA_V1, "ALTER TABLE history ADD COLUMN note TEXT;"],
    )
    .expect("миграция на версию 2");

    let page = store.page(None).expect("страница");
    drop(store);
    assert_eq!(read_user_version(&db_path(data)), 2);
    assert_eq!(ids(&page), vec![id.get()]);
    assert_eq!(page.records[0].title, "Ролик №7");
}

// ───────────────────────────── запись и чтение ─────────────────────────────

/// Все поля переживают круг «вставили — прочитали», включая каждый вид
/// качества: сопоставление вида со строкой базы и `CHECK` схемы — одно
/// множество.
#[test]
fn records_survive_a_round_trip_with_every_quality_kind() {
    let dirs = dirs();
    let store = open(dirs.data.path());
    let folder = dirs.downloads.path();
    let qualities = [
        (QualityKind::Standard, Some(2160)),
        (QualityKind::MaxAvailable, Some(480)),
        (QualityKind::AudioOnly, None),
    ];

    for (n, (kind, height_px)) in (1..).zip(qualities) {
        let new = NewHistoryRecord {
            url: "https://youtu.be/dQw4w9WgXcQ?t=42&si=x".to_owned(),
            title: "«Кавычки» — тире, emoji 🎬 и \u{202e}rtl".to_owned(),
            quality: SelectedQuality { kind, height_px },
            size_bytes: 1 << 40,
            ..record(folder, n, 1_800_000_000 + n)
        };
        let id = store.insert(&new).expect("вставка");

        let got = store
            .get(&id.to_string())
            .expect("чтение")
            .expect("запись есть");
        assert_eq!(kind_from_column(kind_to_column(kind)), Some(kind));
        assert_eq!(got.id, id);
        assert_eq!(got.video_id, new.video_id);
        assert_eq!(got.url, new.url);
        assert_eq!(got.title, new.title);
        assert_eq!(got.quality, new.quality);
        assert_eq!(got.file_name, new.file_name);
        assert_eq!(got.folder, new.folder);
        assert_eq!(got.size_bytes, new.size_bytes);
        assert_eq!(got.finished_at_unix_secs, new.finished_at_unix_secs);
    }

    let bogus = store.with_connection(|conn| {
        conn.execute(
            INSERT_SQL,
            params!["v", "u", "t", "bogus", 1, "f.mp4", "/tmp", 1, 1],
        )
    });
    assert!(bogus.is_err(), "схема приняла неизвестный вид качества");
}

/// Равные времена — по id по убыванию, и сравнение числовое: `12` раньше
/// `9`, хотя как строка меньше. Курсор на стыке несёт id строкой ядра.
#[test]
fn equal_finish_times_are_ordered_by_numeric_id_descending() {
    let dirs = dirs();
    let store = open(dirs.data.path());
    for n in 1..=12 {
        store
            .insert(&record(dirs.downloads.path(), n, 500))
            .expect("вставка");
    }

    let page = store.page(None).expect("страница");
    assert_eq!(ids(&page), (1..=12).rev().collect::<Vec<i64>>());
    assert_eq!(page.records[0].id.to_string(), "12");

    let first = store.page_sized(None, 5).expect("страница");
    assert_eq!(
        first.next_cursor,
        Some(HistoryCursor {
            finished_at_unix_secs: 500,
            id: "8".to_owned()
        })
    );
    assert_eq!(walk(&store, 5), (1..=12).rev().collect::<Vec<i64>>());
}

/// Критерий issue #92: запись, появившаяся между двумя «Показать ещё»,
/// не вызывает на стыке ни пропуска, ни повтора.
///
/// Мутация, которую тест обязан ловить: нестрогое сравнение с курсором
/// (`<=`) или смещение вместо курсора.
#[test]
fn a_page_boundary_neither_skips_nor_repeats_when_records_arrive_between_pages() {
    let dirs = dirs();
    let store = open(dirs.data.path());
    let folder = dirs.downloads.path();
    for n in 1..=10 {
        store.insert(&record(folder, n, 100)).expect("вставка");
    }

    let first = store.page_sized(None, 4).expect("первая страница");
    assert_eq!(ids(&first), vec![10, 9, 8, 7]);

    // Между запросами завершились ещё две загрузки: в ту же секунду и позже.
    store.insert(&record(folder, 11, 100)).expect("вставка");
    store.insert(&record(folder, 12, 200)).expect("вставка");

    let second = store
        .page_sized(first.next_cursor.as_ref(), 4)
        .expect("вторая страница");
    assert_eq!(ids(&second), vec![6, 5, 4, 3]);
    let third = store
        .page_sized(second.next_cursor.as_ref(), 4)
        .expect("третья страница");
    assert_eq!(ids(&third), vec![2, 1]);
    assert_eq!(third.next_cursor, None);
}

/// Постраничный обход совпадает с полной сортировкой в памяти — на тысячах
/// записей со случайными и массово равными временами, после случайных
/// удалений, при разных размерах страницы.
#[test]
fn a_paged_walk_matches_an_in_memory_sort_on_random_equal_times() {
    for seed in [1_u64, 42, 0x9E37_79B9_7F4A_7C15] {
        let dirs = dirs();
        let store = open(dirs.data.path());
        let mut rng = XorShift(seed);

        let records: Vec<NewHistoryRecord> = (0..1_200)
            .map(|n| record(dirs.downloads.path(), n, rng.below(40)))
            .collect();
        let inserted = store.insert_all(&records);

        let mut expected: Vec<(u64, i64)> = Vec::new();
        for (id, new) in inserted.iter().zip(&records) {
            if rng.below(8) == 0 {
                store.delete(&id.to_string()).expect("удаление");
            } else {
                expected.push((new.finished_at_unix_secs, id.get()));
            }
        }
        expected.sort_unstable_by(|a, b| b.cmp(a));
        let expected: Vec<i64> = expected.into_iter().map(|(_, id)| id).collect();

        for size in [
            1,
            3,
            HISTORY_PAGE_SIZE,
            expected.len() - 1,
            expected.len(),
            5_000,
        ] {
            assert_eq!(
                walk(&store, size),
                expected,
                "обход страницами по {size} разошёлся с сортировкой (seed {seed})"
            );
        }
    }
}

/// Полная страница — ровно [`HISTORY_PAGE_SIZE`] и курсор; последняя —
/// без курсора.
#[test]
fn a_full_page_has_a_cursor_and_the_last_page_does_not() {
    let dirs = dirs();
    let store = open(dirs.data.path());
    let records: Vec<NewHistoryRecord> = (1..=31)
        .map(|n| record(dirs.downloads.path(), n, n))
        .collect();
    store.insert_all(&records);

    let first = store.page(None).expect("страница");
    assert_eq!(first.records.len(), HISTORY_PAGE_SIZE);
    assert_eq!(
        first.next_cursor,
        Some(HistoryCursor {
            finished_at_unix_secs: 2,
            id: "2".to_owned()
        })
    );
    let last = store.page(first.next_cursor.as_ref()).expect("страница");
    assert_eq!(ids(&last), vec![1]);
    assert_eq!(last.next_cursor, None);
}

/// Контракт: id уникален навсегда. Удалить последнюю запись и вставить
/// новую — id больше удалённого; то же после очистки.
///
/// Мутация, которую тест обязан ловить: схема без `AUTOINCREMENT`.
#[test]
fn deleting_the_newest_record_never_reuses_its_id() {
    let dirs = dirs();
    let store = open(dirs.data.path());
    let folder = dirs.downloads.path();
    for n in 1..=3 {
        store.insert(&record(folder, n, n)).expect("вставка");
    }

    store.delete("3").expect("удаление последней");
    let after_delete = store.insert(&record(folder, 4, 4)).expect("вставка");
    assert!(
        after_delete.get() > 3,
        "id {after_delete} выдан повторно после удаления записи 3"
    );

    store.clear().expect("очистка");
    let after_clear = store.insert(&record(folder, 5, 5)).expect("вставка");
    assert!(
        after_clear.get() > after_delete.get(),
        "id {after_clear} выдан повторно после очистки"
    );
}

/// Курсор, которого ядро не выдавало, — пустая последняя страница, не
/// паника и не отказ. Пометок такой ответ не выдаёт.
#[test]
fn a_cursor_the_core_never_issued_gives_an_empty_last_page() {
    let dirs = dirs();
    let store = open(dirs.data.path());
    store
        .insert(&record(dirs.downloads.path(), 1, 1))
        .expect("вставка");

    let foreign = [
        (1, "abc"),
        (1, ""),
        (1, "0"),
        (1, "-3"),
        (1, "007"),
        (1, "+5"),
        (1, "1.0"),
        (1, "99999999999999999999"),
        (u64::MAX, "1"),
    ];
    for (finished_at_unix_secs, id) in foreign {
        let cursor = HistoryCursor {
            finished_at_unix_secs,
            id: id.to_owned(),
        };
        let page = store.page(Some(&cursor)).expect("не отказ");
        assert_eq!(
            page,
            HistoryRecordsPage {
                records: Vec::new(),
                next_cursor: None,
                notices: Vec::new(),
            },
            "курсор {cursor:?}"
        );
    }
}

/// Курсору не нужна существующая запись: удаление записи, на которой
/// остановился экран, следующую страницу не ломает.
#[test]
fn a_cursor_on_a_deleted_record_still_pages_correctly() {
    let dirs = dirs();
    let store = open(dirs.data.path());
    for n in 1..=6 {
        store
            .insert(&record(dirs.downloads.path(), n, 10))
            .expect("вставка");
    }
    let first = store.page_sized(None, 3).expect("страница");
    assert_eq!(ids(&first), vec![6, 5, 4]);

    store.delete("4").expect("удаление");

    let second = store
        .page_sized(first.next_cursor.as_ref(), 3)
        .expect("страница");
    assert_eq!(ids(&second), vec![3, 2, 1]);
}

// ───────────────────────────── пометки ─────────────────────────────

/// Две пометки сразу; запрос с курсором их не выдаёт и не гасит; первый
/// запрос без курсора отдаёт обе; второй — ни одной.
#[test]
fn notices_both_arrive_once_and_a_cursor_request_does_not_consume_them() {
    let dirs = dirs();
    let data = dirs.data.path();
    fs::write(db_path(data), "не база ".repeat(40)).expect("фикстура");
    let store = open(data);
    let folder = dirs.downloads.path();
    store.insert(&record(folder, 1, 1)).expect("вставка");

    let refused = store.insert(&NewHistoryRecord {
        folder: PathBuf::from("относительная/папка"),
        ..record(folder, 2, 2)
    });
    assert!(matches!(
        refused,
        Err(HistoryWriteError::InvalidRecord { .. })
    ));

    let with_cursor = store
        .page(Some(&HistoryCursor {
            finished_at_unix_secs: 5,
            id: "1".to_owned(),
        }))
        .expect("страница с курсором");
    assert!(with_cursor.notices.is_empty());
    let foreign_cursor = store
        .page(Some(&HistoryCursor {
            finished_at_unix_secs: 5,
            id: "чужой".to_owned(),
        }))
        .expect("страница с чужим курсором");
    assert!(foreign_cursor.notices.is_empty());

    let first = store.page(None).expect("страница без курсора");
    assert_eq!(
        first.notices,
        vec![
            HistoryNotice::BaseRecreated,
            HistoryNotice::LastWriteFailed {
                cause: HistoryWriteFailure::StorageFailed
            },
        ]
    );
    let second = store.page(None).expect("страница без курсора");
    assert!(second.notices.is_empty(), "пометки выданы дважды");
}

/// Метод для TL-89: отказ, случившийся до вставки, выдаётся один раз, и
/// последняя причина вытесняет прежнюю.
#[test]
fn a_write_failure_recorded_by_the_caller_is_reported_once() {
    let dirs = dirs();
    let store = open(dirs.data.path());

    store.record_write_failure(HistoryWriteFailure::NoAccess);
    store.record_write_failure(HistoryWriteFailure::DiskFull);

    assert_eq!(
        store.page(None).expect("страница").notices,
        vec![HistoryNotice::LastWriteFailed {
            cause: HistoryWriteFailure::DiskFull
        }]
    );
    assert!(store.page(None).expect("страница").notices.is_empty());
}

/// Настоящий отказ диска на вставке (журнал не создаётся в каталоге
/// только на чтение) классифицирован как `noAccess` и сам выставил
/// пометку.
#[cfg(unix)]
#[test]
fn a_real_storage_failure_on_insert_sets_the_notice() {
    let dirs = dirs();
    let data = dirs.data.path();
    let store = open(data);
    set_mode(data, 0o555);

    let result = store.insert(&record(dirs.downloads.path(), 1, 1));
    set_mode(data, 0o755);

    match &result {
        Err(HistoryWriteError::Storage(HistoryStorageError {
            failure: StorageFailure::NoAccess,
            ..
        })) => {}
        other => panic!("вставка в каталог только на чтение дала {other:?}"),
    }
    let page = store.page(None).expect("страница");
    assert!(page.records.is_empty(), "запись всё же вставлена");
    assert_eq!(
        page.notices,
        vec![HistoryNotice::LastWriteFailed {
            cause: HistoryWriteFailure::NoAccess
        }]
    );
}

// ───────────────────────────── статус файла ─────────────────────────────

/// Ф-5: статус — диск в момент чтения. Удалили файл — `missing` с папкой;
/// нет папки — `missing` без неё.
#[test]
fn file_status_follows_the_disk() {
    let dirs = dirs();
    let store = open(dirs.data.path());
    let folder = dirs.downloads.path();

    let present = record(folder, 1, 3);
    fs::write(folder.join(&present.file_name), b"video").expect("файл");
    store.insert(&present).expect("вставка");
    store.insert(&record(folder, 2, 2)).expect("вставка");
    store
        .insert(&record(&folder.join("нет-такой-папки"), 3, 1))
        .expect("вставка");

    let statuses = |store: &HistoryStore| -> Vec<HistoryFileStatus> {
        store
            .page(None)
            .expect("страница")
            .records
            .into_iter()
            .map(|record| record.file_status)
            .collect()
    };

    assert_eq!(
        statuses(&store),
        vec![
            HistoryFileStatus::Present,
            HistoryFileStatus::Missing {
                folder_exists: true
            },
            HistoryFileStatus::Missing {
                folder_exists: false
            },
        ]
    );

    fs::remove_file(folder.join(&present.file_name)).expect("файл удалён");
    assert_eq!(
        statuses(&store)[0],
        HistoryFileStatus::Missing {
            folder_exists: true
        }
    );
}

/// Имя файла из отредактированной базы с `..` не доходит до диска: рядом
/// с папкой записи лежит настоящий файл, до которого `../` дотянулся бы,
/// но статус — `missing`, и путь не строится.
#[test]
fn a_tampered_file_name_never_reaches_the_disk() {
    let dirs = dirs();
    let store = open(dirs.data.path());
    let downloads = dirs.downloads.path();
    let inner = downloads.join("inner");
    fs::create_dir(&inner).expect("папка");
    fs::write(downloads.join("escape.mp4"), b"video").expect("файл снаружи");
    let inner_text = inner.to_str().expect("путь в UTF-8");

    store.with_connection(|conn| {
        conn.execute(
            INSERT_SQL,
            params![
                "v",
                "u",
                "t",
                "standard",
                1080,
                "../escape.mp4",
                inner_text,
                1,
                2
            ],
        )
        .expect("подложная строка");
        conn.execute(
            INSERT_SQL,
            params![
                "v",
                "u",
                "t",
                "standard",
                1080,
                "escape.mp4",
                "relative",
                1,
                1
            ],
        )
        .expect("подложная строка");
    });

    let records = store.page(None).expect("страница").records;
    assert_eq!(
        records[0].file_status,
        HistoryFileStatus::Missing {
            folder_exists: true
        }
    );
    assert_eq!(records[0].file_path(), None);
    assert_eq!(
        records[1].file_status,
        HistoryFileStatus::Missing {
            folder_exists: false
        }
    );
    assert_eq!(records[1].file_path(), None);
}

/// Белый список записи: имя — ровно одно имя, папка — абсолютная, числа —
/// в пределах базы. Отказ до базы: строк не прибавилось, пометка одна.
#[test]
fn invalid_records_are_rejected_before_the_base() {
    let dirs = dirs();
    let store = open(dirs.data.path());
    let folder = dirs.downloads.path();
    let base = || record(folder, 1, 1);

    let mut invalid: Vec<NewHistoryRecord> =
        ["", ".", "..", "../x.mp4", "a/b.mp4", "a\\b.mp4", "/abs.mp4"]
            .into_iter()
            .map(|file_name| NewHistoryRecord {
                file_name: file_name.to_owned(),
                ..base()
            })
            .collect();
    invalid.push(NewHistoryRecord {
        folder: PathBuf::from("downloads"),
        ..base()
    });
    invalid.push(NewHistoryRecord {
        size_bytes: u64::MAX,
        ..base()
    });
    invalid.push(NewHistoryRecord {
        finished_at_unix_secs: u64::MAX,
        ..base()
    });

    for new in &invalid {
        let result = store.insert(new);
        assert!(
            matches!(result, Err(HistoryWriteError::InvalidRecord { .. })),
            "запись {new:?} принята: {result:?}"
        );
    }

    let page = store.page(None).expect("страница");
    assert!(page.records.is_empty());
    assert_eq!(
        page.notices,
        vec![HistoryNotice::LastWriteFailed {
            cause: HistoryWriteFailure::StorageFailed
        }]
    );
}

// ───────────────────────────── удаление и очистка ─────────────────────────────

/// Ф-6: удаление несуществующей записи — типизированный отказ, не паника.
#[test]
fn deleting_an_unknown_record_is_a_typed_refusal() {
    let dirs = dirs();
    let store = open(dirs.data.path());
    let id = store
        .insert(&record(dirs.downloads.path(), 1, 1))
        .expect("вставка");

    for unknown in ["999", "abc", "", "0", "01"] {
        assert_eq!(
            store.delete(unknown),
            Err(HistoryDeleteError::UnknownRecord {
                id: unknown.to_owned()
            })
        );
    }
    store.delete(&id.to_string()).expect("удаление");
    assert!(matches!(
        store.delete(&id.to_string()),
        Err(HistoryDeleteError::UnknownRecord { .. })
    ));
    assert_eq!(store.get(&id.to_string()), Ok(None));
}

/// К-9: удаление и очистка трогают только базу. Обход ФС после них: в
/// папке назначения те же файлы с теми же байтами, в каталоге данных — только
/// файл базы.
#[test]
fn delete_and_clear_touch_nothing_but_the_base() {
    let dirs = dirs();
    let data = dirs.data.path();
    let downloads = dirs.downloads.path();
    let nested = downloads.join("вложенная");
    fs::create_dir(&nested).expect("папка");

    let store = open(data);
    let mut ids = Vec::new();
    for (n, folder) in [(1, downloads), (2, downloads), (3, nested.as_path())] {
        let new = record(folder, n, n);
        fs::write(folder.join(&new.file_name), format!("содержимое {n}")).expect("файл");
        ids.push(store.insert(&new).expect("вставка"));
    }
    fs::write(downloads.join("чужой.txt"), b"not ours").expect("посторонний файл");
    let before = snapshot(downloads);

    store.delete(&ids[1].to_string()).expect("удаление");
    assert_eq!(
        ids_of(&store),
        vec![ids[2].get(), ids[0].get()],
        "удалена не та запись"
    );
    assert_eq!(snapshot(downloads), before, "удаление тронуло файлы");

    store.clear().expect("очистка");
    assert!(store.page(None).expect("страница").records.is_empty());
    assert_eq!(snapshot(downloads), before, "очистка тронула файлы");
    assert_eq!(names_in(data), vec![HISTORY_FILE_NAME.to_owned()]);
}

fn ids_of(store: &HistoryStore) -> Vec<i64> {
    ids(&store.page(None).expect("страница"))
}

/// Н-3, Ф-7: обе страничные выборки идут по индексу и не сортируют во
/// временном дереве — список не деградирует от размера.
///
/// Первая страница законно обходит индекс с начала (`SCAN ... USING INDEX`)
/// и останавливается на `LIMIT`. Страница после курсора обязана **искать**
/// по индексу с условием на время (`SEARCH ... (finished_at_unix_secs<?)`):
/// полный проход индекса с фильтром на каждой строке тоже «по индексу и без
/// сортировки», но растёт с глубиной листания.
///
/// Мутация, которую тест обязан ловить: `finished_at_unix_secs + 0` в
/// условии курсора — план становится `SCAN`.
#[test]
fn the_index_serves_both_page_queries_without_sorting() {
    let dirs = dirs();
    let store = open(dirs.data.path());

    let plan = |sql: &str, params: &[&dyn rusqlite::ToSql]| -> String {
        store.with_connection(|conn| {
            let mut statement = conn
                .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
                .expect("план готовится");
            let details: Vec<String> = statement
                .query_map(params, |row| row.get(3))
                .expect("план читается")
                .collect::<Result<_, _>>()
                .expect("строки плана");
            details.join(" | ")
        })
    };

    let first = plan(PAGE_FIRST_SQL, params![31]);
    let after = plan(PAGE_AFTER_SQL, params![10, 5, 31]);
    for detail in [&first, &after] {
        assert!(!detail.contains("TEMP B-TREE"), "план сортирует: {detail}");
    }
    assert_eq!(
        first, "SCAN history USING INDEX history_newest_first",
        "первая страница не по индексу"
    );
    assert!(
        after
            .starts_with("SEARCH history USING INDEX history_newest_first (finished_at_unix_secs<"),
        "страница после курсора не ищет по индексу со временем: {after}"
    );
    assert!(
        !after.contains("SCAN"),
        "страница после курсора сканирует: {after}"
    );
}

/// Строковая форма id — только каноническая.
#[test]
fn record_ids_parse_only_their_canonical_form() {
    assert_eq!(RecordId::parse("1"), Some(RecordId(1)));
    assert_eq!(
        RecordId::parse("9223372036854775807"),
        Some(RecordId(i64::MAX))
    );
    for foreign in [
        "",
        "0",
        "-1",
        "+1",
        "01",
        " 1",
        "1 ",
        "1e3",
        "9223372036854775808",
    ] {
        assert_eq!(RecordId::parse(foreign), None, "{foreign:?}");
    }
}

/// Правило подписи папки (ревью TL-89, S3: функция переехала сюда из слоя
/// команд): покомпонентное равенство с системной «Загрузками», без диска.
#[test]
fn folder_display_names_the_system_downloads_only_by_equal_path() {
    use crate::types::FolderDisplay;

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
