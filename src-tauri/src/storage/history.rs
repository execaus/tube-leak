//! История загрузок на диске: схема, миграции, запись, постраничное чтение,
//! удаление (TL-85; Ф-1, Ф-2, Ф-6, Ф-7 эпика E5).
//!
//! Команд Tauri здесь нет — их тела подставит TL-90. Запись в момент Done
//! сюда тоже не входит, её делает оркестрация (TL-89). Обе задачи обращаются
//! только к API этого модуля и друг с другом не координируются: пометки
//! «выдано один раз» живут внутри [`HistoryStore`].
//!
//! Типы здесь свои, и сведение к контракту делают потребители (issue #92).
//! Исключение — значения, у которых вторая форма была бы дублем без смысла:
//! [`HistoryCursor`], [`HistoryNotice`], [`HistoryWriteFailure`],
//! [`HistoryFileStatus`] и [`SelectedQuality`] приходят из `crate::types`
//! как есть. Так поступает и снимок очереди со `StartDownloadRequest`.
//!
//! # Файл
//!
//! `history.sqlite` лежит в корне каталога данных приложения, рядом с
//! `queue.json` и `instance.lock`, и **вне** `yt-dlp/`: уборка контура
//! обновления обходит корень установок и сносит там всё чужое.
//!
//! Журнал — режим SQLite по умолчанию (`DELETE`, файл `history.sqlite-journal`
//! на время транзакции, `synchronous = FULL`). WAL не нужен: писатель один
//! (TL-20), записи редкие, одна на завершённую загрузку (Н-2).
//!
//! # Версия схемы — `PRAGMA user_version`
//!
//! Версия хранится в заголовке самого файла (смещение 60, четыре байта
//! big-endian), а не в таблице версий, и выбрана она по двум причинам.
//!
//! 1. **Подъём версии транзакционен вместе с миграцией.** `PRAGMA
//!    user_version = N` пишет первую страницу через тот же пейджер, что и
//!    `CREATE TABLE`, поэтому схема и номер фиксируются одним `COMMIT` или
//!    откатываются вместе. У таблицы версий то же свойство, но она сама —
//!    схема, которую пришлось бы создавать до первой миграции.
//! 2. **Версию можно прочитать, не открывая файл в SQLite.** Заголовок
//!    документирован форматом файла, и чужую версию модуль узнаёт чтением
//!    ста байт обычным `File::open`. SQLite при этом не вызывается вовсе, и
//!    доктрина «не открывать и не переписывать» соблюдается конструкцией, а
//!    не аккуратностью (см. ниже).
//!
//! Миграции — упорядоченный список [`MIGRATIONS`]; версия схемы — его длина
//! ([`SCHEMA_VERSION`]). Применяются только вперёд, **каждая в своей
//! транзакции** (Ф-1): база на версии 0 при отказе второй миграции остаётся
//! на версии 1, а не на 0. Промежуточной версии не бывает — только
//! «до шага» и «после шага».
//!
//! # Доктрина отказов открытия (Ф-1 а–д, прецедент TL-71)
//!
//! | Что на диске | Что делает [`HistoryStore::open`] |
//! |---|---|
//! | файла нет или он пуст | создаёт базу, применяет все миграции |
//! | версия ниже текущей | доводит миграциями; отказ шага — откат шага, [`HistoryOpenError::MigrationFailed`] |
//! | рядом лежит `history.sqlite-wal` или заголовок объявляет WAL — при любом содержимом файла | **не открывает и не пишет**, [`HistoryOpenError::NewerVersion`] с `wal: true` |
//! | версия выше текущей или отрицательная | **не открывает и не пишет**, [`HistoryOpenError::NewerVersion`] |
//! | не SQLite (заголовок) или порча (`PRAGMA integrity_check`, `SQLITE_NOTADB`, `SQLITE_CORRUPT`) | откладывает файл под `history.sqlite.broken-<unix-секунды>`, заводит новую базу, пометка [`HistoryNotice::BaseRecreated`] |
//! | нет прав на каталог или файл, файл открывается только на чтение | [`HistoryOpenError::NoAccess`] |
//!
//! Строки проверяются сверху вниз: первые две — до любого вызова SQLite.
//! Всё это — у первого вызова за процесс. Второй и последующие диск не
//! трогают вовсе и отвечают [`HistoryOpenError::AlreadyOpen`] (doc
//! [`HistoryStore::open`]).
//!
//! **Отложить, а не удалить.** Порча — это данные пользователя, которые,
//! возможно, ещё читаются чужим инструментом. Отложенное имя никогда не
//! затирает прежнюю отложенную копию: занятая метка получает суффикс `-1`,
//! `-2`, … Что именно окажется под этим именем, зависит от того, где порча
//! замечена.
//!
//! - **Файл отвергнут по заголовку** (не SQLite). SQLite его не открывал, и
//!   файл уезжает байт в байт вместе со спутниками (`-journal`, `-shm`) под
//!   тем же новым именем с тем же суффиксом. Журнал, оставшийся рядом с
//!   новой пустой базой, SQLite удалил бы как «не горячий». Под именем
//!   отложенной копии он остаётся парой к ней.
//! - **Порча страниц в настоящей SQLite-базе** (заголовок цел, отказ
//!   `integrity_check`, `SQLITE_CORRUPT`, `SQLITE_NOTADB`). Это замечено уже
//!   внутри SQLite. Горячий `-journal` SQLite применяет при первом обращении
//!   (`PRAGMA user_version` в `connect`), то есть до откладывания, и сам
//!   убирает. Копия — база **после** штатного восстановления журнала, а не
//!   байт в байт то, что лежало на диске. Данные при этом не теряются: то же
//!   восстановление сделал бы любой клиент SQLite.
//!
//! `-wal` сюда не доходит ни в одном случае: с ним открытие отказывает
//! раньше (ниже). [`set_aside`] переносит его по-прежнему, но только как
//! запас на случай, если этот порядок когда-нибудь поменяют.
//!
//! **Проверка — `integrity_check`** (Ф-1 г, issue #92). Кроме целости
//! страниц она сверяет содержимое индексов с таблицей, и рассинхрон
//! `history_newest_first` с таблицей тоже считается порчей. Цена каждого
//! старта растёт с историей (Р-7), но по замеру ревью разница с
//! `quick_check` — около 15 мс на 100 000 записей в релизной сборке.
//!
//! **WAL не открывается вовсе.** Эта версия WAL не создаёт, значит его
//! создала будущая версия приложения или чужой инструмент. Опасен он
//! дважды.
//!
//! - Заголовок файла в режиме WAL может отставать от `-wal`: в заголовке
//!   наша версия схемы, а в журнале уже новее.
//! - SQLite применяет найденный рядом `-wal` при первом чтении, даже если
//!   заголовок объявляет журнал `DELETE`. Чужой `-wal` рядом с нашей базой
//!   подменил бы её страницы, `page` отдал бы `no such table`, а закрытие
//!   перенесло бы подмену в файл, и история пропала бы без `.broken`-копии.
//!   Рядом с пустым файлом SQLite этот `-wal` удалил бы.
//!
//! Порядок «сначала открыть в SQLite, потом сверить версию» опоздал бы:
//! изменения к моменту сверки уже на диске. Поэтому до любого
//! вызова SQLite, только по файловой системе и байтам заголовка, проверяются
//! два признака: существует `history.sqlite-wal` (в том числе символьной
//! ссылкой) и байт 18 или 19 заголовка (версия формата записи и чтения)
//! равен 2. Любой из них — [`HistoryOpenError::NewerVersion`], файлы не
//! тронуты.
//!
//! Остаточный риск — `-wal`, появившийся между проверкой и открытием. Его
//! может создать только другой процесс, а единственность открытия
//! хранилища — TL-90.
//!
//! # Порядок и курсор
//!
//! Записи идут строго по убыванию пары `(finished_at_unix_secs, id)`, и
//! сравнение `id` числовое: это целый ключ базы. Страница после курсора —
//! записи строго меньше пары курсора (сравнение значений строк SQLite),
//! индекс `history_newest_first` обслуживает и фильтр, и порядок без
//! сортировки во временном дереве (проверено `EXPLAIN QUERY PLAN` в тестах).
//! Курсору не нужна существующая запись: удаление записи, на которой
//! остановился экран, следующую страницу не ломает.
//!
//! Курсор, которого ядро не выдавало (`id` — не каноническое положительное
//! целое, время за пределом `i64`), даёт пустую последнюю страницу.
//!
//! **`id` уникален навсегда**: `INTEGER PRIMARY KEY AUTOINCREMENT`. Голый
//! `INTEGER PRIMARY KEY` после удаления строки с наибольшим ключом, как и
//! после очистки, выдал бы тот же ключ заново, и курсор или «Удалить» в
//! руках открытого экрана указали бы на чужую запись.
//!
//! # Пометки «выдано один раз»
//!
//! [`HistoryNotice::BaseRecreated`] выставляет открытие.
//! [`HistoryNotice::LastWriteFailed`] выставляет отказавший
//! [`HistoryStore::insert`] сам, и оркестрации незачем об этом помнить.
//! Если отказ случился раньше вставки (например, не снялся размер файла),
//! TL-89 зовёт [`HistoryStore::record_write_failure`]. Пометки отдаются
//! только ответом **без курсора** и только после успешного чтения:
//! страница, которая не прочиталась, пометку не съедает. Запрос с курсором
//! их не выдаёт и не гасит.
//!
//! # Файл записи на месте или нет (Ф-5)
//!
//! Статус вычисляется при каждом чтении через `fs::metadata` — содержимое
//! файла не читается. Проверка идёт на потоке вызывающего и **без
//! потолка времени**: отключённый сетевой том может её задержать (Н-3), и
//! звать чтение истории из async-кода TL-90 обязан через `spawn_blocking`.
//! Соединение с базой на время проверок не держится.
//!
//! Имя файла из базы — непроверенный ввод (базу мог отредактировать кто
//! угодно). Путь строится только из абсолютной папки и имени, которое
//! является ровно одним обычным компонентом пути, без разделителей
//! ([`HistoryRecord::file_path`]). Иначе статус — «файла нет», и диск по
//! такому пути не трогается. Тот же белый список применяется к записи
//! при вставке.
//!
//! # Приватность (Н-1)
//!
//! Модуль ничего не печатает. Тексты отказов не содержат ни ссылок, ни
//! названий: SQLite не включает значения параметров в сообщения, а отказы
//! проверки записи — статические строки.

use std::ffi::OsStr;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use rusqlite::ffi::ErrorCode;
use rusqlite::types::Type;
use rusqlite::{
    params, Connection, OpenFlags, OptionalExtension, Row, Transaction, TransactionBehavior,
    MAIN_DB,
};

use crate::types::{
    HistoryCursor, HistoryFileStatus, HistoryNotice, HistoryUnavailableReason, HistoryWriteFailure,
    QualityKind, SelectedQuality, HISTORY_PAGE_SIZE,
};

/// Имя файла базы в корне каталога данных приложения.
pub const HISTORY_FILE_NAME: &str = "history.sqlite";

/// Метка отложенной испорченной базы: `history.sqlite.broken-<метка>`.
const BROKEN_MARKER: &str = ".broken-";

/// Суффикс WAL-журнала базы.
const WAL_SUFFIX: &str = "-wal";

/// Спутники файла базы, которые SQLite ищет по имени базы с суффиксом.
const SIDE_FILE_SUFFIXES: [&str; 3] = ["-journal", WAL_SUFFIX, "-shm"];

/// Смещения версий формата записи и чтения в заголовке (по байту).
const FILE_FORMAT_OFFSETS: [usize; 2] = [18, 19];

/// Значение версии формата у базы в режиме WAL (`1` — классический журнал).
const WAL_FILE_FORMAT: u8 = 2;

/// Магическая строка заголовка файла SQLite 3.
const SQLITE_MAGIC: &[u8; 16] = b"SQLite format 3\0";

/// Длина заголовка файла SQLite.
const HEADER_LEN: usize = 100;

/// Смещение `user_version` в заголовке: четыре байта, big-endian, со знаком.
const USER_VERSION_OFFSET: usize = 60;

/// Схема версии 1 (TL-85).
///
/// Поля — минимум из данных Done (Р-2, Ф-2). Статус «файл на месте» не
/// хранится: он вычисляется при чтении (Ф-5). Папка хранится абсолютным
/// путём отдельно от имени файла: имя уже санитизировано (E3), а путь
/// нужен ядру для «Показать в папке» (Ф-8) и для статуса. Как показать
/// папку (`FolderDisplay`), решает потребитель (TL-89/TL-90), а не схема.
///
/// `quality_kind` хранится своими строками, а не сериализацией serde. Иначе
/// переименование варианта в контракте молча сменило бы значения в уже
/// лежащих у пользователей базах. `CHECK` в схеме и сопоставление
/// [`kind_to_column`]/[`kind_from_column`] — одно и то же множество (тест).
///
/// `STRICT` — SQLite проверяет типы столбцов, а не приводит молча.
const SCHEMA_V1: &str = "
CREATE TABLE history (
    id                    INTEGER PRIMARY KEY AUTOINCREMENT,
    video_id              TEXT    NOT NULL,
    url                   TEXT    NOT NULL,
    title                 TEXT    NOT NULL,
    quality_kind          TEXT    NOT NULL
        CHECK (quality_kind IN ('standard', 'maxAvailable', 'audioOnly')),
    quality_height_px     INTEGER
        CHECK (quality_height_px BETWEEN 0 AND 4294967295),
    file_name             TEXT    NOT NULL,
    folder                TEXT    NOT NULL,
    size_bytes            INTEGER NOT NULL CHECK (size_bytes >= 0),
    finished_at_unix_secs INTEGER NOT NULL CHECK (finished_at_unix_secs >= 0)
) STRICT;
CREATE INDEX history_newest_first ON history (finished_at_unix_secs DESC, id DESC);
";

/// Миграции по порядку: элемент `i` переводит базу с версии `i` на `i + 1`.
///
/// Только дописывается в конец. Правка уже выпущенного элемента — это
/// две разные схемы под одним номером у разных пользователей.
const MIGRATIONS: &[&str] = &[SCHEMA_V1];

/// Версия схемы, которую знает эта сборка.
// Вне тестов не читается: открытие сверяет версию по длине списка миграций.
#[allow(clippy::cast_possible_truncation, dead_code)]
pub const SCHEMA_VERSION: u32 = MIGRATIONS.len() as u32;

/// Первая страница: новые сверху.
const PAGE_FIRST_SQL: &str = "
SELECT id, video_id, url, title, quality_kind, quality_height_px,
       file_name, folder, size_bytes, finished_at_unix_secs
FROM history
ORDER BY finished_at_unix_secs DESC, id DESC
LIMIT ?1";

/// Страница после курсора: строго меньше пары `(время, id)`.
const PAGE_AFTER_SQL: &str = "
SELECT id, video_id, url, title, quality_kind, quality_height_px,
       file_name, folder, size_bytes, finished_at_unix_secs
FROM history
WHERE (finished_at_unix_secs, id) < (?1, ?2)
ORDER BY finished_at_unix_secs DESC, id DESC
LIMIT ?3";

/// Одна запись по id.
const GET_SQL: &str = "
SELECT id, video_id, url, title, quality_kind, quality_height_px,
       file_name, folder, size_bytes, finished_at_unix_secs
FROM history
WHERE id = ?1";

// Запись Done — TL-89; до неё путь вставки зовут только тесты.
#[allow(dead_code)]
const INSERT_SQL: &str = "
INSERT INTO history (video_id, url, title, quality_kind, quality_height_px,
                     file_name, folder, size_bytes, finished_at_unix_secs)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)";

/// Идентификатор записи истории — положительный целый ключ базы.
///
/// Строковая форма (контракт `HistoryEntry::id`) — десятичная запись без
/// знака и ведущих нулей. Разбирается только она: `"007"` и `"+7"` — не
/// идентификаторы, которые ядро выдавало.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RecordId(i64);

impl RecordId {
    /// Разбирает строковую форму, выданную ядром. Всё прочее — `None`.
    pub fn parse(text: &str) -> Option<Self> {
        let value: i64 = text.parse().ok()?;
        (value > 0 && value.to_string() == text).then_some(Self(value))
    }

    /// Числовое значение ключа.
    pub fn get(self) -> i64 {
        self.0
    }
}

impl fmt::Display for RecordId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Запись для вставки — данные Done-задачи на момент завершения (Ф-2, Р-2).
// Строит оркестрация (TL-89); до неё — только тесты.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewHistoryRecord {
    /// Канонический id ролика (TL-72).
    pub video_id: String,
    /// Ссылка, как её вставили.
    pub url: String,
    /// Название ролика.
    pub title: String,
    /// Выбранный пункт качества.
    pub quality: SelectedQuality,
    /// Имя готового файла с расширением: ровно одно имя, без пути.
    pub file_name: String,
    /// Абсолютный путь папки, куда лёг файл. Должен быть UTF-8.
    pub folder: PathBuf,
    /// Размер файла в байтах, снятый с диска при записи.
    pub size_bytes: u64,
    /// Время завершения, Unix-секунды UTC.
    pub finished_at_unix_secs: u64,
}

/// Запись истории, как её прочитали.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryRecord {
    pub id: RecordId,
    pub video_id: String,
    pub url: String,
    pub title: String,
    pub quality: SelectedQuality,
    pub file_name: String,
    /// Папка из базы как есть. Для построения пути —
    /// [`HistoryRecord::file_path`], а не `folder.join(file_name)`.
    pub folder: PathBuf,
    pub size_bytes: u64,
    pub finished_at_unix_secs: u64,
    /// Файл на месте или нет — на момент этого чтения.
    pub file_status: HistoryFileStatus,
}

impl HistoryRecord {
    /// Полный путь к файлу записи — только если папка абсолютная, а имя —
    /// ровно одно обычное имя без разделителей.
    ///
    /// Базу мог отредактировать кто угодно. `None` означает, что путь по
    /// такой записи не строится вовсе; «Показать в папке» (TL-90) обязано
    /// идти через этот метод.
    pub fn file_path(&self) -> Option<PathBuf> {
        record_file_path(&self.folder, &self.file_name)
    }
}

/// Порция истории: записи, курсор следующей порции, пометки.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryRecordsPage {
    /// Не больше размера страницы, строго по убыванию `(время, id)`.
    pub records: Vec<HistoryRecord>,
    /// Курсор следующей порции; `None` — старше этих записей нет.
    pub next_cursor: Option<HistoryCursor>,
    /// Пометки «выдано один раз». Непустым бывает только в ответе без курсора.
    pub notices: Vec<HistoryNotice>,
}

/// Почему история в этом сеансе не открылась.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HistoryOpenError {
    /// База, которую эта сборка не может тронуть, не рискуя чужими данными.
    /// Файл не открыт в SQLite, ни он, ни спутники не изменены.
    ///
    /// Сюда входят:
    /// - версия схемы выше известной этой сборке;
    /// - отрицательная `user_version` — консервативно, текст отказа для
    ///   неё неточен (см. [`refuse_foreign_version`]);
    /// - база в режиме журнала, которого эта версия не создаёт: рядом лежит
    ///   `history.sqlite-wal` или заголовок объявляет WAL (`wal == true`).
    ///   Тогда `found` — версия из заголовка, если он есть, иначе `0`, и
    ///   причина отказа — журнал, а не версия.
    ///
    /// Причина в контракте одна — `newerVersion`: WAL создаёт только
    /// будущая версия приложения.
    #[error("история {path}: {} — файл не тронут", newer_detail(*.found, *.supported, *.wal))]
    NewerVersion {
        path: PathBuf,
        found: i64,
        supported: u32,
        /// Отказ по признаку WAL, а не по версии схемы.
        wal: bool,
    },
    /// Нет прав на каталог данных или файл базы, либо файл не читается.
    #[error("история {path}: нет доступа — {reason}")]
    NoAccess { path: PathBuf, reason: String },
    /// Миграция `from → to` отказала; её транзакция откатилась, база
    /// осталась на версии `from`.
    #[error(
        "история {path}: миграция {from} → {to} отказала, база осталась на версии {from}: {reason}"
    )]
    MigrationFailed {
        path: PathBuf,
        from: u32,
        to: u32,
        reason: String,
    },
    /// Хранилище в этом процессе уже открывалось. Повторное открытие
    /// отклонено до любого обращения к диску ([`HistoryStore::open`]).
    #[error(
        "история: хранилище уже открыто в этом процессе — повторное открытие отклонено, \
         диск не тронут"
    )]
    AlreadyOpen,
}

fn newer_detail(found: i64, supported: u32, wal: bool) -> String {
    if wal {
        "база в режиме журнала WAL, которого эта сборка не создаёт".to_owned()
    } else {
        format!("база чужой версии схемы {found}, эта сборка знает версии до {supported}")
    }
}

impl HistoryOpenError {
    /// Причина в форме контракта. Для [`Self::NewerVersion`] это и чужая
    /// версия схемы, и база в режиме журнала, которого эта версия не создаёт.
    pub fn reason(&self) -> HistoryUnavailableReason {
        match self {
            Self::NewerVersion { .. } => HistoryUnavailableReason::NewerVersion,
            Self::NoAccess { .. } => HistoryUnavailableReason::NoAccess,
            Self::MigrationFailed { .. } => HistoryUnavailableReason::MigrationFailed,
            // Отдельной причины в контракте нет: для экрана это «история на
            // сеанс недоступна», подробность — в `message`.
            Self::AlreadyOpen => HistoryUnavailableReason::NoAccess,
        }
    }
}

/// Класс отказа базы после открытия.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageFailure {
    /// `SQLITE_FULL`.
    DiskFull,
    /// Права, только чтение, журнал не создаётся.
    NoAccess,
    /// Всё прочее, включая строку, которая не разбирается.
    Other,
}

impl StorageFailure {
    /// Причина отказа записи в форме контракта.
    #[allow(dead_code)] // путь записи — TL-89
    pub fn as_write_failure(self) -> HistoryWriteFailure {
        match self {
            Self::DiskFull => HistoryWriteFailure::DiskFull,
            Self::NoAccess => HistoryWriteFailure::NoAccess,
            Self::Other => HistoryWriteFailure::StorageFailed,
        }
    }
}

/// Отказ базы на операции после открытия.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("история: база отказала ({failure:?}): {detail}")]
pub struct HistoryStorageError {
    pub failure: StorageFailure,
    /// Текст SQLite — для лога. Значений параметров в нём нет.
    pub detail: String,
}

/// Почему запись Done не вставлена.
#[allow(dead_code)] // путь записи — TL-89
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HistoryWriteError {
    /// Запись не прошла проверку до базы: имя файла не одно имя, папка не
    /// абсолютная или не UTF-8, число за пределом `i64`.
    #[error("история: запись отклонена до базы — {reason}")]
    InvalidRecord { reason: &'static str },
    /// База отказала.
    #[error(transparent)]
    Storage(#[from] HistoryStorageError),
}

impl HistoryWriteError {
    /// Причина для пометки `lastWriteFailed`.
    #[allow(dead_code)] // путь записи — TL-89
    pub fn failure(&self) -> HistoryWriteFailure {
        match self {
            Self::InvalidRecord { .. } => HistoryWriteFailure::StorageFailed,
            Self::Storage(err) => err.failure.as_write_failure(),
        }
    }
}

/// Почему запись не удалена.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HistoryDeleteError {
    /// Записи с таким id нет (или строка — не id, выданный ядром).
    ///
    /// `id` — непроверенный ввод, в тексте он через [`ShownId`].
    #[error("история: записи {} нет", ShownId(.id))]
    UnknownRecord { id: String },
    /// База отказала.
    #[error(transparent)]
    Storage(#[from] HistoryStorageError),
}

/// Непроверенный `id` (записи или курсора) для строки лога и текста отказа.
///
/// Печатается в форме `{:?}` и не длиннее [`ShownId::MAX_CHARS`] знаков, с
/// `…` при обрезке. Кавычки и экранирование — не косметика: перевод строки
/// или возврат каретки внутри `id` иначе начал бы в логе новую строку,
/// неотличимую от настоящей.
pub(crate) struct ShownId<'a>(pub &'a str);

impl ShownId<'_> {
    /// Сколько знаков `id` показывается.
    pub(crate) const MAX_CHARS: usize = 40;
}

impl fmt::Display for ShownId<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut chars = self.0.chars();
        let shown: String = chars.by_ref().take(Self::MAX_CHARS).collect();
        write!(f, "{shown:?}")?;
        if chars.next().is_some() {
            f.write_str("…")?;
        }
        Ok(())
    }
}

/// Открывалось ли хранилище в этом процессе ([`HistoryStore::open`]).
static OPENED_IN_PROCESS: AtomicBool = AtomicBool::new(false);

/// Пометки, ждущие ответа без курсора.
#[derive(Debug, Default)]
struct PendingNotices {
    base_recreated: bool,
    last_write_failed: Option<HistoryWriteFailure>,
}

/// Открытая история загрузок.
///
/// Один на процесс (писатель один, TL-20). Методы блокирующие; соединение
/// под мьютексом, поэтому хранилище можно делить между потоками.
#[derive(Debug)]
pub struct HistoryStore {
    conn: Mutex<Connection>,
    pending: Mutex<PendingNotices>,
}

impl HistoryStore {
    /// Открывает `history.sqlite` в каталоге данных приложения, создавая
    /// каталог и базу при необходимости. Доктрина отказов — в шапке модуля.
    ///
    /// **Ровно один раз за процесс.** Первый вызов забирает процессный
    /// флаг — при любом исходе, в том числе при отказе. Второй и
    /// последующие отвечают [`HistoryOpenError::AlreadyOpen`], не создавая
    /// ни каталога, ни файла. Причина — ревью TL-85: замок единственности
    /// приложения работает fail-open, а два открытия на испорченном файле
    /// могли бы оставить одно соединение на уже отложенной копии.
    ///
    /// Флаг стоит здесь, а не в `commands::history::HistoryState::open`:
    /// так его не обходит и вызов хранилища мимо состояния (мутация ревью
    /// TL-90 через псевдоним типа). Цена: лишний вызов в продакшене не
    /// ломает процесс, а забирает флаг первым, и история на сеанс
    /// становится недоступна у того, кто пришёл вторым. Место единственного
    /// вызова пинает сторож по исходникам в `commands::history`.
    pub fn open(data_dir: &Path) -> Result<Self, HistoryOpenError> {
        if OPENED_IN_PROCESS.swap(true, Ordering::SeqCst) {
            return Err(HistoryOpenError::AlreadyOpen);
        }
        Self::open_with(data_dir, MIGRATIONS)
    }

    /// [`Self::open`] без процессного флага — только для тестов: открытий
    /// за тестовый процесс много, и порядок их не задан.
    #[cfg(test)]
    pub(crate) fn open_isolated(data_dir: &Path) -> Result<Self, HistoryOpenError> {
        Self::open_with(data_dir, MIGRATIONS)
    }

    /// То же, с явным списком миграций — чтобы тесты могли подложить
    /// миграцию, падающую посередине.
    fn open_with(data_dir: &Path, migrations: &[&str]) -> Result<Self, HistoryOpenError> {
        let path = data_dir.join(HISTORY_FILE_NAME);
        let supported = schema_version_of(migrations);
        let no_access = |reason: String| HistoryOpenError::NoAccess {
            path: path.clone(),
            reason,
        };

        fs::create_dir_all(data_dir)
            .map_err(|err| no_access(format!("каталог данных не создаётся: {err}")))?;

        // До любого вызова SQLite: только файловая система и байты заголовка.
        let header = inspect_header(&path)
            .map_err(|err| no_access(format!("файл базы не читается: {err}")))?;
        let wal_beside = exists_no_follow(&with_suffix(&path, WAL_SUFFIX))
            .map_err(|err| no_access(format!("спутник {WAL_SUFFIX} не проверяется: {err}")))?;
        let (header_version, wal_header) = match header {
            Header::Sqlite { user_version, wal } => (user_version, wal),
            Header::Absent | Header::Foreign => (0, false),
        };
        if wal_beside || wal_header {
            return Err(HistoryOpenError::NewerVersion {
                path,
                found: header_version,
                supported,
                wal: true,
            });
        }

        let mut recreated = false;
        match header {
            Header::Absent => {}
            Header::Sqlite { user_version, .. } => {
                refuse_foreign_version(&path, user_version, supported)?;
            }
            Header::Foreign => {
                set_aside(&path, &broken_label()).map_err(|err| {
                    no_access(format!("испорченная база не откладывается: {err}"))
                })?;
                recreated = true;
            }
        }

        let mut conn = match connect(&path, supported) {
            Ok(conn) => conn,
            Err(Connect::Corrupt(_)) if !recreated => {
                set_aside(&path, &broken_label()).map_err(|err| {
                    no_access(format!("испорченная база не откладывается: {err}"))
                })?;
                recreated = true;
                connect(&path, supported).map_err(|err| err.into_open_error(&path, supported))?
            }
            Err(err) => return Err(err.into_open_error(&path, supported)),
        };

        migrate(&mut conn, &path, migrations)?;

        Ok(Self {
            conn: Mutex::new(conn),
            pending: Mutex::new(PendingNotices {
                base_recreated: recreated,
                last_write_failed: None,
            }),
        })
    }

    /// Вставляет запись Done одной транзакцией и возвращает её id.
    ///
    /// Отказ — любой, включая проверку записи — сам выставляет пометку
    /// [`HistoryNotice::LastWriteFailed`].
    #[allow(dead_code)] // зовёт оркестрация в момент Done — TL-89
    pub fn insert(&self, record: &NewHistoryRecord) -> Result<RecordId, HistoryWriteError> {
        let result = self.try_insert(record);
        if let Err(err) = &result {
            self.record_write_failure(err.failure());
        }
        result
    }

    #[allow(dead_code)] // путь записи — TL-89
    fn try_insert(&self, record: &NewHistoryRecord) -> Result<RecordId, HistoryWriteError> {
        let row = ValidRow::check(record)?;
        let mut conn = self.connection();
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        let id = insert_row(&tx, &row).map_err(storage_error)?;
        tx.commit().map_err(storage_error)?;
        Ok(id)
    }

    /// Запоминает, что запись Done не сохранилась по причине, случившейся
    /// до [`Self::insert`] (для TL-89). Последняя причина вытесняет прежнюю.
    #[allow(dead_code)] // зовёт оркестрация — TL-89
    pub fn record_write_failure(&self, cause: HistoryWriteFailure) {
        self.pending_notices().last_write_failed = Some(cause);
    }

    /// Порция истории размером [`HISTORY_PAGE_SIZE`].
    ///
    /// Без курсора — первая порция и все ждущие пометки (они гасятся только
    /// после успешного чтения). С курсором — порция строго после него,
    /// пометок нет и они не гасятся.
    pub fn page(
        &self,
        cursor: Option<&HistoryCursor>,
    ) -> Result<HistoryRecordsPage, HistoryStorageError> {
        self.page_sized(cursor, HISTORY_PAGE_SIZE)
    }

    fn page_sized(
        &self,
        cursor: Option<&HistoryCursor>,
        size: usize,
    ) -> Result<HistoryRecordsPage, HistoryStorageError> {
        let limit = i64::try_from(size.saturating_add(1)).unwrap_or(i64::MAX);
        let rows = match cursor {
            None => self.query_rows(PAGE_FIRST_SQL, params![limit])?,
            Some(cursor) => match cursor_key(cursor) {
                Some((finished, id)) => {
                    self.query_rows(PAGE_AFTER_SQL, params![finished, id, limit])?
                }
                None => {
                    return Ok(HistoryRecordsPage {
                        records: Vec::new(),
                        next_cursor: None,
                        notices: Vec::new(),
                    })
                }
            },
        };

        let mut rows = rows;
        let next_cursor = if rows.len() > size {
            rows.truncate(size);
            rows.last().map(|last| HistoryCursor {
                finished_at_unix_secs: last.finished_at_unix_secs,
                id: RecordId(last.id).to_string(),
            })
        } else {
            None
        };
        let records = rows.into_iter().map(StoredRow::into_record).collect();
        let notices = if cursor.is_none() {
            self.take_notices()
        } else {
            Vec::new()
        };

        Ok(HistoryRecordsPage {
            records,
            next_cursor,
            notices,
        })
    }

    /// Одна запись по строковому id. Не id, выданный ядром, — `None`.
    pub fn get(&self, id: &str) -> Result<Option<HistoryRecord>, HistoryStorageError> {
        let Some(id) = RecordId::parse(id) else {
            return Ok(None);
        };
        let row = self
            .connection()
            .query_row(GET_SQL, [id.get()], StoredRow::decode)
            .optional()
            .map_err(storage_error)?;
        Ok(row.map(StoredRow::into_record))
    }

    /// Удаляет одну запись (Ф-6). Файлы на диске не трогает.
    pub fn delete(&self, id: &str) -> Result<(), HistoryDeleteError> {
        let unknown = || HistoryDeleteError::UnknownRecord { id: id.to_owned() };
        let key = RecordId::parse(id).ok_or_else(unknown)?;
        let changed = self
            .connection()
            .execute("DELETE FROM history WHERE id = ?1", [key.get()])
            .map_err(storage_error)?;
        if changed == 0 {
            return Err(unknown());
        }
        Ok(())
    }

    /// Очищает историю одной транзакцией (Ф-6). Файлы на диске не трогает;
    /// счётчик `AUTOINCREMENT` не сбрасывается, id после очистки не
    /// повторяются.
    pub fn clear(&self) -> Result<(), HistoryStorageError> {
        let mut conn = self.connection();
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        tx.execute("DELETE FROM history", [])
            .map_err(storage_error)?;
        tx.commit().map_err(storage_error)
    }

    fn query_rows(
        &self,
        sql: &str,
        params: &[&dyn rusqlite::ToSql],
    ) -> Result<Vec<StoredRow>, HistoryStorageError> {
        let conn = self.connection();
        let mut statement = conn.prepare(sql).map_err(storage_error)?;
        let rows = statement
            .query_map(params, StoredRow::decode)
            .map_err(storage_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage_error)?;
        Ok(rows)
    }

    fn take_notices(&self) -> Vec<HistoryNotice> {
        let pending = std::mem::take(&mut *self.pending_notices());
        let mut notices = Vec::new();
        if pending.base_recreated {
            notices.push(HistoryNotice::BaseRecreated);
        }
        if let Some(cause) = pending.last_write_failed {
            notices.push(HistoryNotice::LastWriteFailed { cause });
        }
        notices
    }

    /// Отравленный мьютекс не повод потерять историю: транзакция соседа,
    /// упавшего с паникой, откатилась при раскрутке стека.
    fn connection(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn pending_notices(&self) -> MutexGuard<'_, PendingNotices> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Вставка набора записей одной транзакцией — только для тестов
    /// порядка на больших наборах, где транзакция на запись стоила бы
    /// тысяч `fsync`. Путь вставки тот же: [`ValidRow::check`] и
    /// [`insert_row`].
    #[cfg(test)]
    fn insert_all(&self, records: &[NewHistoryRecord]) -> Vec<RecordId> {
        let mut conn = self.connection();
        let tx = conn.transaction().expect("транзакция");
        let ids = records
            .iter()
            .map(|record| {
                let row = ValidRow::check(record).expect("запись проходит проверку");
                insert_row(&tx, &row).expect("вставка")
            })
            .collect();
        tx.commit().expect("фиксация");
        ids
    }

    #[cfg(test)]
    fn with_connection<T>(&self, f: impl FnOnce(&Connection) -> T) -> T {
        f(&self.connection())
    }
}

/// Проверенная запись, готовая к вставке.
#[allow(dead_code)] // путь записи — TL-89
struct ValidRow<'a> {
    record: &'a NewHistoryRecord,
    folder: &'a str,
    size_bytes: i64,
    finished_at: i64,
}

impl<'a> ValidRow<'a> {
    #[allow(dead_code)] // путь записи — TL-89
    fn check(record: &'a NewHistoryRecord) -> Result<Self, HistoryWriteError> {
        let invalid = |reason| HistoryWriteError::InvalidRecord { reason };
        if !is_single_file_name(&record.file_name) {
            return Err(invalid("имя файла — не одно имя без пути"));
        }
        if !record.folder.is_absolute() {
            return Err(invalid("папка — не абсолютный путь"));
        }
        let folder = record
            .folder
            .to_str()
            .ok_or_else(|| invalid("путь папки не в UTF-8"))?;
        let size_bytes =
            i64::try_from(record.size_bytes).map_err(|_| invalid("размер вне предела i64"))?;
        let finished_at = i64::try_from(record.finished_at_unix_secs)
            .map_err(|_| invalid("время завершения вне предела i64"))?;
        Ok(Self {
            record,
            folder,
            size_bytes,
            finished_at,
        })
    }
}

#[allow(dead_code)] // путь записи — TL-89
fn insert_row(tx: &Transaction<'_>, row: &ValidRow<'_>) -> rusqlite::Result<RecordId> {
    let record = row.record;
    tx.execute(
        INSERT_SQL,
        params![
            record.video_id,
            record.url,
            record.title,
            kind_to_column(record.quality.kind),
            record.quality.height_px,
            record.file_name,
            row.folder,
            row.size_bytes,
            row.finished_at,
        ],
    )?;
    Ok(RecordId(tx.last_insert_rowid()))
}

/// Строка таблицы, разобранная из базы, до вычисления статуса файла.
struct StoredRow {
    id: i64,
    video_id: String,
    url: String,
    title: String,
    quality: SelectedQuality,
    file_name: String,
    folder: String,
    size_bytes: u64,
    finished_at_unix_secs: u64,
}

impl StoredRow {
    fn decode(row: &Row<'_>) -> rusqlite::Result<Self> {
        let kind_text: String = row.get(4)?;
        let kind = kind_from_column(&kind_text)
            .ok_or_else(|| conversion_failure(4, Type::Text, "неизвестный вид качества"))?;
        let size: i64 = row.get(8)?;
        let finished: i64 = row.get(9)?;
        Ok(Self {
            id: row.get(0)?,
            video_id: row.get(1)?,
            url: row.get(2)?,
            title: row.get(3)?,
            quality: SelectedQuality {
                kind,
                height_px: row.get(5)?,
            },
            file_name: row.get(6)?,
            folder: row.get(7)?,
            size_bytes: u64::try_from(size)
                .map_err(|_| conversion_failure(8, Type::Integer, "отрицательный размер"))?,
            finished_at_unix_secs: u64::try_from(finished)
                .map_err(|_| conversion_failure(9, Type::Integer, "отрицательное время"))?,
        })
    }

    fn into_record(self) -> HistoryRecord {
        let folder = PathBuf::from(self.folder);
        let file_status = file_status(&folder, &self.file_name);
        HistoryRecord {
            id: RecordId(self.id),
            video_id: self.video_id,
            url: self.url,
            title: self.title,
            quality: self.quality,
            file_name: self.file_name,
            folder,
            size_bytes: self.size_bytes,
            finished_at_unix_secs: self.finished_at_unix_secs,
            file_status,
        }
    }
}

fn conversion_failure(column: usize, kind: Type, what: &'static str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(column, kind, what.into())
}

#[allow(dead_code)] // путь записи — TL-89
fn kind_to_column(kind: QualityKind) -> &'static str {
    match kind {
        QualityKind::Standard => "standard",
        QualityKind::MaxAvailable => "maxAvailable",
        QualityKind::AudioOnly => "audioOnly",
    }
}

fn kind_from_column(text: &str) -> Option<QualityKind> {
    match text {
        "standard" => Some(QualityKind::Standard),
        "maxAvailable" => Some(QualityKind::MaxAvailable),
        "audioOnly" => Some(QualityKind::AudioOnly),
        _ => None,
    }
}

/// Имя — ровно один обычный компонент пути, без разделителей обеих ОС.
///
/// Разделители проверяются явно, а не только разбором `Path`: на Unix `\`
/// — обычный символ имени, а на Windows — разделитель, и база не должна
/// значить разное на разных ОС.
fn is_single_file_name(name: &str) -> bool {
    if name.contains(['/', '\\']) {
        return false;
    }
    let mut components = Path::new(name).components();
    matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(only)), None) if only == OsStr::new(name)
    )
}

fn record_file_path(folder: &Path, file_name: &str) -> Option<PathBuf> {
    (folder.is_absolute() && is_single_file_name(file_name)).then(|| folder.join(file_name))
}

/// Статус файла записи: только `metadata`, без чтения содержимого.
fn file_status(folder: &Path, file_name: &str) -> HistoryFileStatus {
    let present = record_file_path(folder, file_name)
        .is_some_and(|path| fs::metadata(path).is_ok_and(|meta| meta.is_file()));
    if present {
        HistoryFileStatus::Present
    } else {
        HistoryFileStatus::Missing {
            folder_exists: folder.is_absolute()
                && fs::metadata(folder).is_ok_and(|meta| meta.is_dir()),
        }
    }
}

/// Мог ли ядро выдать этот курсор. `false` — [`HistoryStore::page`] ответит
/// на него пустой страницей; команда (TL-90) пишет такой случай в лог.
pub fn is_issued_cursor(cursor: &HistoryCursor) -> bool {
    cursor_key(cursor).is_some()
}

fn cursor_key(cursor: &HistoryCursor) -> Option<(i64, i64)> {
    let finished = i64::try_from(cursor.finished_at_unix_secs).ok()?;
    let id = RecordId::parse(&cursor.id)?;
    Some((finished, id.get()))
}

fn schema_version_of(migrations: &[&str]) -> u32 {
    u32::try_from(migrations.len()).unwrap_or(u32::MAX)
}

/// Что сказал заголовок файла до SQLite.
#[derive(Debug, PartialEq, Eq)]
enum Header {
    /// Файла нет или он пуст — SQLite заведёт базу с нуля.
    Absent,
    /// Заголовок SQLite 3 с этой `user_version`; `wal` — байт 18 или 19
    /// (версия формата записи или чтения) равен 2.
    Sqlite { user_version: i64, wal: bool },
    /// Не SQLite: чужая магическая строка или файл короче заголовка.
    Foreign,
}

fn inspect_header(path: &Path) -> io::Result<Header> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Header::Absent),
        Err(err) => return Err(err),
    };
    let mut head = Vec::with_capacity(HEADER_LEN);
    file.take(HEADER_LEN as u64).read_to_end(&mut head)?;
    if head.is_empty() {
        return Ok(Header::Absent);
    }
    if head.len() < HEADER_LEN || !head.starts_with(SQLITE_MAGIC) {
        return Ok(Header::Foreign);
    }
    let Some(bytes) = head
        .get(USER_VERSION_OFFSET..USER_VERSION_OFFSET + 4)
        .and_then(|slice| <[u8; 4]>::try_from(slice).ok())
    else {
        return Ok(Header::Foreign);
    };
    let wal = FILE_FORMAT_OFFSETS
        .iter()
        .any(|&offset| head.get(offset) == Some(&WAL_FILE_FORMAT));
    Ok(Header::Sqlite {
        user_version: i64::from(i32::from_be_bytes(bytes)),
        wal,
    })
}

/// Единственное место, где решается «версия наша или нет».
///
/// Зовётся дважды: по заголовку до SQLite и по `PRAGMA user_version` после
/// открытия — страховка, что SQLite видит ту же версию, что и заголовок.
///
/// Отрицательная `user_version` — тоже [`HistoryOpenError::NewerVersion`].
/// Эта сборка такую не пишет, и отказ консервативен: файл сохраняется, а не
/// откладывается и не переписывается. Текст отказа для такого файла
/// неточен: «чужой версии схемы -1, эта сборка знает версии до 1» звучит как
/// «база новее», хотя она просто не наша. Отдельной причины для этого в
/// контракте нет.
fn refuse_foreign_version(path: &Path, found: i64, supported: u32) -> Result<(), HistoryOpenError> {
    match u32::try_from(found) {
        Ok(version) if version <= supported => Ok(()),
        _ => Err(HistoryOpenError::NewerVersion {
            path: path.to_path_buf(),
            found,
            supported,
            wal: false,
        }),
    }
}

/// Отказ подключения к файлу, до миграций.
#[derive(Debug)]
enum Connect {
    Corrupt(String),
    Foreign(HistoryOpenError),
    NoAccess(String),
}

impl Connect {
    fn from_sqlite(err: rusqlite::Error) -> Self {
        match err.sqlite_error_code() {
            Some(ErrorCode::NotADatabase | ErrorCode::DatabaseCorrupt) => {
                Self::Corrupt(err.to_string())
            }
            _ => Self::NoAccess(err.to_string()),
        }
    }

    fn into_open_error(self, path: &Path, supported: u32) -> HistoryOpenError {
        match self {
            Self::Foreign(err) => err,
            Self::NoAccess(reason) => HistoryOpenError::NoAccess {
                path: path.to_path_buf(),
                reason,
            },
            // Порча, пережившая откладывание, — это уже не файл, а том.
            Self::Corrupt(reason) => HistoryOpenError::NoAccess {
                path: path.to_path_buf(),
                reason: format!("новая база схемы {supported} не открылась: {reason}"),
            },
        }
    }
}

fn connect(path: &Path, supported: u32) -> Result<Connection, Connect> {
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
        | OpenFlags::SQLITE_OPEN_CREATE
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(path, flags).map_err(Connect::from_sqlite)?;

    // Файл без права записи SQLite молча открывает только на чтение, и отказ
    // случился бы на первой записи. Называем его сразу.
    if conn.is_readonly(MAIN_DB).map_err(Connect::from_sqlite)? {
        return Err(Connect::NoAccess(
            "файл базы открывается только на чтение".to_owned(),
        ));
    }

    let found = user_version(&conn).map_err(Connect::from_sqlite)?;
    refuse_foreign_version(path, found, supported).map_err(Connect::Foreign)?;

    let verdicts = integrity_check(&conn).map_err(Connect::from_sqlite)?;
    if verdicts != ["ok"] {
        return Err(Connect::Corrupt(verdicts.join("; ")));
    }
    Ok(conn)
}

fn user_version(conn: &Connection) -> rusqlite::Result<i64> {
    conn.pragma_query_value(None, "user_version", |row| row.get(0))
}

fn integrity_check(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut statement = conn.prepare("PRAGMA integrity_check")?;
    let verdicts = statement
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<String>, _>>()?;
    Ok(verdicts)
}

fn migrate(
    conn: &mut Connection,
    path: &Path,
    migrations: &[&str],
) -> Result<(), HistoryOpenError> {
    let found = user_version(conn).map_err(|err| HistoryOpenError::NoAccess {
        path: path.to_path_buf(),
        reason: err.to_string(),
    })?;
    // Версия уже сверена `connect`: здесь она в 0..=len.
    let start = usize::try_from(found).unwrap_or(usize::MAX);

    for (index, sql) in migrations.iter().enumerate().skip(start) {
        let from = u32::try_from(index).unwrap_or(u32::MAX);
        let to = from.saturating_add(1);
        apply_migration(conn, sql, to).map_err(|err| {
            if classify(&err) == StorageFailure::NoAccess {
                HistoryOpenError::NoAccess {
                    path: path.to_path_buf(),
                    reason: format!("миграция {from} → {to}: {err}"),
                }
            } else {
                HistoryOpenError::MigrationFailed {
                    path: path.to_path_buf(),
                    from,
                    to,
                    reason: err.to_string(),
                }
            }
        })?;
    }
    Ok(())
}

/// Один шаг миграции: схема и номер версии в одной транзакции. Отказ
/// любого оператора роняет транзакцию при выходе из функции — откат.
fn apply_migration(conn: &mut Connection, sql: &str, to: u32) -> rusqlite::Result<()> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute_batch(sql)?;
    tx.pragma_update(None, "user_version", to)?;
    tx.commit()
}

fn classify(err: &rusqlite::Error) -> StorageFailure {
    match err.sqlite_error_code() {
        Some(ErrorCode::DiskFull) => StorageFailure::DiskFull,
        Some(ErrorCode::ReadOnly | ErrorCode::CannotOpen | ErrorCode::PermissionDenied) => {
            StorageFailure::NoAccess
        }
        _ => StorageFailure::Other,
    }
}

fn storage_error(err: rusqlite::Error) -> HistoryStorageError {
    HistoryStorageError {
        failure: classify(&err),
        detail: err.to_string(),
    }
}

fn broken_label() -> String {
    crate::clock::now_unix_secs().to_string()
}

/// Откладывает файл базы вместе со спутниками под свободное имя
/// `history.sqlite.broken-<метка>[-N]` и возвращает это имя.
///
/// Спутники переезжают первыми. Если не переедет сам файл, уже
/// перенесённые спутники возвращаются на место, чтобы пара «база — журнал»
/// не разошлась.
fn set_aside(path: &Path, label: &str) -> io::Result<PathBuf> {
    let target = free_broken_name(path, label)?;
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();

    for suffix in SIDE_FILE_SUFFIXES {
        let from = with_suffix(path, suffix);
        if !exists_no_follow(&from)? {
            continue;
        }
        let to = with_suffix(&target, suffix);
        if let Err(err) = fs::rename(&from, &to) {
            undo_moves(&moved);
            return Err(err);
        }
        moved.push((from, to));
    }

    if let Err(err) = fs::rename(path, &target) {
        undo_moves(&moved);
        return Err(err);
    }
    Ok(target)
}

fn undo_moves(moved: &[(PathBuf, PathBuf)]) {
    for (from, to) in moved {
        let _ = fs::rename(to, from);
    }
}

/// Имя, не занятое ни файлом, ни одним из его спутников. `rename` на Unix
/// молча заменяет существующий файл, поэтому занятость проверяется явно:
/// прежняя отложенная копия затёрта не будет. Гонки нет — писатель один.
fn free_broken_name(path: &Path, label: &str) -> io::Result<PathBuf> {
    let base = with_suffix(path, &format!("{BROKEN_MARKER}{label}"));
    for attempt in 0..1000_u32 {
        let candidate = if attempt == 0 {
            base.clone()
        } else {
            with_suffix(&base, &format!("-{attempt}"))
        };
        let mut taken = exists_no_follow(&candidate)?;
        for suffix in SIDE_FILE_SUFFIXES {
            taken |= exists_no_follow(&with_suffix(&candidate, suffix))?;
        }
        if !taken {
            return Ok(candidate);
        }
    }
    Err(io::Error::other(
        "свободное имя для отложенной базы не нашлось за 1000 попыток",
    ))
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn exists_no_follow(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;
