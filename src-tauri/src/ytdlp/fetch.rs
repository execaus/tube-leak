//! Приём архива обновления yt-dlp, его проверка и установка (TL-56).
//!
//! Один путь на два источника (С-10): релизный ассет из сети и вложенный
//! в бандл ресурс приходят сюда одинаково — [`ArchiveSource`], — и дальше
//! идут через тот же конвейер, что подготовка первого запуска
//! (`.staging-*` → [`super::unpack`] → атомарный `rename` → манифест).
//! Второй реализации распаковки не заводится (Ф-4), поэтому здесь её и
//! нет: [`super::prepare::install`] вызывается как есть.
//!
//! # Потолок на размер входного архива
//!
//! Главное, что этот модуль добавляет к уже существовавшему механизму.
//! Потолки TL-18 живут **внутри** распаковки, складываются из заголовков
//! уже открытого архива и метаданных релиза не видят вовсе. Больше того,
//! `super::unpack` прямо просит ограничить размер снаружи: `ZipArchive::new`
//! строит указатель по всему центральному каталогу до первой нашей
//! проверки, и на архиве 218 МБ с двумя миллионами записей это 936 МиБ
//! RSS — амплификация ×4,3 к размеру архива (замер ревью TL-18). В бандле
//! размер архива контролировал пин; здесь его контролирует сеть, и
//! ограничить его больше нечем.
//!
//! Потолок [`MAX_ARCHIVE_BYTES`] стоит **дважды**, и это не
//! перестраховка — это две разные проверки с разными основаниями:
//!
//! 1. **По объявленному размеру, до начала запроса.** `UpdateAsset::size_bytes`
//!    обязателен ровно ради этого шага. Ассет, который апстрим объявляет
//!    больше потолка, не скачивается вовсе — [`ArchiveSource::open`] не
//!    вызывается, то есть соединения не возникает (сторож
//!    `refuses_by_the_declared_size_without_opening_the_stream`).
//! 2. **По фактически принятым байтам, по ходу приёма.** Объявленный
//!    размер приходит от того же, кто отдаёт файл, — доверять ему как
//!    границе нельзя. Счёт идёт вместе с подсчётом sha256, за один проход
//!    по потоку, и проверяется **до** записи очередного куска на диск,
//!    как в [`super::unpack`]: за границу не попадает даже тот байт, на
//!    котором её нарушили.
//!
//! Разрешённый объём — `min(объявленный, потолок)`, тот же приём и тот же
//! смысл, что у бюджета распаковки: **заголовки могут границу только
//! ужесточить, но не ослабить**. Первая проверка делает `min` избыточным
//! ровно сейчас; он оставлен намеренно, чтобы граница держалась и без неё
//! (сторож `the_stream_is_capped_even_when_the_declared_size_lies`).
//!
//! # Чем контрольная сумма не является
//!
//! Средством защиты от подмены — не является. Тот, кто подменил архив,
//! пересчитает и sha256; она ловит порчу при передаче и хранении, не
//! подмену. Модель угроз эпика это и говорит прямо: доверие контура — TLS
//! к GitHub плюс сверка суммы по файлу сумм того же релиза, компрометация
//! апстрима вне модели. Отсюда порядок здесь: границы держат потолки, а
//! сумма стоит **до** распаковки (Ф-3) и решает другой вопрос — «дошло ли
//! то же самое, что обещали».
//!
//! # Чего здесь нет
//!
//! - **Транспорта.** [`ArchiveSource`] — абстракция над потоком байт с
//!   объявленным размером; сетевой источник из неё уже собран
//!   ([`network_source`] по [`super::update::UpdateAsset`]), и незакрытым
//!   остаётся ровно одно место — замыкание `open`, делающее сам запрос.
//!   Выбор HTTP-клиента — решение уровня зависимостей проекта, не этой
//!   задачи: любой из них вводит в релизный граф TLS-стек с C-сборкой, а
//!   собрать Linux и Windows сейчас некому.
//! - **Политики повторов.** Неудачная попытка записывается в журнал рядом
//!   с установкой ([`super::layout::Layout::update_attempt_path`]) — это
//!   та точка, куда TL-58 подключает троттлинг разных классов отказа.
//!   Решение «пробовать ли ещё раз» принимает он, не этот модуль.
//! - **Прогрева и smoke-проверки.** Прогрев ждёт паузы между задачами
//!   (решение дизайна), smoke — TL-57. Отсюда результат
//!   [`PreparedCandidate`], а не «установлено».

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::error::PrepareError;
use super::layout::{ArchiveIdentity, BuildId, Layout, RepairLog};
use super::prepare;
use super::update::UpdateAsset;
use crate::types::YtDlpUpdateFailure;

/// Потолок размера входного архива.
///
/// # Откуда 192 МиБ
///
/// Из замеров настоящих ассетов апстрима, а не из круглого числа. Поле
/// `assetBytes` фикстуры `tests/fixtures/ytdlp-onedir/upstream-trees.json`
/// (снята диапазонными запросами к релизным ассетам 2026-08-27, см. её
/// README): все шесть платформенных ассетов пинованного релиза — 17,2;
/// 20,5; 38,4; 38,6; 38,8; 51,4 МиБ, а четыре macOS-релиза за двадцать
/// месяцев назад — 56,1; 58,0; 61,1; 61,5 МиБ. Самый крупный ассет,
/// который апстрим когда-либо выпускал за эти двадцать месяцев, — 61,5
/// МиБ; 192 МиБ даёт над ним запас чуть больше чем в три раза — тот же
/// множитель, которым выбраны все четыре потолка распаковки.
///
/// Проверено на месте: `src-tauri/resources/yt-dlp.zip` — 53 923 637
/// байт, ровно `assetBytes` строки `2026.08.19 yt-dlp_macos.zip`, и его
/// sha256 совпадает с пином `binaries.lock.json`. То есть замеры,
/// снятые по сети, сходятся с архивом на диске до байта.
///
/// # Почему щедро и почему не щедрее
///
/// Перекос в сторону запаса сознателен и тот же, что у
/// `MAX_UNPACKED_BYTES`: слишком тесный потолок означает, что очередной
/// вырост апстрима ломает обновление сразу у всех пользователей и чинится
/// только выпуском нового релиза приложения — ровно та беда, ради которой
/// обновление yt-dlp вынесено из релизного контура.
///
/// Сверху значение держит цена промаха, и она здесь считается, а не
/// прикидывается. Худший случай — враждебный архив ровно по потолку с
/// раздутым центральным каталогом: 192 МиБ × 4,3 (измеренная ревью TL-18
/// амплификация RSS) ≈ 826 МиБ памяти, занятой `ZipArchive::new` **до**
/// первой проверки распаковки. Это и есть верхняя граница расхода,
/// которой до TL-56 не было вовсе — там на её месте стояло «сколько
/// отдадут». Ниже 192 МиБ её опускать нечем: запас над реальностью уже
/// минимально приличный.
///
/// Значение согласовано с потолком распаковки, а не выбрано отдельно:
/// настоящее дерево yt-dlp разворачивается из архива примерно в 2,4 раза
/// (124,0 МиБ дерева из 51,4 МиБ архива у пинованного macOS-ассета —
/// 130 010 634 и 53 923 637 байт, отношение 2,41), то есть архив «по
/// потолку» честной формы дал бы около 462 МиБ дерева — под
/// `MAX_UNPACKED_BYTES` (512 МиБ). Потолки не спорят друг с другом: тот,
/// кто станет двигать любой, обязан видеть оба.
///
/// Единицы здесь МиБ (1024²) везде, и это не педантизм: `unpackedBytes`
/// пинованного ассета — 130,0 **МБ**, то есть 124,0 МиБ, и в первой
/// редакции этого doc два числа сравнивались в разных единицах. Сравнение
/// от этого не сломалось (отношение единиц не зависит), но читатель,
/// проверяющий 2,4 по этим двум числам, получал 2,53 и не сходился ни с
/// чем.
pub const MAX_ARCHIVE_BYTES: u64 = 192 * 1024 * 1024;

/// Насколько должен вырасти объём принятого, чтобы стоило сообщить о
/// прогрессе. Один мегабайт — те же ~54 обновления на настоящий ассет,
/// что и у распаковки: полоса движется плавно, а событий не больше, чем
/// успевает отрисовать WebView.
const PROGRESS_STEP_BYTES: u64 = 1024 * 1024;

/// Размер буфера чтения — как в распаковке: заметно больше страницы,
/// заметно меньше кеша L2.
const READ_BUFFER_BYTES: usize = 64 * 1024;

/// Откуда приехал архив.
///
/// Различие нужно ровно в одном месте — при классификации оборвавшегося
/// чтения. Оборванный сетевой поток это «нет сети» (К-5 ждёт именно его и
/// именно текста «работаем на X»), а нечитаемый файл из бандла — порча:
/// сети там нет, и предлагать пользователю проверить соединение было бы
/// враньём. Один и тот же `io::Error` значит разное в зависимости от
/// того, откуда он пришёл, и знает это только источник.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Релизный ассет, принимаемый по сети.
    Network,
    /// Ресурс, вложенный в бандл приложения (С-10).
    Bundled,
}

/// Источник архива: объявленный размер плюс поток байт.
///
/// Разделение на «сколько обещано» и «открыть» здесь несущее, а не
/// оформительское: [`fetch_and_install`] спрашивает размер и, если тот
/// уже за потолком, до [`Self::open`] не доходит — то есть запрос не
/// начинается вовсе (пункт 1 потолка, см. doc модуля).
pub trait ArchiveSource {
    /// Что это за источник — определяет разговор об оборванном чтении.
    fn origin(&self) -> Origin;

    /// Человекочитаемый адрес для лога: URL или путь.
    fn describe(&self) -> String;

    /// Сколько байт источник обещает. Для сетевого ассета — размер из
    /// метаданных релиза (`UpdateAsset::size_bytes`), известный **до**
    /// запроса; для ресурса бандла — длина файла.
    fn declared_bytes(&self) -> u64;

    /// Открывает поток. Для сетевого источника здесь и происходит
    /// собственно запрос.
    fn open(&self) -> io::Result<Box<dyn Read>>;
}

/// Архив, вложенный в бандл приложения (С-10).
#[derive(Debug, Clone)]
pub struct BundledArchive {
    path: PathBuf,
    declared_bytes: u64,
}

impl BundledArchive {
    /// Размер спрашивается у файловой системы сразу, а не при открытии:
    /// потолок обязан сработать до чтения, и у бандла тоже. Разницы в
    /// правилах между источниками нет намеренно — иначе «путь единый для
    /// обоих» (С-10) было бы утверждением, которое некому проверить.
    pub fn at(path: &Path) -> Result<Self, FetchError> {
        let declared_bytes = fs::metadata(path)
            .map_err(|err| FetchError::Archive {
                reason: format!("{}: {err}", path.display()),
            })?
            .len();

        Ok(Self {
            path: path.to_path_buf(),
            declared_bytes,
        })
    }
}

impl ArchiveSource for BundledArchive {
    fn origin(&self) -> Origin {
        Origin::Bundled
    }

    fn describe(&self) -> String {
        self.path.display().to_string()
    }

    fn declared_bytes(&self) -> u64 {
        self.declared_bytes
    }

    fn open(&self) -> io::Result<Box<dyn Read>> {
        Ok(Box::new(fs::File::open(&self.path)?))
    }
}

/// Источник поверх произвольного потока байт с объявленным размером.
///
/// Это и есть форма, в которой сюда придёт релизный ассет: у всех
/// рассматривавшихся HTTP-клиентов синхронный ответ реализует
/// [`std::io::Read`], поэтому сетевой источник — это `StreamArchive`,
/// чей `open` делает запрос и отдаёт тело. Никакого другого кода
/// скачиванию не понадобится: потолок, счёт байт, sha256, приём в файл,
/// сверка и установка уже здесь и уже покрыты тестами.
///
/// Второе применение — тесты: обрезанный, бесконечный и лгущий о размере
/// потоки выражаются им же, без сети и без подмены транспорта.
pub struct StreamArchive<F> {
    origin: Origin,
    describe: String,
    declared_bytes: u64,
    open: F,
}

impl<F> StreamArchive<F>
where
    F: Fn() -> io::Result<Box<dyn Read>>,
{
    pub fn new(origin: Origin, describe: impl Into<String>, declared_bytes: u64, open: F) -> Self {
        Self {
            origin,
            describe: describe.into(),
            declared_bytes,
            open,
        }
    }
}

impl<F> ArchiveSource for StreamArchive<F>
where
    F: Fn() -> io::Result<Box<dyn Read>>,
{
    fn origin(&self) -> Origin {
        self.origin
    }

    fn describe(&self) -> String {
        self.describe.clone()
    }

    fn declared_bytes(&self) -> u64 {
        self.declared_bytes
    }

    fn open(&self) -> io::Result<Box<dyn Read>> {
        (self.open)()
    }
}

/// Раскладывает метаданные релизного ассета (TL-55) на идентификатор,
/// которым адресуется установка.
///
/// Три строки кода, а заведены они отдельно и с тестом ровно потому, что
/// перепутать здесь нечего только на вид: `version` и `sha256` — обе
/// строки, и компилятор поменять их местами не мешает. До TL-56
/// «`UpdateAsset` потребляется напрямую» было утверждением задачи, а не
/// кодом: тип упоминался в комментариях, а склейку предстояло написать
/// TL-58 — то есть в третьем месте, без сторожа. Теперь она одна, здесь,
/// и её держит `an_update_asset_is_taken_apart_field_by_field`.
///
/// Цена ошибки после правки ревью — не тихий дефект, а отказ: сумма,
/// попавшая в поле версии, не проходит белый список [`BuildId`]. Это
/// хорошо (ломается громко), но полагаться на это как на проверку
/// нельзя — она про форму, а не про смысл.
impl<'a> From<&'a UpdateAsset> for ArchiveIdentity<'a> {
    fn from(asset: &'a UpdateAsset) -> Self {
        Self {
            version: &asset.version,
            sha256: &asset.sha256,
        }
    }
}

/// Источник поверх релизного ассета: `size_bytes` становится объявленным
/// размером, `url` — адресом для лога и диагностики.
///
/// `open` приходит снаружи и делает сам запрос — транспорта здесь нет
/// (см. doc модуля). Смысл функции в том, что **знаменатель полосы и
/// первый потолок берутся из метаданных, а не из заголовков ответа**:
/// объявленный размер обязан быть известен до открытия потока, иначе
/// проверка «отказ до запроса» не выражается вовсе.
pub fn network_source<F>(asset: &UpdateAsset, open: F) -> StreamArchive<F>
where
    F: Fn() -> io::Result<Box<dyn Read>>,
{
    StreamArchive::new(Origin::Network, asset.url.clone(), asset.size_bytes, open)
}

/// Почему обновление не установилось.
///
/// Домейн говорит `thiserror`-ошибкой, граница конвертирует её в
/// контрактный [`YtDlpUpdateFailure`] (CLAUDE.md, «Конвенции»). Вариантов
/// четыре, а не пять: `smokeCheckFailed` конструирует TL-57 — до запуска
/// подготовленного дерева этот модуль не доходит по построению.
#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    /// Поток не открылся или оборвался на середине — сеть (С-3, К-5).
    #[error("обновление не скачалось: {reason}")]
    Network { reason: String },

    /// Соединение состоялось, а отдали не то: источник кончился раньше
    /// объявленного размера. Это не «нет сети» — сеть как раз была.
    #[error("источник отдал не то, что обещал: {reason}")]
    Source { reason: String },

    /// Архив признан негодным: сумма не сошлась, превышен потолок, архив
    /// не читается, в корне не ровно один исполняемый файл, версия или
    /// сумма не годятся в имя каталога.
    ///
    /// # Когда именно случился отказ — до установки или уже в ней
    ///
    /// Раньше здесь стояло «архив отброшен до того, как что-либо тронуло
    /// активную установку (С-4)». Это неверно, и ревью TL-56 показало
    /// запуском: в этот класс через [`From<PrepareError>`] попадают и
    /// отказы **установки**, а [`super::prepare::install`] перед
    /// распаковкой сносит каталог установки и манифест того build id, в
    /// который ставит. Отказ после этого оставляет каталог данных без
    /// установки, а не «нетронутым».
    ///
    /// Что приходит **до** того, как установка начата (активная не
    /// тронута ничем, и это проверяется `assert_no_debris` в каждом из
    /// тестов):
    ///
    /// - идентификатор кандидата не годится в имя каталога
    ///   ([`super::layout::BuildId`]);
    /// - объявленный размер за потолком либо нулевой у ресурса бандла;
    /// - поток перерос [`MAX_ARCHIVE_BYTES`];
    /// - файл приёма не создался или не пишется;
    /// - ресурс бандла не читается или кончился раньше объявленного;
    /// - **sha256 не сошлась** — сверка стоит до распаковки (Ф-3).
    ///
    /// Что приходит **после** того, как установка началась, то есть уже
    /// после сноса каталога и манифеста целевого build id (всё, что
    /// пришло из [`PrepareError`], кроме нехватки места):
    ///
    /// - архив не открывается как zip, CRC32 записи не сошёлся;
    /// - в корне дерева не ровно один исполняемый файл;
    /// - запись дерева не удалась (нет прав, отказ файловой системы).
    ///
    /// # Предусловие, которым гарантия С-4 держится
    ///
    /// **[`fetch_and_install`] нельзя звать с build id активной
    /// установки.** В сетевом сценарии это выполняется само: кандидат
    /// имеет другую версию и другую сумму, значит другой каталог, и снос
    /// в `install` касается пустого места. Но С-10 (архив из бандла) —
    /// ровно тот случай, где идентификаторы могут совпасть, и тогда
    /// неудачная распаковка уносит рабочую установку.
    ///
    /// Удерживать предусловие обязана оркестрация (TL-58): она владеет
    /// вызовом и знает, какая установка активна. Здесь оно записано, а
    /// не проверено, намеренно — этому модулю не с чем сравнивать: он
    /// получает `layout` и `identity`, а «какая установка активна»
    /// хранится записью TL-54, до которой ему нет дела.
    #[error("архив обновления отброшен: {reason}")]
    Archive { reason: String },

    /// Не хватает места под архив или под распакованное дерево (С-11).
    #[error("не хватает места для обновления: {reason}")]
    Space { reason: String },
}

impl FetchError {
    /// Проекция на контракт Ф-9. Версия приходит снаружи, а не хранится
    /// внутри: три класса из пяти её называют, два — нет, и хранить поле,
    /// которое половина вариантов не читает, значило бы однажды показать
    /// не то (см. doc [`YtDlpUpdateFailure`]).
    pub fn to_failure(&self, version: &str) -> YtDlpUpdateFailure {
        let message = self.to_string();
        match self {
            Self::Network { .. } => YtDlpUpdateFailure::NetworkUnavailable { message },
            Self::Source { .. } => YtDlpUpdateFailure::SourceUnavailable { message },
            Self::Archive { .. } => YtDlpUpdateFailure::ArchiveCorrupted {
                version: version.to_string(),
                message,
            },
            Self::Space { .. } => YtDlpUpdateFailure::NotEnoughSpace {
                version: version.to_string(),
                message,
            },
        }
    }
}

/// Отказы установки приходят из уже существующего механизма (TL-12/TL-18)
/// и раскладываются на классы контура здесь — одним местом на весь модуль.
///
/// `DataDirUnavailable` в этот разбор попасть не может: корень установок
/// создаёт подготовка первого запуска, без которой приложение не работает
/// вовсе, и обновление его не создаёт (см. [`fetch_and_install`]). Если он
/// всё же исчез между стартом и обновлением, отказ придёт от создания
/// `.staging-*` как `UnpackFailed` — то есть по ветке «архив отброшен», с
/// честной причиной в тексте.
impl From<PrepareError> for FetchError {
    fn from(error: PrepareError) -> Self {
        match error {
            PrepareError::NotEnoughSpace { .. } => Self::Space {
                reason: error.to_string(),
            },
            _ => Self::Archive {
                reason: error.to_string(),
            },
        }
    }
}

/// Этап, о котором сообщает [`fetch_and_install`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchStage {
    /// Приём архива. Знаменатель — объявленный размер.
    Downloading,
    /// Распаковка принятого архива. Знаменатель — заявленный объём дерева.
    Unpacking,
}

/// Установка, распакованная и проверенная, но ещё не активная.
///
/// Именно это TL-57 запускает smoke-проверкой, а TL-58 — переключает на
/// границе задач. «Подготовлена, не активна» здесь выражено типом, а не
/// соглашением: активной установку делает запись TL-54, и сделать её
/// отсюда нечем.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedCandidate {
    /// `<версия>-<sha12>` — имя каталога установки и адрес журналов рядом.
    ///
    /// Проверенным типом, а не строкой: отдай этот шаг `String`, и TL-58
    /// смог бы собрать из неё путь мимо проверки — ровно та дыра, которую
    /// [`BuildId`] и закрывает.
    pub build_id: BuildId,
    /// Версия yt-dlp, которую обещали метаданные релиза. Что скажет сам
    /// бинарник — выясняет smoke-проверка (TL-57).
    pub version: String,
    /// Каталог распакованного дерева.
    pub dir: PathBuf,
    /// Полный путь к исполняемому файлу внутри дерева.
    pub executable: PathBuf,
}

/// Принимает архив из `source`, сверяет его с `identity` и устанавливает
/// существующим механизмом.
///
/// `layout` обязан существовать на диске: корень установок создаёт
/// подготовка первого запуска (`super::prepare`), которая по построению
/// выполнилась раньше любого обновления — приложение без неё не работает.
/// Создавать его здесь незачем, а «на всякий случай» вредно: тогда
/// обновление умело бы поднимать каталог данных с нуля, и отказ этого
/// шага пришлось бы называть одним из пяти классов Ф-9, ни один из
/// которых про него не говорит.
///
/// Порядок шагов задан Ф-3 и проверяется тестами по отдельности:
/// проверка идентификатора → потолок по объявленному → место под архив →
/// приём с потолком по факту и sha256 на лету → сверка суммы →
/// распаковка. Ни один побочный файл не переживает отказ на любом из них.
///
/// # Предусловие
///
/// `identity` **не должен** совпадать с идентификатором активной
/// установки. Начиная с распаковки шаг необратим: `install` сносит
/// каталог и манифест целевого build id до того, как появится новое
/// дерево, — то есть отказ на нём уносит установку с тем же именем.
/// Держит это оркестрация (TL-58), которая владеет вызовом и знает
/// активную установку; подробности и полный разбор «что до, что после» —
/// в doc [`FetchError::Archive`].
pub fn fetch_and_install(
    source: &dyn ArchiveSource,
    identity: ArchiveIdentity<'_>,
    layout: &Layout,
    on_progress: &mut dyn FnMut(FetchStage, u64, u64),
) -> Result<PreparedCandidate, FetchError> {
    // Идентификатор — самое первое, что здесь делается, и это не порядок
    // ради порядка. Из него строится **каждый** путь ниже, включая путь
    // журнала неудачных попыток: пока проверки не было, отказ на любом
    // шаге уводил `record_failed_attempt` туда, куда указала версия из
    // чужих метаданных (ревью TL-56 — абсолютный путь и `../`). Поэтому
    // журнал ведётся уже проверенным значением, а неприемлемый
    // идентификатор возвращается до него и не пишет ничего никуда: писать
    // о нём было бы некуда.
    let build_id = identity.build_id()?;
    let outcome = fetch_and_install_inner(source, identity, &build_id, layout, on_progress);

    if let Err(error) = &outcome {
        record_failed_attempt(layout, &build_id, error);
    }

    outcome
}

fn fetch_and_install_inner(
    source: &dyn ArchiveSource,
    identity: ArchiveIdentity<'_>,
    build_id: &BuildId,
    layout: &Layout,
    on_progress: &mut dyn FnMut(FetchStage, u64, u64),
) -> Result<PreparedCandidate, FetchError> {
    let declared = source.declared_bytes();

    // Потолок по объявленному размеру — до всего остального. Для сетевого
    // источника это единственный момент, когда отказ ещё ничего не стоит:
    // соединения нет, байты не приняты.
    check_declared_size(source)?;

    // Место под архив (С-11, «скачивание ИЛИ распаковка упирается в
    // отсутствие места»). Проверка распаковки стоит на объёме дерева и
    // сработает позже; она не знает, что до неё на тот же том лягут ещё
    // шестьдесят мегабайт самого архива.
    ensure_room_for_archive(layout.root(), declared)?;

    let (archive_path, archive_file) = layout
        .create_download_file(build_id)
        .map_err(FetchError::from)?;

    let received = receive(source, declared, archive_file, &archive_path, on_progress);

    let received = match received {
        Ok(received) => received,
        Err(error) => {
            discard(&archive_path);
            return Err(error);
        }
    };

    // Сверка суммы — до распаковки (Ф-3) и без чувствительности к
    // регистру: файл сумм апстрима пишет их строчными, но полагаться на
    // это негде — сравнение по значению, а не по написанию.
    if !received.sha256.eq_ignore_ascii_case(identity.sha256) {
        discard(&archive_path);
        return Err(FetchError::Archive {
            reason: format!(
                "sha256 принятого архива {} не совпадает с ожидаемой {} ({})",
                received.sha256,
                identity.sha256,
                source.describe()
            ),
        });
    }

    let installed = prepare::install(&archive_path, layout, identity, &mut |done, total| {
        on_progress(FetchStage::Unpacking, done, total);
    });

    // Архив больше не нужен ни при каком исходе распаковки: удался
    // `rename` — дерево уже на месте, не удался — повторять будем с
    // нуля, потому что доверять половине скачанного нечем.
    discard(&archive_path);

    let installed = installed?;

    Ok(PreparedCandidate {
        build_id: build_id.clone(),
        version: identity.version.to_string(),
        dir: installed.dir,
        executable: installed.executable,
    })
}

/// Пункт 1 потолка: отказ по тому, что источник о себе объявил.
///
/// Ноль здесь тоже отказ, и не из педантизма: пустой поток не является
/// zip-архивом ни при каком содержимом, а обещанный размер ноль — это
/// метаданные, по которым нечего скачивать. Пропустить его значило бы
/// пойти в сеть за заведомо негодным ассетом.
///
/// Класс у нуля зависит от источника — по тому же правилу, что у обрыва
/// (см. [`Origin`]): у сети «ассет без размера» это ответ, которого от
/// метаданных релиза не ждали, то есть источник отдал не то; у бандла
/// сети нет вовсе, и пустой вложенный ресурс — сломанная установка
/// приложения, то есть порча. Превышение потолка, наоборот, класс имеет
/// один: слишком большой архив одинаково нечем чинить, откуда бы он ни
/// приехал.
fn check_declared_size(source: &dyn ArchiveSource) -> Result<(), FetchError> {
    let declared = source.declared_bytes();
    let describe = source.describe();

    if declared == 0 {
        let reason = format!("{describe} объявляет нулевой размер — скачивать нечего");
        return Err(match source.origin() {
            Origin::Network => FetchError::Source { reason },
            Origin::Bundled => FetchError::Archive { reason },
        });
    }

    if declared > MAX_ARCHIVE_BYTES {
        return Err(FetchError::Archive {
            reason: format!(
                "{describe} объявляет {} МиБ при потолке {} МиБ — \
                 архивов yt-dlp такого размера апстрим не выпускал",
                declared / (1024 * 1024),
                MAX_ARCHIVE_BYTES / (1024 * 1024)
            ),
        });
    }

    Ok(())
}

/// Свободное место под сам архив.
///
/// Запас берётся тот же, что у распаковки ([`super::unpack::SPACE_HEADROOM_BYTES`]),
/// и по той же причине: смысл границы в том, чтобы отказ случался, пока
/// на томе ещё есть чем дышать. Файловая система не ответила — отказа
/// нет: проверка, которая не смогла состояться, не нашла нехватки места,
/// она вообще ничего не нашла (тот же довод, что в `unpack::ensure_room_for`).
fn ensure_room_for_archive(root: &Path, declared: u64) -> Result<(), FetchError> {
    let Some(available) = super::unpack::probe_available_space(root) else {
        return Ok(());
    };

    let needed = declared.saturating_add(super::unpack::SPACE_HEADROOM_BYTES);
    if available < needed {
        return Err(FetchError::Space {
            reason: format!(
                "под архив обновления нужно ещё {} МиБ, свободно {} МиБ ({})",
                needed / (1024 * 1024),
                available / (1024 * 1024),
                root.display()
            ),
        });
    }

    Ok(())
}

/// Что принято: сколько байт и какая у них сумма.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Received {
    bytes: u64,
    sha256: String,
}

/// Приём потока в файл: пункт 2 потолка и sha256 — один проход, один
/// буфер.
///
/// Разносить их по двум функциям (а тем более по двум задачам) было бы
/// либо вторым проходом по сети за тем же архивом, либо протаскиванием
/// состояния открытого потока наружу; выигрыша нет ни в том, ни в другом.
fn receive(
    source: &dyn ArchiveSource,
    declared: u64,
    file: fs::File,
    archive_path: &Path,
    on_progress: &mut dyn FnMut(FetchStage, u64, u64),
) -> Result<Received, FetchError> {
    let mut reader = source.open().map_err(|err| read_failure(source, &err))?;
    let mut writer = io::BufWriter::new(file);

    let received = drain_within_budget(&mut reader, &mut writer, declared, &mut |done| {
        on_progress(FetchStage::Downloading, done, declared);
    });

    // Буфер обязан быть слит до любого разговора об успехе: без этого
    // «принято N байт» относилось бы к счётчику, а не к файлу, который
    // сейчас откроет распаковка.
    let flushed = writer
        .flush()
        .and_then(|()| writer.into_inner().map_err(io::Error::other));

    let received = received.map_err(|stop| match stop {
        Stop::Read(err) => read_failure(source, &err),
        Stop::Write(err) if err.kind() == io::ErrorKind::StorageFull => FetchError::Space {
            reason: format!("{}: {err}", archive_path.display()),
        },
        Stop::Write(err) => FetchError::Archive {
            reason: format!("{}: {err}", archive_path.display()),
        },
        Stop::OverBudget { budget } => FetchError::Archive {
            reason: format!(
                "{} отдаёт больше {} МиБ — при объявленных {} МиБ и потолке {} МиБ",
                source.describe(),
                budget / (1024 * 1024),
                declared / (1024 * 1024),
                MAX_ARCHIVE_BYTES / (1024 * 1024)
            ),
        },
    })?;

    if let Err(err) = flushed {
        return Err(if err.kind() == io::ErrorKind::StorageFull {
            FetchError::Space {
                reason: format!("{}: {err}", archive_path.display()),
            }
        } else {
            FetchError::Archive {
                reason: format!("{}: {err}", archive_path.display()),
            }
        });
    }

    // Поток кончился раньше обещанного — соединение было, отдали не то.
    // Для сети это обрыв (К-5), и класс у него сетевой; для бандла —
    // обрезанный ресурс, то есть порча.
    if received.bytes != declared {
        return Err(match source.origin() {
            Origin::Network => FetchError::Network {
                reason: format!(
                    "принято {} из {} байт — соединение прервалось ({})",
                    received.bytes,
                    declared,
                    source.describe()
                ),
            },
            Origin::Bundled => FetchError::Archive {
                reason: format!(
                    "прочитано {} из {} байт ({})",
                    received.bytes,
                    declared,
                    source.describe()
                ),
            },
        });
    }

    on_progress(FetchStage::Downloading, received.bytes, declared);

    Ok(received)
}

/// Почему приём остановился.
#[derive(Debug)]
enum Stop {
    /// Источник не читается.
    Read(io::Error),
    /// Файл приёма не пишется.
    Write(io::Error),
    /// Разрешённый объём исчерпан: записать этот кусок было бы уже
    /// нарушением границы.
    OverBudget { budget: u64 },
}

/// Переливает поток в файл, считая sha256 и вычитая принятое из бюджета.
///
/// Бюджет — `min(объявленный, потолок)`: **заголовки могут границу только
/// ужесточить**. Проверяется он до `write_all`, а не после, — иначе кусок,
/// нарушивший границу, успевал бы лечь на диск, и сторож объёма
/// превращался бы в сторож «на 64 КиБ позже».
///
/// Сумма считается по тем же байтам и в том же цикле. Второго прохода по
/// файлу нет намеренно: он стоил бы ещё одного чтения шестидесяти
/// мегабайт ровно ради того, что уже прошло через регистры.
fn drain_within_budget(
    reader: &mut dyn Read,
    writer: &mut dyn Write,
    declared: u64,
    on_progress: &mut dyn FnMut(u64),
) -> Result<Received, Stop> {
    let budget = declared.min(MAX_ARCHIVE_BYTES);
    let mut budget_left = budget;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; READ_BUFFER_BYTES];
    let mut received = 0_u64;
    let mut reported = 0_u64;

    loop {
        let read = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(Stop::Read(err)),
        };

        let chunk = read as u64;
        if chunk > budget_left {
            return Err(Stop::OverBudget { budget });
        }

        writer.write_all(&buffer[..read]).map_err(Stop::Write)?;
        hasher.update(&buffer[..read]);

        budget_left -= chunk;
        received += chunk;
        if received - reported >= PROGRESS_STEP_BYTES {
            reported = received;
            on_progress(received);
        }
    }

    Ok(Received {
        bytes: received,
        sha256: hex(&hasher.finalize()),
    })
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    bytes.iter().fold(String::with_capacity(64), |mut out, b| {
        // `write!` в `String` не отказывает; результат игнорируется
        // осознанно, а не по недосмотру.
        let _ = write!(out, "{b:02x}");
        out
    })
}

/// Убирает файл приёма. Отсутствие — не ошибка; неудача удаления не
/// меняет исхода, но обязана быть видна в логе: это те самые
/// шестьдесят мегабайт, которые иначе останутся молча.
fn discard(archive_path: &Path) {
    match fs::remove_file(archive_path) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => eprintln!(
            "yt-dlp: не удалось убрать {}: {err}",
            archive_path.display()
        ),
    }
}

/// Оборвавшееся чтение: класс зависит от того, откуда поток.
fn read_failure(source: &dyn ArchiveSource, err: &io::Error) -> FetchError {
    let reason = format!("{}: {err}", source.describe());
    match source.origin() {
        Origin::Network => FetchError::Network { reason },
        Origin::Bundled => FetchError::Archive { reason },
    }
}

/// Точка подключения троттлинга (С-4: «тот же ассет не перекачивается в
/// бесконечном цикле»).
///
/// Здесь только запись факта — по образцу [`RepairLog`] и в отдельном от
/// него файле (см. `super::layout::Layout::update_attempt_path`). Решение
/// «пробовать ли ещё раз и когда» принимает оркестрация TL-58: у неё
/// разные паузы у разных классов отказа, и знать про плановое расписание
/// и внеплановую проверку С-13 этому модулю нечем.
///
/// Неудача записи журнала не превращается в неудачу обновления: журнал —
/// страховка от долбления, а не условие работы.
fn record_failed_attempt(layout: &Layout, build_id: &BuildId, error: &FetchError) {
    let path = layout.update_attempt_path(build_id);
    let history = RepairLog::read(&path);
    let attempted = history.with_attempt(&error.to_string(), crate::clock::now_unix_secs());
    if let Err(err) = attempted.write_atomic(&path) {
        eprintln!("yt-dlp: не удалось записать историю попыток обновления: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use tempfile::{tempdir, TempDir};
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    /// Версия и сумма, которыми адресуется установка в тестах. Сумма
    /// подставляется настоящая — её считает [`sha256_of`].
    const CANDIDATE_VERSION: &str = "2026.09.01";

    /// Сумма правильной формы, которой заведомо не совпадёт ни один архив.
    /// Форма важна: с TL-56 сумма не той формы отбраковывается раньше, чем
    /// начинается приём, и тест про обрыв потока перестал бы проверять
    /// обрыв.
    const WRONG_SHA256: &str = "0000000000000000000000000000000000000000000000000000000000000000";

    /// Идентификатор заведомо приемлемой пары «версия + сумма».
    ///
    /// `expect`, а не тихий `unwrap_or`: если фикстура перестала проходить
    /// проверку, тест обязан упасть здесь и назвать причину, а не тихо
    /// проверять что-то другое.
    fn build_id_of(identity: ArchiveIdentity<'_>) -> BuildId {
        identity
            .build_id()
            .expect("идентификатор фикстуры обязан проходить проверку")
    }

    /// Готовый каталог данных с созданным корнем установок.
    struct Fixture {
        dir: TempDir,
        layout: Layout,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempdir().expect("tempdir");
            let layout = Layout::new(dir.path());
            layout.create_root().expect("корень создаётся");
            Self { dir, layout }
        }

        fn layout(&self) -> &Layout {
            &self.layout
        }

        /// Каталог данных приложения — тот, **внутри** которого лежит
        /// корень установок. Нужен там, где предмет проверки — что отказ
        /// не написал ничего уровнем выше корня.
        fn data_dir(&self) -> &Path {
            self.dir.path()
        }

        /// Имена всего, что лежит в корне установок.
        fn root_entries(&self) -> Vec<String> {
            entries_of(self.layout.root())
        }

        /// Никакого мусора: ни каталога установки, ни `.staging-*`, ни
        /// недокачанного архива. Журнал попыток исключён намеренно — он
        /// не побочный след, а объявленный результат отказа (точка
        /// троттлинга), и его отсутствие проверяется отдельным тестом.
        fn assert_no_debris(&self, build_id: &BuildId) {
            assert!(
                !self.layout.install_dir(build_id).exists(),
                "каталог установки не должен появиться: {:?}",
                self.root_entries()
            );
            assert!(
                !self.layout.manifest_path(build_id).exists(),
                "манифест не должен появиться: {:?}",
                self.root_entries()
            );
            for name in self.root_entries() {
                assert!(
                    !name.starts_with(".staging-"),
                    "остался каталог распаковки {name}"
                );
                assert!(
                    !name.starts_with(".download-"),
                    "остался недокачанный архив {name}"
                );
            }
        }
    }

    /// Отсортированные имена всего, что лежит в каталоге.
    ///
    /// Отдельной функцией, а не методом фикстуры, потому что тот же
    /// вопрос задаётся трём разным каталогам: корню установок, каталогу
    /// данных над ним и постороннему каталогу, в который целится
    /// враждебный абсолютный путь.
    fn entries_of(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap_or_else(|err| panic!("{} читается: {err}", dir.display()))
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// Поток, который не кончается никогда. Ровно то, чем прикидывается
    /// враждебный источник: заголовки честные, тело бесконечное.
    struct Endless;

    impl Read for Endless {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            buf.fill(0xAB);
            Ok(buf.len())
        }
    }

    /// Приёмник, который считает принятое, но не держит его.
    ///
    /// Нужен там, где предмет проверки — сколько байт ушло в `write_all`,
    /// а не что именно в них было. Копить ради этого 192 МиБ в памяти
    /// значило бы платить настоящей памятью за счётчик.
    #[derive(Default)]
    struct Counting(u64);

    impl Write for Counting {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0 += buf.len() as u64;
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn sha256_of(bytes: &[u8]) -> String {
        hex(&Sha256::digest(bytes))
    }

    fn sha256_of_file(path: &Path) -> String {
        sha256_of(&fs::read(path).expect("файл читается"))
    }

    /// Архив формы апстримного onedir-ассета: один исполняемый файл в
    /// корне плюс каталог `_internal` рядом.
    fn onedir_zip() -> Vec<u8> {
        let mut buffer = io::Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut buffer);
            let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);

            zip.start_file("yt-dlp_macos", stored.unix_permissions(0o755))
                .expect("start_file");
            zip.write_all(b"#!/bin/sh\necho 2026.09.01\n")
                .expect("write");

            zip.add_directory("_internal/", stored.unix_permissions(0o755))
                .expect("add_directory");
            zip.start_file("_internal/lib.so", stored.unix_permissions(0o755))
                .expect("start_file");
            zip.write_all(b"shared-library-bytes").expect("write");

            zip.finish().expect("finish");
        }
        buffer.into_inner()
    }

    /// Источник поверх готового куска байт: объявляет то, что скажут, и
    /// отдаёт то, что дали, — расхождение между этими двумя и есть
    /// предмет половины тестов ниже.
    fn source_of<'a>(
        origin: Origin,
        declared: u64,
        body: &'a [u8],
        opened: &'a Cell<u32>,
    ) -> impl ArchiveSource + 'a {
        StreamArchive::new(origin, "фикстура", declared, move || {
            opened.set(opened.get() + 1);
            Ok(Box::new(io::Cursor::new(body.to_vec())) as Box<dyn Read>)
        })
    }

    // --- сумма и её вычисление -----------------------------------------

    #[test]
    fn the_hash_matches_the_published_vectors_of_sha256() {
        // Значение суммы во всех остальных тестах считает тот же код,
        // который они проверяют, — сравнение было бы замкнутым само на
        // себя. Разрывается это здесь: два опубликованных вектора FIPS
        // 180-4 отвечают на вопрос «а сумма-то настоящая?» независимо от
        // всего остального модуля, вместе с переводом в шестнадцатеричный
        // вид (перепутанный порядок полубайтов дал бы другую строку той
        // же длины).
        assert_eq!(
            sha256_of(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_of(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    // --- склейка с метаданными релиза ------------------------------------

    #[test]
    fn an_update_asset_is_taken_apart_field_by_field() {
        // Сторож ровно на одну ошибку — перепутанные местами поля. Обе
        // строки `UpdateAsset` это строки, оба размера это числа, и
        // компилятор здесь не помогает ничем: `version: &asset.sha256`
        // соберётся молча. Значения подобраны так, чтобы подмена была
        // видна не «по типу», а по значению.
        let asset = UpdateAsset {
            version: "2026.09.01".to_string(),
            url: "https://example.invalid/yt-dlp_macos.zip".to_string(),
            sha256: "a1b2c3d4e5f6".repeat(5) + "0123",
            size_bytes: 53_923_637,
        };

        let identity = ArchiveIdentity::from(&asset);
        assert_eq!(
            identity.version, asset.version,
            "версия обязана прийти из version, а не из sha256"
        );
        assert_eq!(
            identity.sha256, asset.sha256,
            "сумма обязана прийти из sha256, а не из version"
        );
        assert_eq!(
            build_id_of(identity).as_str(),
            "2026.09.01-a1b2c3d4e5f6",
            "идентификатор собирается из версии и первых двенадцати символов суммы"
        );

        let source = network_source(&asset, || {
            unreachable!("склейка метаданных потока не открывает")
        });
        assert_eq!(source.origin(), Origin::Network);
        assert_eq!(
            source.declared_bytes(),
            asset.size_bytes,
            "объявленный размер обязан прийти из size_bytes: на нём стоят и \
             знаменатель полосы, и первый потолок"
        );
        assert_eq!(
            source.describe(),
            asset.url,
            "в диагностике обязан быть адрес ассета"
        );
    }

    // --- идентификатор кандидата как компонент пути -----------------------

    #[test]
    fn a_hostile_identity_never_becomes_a_path_and_never_opens_a_stream() {
        // Правка ревью TL-56 и её главный сторож. Версия и сумма приезжают
        // из метаданных релиза апстрима, то есть это непроверенный ввод, а
        // раскладка строит из них имена в каталоге данных. До проверки
        // ревью показало запуском: `version = "<абсолютный путь>/OWNED"`
        // уводил журнал попыток по абсолютному пути (`Path::join` с
        // абсолютным аргументом отбрасывает корень целиком), а
        // `version = "../PWNED"` — уровнем выше корня установок.
        //
        // Поэтому предмет проверки здесь **не текст ошибки**: он был
        // верным и тогда, когда файл уже лежал не там. Проверяются три
        // каталога, в которых после отказа не должно появиться ничего:
        // корень установок, каталог данных над ним и посторонний каталог,
        // в который целится абсолютный путь.
        let outside = tempdir().expect("tempdir");
        let body = onedir_zip();
        let good_sha = sha256_of(&body);
        let owned = outside.path().join("OWNED").display().to_string();

        let hostile: Vec<(&str, String, String)> = vec![
            ("абсолютный путь", owned, good_sha.clone()),
            (
                "выход на уровень выше",
                "../PWNED".to_string(),
                good_sha.clone(),
            ),
            (
                "то же под Windows",
                "..\\PWNED".to_string(),
                good_sha.clone(),
            ),
            ("обратный слэш", "a\\b".to_string(), good_sha.clone()),
            ("пустая версия", String::new(), good_sha.clone()),
            (
                "ведущая точка — маскировка под остаток работы",
                ".staging-2026.09.01".to_string(),
                good_sha.clone(),
            ),
            (
                "не-hex в сумме",
                CANDIDATE_VERSION.to_string(),
                format!("zz{}", &good_sha[2..]),
            ),
            (
                "сумма короче 64",
                CANDIDATE_VERSION.to_string(),
                good_sha[..12].to_string(),
            ),
            (
                "сумма длиннее 64",
                CANDIDATE_VERSION.to_string(),
                format!("{good_sha}00"),
            ),
        ];

        for (what, version, sha256) in hostile {
            let fixture = Fixture::new();
            let opened = Cell::new(0);
            let source = source_of(Origin::Network, body.len() as u64, &body, &opened);
            let identity = ArchiveIdentity {
                version: &version,
                sha256: &sha256,
            };

            let error = fetch_and_install(&source, identity, fixture.layout(), &mut |_, _, _| {})
                .expect_err(&format!(
                    "{what}: такой идентификатор обязан быть отвергнут"
                ));

            assert!(
                matches!(error, FetchError::Archive { .. }),
                "{what}: отказ обязан быть одним из пяти классов Ф-9, а не паникой: {error}"
            );
            assert_eq!(
                opened.get(),
                0,
                "{what}: поток открывать незачем — из такого идентификатора всё равно \
                 нечего адресовать"
            );
            assert!(
                fixture.root_entries().is_empty(),
                "{what}: в корне установок не должно появиться ничего, а там {:?}",
                fixture.root_entries()
            );
            assert_eq!(
                entries_of(fixture.data_dir()),
                vec!["yt-dlp".to_string()],
                "{what}: уровнем выше корня установок не должно появиться ничего"
            );
            assert!(
                entries_of(outside.path()).is_empty(),
                "{what}: в постороннем каталоге не должно появиться ничего, а там {:?}",
                entries_of(outside.path())
            );
        }
    }

    // --- потолок, проверка первая: по объявленному размеру ---------------

    #[test]
    fn refuses_by_the_declared_size_without_opening_the_stream() {
        // Смысл первой проверки не в отказе как таковом — отказать можно
        // было бы и после, — а в том, что запроса не возникает вовсе.
        // Поэтому сторож здесь не текст ошибки, а счётчик открытий
        // потока: он обязан остаться нулём.
        let fixture = Fixture::new();
        let body = onedir_zip();
        let opened = Cell::new(0);
        let source = source_of(Origin::Network, MAX_ARCHIVE_BYTES + 1, &body, &opened);
        let identity = ArchiveIdentity {
            version: CANDIDATE_VERSION,
            sha256: &sha256_of(&body),
        };

        let error = fetch_and_install(&source, identity, fixture.layout(), &mut |_, _, _| {})
            .expect_err("ассет больше потолка не скачивается");

        assert!(matches!(error, FetchError::Archive { .. }), "{error}");
        assert_eq!(
            opened.get(),
            0,
            "поток не должен открываться: отказ обязан случиться до запроса"
        );
        fixture.assert_no_debris(&build_id_of(identity));
    }

    #[test]
    fn refuses_an_asset_that_declares_nothing_to_download() {
        let fixture = Fixture::new();
        let body = onedir_zip();
        let opened = Cell::new(0);
        let source = source_of(Origin::Network, 0, &body, &opened);
        let identity = ArchiveIdentity {
            version: CANDIDATE_VERSION,
            sha256: &sha256_of(&body),
        };

        let error = fetch_and_install(&source, identity, fixture.layout(), &mut |_, _, _| {})
            .expect_err("нулевой размер — не ассет");

        assert!(matches!(error, FetchError::Source { .. }), "{error}");
        assert_eq!(opened.get(), 0, "и здесь запроса быть не должно");
    }

    #[test]
    fn accepts_the_largest_asset_upstream_has_ever_shipped() {
        // Вторая половина белого списка: потолок обязан пропускать то,
        // ради чего заведён. Число — не круглое, а самое большое
        // `assetBytes` набора замеров.
        let largest = measured_assets()
            .assets
            .iter()
            .map(|asset| asset.asset_bytes)
            .max()
            .expect("набор непуст");

        let source = StreamArchive::new(
            Origin::Network,
            "самый крупный ассет апстрима",
            largest,
            || unreachable!("проверка объявленного размера потока не открывает"),
        );
        check_declared_size(&source).expect("настоящий ассет обязан проходить потолок");
    }

    #[test]
    fn an_empty_source_is_told_apart_by_where_it_came_from() {
        // Нулевой размер у сети — «источник отдал не то» (строка 9
        // таблицы состояний, «GitHub не отвечает»); у бандла сети нет
        // вовсе, и предлагать пользователю ждать GitHub было бы враньём.
        for (origin, expect_source_class) in [(Origin::Network, true), (Origin::Bundled, false)] {
            let source = StreamArchive::new(origin, "пустой источник", 0, || {
                unreachable!("до открытия дело не доходит")
            });
            let error = check_declared_size(&source).expect_err("нулевой размер — не ассет");
            assert_eq!(
                matches!(error, FetchError::Source { .. }),
                expect_source_class,
                "{origin:?}: {error}"
            );
        }
    }

    // --- потолок, проверка вторая: по фактически принятым байтам ---------

    #[test]
    fn the_stream_is_capped_even_when_the_declared_size_lies() {
        // Проверка второй границы **в обход первой**: объявленный размер
        // здесь заведомо за потолком, то есть до приёма дело не дошло бы
        // вовсе. Тест зовёт приём напрямую именно затем, чтобы граница
        // была доказана сама по себе, а не через соседа. Убрать `min` в
        // `drain_within_budget` — и он покраснеет, оставив остальные
        // зелёными.
        let mut sink = Counting::default();
        let stop = drain_within_budget(&mut Endless, &mut sink, MAX_ARCHIVE_BYTES + 1, &mut |_| {})
            .expect_err("бесконечный поток обязан упереться в потолок");

        match stop {
            Stop::OverBudget { budget } => assert_eq!(
                budget, MAX_ARCHIVE_BYTES,
                "бюджет обязан быть потолком, а не объявленным размером"
            ),
            other => panic!("ожидался отказ по потолку, а не {other:?}"),
        }
        assert!(
            sink.0 <= MAX_ARCHIVE_BYTES,
            "за границу не должен попасть даже байт, на котором её нарушили: {}",
            sink.0
        );
    }

    #[test]
    fn refuses_a_stream_that_outgrows_what_it_promised() {
        // Живой сценарий: метаданные честные и потолок проходят, а тело
        // не кончается. Отказ обязан случиться на объявленном размере, и
        // недокачанный архив — уйти.
        let fixture = Fixture::new();
        let declared = 4 * PROGRESS_STEP_BYTES;
        let identity = ArchiveIdentity {
            version: CANDIDATE_VERSION,
            sha256: WRONG_SHA256,
        };
        let source = StreamArchive::new(
            Origin::Network,
            "бесконечный поток",
            declared,
            || Ok(Box::new(Endless) as Box<dyn Read>),
        );

        let mut seen = 0_u64;
        let error = fetch_and_install(
            &source,
            identity,
            fixture.layout(),
            &mut |stage, done, _| {
                if stage == FetchStage::Downloading {
                    seen = done;
                }
            },
        )
        .expect_err("поток, который не кончается, обязан быть оборван");

        assert!(matches!(error, FetchError::Archive { .. }), "{error}");
        assert!(
            seen <= declared,
            "прогресс не должен уходить за объявленный размер: {seen} из {declared}"
        );
        fixture.assert_no_debris(&build_id_of(identity));
    }

    // --- сумма и распаковка ---------------------------------------------

    #[test]
    fn does_not_unpack_an_archive_whose_sha256_is_not_what_was_promised() {
        // Критерий приёмки: распаковка не вызывается **вовсе**. Проверяется
        // не заглушкой вместо распаковки, а тем, что она обязана была бы
        // оставить, — каталогом установки, манифестом и `.staging-*`.
        let fixture = Fixture::new();
        let body = onedir_zip();
        let opened = Cell::new(0);
        let source = source_of(Origin::Network, body.len() as u64, &body, &opened);
        let identity = ArchiveIdentity {
            version: CANDIDATE_VERSION,
            sha256: WRONG_SHA256,
        };

        let error = fetch_and_install(&source, identity, fixture.layout(), &mut |_, _, _| {})
            .expect_err("сумма не сошлась");

        assert!(matches!(error, FetchError::Archive { .. }), "{error}");
        assert!(
            error.to_string().contains(&sha256_of(&body)),
            "в диагностике обязана быть фактическая сумма: {error}"
        );
        assert_eq!(opened.get(), 1, "поток открывался ровно один раз");
        fixture.assert_no_debris(&build_id_of(identity));
    }

    #[test]
    fn refuses_a_body_that_is_not_an_archive_at_all() {
        // Сумма сошлась, а внутри не zip: порча ловится уже распаковкой,
        // и её класс обязан остаться тем же «архив отброшен».
        let fixture = Fixture::new();
        let body = b"\x50\x4b\x03\x04 no, this is not a zip".to_vec();
        let opened = Cell::new(0);
        let source = source_of(Origin::Network, body.len() as u64, &body, &opened);
        let identity = ArchiveIdentity {
            version: CANDIDATE_VERSION,
            sha256: &sha256_of(&body),
        };

        let error = fetch_and_install(&source, identity, fixture.layout(), &mut |_, _, _| {})
            .expect_err("это не архив");

        assert!(matches!(error, FetchError::Archive { .. }), "{error}");
        fixture.assert_no_debris(&build_id_of(identity));
    }

    // --- успешный путь, оба источника ------------------------------------

    #[test]
    fn installs_a_verified_archive_and_leaves_no_partial_download() {
        let fixture = Fixture::new();
        let body = onedir_zip();
        let opened = Cell::new(0);
        let source = source_of(Origin::Network, body.len() as u64, &body, &opened);
        let sha256 = sha256_of(&body);
        let identity = ArchiveIdentity {
            version: CANDIDATE_VERSION,
            sha256: &sha256,
        };

        let mut stages = Vec::new();
        let prepared =
            fetch_and_install(&source, identity, fixture.layout(), &mut |stage, _, _| {
                if stages.last() != Some(&stage) {
                    stages.push(stage);
                }
            })
            .expect("целый архив с сошедшейся суммой обязан установиться");

        assert_eq!(prepared.build_id, build_id_of(identity));
        assert_eq!(prepared.version, CANDIDATE_VERSION);
        assert_eq!(
            prepared.dir,
            fixture.layout().install_dir(&prepared.build_id)
        );
        assert!(prepared.executable.is_file(), "исполняемый файл на месте");
        assert_eq!(
            stages,
            vec![FetchStage::Downloading, FetchStage::Unpacking],
            "этапы обязаны идти в этом порядке и оба"
        );

        for name in fixture.root_entries() {
            assert!(
                !name.starts_with(".download-") && !name.starts_with(".staging-"),
                "после удачной установки не должно остаться {name}"
            );
        }
    }

    #[test]
    fn the_manifest_records_the_version_that_was_installed_not_the_one_in_the_bundle() {
        // Регрессия ровно на то, чем манифест был до TL-56: версия и
        // сумма брались из констант пина. Установка обновления записала
        // бы в свой манифест версию бандла, `validate` этого не заметила
        // бы (она сверяет схему, число файлов и объём), а служебный экран
        // и уборка читают версию именно оттуда.
        let fixture = Fixture::new();
        let body = onedir_zip();
        let opened = Cell::new(0);
        let source = source_of(Origin::Network, body.len() as u64, &body, &opened);
        let sha256 = sha256_of(&body);
        let identity = ArchiveIdentity {
            version: CANDIDATE_VERSION,
            sha256: &sha256,
        };

        let prepared = fetch_and_install(&source, identity, fixture.layout(), &mut |_, _, _| {})
            .expect("установка удалась");

        let manifest = super::super::layout::Manifest::read(
            &fixture.layout().manifest_path(&prepared.build_id),
        )
        .expect("манифест написан");

        assert_eq!(manifest.yt_dlp_version, CANDIDATE_VERSION);
        assert_eq!(manifest.archive_sha256, sha256);
        assert_ne!(
            manifest.yt_dlp_version,
            super::super::layout::BUNDLED_VERSION,
            "версия кандидата и версия пина обязаны различаться в этом тесте, \
             иначе он ничего не проверяет"
        );
    }

    #[test]
    fn a_bundled_resource_goes_through_the_very_same_path() {
        // С-10: архив из бандла устанавливается тем же вызовом, что
        // сетевой, — отличается только источник. Если однажды путей
        // станет два, этот тест придётся переписать, и это ровно то
        // предупреждение, ради которого он есть.
        let fixture = Fixture::new();
        let staged = tempdir().expect("tempdir");
        let archive = staged.path().join("yt-dlp.zip");
        fs::write(&archive, onedir_zip()).expect("архив кладётся");

        let sha256 = sha256_of_file(&archive);
        let identity = ArchiveIdentity {
            version: CANDIDATE_VERSION,
            sha256: &sha256,
        };
        let source = BundledArchive::at(&archive).expect("ресурс бандла читается");
        assert_eq!(
            source.declared_bytes(),
            fs::metadata(&archive).unwrap().len()
        );

        let prepared = fetch_and_install(&source, identity, fixture.layout(), &mut |_, _, _| {})
            .expect("ресурс бандла обязан ставиться тем же путём");

        assert_eq!(prepared.build_id, build_id_of(identity));
        assert!(prepared.executable.is_file());
        assert!(
            archive.is_file(),
            "исходный ресурс бандла удалять нечего — он не наш временный файл"
        );
    }

    // --- обрыв: сеть и бандл говорят разное -------------------------------

    #[test]
    fn a_stream_that_stops_short_is_a_network_failure_when_it_came_from_the_network() {
        // К-5: обрыв посреди скачивания обязан читаться как «нет сети», а
        // не как «архив повреждён» — от этого зависит, какую строку
        // покажет блок обновления (8 против 10) и что пользователь
        // подумает про своё приложение.
        let fixture = Fixture::new();
        let body = onedir_zip();
        let opened = Cell::new(0);
        let source = source_of(Origin::Network, body.len() as u64 + 1024, &body, &opened);
        let identity = ArchiveIdentity {
            version: CANDIDATE_VERSION,
            sha256: &sha256_of(&body),
        };

        let error = fetch_and_install(&source, identity, fixture.layout(), &mut |_, _, _| {})
            .expect_err("недополученный поток — не установка");

        assert!(matches!(error, FetchError::Network { .. }), "{error}");
        fixture.assert_no_debris(&build_id_of(identity));
    }

    #[test]
    fn the_same_short_stream_from_the_bundle_is_a_corrupt_archive() {
        // Тот же обрыв, другой источник, другой класс: сети у бандла нет,
        // и предлагать пользователю проверить соединение было бы враньём.
        let fixture = Fixture::new();
        let body = onedir_zip();
        let opened = Cell::new(0);
        let source = source_of(Origin::Bundled, body.len() as u64 + 1024, &body, &opened);
        let identity = ArchiveIdentity {
            version: CANDIDATE_VERSION,
            sha256: &sha256_of(&body),
        };

        let error = fetch_and_install(&source, identity, fixture.layout(), &mut |_, _, _| {})
            .expect_err("обрезанный ресурс — не установка");

        assert!(matches!(error, FetchError::Archive { .. }), "{error}");
    }

    #[test]
    fn a_stream_that_fails_to_open_keeps_the_class_of_its_origin() {
        let fixture = Fixture::new();
        let identity = ArchiveIdentity {
            version: CANDIDATE_VERSION,
            sha256: WRONG_SHA256,
        };

        for (origin, expect_network) in [(Origin::Network, true), (Origin::Bundled, false)] {
            let source = StreamArchive::new(
                origin,
                "недоступный источник",
                1024,
                || Err(io::Error::new(io::ErrorKind::ConnectionReset, "оборвано")),
            );
            let error = fetch_and_install(&source, identity, fixture.layout(), &mut |_, _, _| {})
                .expect_err("источник не открылся");

            assert_eq!(
                matches!(error, FetchError::Network { .. }),
                expect_network,
                "{origin:?}: {error}"
            );
        }
    }

    // --- классы отказа контракта ------------------------------------------

    #[test]
    fn every_failure_maps_to_its_own_contract_class_and_carries_a_message() {
        let cases = [
            (
                FetchError::Network {
                    reason: "нет сети".into(),
                },
                "networkUnavailable",
                false,
            ),
            (
                FetchError::Source {
                    reason: "не отвечает".into(),
                },
                "sourceUnavailable",
                false,
            ),
            (
                FetchError::Archive {
                    reason: "сумма".into(),
                },
                "archiveCorrupted",
                true,
            ),
            (
                FetchError::Space {
                    reason: "места нет".into(),
                },
                "notEnoughSpace",
                true,
            ),
        ];

        for (error, expected_kind, names_version) in cases {
            let failure = error.to_failure(CANDIDATE_VERSION);
            let json = serde_json::to_value(&failure).expect("сериализуется");
            assert_eq!(json["kind"], expected_kind);
            assert!(
                !failure.message().is_empty(),
                "по сообщению разбирают жалобу: {expected_kind}"
            );
            assert_eq!(
                failure.version(),
                names_version.then_some(CANDIDATE_VERSION),
                "{expected_kind}: версию называют ровно те классы, которым она нужна"
            );
        }
    }

    #[test]
    fn running_out_of_room_for_the_archive_is_told_apart_from_a_corrupt_one() {
        // С-11 требует отдельного разговора про место: у него есть
        // действие пользователя, которого нет у «архив повреждён».
        let fixture = Fixture::new();
        let error = ensure_room_for_archive(fixture.layout().root(), u64::MAX / 2)
            .expect_err("столько места нет ни на одном томе");

        assert!(matches!(error, FetchError::Space { .. }), "{error}");
        assert!(
            matches!(
                error.to_failure(CANDIDATE_VERSION),
                YtDlpUpdateFailure::NotEnoughSpace { .. }
            ),
            "{error}"
        );
    }

    #[test]
    fn a_prepare_failure_about_space_does_not_become_a_corrupt_archive() {
        // Установка приходит чужой ошибкой (TL-12/TL-18), и раскладка её
        // на классы контура — единственное место, где «нет места» может
        // потеряться среди «архив отброшен».
        let space: FetchError = PrepareError::NotEnoughSpace {
            path: "/x".into(),
            needed: 1,
            available: 0,
        }
        .into();
        assert!(matches!(space, FetchError::Space { .. }), "{space}");

        for other in [
            PrepareError::ArchiveCorrupted {
                reason: "crc".into(),
            },
            PrepareError::UnpackFailed {
                reason: "нет прав".into(),
            },
            PrepareError::LayoutUnexpected {
                reason: "два файла".into(),
            },
            PrepareError::ArchiveMissing { path: "/x".into() },
            PrepareError::DataDirUnavailable {
                reason: "исчез".into(),
            },
        ] {
            let mapped: FetchError = other.into();
            assert!(
                matches!(mapped, FetchError::Archive { .. }),
                "остальные отказы установки — один класс: {mapped}"
            );
        }
    }

    // --- точка троттлинга --------------------------------------------------

    #[test]
    fn a_failed_attempt_is_written_next_to_the_installation_and_not_into_the_repair_log() {
        // Форма журнала та же, файл — другой. У С-10 build id обновления
        // совпадает с build id пина, и общий файл означал бы, что неудача
        // скачивания обновления считается неудачей починки и однажды
        // запрещает подготовку рабочей установки на старте.
        let fixture = Fixture::new();
        let body = onedir_zip();
        let opened = Cell::new(0);
        let identity = ArchiveIdentity {
            version: CANDIDATE_VERSION,
            sha256: WRONG_SHA256,
        };

        for expected in 1..=2 {
            let source = source_of(Origin::Network, body.len() as u64, &body, &opened);
            fetch_and_install(&source, identity, fixture.layout(), &mut |_, _, _| {})
                .expect_err("сумма не сошлась");

            let log =
                RepairLog::read(&fixture.layout().update_attempt_path(&build_id_of(identity)));
            assert_eq!(
                log.attempts, expected,
                "попытки обязаны накапливаться — иначе троттлингу TL-58 не на чем стоять"
            );
            assert!(!log.last_reason.is_empty(), "причина обязана быть записана");
        }

        assert!(
            !fixture
                .layout()
                .repair_path(&build_id_of(identity))
                .exists(),
            "журнал починки подготовки трогать нельзя"
        );
    }

    #[test]
    fn a_successful_install_does_not_write_an_attempt_log() {
        let fixture = Fixture::new();
        let body = onedir_zip();
        let opened = Cell::new(0);
        let source = source_of(Origin::Network, body.len() as u64, &body, &opened);
        let sha256 = sha256_of(&body);
        let identity = ArchiveIdentity {
            version: CANDIDATE_VERSION,
            sha256: &sha256,
        };

        fetch_and_install(&source, identity, fixture.layout(), &mut |_, _, _| {})
            .expect("установка удалась");

        assert!(
            !fixture
                .layout()
                .update_attempt_path(&build_id_of(identity))
                .exists(),
            "удачная установка не должна оставлять записи о неудаче"
        );
    }

    // --- потолок против настоящих ассетов апстрима -------------------------

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct MeasuredAsset {
        release: String,
        asset: String,
        asset_bytes: u64,
    }

    #[derive(serde::Deserialize)]
    struct MeasuredAssets {
        #[serde(rename = "trees")]
        assets: Vec<MeasuredAsset>,
    }

    fn measured_assets() -> MeasuredAssets {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/ytdlp-onedir/upstream-trees.json");
        let raw = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("{} не читается: {err}", path.display()));
        serde_json::from_str(&raw)
            .unwrap_or_else(|err| panic!("{} — не тот конверт: {err}", path.display()))
    }

    #[test]
    fn the_archive_ceiling_clears_every_asset_upstream_has_ever_shipped() {
        // Тот же трёхкратный запас и та же причина, что у потолков
        // распаковки: потолок, к которому реальность подошла вплотную, —
        // это отложенная поломка обновления сразу у всех пользователей.
        // Что фикстура снята с того релиза, который вложен в бандл,
        // держит сторож `unpack::tests::the_pinned_release_is_the_one…`.
        const HEADROOM_FACTOR: u64 = 3;

        let measured = measured_assets();
        assert!(
            measured.assets.len() >= 6,
            "набор замеров подозрительно мал: {}",
            measured.assets.len()
        );

        for asset in &measured.assets {
            assert!(
                asset.asset_bytes * HEADROOM_FACTOR <= MAX_ARCHIVE_BYTES,
                "{} {}: архив {} МиБ против потолка {} МиБ — запаса меньше чем в \
                 {HEADROOM_FACTOR} раза",
                asset.release,
                asset.asset,
                asset.asset_bytes / (1024 * 1024),
                MAX_ARCHIVE_BYTES / (1024 * 1024)
            );
        }
    }

    // `assertions_on_constants` здесь именно то, что нужно: тест на то и
    // существует, чтобы утверждение о константе было записано отдельно от
    // неё самой и ломалось при её правке.
    #[allow(clippy::assertions_on_constants)]
    #[test]
    fn the_archive_ceiling_is_pinned_from_above_as_well_as_from_below() {
        // Снизу потолок держит тест выше — он обязан быть не меньше
        // замеров с запасом. Сверху его не держит больше ничто: мутация
        // `MAX_ARCHIVE_BYTES = 8 ГиБ` оставила бы набор зелёным, а расход
        // памяти на разбор центрального каталога вырос бы вместе с ней в
        // те же 4,3 раза.
        assert!(
            MAX_ARCHIVE_BYTES <= 192 * 1024 * 1024,
            "потолок размера архива поднят выше обоснованного замерами"
        );
        // И сходимость с соседом: архив честной формы разворачивается в
        // дерево заметно большего объёма, поэтому архив «по потолку»
        // обязан укладываться в потолок распаковки. Двинуть один потолок,
        // не взглянув на другой, этот assert не даст.
        //
        // Множитель — 13/5, то есть 2,6, и взят он не у пина, а у худшего
        // из замеров, округлённого ВВЕРХ. Числа, чтобы следующий читатель
        // не выводил их заново (оба — из `upstream-trees.json`, поля
        // `assetBytes` и `unpackedBytes`):
        //
        //   пин   2026.08.19 yt-dlp_macos.zip
        //         53 923 637 → 130 010 634 байт, отношение 2,411
        //   худшее 2024.12.03 yt-dlp_macos.zip
        //         58 875 811 → 150 646 457 байт, отношение 2,559
        //
        // Первая редакция сторожа стояла на 2,4 — отношении пина,
        // округлённом ВНИЗ, — то есть охраняла границу оптимистичнее
        // измерений и утверждала при этом обратное (ревью TL-56). Правило
        // здесь простое и общее: сторож не имеет права быть оптимистичнее
        // того, что измерено, иначе он охраняет не ту границу, о которой
        // говорит.
        //
        // Сходимость от смены множителя не ломается: 192 МиБ × 2,6 =
        // 499,2 МиБ против 512 МиБ, запас 2,5 % вместо прежних 10 %.
        // Запас честно тонкий, и это его работа: следующее движение
        // любого из двух потолков обязано упереться сюда, а не разойтись
        // молча.
        assert!(
            MAX_ARCHIVE_BYTES * 13 / 5 <= super::super::unpack::MAX_UNPACKED_BYTES,
            "потолок архива разошёлся с потолком распаковки: {} МиБ × 2,6 (худшее \
             измеренное отношение разворачивания) не влезает в {} МиБ",
            MAX_ARCHIVE_BYTES / (1024 * 1024),
            super::super::unpack::MAX_UNPACKED_BYTES / (1024 * 1024)
        );
    }
}
