//! Распаковка onedir-архива yt-dlp в каталог данных приложения (TL-12).
//!
//! # Целостность
//!
//! Каждая запись zip несёт CRC32, и он проверяется здесь самим фактом
//! дочитывания записи до конца: `zip::read::ZipFile` сверяет контрольную
//! сумму на EOF и возвращает ошибку при расхождении. Поэтому оборванный
//! или побитый архив не превращается в «успешно распакованное» дерево —
//! распаковка падает на первой же несошедшейся записи, а `.staging-*`
//! удаляется вызывающей стороной ([`super::prepare`]).
//!
//! Отдельной сверки sha256 архива на рантайме нет намеренно: контрольная
//! сумма из пина проверяется при доставке ассета
//! (`scripts/fetch-binaries`), сам архив приезжает внутри бандла и на
//! macOS/Windows покрыт подписью приложения, а CRC32 на каждой записи
//! ловит ту же порчу дешевле — по мере распаковки, без второго прохода по
//! 54 МиБ.
//!
//! **Этот вывод верен только для поставки архива внутри бандла и не
//! переносится в E6 как есть.** Он опирается ровно на одну посылку: за
//! архивом стоит подпись приложения, проверенная до запуска. У архива,
//! скачанного на рантайме (обновление yt-dlp отдельным контуром), такой
//! подписи нет, а CRC32 средством защиты не является вовсе — атакующий,
//! подменивший содержимое, пересчитает контрольные суммы записей вместе с
//! ним. Значит, переиспользуя этот модуль в E6, сверку sha256 скачанного
//! архива с доверенным источником надо делать **до** вызова [`unpack`].
//!
//! С TL-56 это и сделано, и здесь по-прежнему ничего не сверяется:
//! сверку выполняет [`super::fetch`] — до того, как позовёт установку.
//! Вызывать [`unpack`] с непроверенным архивом по-прежнему нельзя, и
//! ничто в этом модуле такому вызову не мешает.
//!
//! # Безопасность путей
//!
//! Имена внутри архива — недоверенный ввод (CLAUDE.md: «Ни один компонент
//! пути не строится из непроверенного ввода»). Используется
//! `ZipFile::enclosed_name`, который отвергает абсолютные пути, `..` и
//! прочие попытки выйти за каталог назначения; запись с таким именем — не
//! повод «почистить» путь и продолжить, а повод отказаться от архива
//! целиком.
//!
//! Записи-символические ссылки отвергаются тем же способом и по той же
//! причине: распакованная ссылка — это путь, по которому потом пройдёт
//! запись следующей записи архива, то есть обход проверки имён. Отказ
//! здесь явный (`ZipFile::is_symlink`), а не следствие того, что ссылка
//! записалась бы обычным файлом: апстримный ассет ссылок не содержит,
//! поэтому терять нечего, а неявная защита сломается при первой же
//! переделке цикла распаковки.
//!
//! Права на распакованные файлы **выводятся**, а не переносятся из
//! архива: единственное, что берётся из его метаданных, — бит выполнения
//! (см. [`apply_mode`]).
//!
//! # Границы объёма
//!
//! Распаковка ограничена сверху и по объёму (TL-18). Как и с именами,
//! опасное здесь не перечисляется — перечисляется **разрешённое**:
//! [`MAX_UNPACKED_BYTES`] и [`MAX_ENTRIES`] задают конверт, в который
//! обязано укладываться любое дерево yt-dlp, и всё, что за него выходит,
//! отвергается независимо от того, чем оно себя объявляет. Список
//! «подозрительных» архивов здесь не помог бы: бомба из одного
//! deflate-потока выглядит как обычная запись, и отличает её только
//! фактический объём.
//!
//! Ключевое свойство сторожа — направление, в котором на него влияют
//! заголовки. Разрешённый объём считается как `min(заявленный, потолок)`,
//! поэтому **заголовки могут границу только ужесточить, но не ослабить**.
//! Проверяется она по ходу записи и *до* того, как очередной кусок уйдёт
//! на диск: за границу не попадает даже тот байт, на котором её нарушили.
//! Верить заявленным размерам нельзя — `zip` ограничивает вывод записи
//! размером её сжатых данных, а не объявленным `size()`, так что
//! килобайтная запись законно разворачивается в гигабайт (проверено
//! тестом `refuses_a_deflate_bomb_that_lies_about_its_size`).
//!
//! Перед записью проверяется свободное место ([`ensure_room_for`]). Это
//! не сторож безопасности, а диагностика: без него забитый диск даёт
//! отказ записи посреди дерева, и пользователю остаётся `ENOSPC` в
//! «Подробнее» вместо «освободите место». Отказ типизирован отдельно
//! ([`PrepareError::NotEnoughSpace`]) именно поэтому: это единственная
//! причина отказа записи, из которой пользователь может выйти сам.

use std::fs::{self, File};
use std::io;
use std::path::{Component, Path, PathBuf};

use zip::ZipArchive;

use super::error::PrepareError;

/// Потолок суммарного объёма распакованного дерева.
///
/// # Откуда 512 МиБ
///
/// Из замера всех onedir-ассетов апстрима, а не из круглого числа.
/// Центральные каталоги релиза 2026.08.19 (того самого, что стоит в
/// `binaries.lock.json`) прочитаны диапазонными запросами 2026-08-27 и
/// дают такие деревья: macOS — 124,0 МиБ / 162 записи, linux — 91,5 /
/// 175, linux_aarch64 — 93,2 / 173, musllinux — 89,8 / 179, win — 29,6 /
/// 143, win_arm64 — 37,0 / 139. Четыре macOS-релиза за двадцать месяцев
/// назад (2024.12.03, 2025.09.05, 2025.12.08, 2026.02.04) — 143,7, 136,7,
/// 143,6 и 145,1 МиБ при 179–186 записях.
///
/// То есть самое большое дерево, которое апстрим когда-либо выпускал за
/// эти двадцать месяцев, — 145,1 МиБ, и колебалось оно в пределах ±17 %.
/// 512 МиБ — примерно трёхкратный запас над этим максимумом.
///
/// Запас выбран щедрым сознательно, и перекос именно в эту сторону. В E6
/// тем же кодом распаковывается обновление, скачанное на рантайме;
/// слишком тесный потолок означал бы, что очередной вырост апстрима
/// ломает обновление сразу у всех пользователей и чинится только выпуском
/// нового релиза приложения — ровно та беда, ради которой обновление
/// yt-dlp вынесено из релизного контура. Цена промаха в другую сторону
/// несопоставимо меньше: враждебный архив успеет записать полгигабайта
/// и будет остановлен, а не заполнит диск целиком.
pub(super) const MAX_UNPACKED_BYTES: u64 = 512 * 1024 * 1024;

/// Потолок числа записей в архиве.
///
/// Ловит класс, который потолок в байтах пропускает: миллион пустых
/// файлов не весит ничего, но съедает inode и делает каталог данных
/// неудаляемым за разумное время. Наблюдалось 139–186 записей (см.
/// [`MAX_UNPACKED_BYTES`]), взято 4096 — двадцатикратный запас.
///
/// Это единственное заявление архива, на которое можно опереться **до**
/// распаковки, и опереться не по доверию, а по устройству: число записей
/// центрального каталога — это ровно граница цикла, который мы сами и
/// крутим (`0..archive.len()`). Соврать в большую сторону здесь нельзя:
/// сколько заявлено, столько мы и обойдём.
///
/// Чего этот потолок **не** делает — не ограничивает память на разбор
/// архива: к моменту проверки центральный каталог уже разобран целиком
/// (см. комментарий на месте вызова).
const MAX_ENTRIES: usize = 4096;

/// Потолок глубины пути внутри архива.
///
/// Закрывает обход [`MAX_ENTRIES`]: одна запись разворачивается в столько
/// каталогов, сколько компонентов в её имени, а каталоги не считает ни
/// один из потолков выше. Воспроизведено при ревью TL-18: архив 696 КБ,
/// 1001 запись — при потолке 4096 — и имена глубиной 150 дали 151 000
/// каталогов, `unpack` вернул `Ok`, и дерево уехало в `promote`; 8,5 с на
/// создание, 12,4 с на удаление. Места на APFS это не ест, ест inode и
/// время — ровно тот класс, ради которого заведён [`MAX_ENTRIES`].
///
/// Измерено (фикстура `tests/fixtures/ytdlp-onedir/`): все десять снятых
/// деревьев укладываются в шесть уровней и держат эту цифру двадцать
/// месяцев — macOS 6, linux и windows 5. Самый глубокий путь набора:
/// `_internal/Python.framework/Versions/3.14/Resources/Info.plist`.
/// Запас тот же трёхкратный, что у [`MAX_UNPACKED_BYTES`].
///
/// Вместе с [`MAX_ENTRIES`] это и есть граница числа создаваемых
/// каталогов: не больше 4096 × 18 = 73 728. Цена названа здесь явно и
/// измерена, а не прикинута: архив из 4096 записей глубины 18 — ровно
/// то, что потолки ещё пропускают, — распаковывается за 3,56 с. Столько
/// и стоит выбранный запас в худшем разрешённом случае; следующий, кто
/// станет двигать любой из двух потолков, должен видеть произведение, а
/// не два сомножителя порознь.
const MAX_PATH_DEPTH: usize = 18;

/// Потолок длины имени записи в байтах.
///
/// Вторая половина того же белого списка: глубина ограничивает число
/// компонентов пути, длина — их суммарный размер. Без неё запись
/// остаётся местом, откуда в путь приходит произвольно длинная строка.
///
/// Измерено: самое длинное имя в десяти деревьях — 82 байта
/// (`_internal/python3.14/lib-dynload/_multiprocessing.cpython-314-aarch64-linux-gnu.so`),
/// самый длинный отдельный компонент — 56. 256 байт — примерно
/// трёхкратный запас, как и у остальных потолков.
///
/// Чего он не гарантирует: на Windows в `MAX_PATH` (260 без включённых
/// длинных путей) упирается путь целиком — каталог данных плюс имя из
/// архива, — а этот потолок ограничивает только вторую половину суммы.
/// Настоящее дерево с его 82 байтами до предела не достаёт с большим
/// зазором; отдельного сторожа на полную длину здесь нет намеренно, и
/// отказ такой записи придёт от файловой системы как
/// [`PrepareError::UnpackFailed`].
const MAX_NAME_BYTES: usize = 256;

/// Запас свободного места сверх объёма дерева.
///
/// Значение выбрано, а не измерено, и это важно не спутать. Сама
/// распаковка второй копии не требует: дерево пишется в `.staging-*` и
/// переезжает на рабочее место `rename`'ом в пределах того же тома. Запас
/// нужен на другое — на то, что происходит сразу после: подготовка
/// прогревает распакованный yt-dlp, а тот пишет свои кеши и временные
/// файлы туда же. И на то, что `available_space` — снимок, за который с
/// нами соревнуются другие процессы.
///
/// 64 МиБ — примерно половина дерева. Смысл границы в том, чтобы отказ
/// случался, пока на томе ещё есть чем дышать, а не ровно в нуле.
pub(super) const SPACE_HEADROOM_BYTES: u64 = 64 * 1024 * 1024;

/// Что получилось после распаковки.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unpacked {
    /// Имя исполняемого файла в корне дерева.
    pub executable: String,
    /// Число распакованных обычных файлов.
    pub file_count: u64,
    /// Сколько байт легло на диск на самом деле, а не сумма заявленных в
    /// архиве размеров. У целого архива это одно и то же число; у
    /// вравшего распаковки не будет вовсе — см. «Границы объёма».
    pub total_bytes: u64,
}

/// Распаковывает `archive` в `dest` (каталог должен не существовать или
/// быть пустым — вызывающая сторона готовит `.staging-*` сама).
///
/// `on_progress(done_bytes, total_bytes)` вызывается по мере записи —
/// достаточно часто, чтобы прогресс двигался, но не на каждый байт (см.
/// [`PROGRESS_STEP_BYTES`]).
pub fn unpack(
    archive_path: &Path,
    dest: &Path,
    on_progress: &mut dyn FnMut(u64, u64),
) -> Result<Unpacked, PrepareError> {
    unpack_with_space_probe(archive_path, dest, on_progress, &probe_available_space)
}

/// Свободное место на томе, где лежит `path`, — или `None`, если файловая
/// система не ответила.
///
/// Берётся именно «доступное непривилегированному пользователю»
/// (`f_bavail` на Unix, `GetDiskFreeSpaceExW`/`BytesAvailableToCaller` на
/// Windows), а не «свободное на томе»: приложение работает не под root,
/// зарезервированные суперпользователю блоки и дисковая квота ему не
/// достанутся, и считать их своими значило бы обещать место, которого нет.
pub(super) fn probe_available_space(path: &Path) -> Option<u64> {
    fs4::available_space(path).ok()
}

/// [`unpack`] со швом для проверки свободного места.
///
/// Шов существует ради тестов, и другого способа их написать нет: чтобы
/// проверить отказ по месту честной файловой системой, тесту пришлось бы
/// создавать том нужного размера — операция привилегированная,
/// платформозависимая и в CI недоступная. Живьём ветка всё равно проверена
/// на смонтированном 20-мегабайтном образе, см. отчёт TL-18.
fn unpack_with_space_probe(
    archive_path: &Path,
    dest: &Path,
    on_progress: &mut dyn FnMut(u64, u64),
    available_space: &dyn Fn(&Path) -> Option<u64>,
) -> Result<Unpacked, PrepareError> {
    let file = File::open(archive_path).map_err(|err| {
        if err.kind() == io::ErrorKind::NotFound {
            PrepareError::ArchiveMissing {
                path: archive_path.display().to_string(),
            }
        } else {
            PrepareError::ArchiveCorrupted {
                reason: format!("{}: {err}", archive_path.display()),
            }
        }
    })?;

    let mut archive = ZipArchive::new(file).map_err(|err| PrepareError::ArchiveCorrupted {
        reason: format!("{}: {err}", archive_path.display()),
    })?;

    // Потолок числа записей ограничивает то, что уйдёт в файловую
    // систему, и только это. Расход памяти на разбор самого архива он не
    // ограничивает и ограничить не может: `ZipArchive::new` строкой выше
    // уже построил указатель по всему центральному каталогу и вернул
    // управление только после этого. Измерено при ревью TL-18 на архиве
    // 218 МБ с 2 000 001 записью — 10 МиБ RSS до открытия и 936 МиБ
    // сразу после `ZipArchive::new`, амплификация ×4,3 к размеру архива,
    // и всё это до первой нашей проверки. Отказ ниже честный, но память
    // к тому моменту уже занята.
    //
    // Для E6 отсюда следовал практический вывод: ограничивать надо
    // размер скачанного архива — снаружи, до вызова [`unpack`], — потому
    // что изнутри этот расход не виден и не управляем. Ограничение
    // заведено в TL-56: `super::fetch::MAX_ARCHIVE_BYTES`, дважды — по
    // объявленному размеру до запроса и по фактически принятым байтам.
    check_entry_count(archive.len())?;

    // Полный размер дерева нужен до распаковки, чтобы прогресс считался
    // от него, а не «сколько-то из неизвестного». `by_index_raw` не
    // распаковывает данные — только читает заголовок записи.
    //
    // Сложение насыщающее: слагаемые приходят из заголовков архива, то
    // есть из недоверенного ввода, и подобранная пара `size()` по 2^63
    // переполнила бы `u64` — в отладочной сборке паникой, в релизной
    // молча обнулив заявленный объём.
    let mut declared_bytes = 0_u64;
    for index in 0..archive.len() {
        let Ok(entry) = archive.by_index_raw(index) else {
            continue;
        };

        declared_bytes = declared_bytes.saturating_add(entry.size());
        check_entry_shape(index, entry.name(), entry.enclosed_name().as_deref())?;
    }

    check_declared_shape(archive.len(), declared_bytes)?;

    fs::create_dir_all(dest).map_err(|err| PrepareError::UnpackFailed {
        reason: format!("{}: {err}", dest.display()),
    })?;

    // Проверка места идёт после создания каталога назначения, а не до:
    // спрашивать файловую систему можно только про существующий путь, а
    // сам каталог места не занимает. Распаковкой это ещё не является —
    // ни одного байта содержимого архива на диск не ушло.
    ensure_room_for(dest, declared_bytes, available_space)?;

    // Разрешённый объём. `min` — то самое «заголовки могут только
    // ужесточить»: заявленный размер сужает границу, потолок не даёт ей
    // расшириться. Проверка `declared_bytes > MAX_UNPACKED_BYTES` выше
    // делает `min` избыточным ровно сейчас; он оставлен намеренно, чтобы
    // граница держалась и без неё.
    let mut budget_left = declared_bytes.min(MAX_UNPACKED_BYTES);

    let mut written = 0_u64;
    let mut reported = 0_u64;
    let mut file_count = 0_u64;
    let mut executables_at_root = Vec::new();

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|err| PrepareError::ArchiveCorrupted {
                reason: format!("запись {index}: {err}"),
            })?;

        // Отказ всего архива, а не пропуск записи: ссылка в дереве, которое
        // мы сами же и собрали, означает, что архив не тот, за который себя
        // выдаёт, и распаковывать из него остальное незачем.
        if entry.is_symlink() {
            return Err(PrepareError::ArchiveCorrupted {
                reason: format!(
                    "запись {index} — символическая ссылка ({}), \
                     а в дереве yt-dlp ссылок не бывает",
                    entry.name()
                ),
            });
        }

        let Some(relative) = entry.enclosed_name() else {
            return Err(PrepareError::ArchiveCorrupted {
                reason: format!(
                    "запись {index} ведёт за пределы каталога назначения: {}",
                    entry.name()
                ),
            });
        };

        let target = dest.join(&relative);

        if entry.is_dir() {
            fs::create_dir_all(&target).map_err(|err| PrepareError::UnpackFailed {
                reason: format!("{}: {err}", target.display()),
            })?;
            continue;
        }

        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|err| PrepareError::UnpackFailed {
                reason: format!("{}: {err}", parent.display()),
            })?;
        }

        let mode = entry.unix_mode();
        let mut out = File::create(&target).map_err(|err| PrepareError::UnpackFailed {
            reason: format!("{}: {err}", target.display()),
        })?;

        // `io::copy` дочитывает запись до конца, а `ZipFile` на EOF
        // сверяет CRC32 — вот здесь и ловится повреждённый архив.
        // Прогресс обновляется поэтапно, для этого чтение идёт через
        // счётчик, а не одним вызовом `io::copy`.
        //
        // Копирование и разбор его отказа разнесены на два шага, чтобы у
        // разбора был доступ к `written`: замыкание прогресса держит его
        // заимствованным до конца своего выражения.
        let copied = copy_within_budget(&mut entry, &mut out, &mut budget_left, &mut |chunk| {
            written += chunk;
            if written - reported >= PROGRESS_STEP_BYTES || written == declared_bytes {
                reported = written;
                on_progress(written, declared_bytes);
            }
        });

        copied.map_err(|err| match err {
            CopyStop::Io(err) => {
                let remaining = declared_bytes.saturating_sub(written);
                classify_copy_error(&target, err, dest, remaining, available_space)
            }
            CopyStop::OverBudget => over_the_ceiling_error(
                declared_bytes.min(MAX_UNPACKED_BYTES),
                "фактически отдаёт больше, чем",
            ),
        })?;

        drop(out);
        apply_mode(&target, mode)?;

        file_count += 1;
        if is_root_executable(&relative, mode) {
            executables_at_root.push(
                relative
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default()
                    .to_string(),
            );
        }
    }

    on_progress(declared_bytes, declared_bytes);

    if executables_at_root.len() != 1 {
        return Err(PrepareError::LayoutUnexpected {
            reason: format!(
                "в корне архива {} исполняемых файлов вместо одного{}",
                executables_at_root.len(),
                if executables_at_root.is_empty() {
                    String::new()
                } else {
                    format!(": {}", executables_at_root.join(", "))
                }
            ),
        });
    }
    let executable = executables_at_root.remove(0);

    Ok(Unpacked {
        executable,
        file_count,
        total_bytes: written,
    })
}

/// Отвергает архив, чья заявленная форма выходит за разрешённый конверт.
///
/// Оба потолка собраны в одну функцию, чтобы тест мог спросить у самой
/// распаковки «а этот настоящий ассет ты бы пропустила?» её собственным
/// кодом, а не повторяя арифметику рядом.
fn check_declared_shape(entries: usize, declared_bytes: u64) -> Result<(), PrepareError> {
    check_entry_count(entries)?;

    if declared_bytes > MAX_UNPACKED_BYTES {
        return Err(over_the_ceiling_error(declared_bytes, "заявляет"));
    }

    Ok(())
}

/// Отвергает запись, чьё имя выходит за разрешённую форму.
///
/// Проверяется в предпроходе, до `create_dir_all(dest)`, поэтому отказ по
/// форме имени не оставляет на диске ни одного каталога.
///
/// Глубина считается по **обеззараженному** имени (`enclosed_name`), а не
/// по сырому: создавать каталоги будет именно оно. Запись, у которой
/// обеззараженного имени нет вовсе, здесь пропускается — её отвергнет
/// основной цикл со своим сообщением про выход за каталог назначения, и
/// дублировать этот отказ двумя формулировками незачем.
fn check_entry_shape(
    index: usize,
    name: &str,
    relative: Option<&Path>,
) -> Result<(), PrepareError> {
    if name.len() > MAX_NAME_BYTES {
        return Err(PrepareError::ArchiveCorrupted {
            reason: format!(
                "имя записи {index} — {} байт при потолке {MAX_NAME_BYTES}: \
                 в дереве yt-dlp таких имён нет",
                name.len()
            ),
        });
    }

    if let Some(relative) = relative {
        let depth = relative.components().count();
        if depth > MAX_PATH_DEPTH {
            return Err(PrepareError::ArchiveCorrupted {
                reason: format!(
                    "запись {index} лежит на глубине {depth} при потолке \
                     {MAX_PATH_DEPTH}: столько вложенных каталогов дерево \
                     yt-dlp не заводит ни на одной платформе ({name})"
                ),
            });
        }
    }

    Ok(())
}

fn check_entry_count(entries: usize) -> Result<(), PrepareError> {
    if entries > MAX_ENTRIES {
        return Err(PrepareError::ArchiveCorrupted {
            reason: format!(
                "в архиве {entries} записей при потолке {MAX_ENTRIES} — \
                 столько дерево yt-dlp не содержит ни на одной платформе"
            ),
        });
    }

    Ok(())
}

/// Отказ по границе объёма.
///
/// Класс — «архив повреждён», а не отдельный: для пользователя это то же
/// самое, что запись с именем наружу или символическая ссылка внутри, —
/// архив не тот, за который себя выдаёт, и делать с ним нечего, кроме как
/// взять другой. Отдельный класс появился бы, только если бы действие
/// пользователя отличалось; оно не отличается.
fn over_the_ceiling_error(bound: u64, verb: &str) -> PrepareError {
    PrepareError::ArchiveCorrupted {
        reason: format!(
            "архив {verb} {} МиБ при потолке {} МиБ — \
             дерево yt-dlp такого размера не бывает",
            bound / (1024 * 1024),
            MAX_UNPACKED_BYTES / (1024 * 1024)
        ),
    }
}

/// Отказывает, если на томе назначения не хватает места под дерево.
///
/// Не сторож безопасности: враждебный архив ограничен потолком, а не
/// этой проверкой. Смысл в диагностике — сказать «освободите место»
/// заранее, вместо `ENOSPC` посреди дерева.
///
/// Если файловая система не ответила, отказа нет. Проверка, которая не
/// смогла состояться, не должна запрещать установку: она не нашла
/// нехватки места, она вообще ничего не нашла. Забитый диск в этом случае
/// проявит себя отказом записи, и тот всё равно будет распознан по
/// [`io::ErrorKind::StorageFull`].
fn ensure_room_for(
    dest: &Path,
    tree_bytes: u64,
    available_space: &dyn Fn(&Path) -> Option<u64>,
) -> Result<(), PrepareError> {
    let Some(available) = available_space(dest) else {
        return Ok(());
    };

    let needed = tree_bytes.saturating_add(SPACE_HEADROOM_BYTES);
    if available < needed {
        return Err(PrepareError::NotEnoughSpace {
            path: dest.display().to_string(),
            needed,
            available,
        });
    }

    Ok(())
}

/// Насколько должен вырасти объём записанного, чтобы стоило сообщить о
/// прогрессе. Один мегабайт на 124 МиБ дерева — около 124 обновлений на
/// всю распаковку: индикатор движется плавно, а событий не больше, чем
/// успевает отрисовать WebView.
const PROGRESS_STEP_BYTES: u64 = 1024 * 1024;

/// Размер буфера чтения. 64 КиБ — обычный компромисс: заметно больше
/// размера страницы, заметно меньше кеша L2.
const COPY_BUFFER_BYTES: usize = 64 * 1024;

/// Почему копирование остановилось.
enum CopyStop {
    /// Отказ чтения из архива или записи на диск.
    Io(io::Error),
    /// Разрешённый объём исчерпан: записать этот кусок было бы уже
    /// нарушением границы.
    OverBudget,
}

/// Переливает запись архива в файл, вычитая записанное из общего на всё
/// дерево бюджета `budget_left`.
///
/// Бюджет проверяется **до** `write_all`, а не после: иначе кусок,
/// нарушивший границу, успевал бы лечь на диск, и сторож объёма
/// превращался бы в сторож «на 64 КиБ позже». Бюджет один на всё дерево и
/// живёт между вызовами — иначе архив из тысячи записей по бюджету каждая
/// обходил бы границу целого.
fn copy_within_budget(
    reader: &mut impl io::Read,
    writer: &mut impl io::Write,
    budget_left: &mut u64,
    on_chunk: &mut dyn FnMut(u64),
) -> Result<(), CopyStop> {
    let mut buffer = vec![0_u8; COPY_BUFFER_BYTES];
    loop {
        let read = reader.read(&mut buffer).map_err(CopyStop::Io)?;
        if read == 0 {
            return Ok(());
        }

        let read = read as u64;
        if read > *budget_left {
            return Err(CopyStop::OverBudget);
        }

        writer
            .write_all(&buffer[..read as usize])
            .map_err(CopyStop::Io)?;
        *budget_left -= read;
        on_chunk(read);
    }
}

/// Раскладывает отказ копирования на три разных разговора с
/// пользователем: «архив побит», «кончилось место», «диск не принял».
///
/// `zip` сообщает о несошедшемся CRC32 как об `io::Error` с
/// `ErrorKind::InvalidData` — это порча архива, а не отказ файловой
/// системы: переустановить приложение.
///
/// Нехватка места опознаётся по [`io::ErrorKind::StorageFull`] — по
/// собственной классификации стандартной библиотеки, а не по списку кодов
/// ОС, который пришлось бы вести самим и который на очередной платформе
/// оказался бы неполным. На macOS 15 / HFS+ ветка проверена живьём:
/// запись в смонтированный 20-мегабайтный образ после его заполнения
/// даёт ровно `StorageFull` (`os error 28`), см. отчёт TL-18.
///
/// Числа в отказе значат ровно то же, что и в [`ensure_room_for`]:
/// `needed` — сколько ещё должно было поместиться (недописанный остаток
/// дерева плюс тот же запас), `available` — что том отдаёт сейчас. Не
/// ответил — считаем, что ничего: отказ по `StorageFull` уже состоялся, и
/// от неудачи второго вопроса он не перестаёт быть нехваткой места.
fn classify_copy_error(
    target: &Path,
    err: io::Error,
    dest: &Path,
    remaining: u64,
    available_space: &dyn Fn(&Path) -> Option<u64>,
) -> PrepareError {
    match err.kind() {
        io::ErrorKind::InvalidData => PrepareError::ArchiveCorrupted {
            reason: format!("{}: {err}", target.display()),
        },
        io::ErrorKind::StorageFull => PrepareError::NotEnoughSpace {
            path: dest.display().to_string(),
            needed: remaining.saturating_add(SPACE_HEADROOM_BYTES),
            available: available_space(dest).unwrap_or(0),
        },
        _ => PrepareError::UnpackFailed {
            reason: format!("{}: {err}", target.display()),
        },
    }
}

/// Ставит распакованному файлу права, выведенные из одного бита архива.
///
/// Бит выполнения перенести необходимо: без него исполняемый файл yt-dlp и
/// сотня `.so` внутри дерева оказались бы незапускаемыми, и подготовка
/// «успешно» оставляла бы нерабочую установку. Всё остальное из архива не
/// берётся: его метаданные — недоверенный ввод, и объявленный в них
/// `0o777` дал бы world-writable файлы в каталоге данных, которые потом
/// `dlopen`'ит yt-dlp. Поэтому режим не копируется, а выводится —
/// [`EXECUTABLE_MODE`] или [`REGULAR_MODE`].
///
/// Поведение на реальных ассетах от этого не меняется: во всех трёх
/// апстримных onedir-архивах (macOS, Linux, Windows) встречаются ровно
/// `0o755` и `0o644` — проверено при ревью TL-12.
///
/// На Windows прав в этом смысле нет, и архив апстрима их не несёт
/// (создан на FAT-совместимой системе — `unix_mode` там `None`), поэтому
/// шаг применим только к Unix.
#[cfg(unix)]
fn apply_mode(path: &Path, mode: Option<u32>) -> Result<(), PrepareError> {
    use std::os::unix::fs::PermissionsExt;

    let Some(mode) = mode else {
        return Ok(());
    };

    fs::set_permissions(path, fs::Permissions::from_mode(derived_mode(mode))).map_err(|err| {
        PrepareError::UnpackFailed {
            reason: format!("права {}: {err}", path.display()),
        }
    })
}

/// Права запускаемого файла: владельцу — запись, всем — чтение и запуск.
const EXECUTABLE_MODE: u32 = 0o755;

/// Права обычного файла: владельцу — запись, всем — чтение.
const REGULAR_MODE: u32 = 0o644;

/// Единственное, что берётся из недоверенных метаданных записи.
fn derived_mode(archive_mode: u32) -> u32 {
    if archive_mode & 0o111 != 0 {
        EXECUTABLE_MODE
    } else {
        REGULAR_MODE
    }
}

#[cfg(not(unix))]
fn apply_mode(_path: &Path, _mode: Option<u32>) -> Result<(), PrepareError> {
    Ok(())
}

/// Является ли запись исполняемым файлом в корне дерева.
///
/// Имя исполняемого файла у каждой платформы своё (`yt-dlp_macos`,
/// `yt-dlp_linux`, `yt-dlp.exe`), поэтому оно не захардкожено, а
/// определяется по свойству: единственный запускаемый файл в корне архива.
/// На Unix признак — бит выполнения из архива; в windows-ассете апстрима
/// прав нет вовсе (архив создан не Unix-системой), и там признак —
/// расширение `.exe`.
fn is_root_executable(relative: &Path, mode: Option<u32>) -> bool {
    let at_root = relative.components().count() == 1
        && matches!(relative.components().next(), Some(Component::Normal(_)));
    if !at_root {
        return false;
    }

    if let Some(mode) = mode {
        if mode & 0o111 != 0 {
            return true;
        }
    }

    relative
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
}

/// Удаляет каталог, если он есть; отсутствие каталога — не ошибка.
pub fn remove_dir_if_exists(dir: &Path) -> Result<(), PrepareError> {
    match fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(PrepareError::UnpackFailed {
            reason: format!("удаление {}: {err}", dir.display()),
        }),
    }
}

/// Переименовывает полностью распакованное дерево на его окончательное
/// место — единственный момент, в который установка «появляется».
///
/// `rename` в пределах одной файловой системы атомарен: каталог
/// назначения либо ещё не существует, либо уже полон. Это и есть ответ на
/// прерванную подготовку — недостроенное дерево физически не может
/// оказаться по рабочему пути.
pub fn promote(staging: &Path, install: &Path) -> Result<(), PrepareError> {
    fs::rename(staging, install).map_err(|err| PrepareError::UnpackFailed {
        reason: format!(
            "перенос {} в {}: {err}",
            staging.display(),
            install.display()
        ),
    })
}

/// Собирает пути `.staging-*` в `root` — мусор от прерванных подготовок.
pub fn stale_staging_dirs(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };

    entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(".staging-"))
        })
        .map(|entry| entry.path())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    /// Собирает zip, похожий по форме на апстримный onedir-ассет:
    /// исполняемый файл в корне плюс каталог `_internal` рядом.
    fn write_onedir_zip(path: &Path, executable_name: &str, executable_mode: u32) {
        let file = File::create(path).expect("fixture archive must be creatable");
        let mut zip = ZipWriter::new(file);
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);

        zip.start_file(executable_name, stored.unix_permissions(executable_mode))
            .expect("start_file");
        zip.write_all(b"#!/bin/sh\necho 2026.08.19\n")
            .expect("write");

        zip.add_directory("_internal/", stored.unix_permissions(0o755))
            .expect("add_directory");

        zip.start_file("_internal/lib.so", stored.unix_permissions(0o755))
            .expect("start_file");
        zip.write_all(b"shared-library-bytes").expect("write");

        zip.start_file("_internal/data.txt", stored.unix_permissions(0o644))
            .expect("start_file");
        zip.write_all(b"plain data").expect("write");

        zip.finish().expect("finish");
    }

    fn unpack_fixture(archive: &Path, dest: &Path) -> Result<Unpacked, PrepareError> {
        unpack(archive, dest, &mut |_, _| {})
    }

    /// Распаковка со сколь угодно щедрой или скупой файловой системой.
    fn unpack_with_free_space(
        archive: &Path,
        dest: &Path,
        available: Option<u64>,
    ) -> Result<Unpacked, PrepareError> {
        unpack_with_space_probe(archive, dest, &mut |_, _| {}, &|_| available)
    }

    /// Переписывает в центральном каталоге объявленный размер каждой
    /// записи, не трогая сами данные.
    ///
    /// Это и есть «архив врёт о своём объёме» в чистом виде, и собрать
    /// такой архив обычным `ZipWriter` нельзя — он честно пишет то, что
    /// получилось. Правится именно центральный каталог: `zip` берёт
    /// `size()` оттуда, оттуда же его берёт и наша проверка заявленного.
    fn overwrite_declared_sizes(archive: &Path, declared: u32) {
        let mut bytes = fs::read(archive).expect("архив фикстуры обязан читаться");

        let eocd = bytes
            .windows(4)
            .rposition(|window| window == b"PK\x05\x06")
            .expect("хвост центрального каталога обязан найтись");
        let entries = u16::from_le_bytes([bytes[eocd + 10], bytes[eocd + 11]]) as usize;
        let mut cursor = u32::from_le_bytes([
            bytes[eocd + 16],
            bytes[eocd + 17],
            bytes[eocd + 18],
            bytes[eocd + 19],
        ]) as usize;

        let le16 =
            |bytes: &[u8], at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]) as usize;

        for _ in 0..entries {
            assert_eq!(&bytes[cursor..cursor + 4], b"PK\x01\x02", "запись каталога");
            bytes[cursor + 24..cursor + 28].copy_from_slice(&declared.to_le_bytes());
            cursor += 46
                + le16(&bytes, cursor + 28)
                + le16(&bytes, cursor + 30)
                + le16(&bytes, cursor + 32);
        }

        fs::write(archive, &bytes).expect("архив фикстуры обязан писаться");
    }

    /// Объявляет каждой записи 64-битный размер `declared`, спрятав его
    /// в zip64-поле центрального каталога.
    ///
    /// Одним лишь 32-битным полем такой архив не собрать: в нём
    /// помещается меньше 4 ГиБ, а для переполнения `u64` нужны слагаемые
    /// около 2^63. Формат для этого и предусматривает extra-поле
    /// `0x0001`: 32-битный размер выставляется в `0xFFFFFFFF`, а
    /// настоящий лежит рядом восемью байтами. `zip` читает его ровно по
    /// этому признаку (`read.rs`: `len >= 24 || uncompressed_size ==
    /// ZIP64_BYTES_THR`).
    fn declare_zip64_sizes(archive: &Path, declared: u64) {
        const ZIP64_THRESHOLD: u32 = u32::MAX;

        let bytes = fs::read(archive).expect("архив фикстуры обязан читаться");
        let eocd = bytes
            .windows(4)
            .rposition(|window| window == b"PK\x05\x06")
            .expect("хвост центрального каталога обязан найтись");
        let le16 =
            |bytes: &[u8], at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]) as usize;
        let le32 = |bytes: &[u8], at: usize| {
            u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]) as usize
        };

        let entries = le16(&bytes, eocd + 10);
        let directory_at = le32(&bytes, eocd + 16);

        let mut directory = Vec::new();
        let mut cursor = directory_at;
        for _ in 0..entries {
            assert_eq!(&bytes[cursor..cursor + 4], b"PK\x01\x02", "запись каталога");
            let (name_len, extra_len, comment_len) = (
                le16(&bytes, cursor + 28),
                le16(&bytes, cursor + 30),
                le16(&bytes, cursor + 32),
            );

            let mut zip64 = Vec::with_capacity(12);
            zip64.extend_from_slice(&1_u16.to_le_bytes());
            zip64.extend_from_slice(&8_u16.to_le_bytes());
            zip64.extend_from_slice(&declared.to_le_bytes());

            let mut header = bytes[cursor..cursor + 46].to_vec();
            header[24..28].copy_from_slice(&ZIP64_THRESHOLD.to_le_bytes());
            header[30..32].copy_from_slice(
                &u16::try_from(extra_len + zip64.len())
                    .unwrap()
                    .to_le_bytes(),
            );

            directory.extend_from_slice(&header);
            directory.extend_from_slice(&bytes[cursor + 46..cursor + 46 + name_len + extra_len]);
            directory.extend_from_slice(&zip64);
            let comment_at = cursor + 46 + name_len + extra_len;
            directory.extend_from_slice(&bytes[comment_at..comment_at + comment_len]);

            cursor = comment_at + comment_len;
        }

        let mut tail = bytes[eocd..].to_vec();
        tail[12..16].copy_from_slice(&u32::try_from(directory.len()).unwrap().to_le_bytes());

        let mut patched = bytes[..directory_at].to_vec();
        patched.extend_from_slice(&directory);
        patched.extend_from_slice(&tail);
        fs::write(archive, &patched).expect("архив фикстуры обязан писаться");
    }

    /// Архив из одной записи с заданным именем.
    fn write_single_entry_zip(path: &Path, name: &str) {
        let file = File::create(path).expect("fixture archive must be creatable");
        let mut zip = ZipWriter::new(file);
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        zip.start_file(name, stored.unix_permissions(0o755))
            .expect("start_file");
        zip.write_all(b"#!/bin/sh\n").expect("write");
        zip.finish().expect("finish");
    }

    /// Сколько байт лежит в дереве прямо сейчас.
    fn bytes_on_disk(dir: &Path) -> u64 {
        let Ok(entries) = fs::read_dir(dir) else {
            return 0;
        };

        entries
            .filter_map(Result::ok)
            .map(|entry| match entry.file_type() {
                Ok(kind) if kind.is_dir() => bytes_on_disk(&entry.path()),
                Ok(_) => entry.metadata().map(|meta| meta.len()).unwrap_or(0),
                Err(_) => 0,
            })
            .sum()
    }

    #[test]
    fn unpacks_the_tree_and_finds_the_single_root_executable() {
        let dir = tempdir().expect("tempdir");
        let archive = dir.path().join("yt-dlp.zip");
        write_onedir_zip(&archive, "yt-dlp_macos", 0o755);

        let dest = dir.path().join("staging");
        let unpacked = unpack_fixture(&archive, &dest).expect("распаковка обязана пройти");

        assert_eq!(unpacked.executable, "yt-dlp_macos");
        assert_eq!(unpacked.file_count, 3);
        assert!(dest.join("_internal/lib.so").exists());
        assert!(dest.join("_internal/data.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn preserves_the_execute_bit_of_every_entry_that_had_it() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().expect("tempdir");
        let archive = dir.path().join("yt-dlp.zip");
        write_onedir_zip(&archive, "yt-dlp_macos", 0o755);

        let dest = dir.path().join("staging");
        unpack_fixture(&archive, &dest).expect("распаковка обязана пройти");

        let mode = |relative: &str| {
            fs::metadata(dest.join(relative))
                .expect("файл обязан существовать")
                .permissions()
                .mode()
                & 0o777
        };

        assert_eq!(mode("yt-dlp_macos"), 0o755);
        // Сотня .so внутри дерева тоже должна остаться исполняемой: их
        // грузит dyld, и именно на них уходит время прогрева.
        assert_eq!(mode("_internal/lib.so"), 0o755);
        assert_eq!(mode("_internal/data.txt"), 0o644);
    }

    #[test]
    fn reports_progress_that_ends_at_the_full_size() {
        let dir = tempdir().expect("tempdir");
        let archive = dir.path().join("yt-dlp.zip");
        write_onedir_zip(&archive, "yt-dlp_macos", 0o755);

        let mut updates = Vec::new();
        let dest = dir.path().join("staging");
        let unpacked = unpack(&archive, &dest, &mut |done, total| {
            updates.push((done, total))
        })
        .expect("распаковка обязана пройти");

        let (done, total) = *updates.last().expect("хотя бы одно обновление прогресса");
        assert_eq!(done, total);
        assert_eq!(total, unpacked.total_bytes);
        assert!(
            updates.iter().all(|(done, total)| done <= total),
            "прогресс не может превышать сто процентов: {updates:?}"
        );
    }

    #[test]
    fn refuses_an_archive_whose_entry_escapes_the_destination() {
        let dir = tempdir().expect("tempdir");
        let archive = dir.path().join("evil.zip");
        {
            let file = File::create(&archive).expect("create");
            let mut zip = ZipWriter::new(file);
            let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            zip.start_file("../escaped.txt", stored)
                .expect("start_file");
            zip.write_all(b"nope").expect("write");
            zip.finish().expect("finish");
        }

        let dest = dir.path().join("staging");
        let error = unpack_fixture(&archive, &dest).expect_err("выход за каталог недопустим");

        assert!(
            matches!(error, PrepareError::ArchiveCorrupted { .. }),
            "{error}"
        );
        assert!(
            !dir.path().join("escaped.txt").exists(),
            "файл не должен появиться за пределами каталога назначения"
        );
    }

    #[test]
    fn refuses_an_archive_containing_a_symlink_entry() {
        // Ссылка в архиве — способ обойти проверку имён: следующая запись
        // пишется «внутрь» каталога назначения, а физически уходит туда,
        // куда ведёт ссылка. Поэтому отказ всего архива, а не пропуск
        // записи, и поэтому же тест кладёт в фикстуру настоящую запись
        // с `S_IFLNK`, а не файл с путём в содержимом.
        let dir = tempdir().expect("tempdir");
        let outside = dir.path().join("outside");
        fs::create_dir_all(&outside).expect("mkdir");

        let archive = dir.path().join("symlinked.zip");
        {
            let file = File::create(&archive).expect("create");
            let mut zip = ZipWriter::new(file);
            let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            zip.start_file("yt-dlp_macos", stored.unix_permissions(0o755))
                .expect("start_file");
            zip.write_all(b"#!/bin/sh\n").expect("write");
            zip.add_symlink("_internal", outside.to_str().expect("utf-8 path"), stored)
                .expect("add_symlink");
            zip.start_file("_internal/planted.txt", stored.unix_permissions(0o644))
                .expect("start_file");
            zip.write_all(b"pwned").expect("write");
            zip.finish().expect("finish");
        }

        let dest = dir.path().join("staging");
        let error = unpack_fixture(&archive, &dest).expect_err("ссылки в архиве недопустимы");

        assert!(
            matches!(error, PrepareError::ArchiveCorrupted { .. }),
            "{error}"
        );
        assert!(
            !outside.join("planted.txt").exists(),
            "запись сквозь ссылку не должна была состояться"
        );
        assert!(
            !dest.join("_internal").exists(),
            "сама ссылка не должна была появиться в дереве"
        );
    }

    #[cfg(unix)]
    #[test]
    fn derives_permissions_instead_of_copying_them_from_the_archive() {
        use std::os::unix::fs::PermissionsExt;

        // Архив объявляет world-writable права и setuid — распакованное
        // дерево не обязано им верить: из метаданных берётся только бит
        // выполнения.
        let dir = tempdir().expect("tempdir");
        let archive = dir.path().join("greedy.zip");
        {
            let file = File::create(&archive).expect("create");
            let mut zip = ZipWriter::new(file);
            let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            zip.start_file("yt-dlp_macos", stored.unix_permissions(0o4777))
                .expect("start_file");
            zip.write_all(b"#!/bin/sh\n").expect("write");
            zip.start_file("_internal/data.txt", stored.unix_permissions(0o666))
                .expect("start_file");
            zip.write_all(b"plain data").expect("write");
            zip.finish().expect("finish");
        }

        let dest = dir.path().join("staging");
        unpack_fixture(&archive, &dest).expect("распаковка обязана пройти");

        let mode = |relative: &str| {
            fs::metadata(dest.join(relative))
                .expect("файл обязан существовать")
                .permissions()
                .mode()
                & 0o7777
        };

        assert_eq!(mode("yt-dlp_macos"), EXECUTABLE_MODE);
        assert_eq!(mode("_internal/data.txt"), REGULAR_MODE);
    }

    #[cfg(unix)]
    #[test]
    fn derived_permissions_are_never_writable_by_anyone_but_the_owner() {
        for archive_mode in [0o777, 0o666, 0o755, 0o644, 0o000, 0o4755, 0o2777, 0o1777] {
            let derived = derived_mode(archive_mode);
            assert_eq!(
                derived & 0o022,
                0,
                "режим {archive_mode:o} дал бы {derived:o} — запись вне владельца"
            );
            assert_eq!(
                derived & 0o7000,
                0,
                "режим {archive_mode:o} дал бы {derived:o} — setuid/setgid/sticky"
            );
            assert_eq!(
                derived & 0o111 != 0,
                archive_mode & 0o111 != 0,
                "бит выполнения — единственное, что переносится из архива"
            );
        }
    }

    #[test]
    fn refuses_a_file_that_is_not_a_zip_at_all() {
        let dir = tempdir().expect("tempdir");
        let archive = dir.path().join("not-a-zip.zip");
        fs::write(&archive, b"this is not an archive").expect("write");

        let error = unpack_fixture(&archive, &dir.path().join("staging"))
            .expect_err("не-zip не может распаковаться");

        assert!(
            matches!(error, PrepareError::ArchiveCorrupted { .. }),
            "{error}"
        );
    }

    #[test]
    fn reports_a_missing_archive_separately_from_a_broken_one() {
        let dir = tempdir().expect("tempdir");

        let error = unpack_fixture(&dir.path().join("absent.zip"), &dir.path().join("staging"))
            .expect_err("отсутствующий архив — ошибка");

        assert!(
            matches!(error, PrepareError::ArchiveMissing { .. }),
            "{error}"
        );
    }

    #[test]
    fn detects_a_corrupted_entry_through_its_crc32() {
        // Правим байты полезной нагрузки, не трогая CRC32 в заголовке —
        // ровно то, что делает битый сектор или оборванная закачка.
        let dir = tempdir().expect("tempdir");
        let archive = dir.path().join("yt-dlp.zip");
        {
            let file = File::create(&archive).expect("create");
            let mut zip = ZipWriter::new(file);
            let deflated =
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            zip.start_file("yt-dlp_macos", deflated.unix_permissions(0o755))
                .expect("start_file");
            zip.write_all(b"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
                .expect("write");
            zip.finish().expect("finish");
        }

        let mut bytes = fs::read(&archive).expect("read");
        let offset = bytes
            .windows(4)
            .position(|window| window == b"AAAA")
            .expect("полезная нагрузка обязана найтись");
        bytes[offset] = b'B';
        fs::write(&archive, &bytes).expect("write");

        let error = unpack_fixture(&archive, &dir.path().join("staging"))
            .expect_err("несошедшийся CRC32 обязан быть замечен");

        assert!(
            matches!(error, PrepareError::ArchiveCorrupted { .. }),
            "{error}"
        );
    }

    #[test]
    fn refuses_an_archive_without_a_single_root_executable() {
        let dir = tempdir().expect("tempdir");
        let archive = dir.path().join("no-exe.zip");
        {
            let file = File::create(&archive).expect("create");
            let mut zip = ZipWriter::new(file);
            let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            zip.start_file("readme.txt", stored.unix_permissions(0o644))
                .expect("start_file");
            zip.write_all(b"no executable here").expect("write");
            zip.finish().expect("finish");
        }

        let error = unpack_fixture(&archive, &dir.path().join("staging"))
            .expect_err("дерево без исполняемого файла непригодно");

        assert!(
            matches!(error, PrepareError::LayoutUnexpected { .. }),
            "{error}"
        );
    }

    #[test]
    fn refuses_an_archive_with_two_root_executables() {
        let dir = tempdir().expect("tempdir");
        let archive = dir.path().join("two-exe.zip");
        {
            let file = File::create(&archive).expect("create");
            let mut zip = ZipWriter::new(file);
            let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            for name in ["yt-dlp_macos", "yt-dlp_other"] {
                zip.start_file(name, stored.unix_permissions(0o755))
                    .expect("start_file");
                zip.write_all(b"#!/bin/sh\n").expect("write");
            }
            zip.finish().expect("finish");
        }

        let error = unpack_fixture(&archive, &dir.path().join("staging"))
            .expect_err("гадать, что запускать, нельзя");

        assert!(
            matches!(error, PrepareError::LayoutUnexpected { .. }),
            "{error}"
        );
    }

    #[test]
    fn recognises_a_windows_style_executable_without_unix_permissions() {
        // Апстримный yt-dlp_win.zip создан не Unix-системой: прав в нём
        // нет вовсе, и единственный признак — расширение.
        assert!(is_root_executable(Path::new("yt-dlp.exe"), None));
        assert!(is_root_executable(Path::new("yt-dlp.EXE"), None));
        assert!(!is_root_executable(Path::new("readme.txt"), None));
        assert!(!is_root_executable(Path::new("_internal/lib.exe"), None));
        assert!(!is_root_executable(
            Path::new("_internal/tool"),
            Some(0o755)
        ));
    }

    #[test]
    fn promotes_staging_to_the_install_directory_atomically() {
        let dir = tempdir().expect("tempdir");
        let staging = dir.path().join(".staging-x");
        let install = dir.path().join("x");
        fs::create_dir_all(staging.join("_internal")).expect("mkdir");
        fs::write(staging.join("marker"), b"1").expect("write");

        promote(&staging, &install).expect("перенос обязан пройти");

        assert!(!staging.exists());
        assert!(install.join("marker").exists());
    }

    #[test]
    fn lists_only_staging_leftovers() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join(".staging-a")).expect("mkdir");
        fs::create_dir_all(dir.path().join(".staging-b")).expect("mkdir");
        fs::create_dir_all(dir.path().join("2026.08.19-abc")).expect("mkdir");
        fs::write(dir.path().join("2026.08.19-abc.json"), b"{}").expect("write");

        let mut found: Vec<_> = stale_staging_dirs(dir.path())
            .into_iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        found.sort();

        assert_eq!(found, vec![".staging-a", ".staging-b"]);
    }

    #[test]
    fn refuses_an_archive_that_declares_more_than_the_ceiling() {
        // Дешёвый отказ по заголовкам: если архив сам говорит, что не
        // поместится, читать его незачем. Настоящий сторож — не этот, а
        // тот, что ниже; этот лишь избавляет от бессмысленной работы.
        let dir = tempdir().expect("tempdir");
        let archive = dir.path().join("huge.zip");
        write_onedir_zip(&archive, "yt-dlp_macos", 0o755);
        overwrite_declared_sizes(&archive, 600 * 1024 * 1024);

        let dest = dir.path().join("staging");
        let error = unpack_fixture(&archive, &dest).expect_err("дерево такого размера не бывает");

        assert!(
            matches!(error, PrepareError::ArchiveCorrupted { .. }),
            "{error}"
        );
        assert!(
            !dest.exists(),
            "отказ по заявленному объёму обязан случиться до того, как \
             появится сам каталог назначения"
        );
    }

    #[test]
    fn refuses_a_deflate_bomb_that_lies_about_its_size() {
        // Суть требования TL-18. Заявленный размер записи ничего не
        // ограничивает: `zip` держит `take` на СЖАТОМ потоке
        // (`read.rs`, `make_crypto_reader`), а сколько из него
        // развернётся — его не касается. Восемь мегабайт нулей сжимаются
        // в единицы килобайт, и архив объявляет их одним мегабайтом.
        //
        // Поэтому граница считается по фактически записанному, и лишнее
        // на диск не попадает вовсе.
        const REAL_BYTES: usize = 8 * 1024 * 1024;
        const DECLARED_BYTES: u32 = 1024 * 1024;

        let dir = tempdir().expect("tempdir");
        let archive = dir.path().join("bomb.zip");
        {
            let file = File::create(&archive).expect("create");
            let mut zip = ZipWriter::new(file);
            let deflated =
                SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
            zip.start_file("yt-dlp_macos", deflated.unix_permissions(0o755))
                .expect("start_file");
            zip.write_all(&vec![0_u8; REAL_BYTES]).expect("write");
            zip.finish().expect("finish");
        }
        let compressed = fs::metadata(&archive).expect("metadata").len();
        assert!(
            compressed < u64::from(DECLARED_BYTES),
            "фикстура обязана быть бомбой: {compressed} сжатых байт не меньше заявленного"
        );
        overwrite_declared_sizes(&archive, DECLARED_BYTES);

        let dest = dir.path().join("staging");
        let error = unpack_fixture(&archive, &dest).expect_err("бомба обязана быть остановлена");

        assert!(
            matches!(error, PrepareError::ArchiveCorrupted { .. }),
            "{error}"
        );
        let written = bytes_on_disk(&dest);
        assert!(
            written <= u64::from(DECLARED_BYTES),
            "на диск ушло {written} байт при разрешённых {DECLARED_BYTES} — \
             граница проверяется не по ходу записи"
        );
        assert!(
            written < REAL_BYTES as u64,
            "распаковалась вся бомба целиком ({written} байт) — сторожа нет"
        );
    }

    #[test]
    fn refuses_more_entries_than_any_tree_of_yt_dlp_has() {
        // Класс, который потолок в байтах не ловит: записи пустые, весит
        // дерево ноль, а inode кончаются.
        //
        // Число записей — литерал, а не `MAX_ENTRIES + 1`. Выведенная из
        // константы фикстура проверяла бы тавтологию «на единицу больше
        // потолка больше потолка» и оставалась бы зелёной при любом его
        // значении; ревью TL-18 показало это мутацией `MAX_ENTRIES =
        // 1_000_000`.
        const OVER_THE_CEILING: usize = 4097;

        let dir = tempdir().expect("tempdir");
        let archive = dir.path().join("swarm.zip");
        {
            let file = File::create(&archive).expect("create");
            let mut zip = ZipWriter::new(file);
            let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            zip.start_file("yt-dlp_macos", stored.unix_permissions(0o755))
                .expect("start_file");
            zip.write_all(b"#!/bin/sh\n").expect("write");
            for index in 0..OVER_THE_CEILING {
                zip.start_file(format!("_internal/{index}"), stored.unix_permissions(0o644))
                    .expect("start_file");
            }
            zip.finish().expect("finish");
        }

        let dest = dir.path().join("staging");
        let error = unpack_fixture(&archive, &dest).expect_err("столько записей не бывает");

        assert!(
            matches!(error, PrepareError::ArchiveCorrupted { .. }),
            "{error}"
        );
        assert!(
            !dest.exists(),
            "отказ по числу записей обязан случиться до создания каталога"
        );
    }

    #[test]
    fn refuses_a_path_deeper_than_any_tree_of_yt_dlp_has() {
        // Обход потолка записей, найденный ревью TL-18: записей мало,
        // каталогов из них разворачивается сколько угодно. Глубина —
        // литерал по той же причине, что и число записей выше.
        const OVER_THE_CEILING: usize = 19;

        let dir = tempdir().expect("tempdir");
        let archive = dir.path().join("deep.zip");
        let name = vec!["nested"; OVER_THE_CEILING].join("/");
        write_single_entry_zip(&archive, &name);

        let dest = dir.path().join("staging");
        let error = unpack_fixture(&archive, &dest).expect_err("такой глубины дерево не бывает");

        assert!(
            matches!(error, PrepareError::ArchiveCorrupted { .. }),
            "{error}"
        );
        assert!(
            !dest.exists(),
            "отказ по глубине обязан случиться до создания каталогов — \
             в них весь смысл отказа"
        );
    }

    #[test]
    fn accepts_a_path_as_deep_as_the_real_tree_goes() {
        // Вторая половина белого списка: он обязан пропускать то, ради
        // чего заведён. Шесть уровней — максимум по всем десяти снятым
        // деревьям, и путь взят настоящий.
        let deepest = Path::new("_internal/Python.framework/Versions/3.14/Resources/Info.plist");
        assert_eq!(deepest.components().count(), 6);

        check_entry_shape(0, deepest.to_str().expect("utf-8"), Some(deepest))
            .expect("настоящий самый глубокий путь апстрима обязан проходить");
    }

    #[test]
    fn refuses_a_name_longer_than_any_tree_of_yt_dlp_has() {
        const OVER_THE_CEILING: usize = 257;

        let dir = tempdir().expect("tempdir");
        let archive = dir.path().join("verbose.zip");
        write_single_entry_zip(&archive, &"n".repeat(OVER_THE_CEILING));

        let dest = dir.path().join("staging");
        let error = unpack_fixture(&archive, &dest).expect_err("таких имён в дереве нет");

        assert!(
            matches!(error, PrepareError::ArchiveCorrupted { .. }),
            "{error}"
        );
        assert!(!dest.exists(), "отказ по имени — тоже до касания диска");
    }

    // `assertions_on_constants` здесь именно то, что нужно: тест на то и
    // существует, чтобы утверждение о константах было записано отдельно
    // от самих констант и ломалось при их правке. Clippy предполагает,
    // что такая проверка бесполезна, — здесь она и есть предмет.
    #[allow(clippy::assertions_on_constants)]
    #[test]
    fn the_ceilings_are_pinned_from_above_as_well_as_from_below() {
        // Снизу потолки держит `the_ceilings_clear_every_tree_upstream…`:
        // они обязаны быть не меньше замеров с запасом. Сверху их не
        // держало ничто — ревью TL-18 показало это мутацией
        // `MAX_ENTRIES = 1_000_000`, которая оставила набор зелёным.
        //
        // Числа здесь литералы намеренно. Это не дубликат объявления, а
        // вторая подпись под ним: поднять потолок по-прежнему можно, но
        // не молча — придётся тронуть и это место, а значит объяснить,
        // откуда взялось новое значение, и переснять замеры в фикстуре.
        assert!(
            MAX_UNPACKED_BYTES <= 512 * 1024 * 1024,
            "потолок объёма поднят выше обоснованного замерами"
        );
        assert!(
            MAX_ENTRIES <= 4096,
            "потолок числа записей поднят выше обоснованного замерами"
        );
        assert!(
            MAX_PATH_DEPTH <= 18,
            "потолок глубины поднят выше обоснованного замерами"
        );
        assert!(
            MAX_NAME_BYTES <= 256,
            "потолок длины имени поднят выше обоснованного замерами"
        );
    }

    #[test]
    fn a_pair_of_entries_cannot_overflow_the_sum_of_declared_sizes() {
        // Регрессия на насыщающее сложение. Два слагаемых по 2^63 в
        // сумме дают ровно 2^64: `+=` в отладочной сборке паникует, а
        // `wrapping_add` возвращает ноль — и ноль этот выглядит как
        // «архив ничего не обещает», то есть проходит проверку формы.
        //
        // Сторож — не текст сообщения, а место отказа. При насыщении
        // отказ приходит от `check_declared_shape`, до
        // `create_dir_all(dest)`, и каталога назначения не появляется
        // вовсе. При заворачивании проверка формы пройдена, каталог
        // создан, и остановит распаковку уже бюджет — на шаг позже.
        let dir = tempdir().expect("tempdir");
        let archive = dir.path().join("overflow.zip");
        {
            let file = File::create(&archive).expect("create");
            let mut zip = ZipWriter::new(file);
            let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            for name in ["yt-dlp_macos", "_internal/data.txt"] {
                zip.start_file(name, stored.unix_permissions(0o644))
                    .expect("start_file");
                zip.write_all(b"x").expect("write");
            }
            zip.finish().expect("finish");
        }
        declare_zip64_sizes(&archive, 1_u64 << 63);

        let dest = dir.path().join("staging");
        let error = unpack_fixture(&archive, &dest).expect_err("два по 2^63 — не дерево yt-dlp");

        assert!(
            matches!(error, PrepareError::ArchiveCorrupted { .. }),
            "{error}"
        );
        assert!(
            !dest.exists(),
            "переполнение обнулило заявленный объём: проверка формы его \
             пропустила, и отказ пришёл на шаг позже"
        );
    }

    #[test]
    fn refuses_to_start_when_the_volume_has_no_room_for_the_tree() {
        let dir = tempdir().expect("tempdir");
        let archive = dir.path().join("yt-dlp.zip");
        write_onedir_zip(&archive, "yt-dlp_macos", 0o755);

        let dest = dir.path().join("staging");
        let error = unpack_with_free_space(&archive, &dest, Some(1024))
            .expect_err("на килобайте дерево не разложится");

        let PrepareError::NotEnoughSpace {
            needed, available, ..
        } = error
        else {
            panic!("нехватка места обязана быть отдельным классом, а не общим отказом: {error}");
        };
        assert_eq!(available, 1024);
        assert!(
            needed >= SPACE_HEADROOM_BYTES,
            "в требуемое место обязан входить запас: {needed}"
        );
        assert_eq!(
            bytes_on_disk(&dest),
            0,
            "проверка места на то и до распаковки, чтобы на диск ничего не ушло"
        );
    }

    #[test]
    fn does_not_confuse_a_full_disk_with_a_disk_that_refuses_to_write() {
        // Обещание задачи: нехватка места отличима от прочих сбоев
        // записи не текстом сообщения, а классом. Разбор идёт по
        // `ErrorKind` стандартной библиотеки; на macOS 15 живьём
        // проверено, что забитый том даёт ровно `StorageFull`.
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("file");

        let classify = |kind: io::ErrorKind| {
            classify_copy_error(&target, io::Error::from(kind), dir.path(), 100, &|_| {
                Some(7)
            })
        };

        assert!(matches!(
            classify(io::ErrorKind::StorageFull),
            PrepareError::NotEnoughSpace {
                available: 7,
                needed,
                ..
            } if needed == 100 + SPACE_HEADROOM_BYTES
        ));
        assert!(matches!(
            classify(io::ErrorKind::PermissionDenied),
            PrepareError::UnpackFailed { .. }
        ));
        assert!(matches!(
            classify(io::ErrorKind::ReadOnlyFilesystem),
            PrepareError::UnpackFailed { .. }
        ));
        assert!(matches!(
            classify(io::ErrorKind::InvalidData),
            PrepareError::ArchiveCorrupted { .. }
        ));
    }

    #[test]
    fn a_filesystem_that_will_not_say_how_much_is_free_does_not_block_the_install() {
        // Проверка, которая не смогла состояться, не нашла нехватки
        // места — она вообще ничего не нашла, и запрещать по ней
        // установку значило бы выдумать отказ.
        let dir = tempdir().expect("tempdir");
        let archive = dir.path().join("yt-dlp.zip");
        write_onedir_zip(&archive, "yt-dlp_macos", 0o755);

        let dest = dir.path().join("staging");
        unpack_with_free_space(&archive, &dest, None).expect("молчание тома — не отказ");

        assert!(dest.join("_internal/lib.so").exists());
    }

    #[test]
    fn asks_the_real_filesystem_and_lets_the_happy_path_through() {
        // Сторож против «шов работает, а настоящий вызов нет»: здесь
        // свободное место спрашивается у настоящей файловой системы,
        // тем же кодом, что и в приложении.
        let dir = tempdir().expect("tempdir");
        let archive = dir.path().join("yt-dlp.zip");
        write_onedir_zip(&archive, "yt-dlp_macos", 0o755);

        let dest = dir.path().join("staging");
        let unpacked = unpack_fixture(&archive, &dest).expect("на рабочей машине место есть");

        assert_eq!(unpacked.file_count, 3);
        assert!(
            probe_available_space(dir.path()).is_some(),
            "том, на котором идут тесты, обязан отвечать о свободном месте"
        );
    }

    // --- потолки против настоящих ассетов апстрима ----------------------

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct MeasuredTree {
        release: String,
        asset: String,
        entries: usize,
        unpacked_bytes: u64,
        max_path_depth: usize,
        deepest_path: String,
        max_name_bytes: usize,
        longest_name: String,
    }

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct MeasuredCapture {
        pinned_release: String,
    }

    #[derive(serde::Deserialize)]
    struct UpstreamTrees {
        #[serde(rename = "_capture")]
        capture: MeasuredCapture,
        trees: Vec<MeasuredTree>,
    }

    fn measured_trees() -> UpstreamTrees {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/ytdlp-onedir/upstream-trees.json");
        let raw = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("{} не читается: {err}", path.display()));
        serde_json::from_str(&raw)
            .unwrap_or_else(|err| panic!("{} — не тот конверт: {err}", path.display()))
    }

    #[test]
    fn the_pinned_release_is_the_one_the_sizes_were_measured_from() {
        // Тот же сторож, что у фикстур вывода yt-dlp и ffmpeg: замеры
        // заморожены, апстрим нет. Смена пина может изменить и форму
        // дерева, поэтому она обязана ломать этот тест, а не тихо
        // оставлять потолки стоять на позапрошлой реальности.
        let measured = measured_trees();
        let pinned = std::env!("TUBE_LEAK_YTDLP_VERSION");

        assert_eq!(
            measured.capture.pinned_release, pinned,
            "замеры сняты с релиза {}, а вкладывается {pinned}. Пин сменили — \
             переснимите замеры по README рядом с фикстурой, а не правьте эту строку",
            measured.capture.pinned_release
        );
    }

    #[test]
    fn the_ceilings_clear_every_tree_upstream_has_ever_shipped() {
        // Запас, а не просто «проходит». Потолок, к которому реальность
        // подошла вплотную, — это отложенная поломка обновления yt-dlp
        // сразу у всех пользователей (E6), и заметить её надо здесь.
        const HEADROOM_FACTOR: u64 = 3;

        let measured = measured_trees();
        assert!(
            measured.trees.len() >= 6,
            "набор замеров подозрительно мал: {}",
            measured.trees.len()
        );

        for tree in &measured.trees {
            let name = format!("{} {}", tree.release, tree.asset);
            assert!(
                tree.unpacked_bytes * HEADROOM_FACTOR <= MAX_UNPACKED_BYTES,
                "{name}: дерево {} МиБ против потолка {} МиБ — запаса меньше \
                 чем в {HEADROOM_FACTOR} раза",
                tree.unpacked_bytes / (1024 * 1024),
                MAX_UNPACKED_BYTES / (1024 * 1024)
            );
            assert!(
                tree.entries * (HEADROOM_FACTOR as usize) <= MAX_ENTRIES,
                "{name}: {} записей против потолка {MAX_ENTRIES}",
                tree.entries
            );
            assert!(
                tree.max_path_depth * (HEADROOM_FACTOR as usize) <= MAX_PATH_DEPTH,
                "{name}: глубина {} против потолка {MAX_PATH_DEPTH}",
                tree.max_path_depth
            );
            assert!(
                tree.max_name_bytes * (HEADROOM_FACTOR as usize) <= MAX_NAME_BYTES,
                "{name}: имя {} байт против потолка {MAX_NAME_BYTES}",
                tree.max_name_bytes
            );
        }
    }

    #[test]
    fn no_measured_tree_is_refused_by_the_unpacker() {
        // На вопрос «а настоящий ассет мы бы не отвергли?» отвечает код
        // распаковки, а не арифметика в голове читающего.
        for tree in measured_trees().trees {
            let where_from = format!("{} {}", tree.release, tree.asset);

            let verdict = check_declared_shape(tree.entries, tree.unpacked_bytes);
            assert!(
                verdict.is_ok(),
                "{where_from} отвергается по объёму или числу записей: {}",
                verdict.unwrap_err()
            );

            // Самый глубокий и самое длинное имя — разные записи, и
            // проверяются они порознь, каждая своей границей.
            let deepest = Path::new(&tree.deepest_path);
            assert_eq!(
                deepest.components().count(),
                tree.max_path_depth,
                "{where_from}: замер глубины и сам путь разошлись"
            );
            let verdict = check_entry_shape(0, &tree.deepest_path, Some(deepest));
            assert!(
                verdict.is_ok(),
                "{where_from} отвергается по глубине: {}",
                verdict.unwrap_err()
            );

            assert_eq!(
                tree.longest_name.len(),
                tree.max_name_bytes,
                "{where_from}: замер длины имени и само имя разошлись"
            );
            let verdict =
                check_entry_shape(0, &tree.longest_name, Some(Path::new(&tree.longest_name)));
            assert!(
                verdict.is_ok(),
                "{where_from} отвергается по длине имени: {}",
                verdict.unwrap_err()
            );
        }
    }

    #[test]
    fn removing_a_missing_directory_is_not_an_error() {
        let dir = tempdir().expect("tempdir");
        remove_dir_if_exists(&dir.path().join("never-existed")).expect("отсутствие — не ошибка");
    }
}
