//! Снимок очереди на диске: чтение, атомарная запись, типизированные
//! отказы (TL-71, Ф-9 и решение Р-2 эпика E4).
//!
//! Формат сюда не входит — он объявлен соседним модулем
//! [`super::snapshot`] контрактной задачей TL-70, и второй его копии здесь
//! нет намеренно: два описания одного файла расходятся на первой же
//! правке. Здесь только ввод-вывод: где файл лежит, как он появляется
//! целиком и что происходит, когда его нет, он испорчен или написан
//! чужой версией приложения.
//!
//! # Почему запись атомарна
//!
//! Половина JSON под именем снимка — это не «часть очереди», а очередь,
//! которая не читается вовсе: разбор упадёт на обрезанном файле целиком,
//! и пользователь потеряет весь список, а не последнюю задачу. Поэтому
//! данные всегда пишутся в соседний временный файл и попадают под рабочее
//! имя одним `rename` (приём манифеста установки,
//! `ytdlp::layout::write_json_atomic`, и settings). В любой момент на
//! диске лежит либо прежний снимок целиком, либо новый целиком.
//!
//! Дополнительно к приёму манифеста здесь есть `sync_all` перед
//! переименованием: очередь маленькая и пишется редко (единицы записей на
//! сеанс, Н-2), поэтому сброс на диск ничего не стоит, а без него `rename`
//! мог бы опубликовать имя раньше содержимого. Каталог после `rename` не
//! синхронизируется: сама запись каталога — единственное, что осталось
//! незакреплённым, и её потеря при отказе питания даёт **прежний** снимок,
//! то есть то же состояние, что и отказ записи. Терять при этом нечего:
//! пропавшая задача добавляется заново, а испорченный список — нет.
//!
//! # Двух писателей не бывает
//!
//! Опора не на удачу, а на TL-20: приложение держит эксклюзивную
//! блокировку файла в каталоге данных и вторым экземпляром не
//! запускается (`crate::single_instance`). Поэтому здесь нет ни
//! блокировки файла снимка, ни разрешения конфликтов: единственный
//! оставшийся конкурент за временный файл — **предыдущий, убитый**
//! процесс этого же приложения, и его мусор убирается при чтении.
//!
//! # Момент записи
//!
//! Ф-9 требует писать при **структурных** изменениях очереди (добавление,
//! старт, терминальный исход, отмена) и не писать на каждое событие
//! прогресса. Кто вызывает запись, решает планировщик (TL-73), но
//! дисциплина здесь не оставлена одним лишь комментарием: хранилище
//! помнит байты последней удавшейся записи и на совпадающем составе
//! возвращает [`Saved::Unchanged`], не трогая диск. В снимок не входит
//! ни прогресс, ни счётчик попыток (TL-70), поэтому событие прогресса
//! состав изменить не может — и, чем бы ни был вызван `save`, записи от
//! него не произойдёт. Проверяется это наблюдаемым фактом: файл после
//! такого вызова остаётся тем же байт в байт.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use serde::Deserialize;

use super::snapshot::{QueueSnapshotEntry, QueueSnapshotFile, SNAPSHOT_FORMAT_VERSION};

/// Имя файла-снимка в каталоге данных приложения.
///
/// Лежит в корне каталога данных — рядом с замком единственности и
/// **вне** `yt-dlp/`: уборка контура обновления (E6) обходит корень
/// установок и сносит там всё, что не принадлежит сохраняемой сборке
/// (`ytdlp::layout::Layout::belongs_to`). Очереди в этом каталоге делать
/// нечего.
pub const SNAPSHOT_FILE_NAME: &str = "queue.json";

/// Имя временного файла, из которого снимок переименовывается на место.
///
/// Имя постоянное, а не случайное: случайное после падения процесса
/// пришлось бы искать шаблоном, а постоянное убирается при чтении одной
/// строкой. Писателей больше одного не бывает (TL-20), поэтому
/// столкнуться за это имя некому.
pub const SNAPSHOT_TEMP_FILE_NAME: &str = "queue.json.tmp";

/// Почему снимок не прочитан.
///
/// Отсутствия файла в перечислении нет, и это решение, а не пропуск:
/// «файла нет» — не отказ, а пустая очередь (первый запуск, чистая
/// установка), и [`SnapshotStore::load`] возвращает на него `Ok` с пустым
/// списком. Всё остальное — настоящие отказы: они не роняют старт (Р-2:
/// восстанавливать нечего — начинаем с пустой очереди), но вызывающий
/// обязан иметь возможность назвать причину в логе, а не свести три
/// разных случая к одному молчанию.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SnapshotReadError {
    /// Файл есть, но не читается: права, каталог вместо файла, отказ тома.
    #[error("снимок очереди {path} не читается: {reason}")]
    Unreadable {
        /// Путь к снимку.
        path: PathBuf,
        /// Текст системной ошибки.
        reason: String,
    },
    /// Файл прочитан, но не разбирается: обрезан, испорчен, без версии.
    #[error("снимок очереди {path} не разбирается: {reason}")]
    Malformed {
        /// Путь к снимку.
        path: PathBuf,
        /// Текст ошибки разбора.
        reason: String,
    },
    /// В файле чужая версия формата.
    ///
    /// Отдельно от [`Self::Malformed`] намеренно. Это не порча, а откат
    /// приложения на версию назад: файл написан кодом, который знал
    /// больше нашего. Понимать его мы не пытаемся (доктрина
    /// [`SNAPSHOT_FORMAT_VERSION`]), но и переписывать при чтении не
    /// станем — очередь пользователя вернётся к нему, когда он вернётся
    /// на новую сборку. Здесь же — место будущей миграции: сейчас
    /// известна одна версия, и ветки перевода нет.
    #[error(
        "снимок очереди {path} чужой версии формата: в файле {found}, \
         эта сборка читает {expected}"
    )]
    ForeignVersion {
        /// Путь к снимку.
        path: PathBuf,
        /// Версия, записанная в файле.
        found: u32,
        /// Версия, которую понимает эта сборка.
        expected: u32,
    },
}

/// Почему снимок не записан.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SnapshotWriteError {
    /// Состав очереди не сериализуется.
    ///
    /// Практически недостижимо (структура из строк и чисел), но
    /// `unwrap()` в продакшен-пути недопустим, а `expect` здесь означал
    /// бы падение приложения из-за задачи в очереди.
    #[error("снимок очереди не сериализуется: {reason}")]
    Serialize {
        /// Текст ошибки сериализации.
        reason: String,
    },
    /// Отказ файловой системы на любом шаге записи.
    ///
    /// Шаг не разносится по вариантам сознательно: делать с ними
    /// вызывающему нечего — состояние диска после любого из них одно и то
    /// же (прежний снимок цел, временного файла нет), а разница между
    /// «не создался временный» и «не переименовался» интересна только
    /// логу, куда она и попадает текстом.
    #[error("снимок очереди {path} не записан: {reason}")]
    Io {
        /// Путь к снимку.
        path: PathBuf,
        /// Текст системной ошибки.
        reason: String,
    },
}

/// Что сделала [`SnapshotStore::save`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Saved {
    /// Состав изменился — снимок переписан.
    Written,
    /// Состав тот же, что в последней удавшейся записи, — диск не тронут.
    Unchanged,
}

/// Файл-снимок очереди в каталоге данных приложения.
///
/// Владеет путями и памятью о последней записи; составом очереди не
/// владеет и о состояниях задач не знает — какие задачи нетерминальны и
/// когда изменение структурно, решает планировщик (TL-73).
#[derive(Debug)]
pub struct SnapshotStore {
    path: PathBuf,
    temp_path: PathBuf,
    /// Байты последней удавшейся записи.
    ///
    /// Заполняется **только** успешной записью: после отказа память
    /// остаётся прежней, и следующий `save` тем же составом попробует
    /// снова, а не сочтёт его уже лежащим на диске. Чтением не
    /// заполняется тоже — первая запись за сеанс всегда настоящая.
    last_written: Mutex<Option<Vec<u8>>>,
}

impl SnapshotStore {
    /// Хранилище в каталоге данных приложения
    /// (`AppHandle::path().app_data_dir()`).
    pub fn new(data_dir: &Path) -> Self {
        Self {
            path: data_dir.join(SNAPSHOT_FILE_NAME),
            temp_path: data_dir.join(SNAPSHOT_TEMP_FILE_NAME),
            last_written: Mutex::new(None),
        }
    }

    /// Путь к файлу-снимку.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Путь к временному файлу записи.
    ///
    /// Спрашивают его только собственные тесты хранилища — те, что
    /// проверяют наблюдаемым фактом: временный файл не переживает ни
    /// удавшуюся запись, ни отказавшую. Планировщику он не нужен вовсе,
    /// поэтому вне тестов метода нет — иначе `dead_code` в этом модуле
    /// снова перестал бы что-либо означать (урок глушителя, снятого в
    /// TL-73).
    #[cfg(test)]
    pub fn temp_path(&self) -> &Path {
        &self.temp_path
    }

    /// Читает снимок. Отсутствие файла — пустая очередь, а не отказ.
    ///
    /// Заодно убирает временный файл: пережить запись он не может (см.
    /// [`Self::save`]), поэтому найденный на старте — мусор от убитого
    /// процесса, а не чьё-то незаконченное дело (писателей больше одного
    /// не бывает, TL-20).
    pub fn load(&self) -> Result<Vec<QueueSnapshotEntry>, SnapshotReadError> {
        let _ = fs::remove_file(&self.temp_path);

        let raw = match fs::read(&self.path) {
            Ok(raw) => raw,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => {
                return Err(SnapshotReadError::Unreadable {
                    path: self.path.clone(),
                    reason: err.to_string(),
                })
            }
        };

        // Версия читается отдельным проходом по тем же байтам, до разбора
        // файла целиком. Иначе снимок будущей версии, у которого
        // изменилась форма задачи, назывался бы «испорченным» — и код,
        // который однажды научится его переводить, не узнал бы, что
        // переводить есть что.
        let probe: VersionProbe =
            serde_json::from_slice(&raw).map_err(|err| SnapshotReadError::Malformed {
                path: self.path.clone(),
                reason: err.to_string(),
            })?;
        if probe.version != SNAPSHOT_FORMAT_VERSION {
            return Err(SnapshotReadError::ForeignVersion {
                path: self.path.clone(),
                found: probe.version,
                expected: SNAPSHOT_FORMAT_VERSION,
            });
        }

        let file: QueueSnapshotFile =
            serde_json::from_slice(&raw).map_err(|err| SnapshotReadError::Malformed {
                path: self.path.clone(),
                reason: err.to_string(),
            })?;

        Ok(file.tasks)
    }

    /// Записывает состав очереди — атомарно, через временный файл и
    /// `rename`.
    ///
    /// Вызывается на структурных изменениях очереди (Ф-9). На составе,
    /// совпадающем с последней удавшейся записью, возвращает
    /// [`Saved::Unchanged`] и не трогает ни одного файла.
    ///
    /// После отказа на диске остаётся прежний снимок целиком и **ни
    /// одного** временного файла: недописанный убирается тем же
    /// обработчиком, который возвращает ошибку.
    pub fn save(&self, tasks: &[QueueSnapshotEntry]) -> Result<Saved, SnapshotWriteError> {
        let file = QueueSnapshotFile {
            version: SNAPSHOT_FORMAT_VERSION,
            tasks: tasks.to_vec(),
        };
        let json =
            serde_json::to_vec_pretty(&file).map_err(|err| SnapshotWriteError::Serialize {
                reason: err.to_string(),
            })?;

        // Отравленный мьютекс не повод потерять очередь: значение под ним
        // — кэш, а не инвариант, и худшее, что даёт его чтение после
        // паники соседа, — лишняя запись того же состава.
        let mut last = self
            .last_written
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if last.as_deref() == Some(json.as_slice()) {
            return Ok(Saved::Unchanged);
        }

        write_atomic(&self.path, &self.temp_path, &json).map_err(|err| {
            let _ = fs::remove_file(&self.temp_path);
            SnapshotWriteError::Io {
                path: self.path.clone(),
                reason: err.to_string(),
            }
        })?;

        *last = Some(json);
        Ok(Saved::Written)
    }
}

/// Только версия формата — для первого прохода по байтам файла.
///
/// Поля задач здесь нет: их форма в чужой версии может быть какой угодно,
/// и требовать её разбора значило бы разбирать то, что мы всё равно
/// откажемся понимать.
#[derive(Debug, Deserialize)]
struct VersionProbe {
    version: u32,
}

/// Кладёт байты под именем `path` целиком или не кладёт вовсе.
///
/// Уборка временного файла — на вызывающем: он же формирует отказ, и
/// разводить эти два действия по разным местам значило бы завести путь,
/// на котором ошибка вернулась, а мусор остался.
fn write_atomic(path: &Path, temp_path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = File::create(temp_path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);

    fs::rename(temp_path, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        QualityKind, QualitySize, QualityStreams, SelectedQuality, StartDownloadRequest,
    };
    use tempfile::{tempdir, TempDir};

    fn entry(id: &str) -> QueueSnapshotEntry {
        QueueSnapshotEntry {
            task_id: id.to_string(),
            request: StartDownloadRequest {
                url: format!("https://www.youtube.com/watch?v={id}"),
                title: format!("Ролик {id}"),
                quality: SelectedQuality {
                    kind: QualityKind::Standard,
                    height_px: Some(1080),
                },
                streams: QualityStreams {
                    video_format_id: Some("137".to_string()),
                    audio_format_id: Some("140".to_string()),
                },
                size: QualitySize::Known { bytes: 303_038_464 },
            },
        }
    }

    fn store() -> (TempDir, SnapshotStore) {
        let dir = tempdir().expect("временный каталог");
        let store = SnapshotStore::new(dir.path());
        (dir, store)
    }

    /// Первый запуск: файла нет — очередь пуста, и это не отказ.
    #[test]
    fn a_missing_snapshot_is_an_empty_queue_and_not_a_failure() {
        let (_dir, store) = store();

        assert_eq!(store.load(), Ok(Vec::new()));
    }

    /// Каталога данных ещё нет вовсе — тот же случай, что и файла нет.
    #[test]
    fn a_missing_data_directory_is_an_empty_queue_too() {
        let dir = tempdir().expect("временный каталог");
        let store = SnapshotStore::new(&dir.path().join("не-создан"));

        assert_eq!(store.load(), Ok(Vec::new()));
    }

    /// Состав и порядок переживают круг «записали — прочитали».
    ///
    /// Порядок здесь — не оформление, а очередь: он и есть порядок
    /// выполнения (Р-4).
    #[test]
    fn tasks_survive_a_write_and_a_read_in_the_same_order() {
        let (_dir, store) = store();
        let tasks = vec![entry("dl-1"), entry("dl-2"), entry("dl-3")];

        assert_eq!(store.save(&tasks), Ok(Saved::Written));
        assert_eq!(store.load(), Ok(tasks));
    }

    /// Пустая очередь пишется как пустая, а не как «нечего писать».
    #[test]
    fn an_empty_queue_is_written_and_read_back_empty() {
        let (_dir, store) = store();

        assert_eq!(store.save(&[]), Ok(Saved::Written));
        assert!(store.path().exists(), "снимок пустой очереди не создан");
        assert_eq!(store.load(), Ok(Vec::new()));
    }

    /// Испорченный файл — типизированный отказ, а не паника и не пустой
    /// список молча.
    #[test]
    fn a_malformed_snapshot_is_a_typed_failure() {
        let (_dir, store) = store();
        fs::write(store.path(), b"{ \"version\": 1, \"tasks\": [").expect("фикстура");

        match store.load() {
            Err(SnapshotReadError::Malformed { path, .. }) => assert_eq!(path, store.path()),
            other => panic!("обрезанный снимок разобран как {other:?}"),
        }
    }

    /// Файл без версии формата — тоже отказ разбора, а не «версия 0».
    #[test]
    fn a_snapshot_without_a_version_is_a_typed_failure() {
        let (_dir, store) = store();
        fs::write(store.path(), br#"{"tasks":[]}"#).expect("фикстура");

        assert!(
            matches!(store.load(), Err(SnapshotReadError::Malformed { .. })),
            "файл без версии формата не назван испорченным"
        );
    }

    /// Снимок будущей версии называет свою версию — и не назван порчей.
    ///
    /// Задачи в фикстуре нарочно чужой формы: так проверяется, что версия
    /// читается **до** разбора файла целиком. Если сверка версии уедет
    /// после полного разбора, этот файл станет «испорченным», и место
    /// будущей миграции окажется недостижимым — а откат приложения на
    /// версию назад молча превратится в потерю очереди.
    #[test]
    fn a_snapshot_of_a_foreign_version_names_its_version() {
        let (_dir, store) = store();
        fs::write(
            store.path(),
            r#"{"version":2,"tasks":[{"taskId":"dl-1","shape":"из будущего"}]}"#,
        )
        .expect("фикстура");

        assert_eq!(
            store.load(),
            Err(SnapshotReadError::ForeignVersion {
                path: store.path().to_path_buf(),
                found: 2,
                expected: SNAPSHOT_FORMAT_VERSION,
            })
        );
    }

    /// Чтение снимка чужой версии его не переписывает.
    ///
    /// Иначе откат на предыдущую сборку стирал бы очередь, собранную
    /// новой, — и обратно она бы уже не вернулась.
    #[test]
    fn reading_a_foreign_version_leaves_the_file_alone() {
        let (_dir, store) = store();
        let fixture = br#"{"version":2,"tasks":[]}"#;
        fs::write(store.path(), fixture).expect("фикстура");

        let _ = store.load();

        assert_eq!(fs::read(store.path()).expect("файл на месте"), fixture);
    }

    /// Отказ записи не оставляет временного файла.
    ///
    /// Отказ настоящий, а не подстроенный заглушкой: на месте снимка
    /// стоит каталог, и `rename` файла на него не сработает ни на одной
    /// поддерживаемой платформе.
    #[test]
    fn a_failed_write_leaves_no_temporary_file() {
        let (_dir, store) = store();
        fs::create_dir(store.path()).expect("каталог на месте снимка");

        let result = store.save(&[entry("dl-1")]);

        assert!(
            matches!(result, Err(SnapshotWriteError::Io { .. })),
            "запись поверх каталога сообщила об успехе: {result:?}"
        );
        assert!(
            !store.temp_path().exists(),
            "после отказа записи остался временный файл {}",
            store.temp_path().display()
        );
    }

    /// После отказа записи следующая попытка тем же составом — настоящая
    /// запись, а не «уже записано».
    ///
    /// Память о записанном заполняется только успехом; иначе первый же
    /// отказ вычеркнул бы состав из очереди на диске до конца сеанса.
    #[test]
    fn a_failed_write_is_retried_by_the_next_save() {
        let (_dir, store) = store();
        let tasks = vec![entry("dl-1")];
        fs::create_dir(store.path()).expect("каталог на месте снимка");
        assert!(store.save(&tasks).is_err(), "отказ не воспроизвёлся");

        fs::remove_dir(store.path()).expect("каталог убран");

        assert_eq!(store.save(&tasks), Ok(Saved::Written));
        assert_eq!(store.load(), Ok(tasks));
    }

    /// Отказ записи не портит уже лежащий снимок.
    ///
    /// Каталог снимка делается доступным только на чтение — тогда ни
    /// временный файл, ни `rename` в нём невозможны, а прежний файл
    /// читается. Проверяется наблюдаемый факт: на диске прежний снимок
    /// целиком и ни одного временного файла.
    #[cfg(unix)]
    #[test]
    fn a_failed_write_keeps_the_previous_snapshot_whole() {
        use std::os::unix::fs::PermissionsExt;

        let (dir, store) = store();
        let before = vec![entry("dl-1")];
        store.save(&before).expect("первая запись");

        let readonly = fs::Permissions::from_mode(0o555);
        let writable = fs::Permissions::from_mode(0o755);
        fs::set_permissions(dir.path(), readonly).expect("каталог только на чтение");

        let result = store.save(&[entry("dl-1"), entry("dl-2")]);

        let leftovers: Vec<PathBuf> = fs::read_dir(dir.path())
            .expect("каталог обходится")
            .filter_map(|item| item.ok().map(|item| item.path()))
            .filter(|path| path != store.path())
            .collect();
        let after = store.load();
        fs::set_permissions(dir.path(), writable).expect("права возвращены");

        assert!(
            matches!(result, Err(SnapshotWriteError::Io { .. })),
            "запись в каталог только на чтение сообщила об успехе: {result:?}"
        );
        assert_eq!(leftovers, Vec::<PathBuf>::new(), "на диске остался мусор");
        assert_eq!(after, Ok(before), "прежний снимок пострадал от отказа");
    }

    /// Прерванная запись (временный файл есть, `rename` не случился) не
    /// портит снимок, а её мусор не переживает чтение.
    ///
    /// Так выглядит диск после убитого процесса — единственный способ,
    /// которым временный файл вообще может дожить до следующего запуска.
    #[test]
    fn an_interrupted_write_neither_corrupts_nor_survives() {
        let (_dir, store) = store();
        let tasks = vec![entry("dl-1")];
        store.save(&tasks).expect("запись");
        fs::write(store.temp_path(), b"{\"version\":1,\"tasks\":[").expect("огрызок записи");

        assert_eq!(store.load(), Ok(tasks));
        assert!(
            !store.temp_path().exists(),
            "огрызок прерванной записи пережил чтение"
        );
    }

    /// Состав не изменился — диск не тронут.
    ///
    /// Это и есть Ф-9 «не на каждое событие прогресса», проверенная
    /// наблюдаемым фактом: содержимое файла подменяется меткой, и после
    /// повторного `save` тем же составом метка на месте — значит записи
    /// не было.
    #[test]
    fn saving_the_same_composition_does_not_touch_the_disk() {
        let (_dir, store) = store();
        let tasks = vec![entry("dl-1")];
        assert_eq!(store.save(&tasks), Ok(Saved::Written));

        let mark = "метка: этот файл никто не переписывал".as_bytes();
        fs::write(store.path(), mark).expect("метка");

        let outcome = store.save(&tasks);

        assert_eq!(
            fs::read(store.path()).expect("файл на месте"),
            mark,
            "снимок переписан на составе, который не менялся"
        );
        assert_eq!(outcome, Ok(Saved::Unchanged));
    }

    /// Структурное изменение записывается.
    #[test]
    fn a_structural_change_is_written() {
        let (_dir, store) = store();
        let one = vec![entry("dl-1")];
        let two = vec![entry("dl-1"), entry("dl-2")];

        assert_eq!(store.save(&one), Ok(Saved::Written));
        assert_eq!(store.save(&two), Ok(Saved::Written));
        assert_eq!(store.load(), Ok(two));

        // Снятие задачи (отмена, терминальный исход) — тоже структурное
        // изменение, и порядок остальных оно не трогает.
        assert_eq!(store.save(&one), Ok(Saved::Written));
        assert_eq!(store.load(), Ok(one));
    }
}
