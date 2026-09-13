//! «Показать в папке» (Ф-8, Р-8 эпика E5) — TL-88.
//!
//! Свои вызовы системных команд, ноль крейтов, ни одной строки через shell.
//! `tauri-plugin-opener` отклонён измерением (Н-5: +37 крейтов на Linux).
//!
//! Модуль делится на две половины:
//!
//! - [`build_plan`] — **чистая** функция. Она не трогает диск и не
//!   запускает процессы, а ОС получает параметром, а не через `cfg!`.
//!   Поэтому все три ветки проверяются тестами на macOS;
//! - [`reveal_with`] — три случая Ф-8 поверх плана: `stat` файла и папки,
//!   затем запуск через [`Launcher`]. В тестах запускатель подменяется. В
//!   продакшене работает [`SystemLauncher`]. [`reveal_with_path`] ищет
//!   утилиты Linux в переданном `PATH`, а [`reveal`] собирает всё для
//!   текущей ОС и `PATH` процесса.
//!
//! Команда Tauri `show_in_folder` — TL-90. Она строит путь через
//! `HistoryRecord::file_path()`, зовёт [`reveal`] в блокирующем пуле
//! (`spawn_blocking`) и переводит [`RevealError`] в контрактный
//! `ShowInFolderError` (соответствие — у [`RevealError`]).
//!
//! # Команды по ОС
//!
//! | ОС | файл есть | файла нет, папка есть |
//! |---|---|---|
//! | macOS | `/usr/bin/open -R -- <файл>` | `/usr/bin/open -R -- <папка>` |
//! | Windows | `explorer.exe /select,"<файл>"` (сырая строка) | `explorer.exe /select,"<папка>"` |
//! | Linux | D-Bus `ShowItems([file://…])`, при отказе `<xdg-open> <папка>` | `<xdg-open> <папка>` |
//!
//! На Linux каждая утилита запускается по абсолютному пути, найденному в
//! абсолютных каталогах `PATH` ([`find_executable`]); `<xdg-open>` в таблице
//! — такой путь. По имени запускается только `explorer.exe`.
//!
//! ## macOS: `--` работает, папка показывается через `-R`
//!
//! Измерено на `/usr/bin/open` (macOS 26, 2026-09-14):
//!
//! ```text
//! $ /usr/bin/open -R -rf.mp4
//! open: invalid option -- r                       ← без `--` имя стало опциями
//! $ /usr/bin/open -R -- -rf.mp4
//! The file /…/-rf.mp4 does not exist.             ← с `--` это имя файла
//! ```
//!
//! Путь к тому же всегда абсолютный, то есть начинается с `/`. `--` — вторая
//! защита, и она не зависит от первой.
//!
//! Папку macOS тоже **показывает** (`-R`), а не открывает. `open <папка>`
//! для каталога-пакета (`Foo.app`) запустит приложение, а С-2 запрещает
//! запускать что-либо, кроме файлового менеджера. Узнать, пакет ли каталог,
//! можно только у LaunchServices, а не по имени. Цена: Finder открывается
//! на родительской папке с выделенной папкой назначения, и в неё нужен ещё
//! один двойной щелчок.
//!
//! ## Windows: сырая командная строка, а не `Command::arg`
//!
//! `explorer.exe` не разбирает командную строку правилами MSVCRT
//! (`CommandLineToArgvW`). Он делит её на ключи по запятым, а кавычки
//! защищают запятые и пробелы внутри пути. `std::process::Command::arg` на
//! Windows экранирует аргумент по правилам MSVCRT
//! (`library/std/src/sys/args/windows.rs`, `append_arg`, rustc 1.98). Он
//! берёт аргумент в кавычки **целиком** и только если в нём есть пробел или
//! табуляция. Отсюда оба провала очевидных вариантов:
//!
//! | как собрать | строка для explorer | что выйдет |
//! |---|---|---|
//! | `.arg("/select,C:\a b\x.mp4")` | `"/select,C:\a b\x.mp4"` | весь токен в кавычках, ключа `/select` в нём нет, и explorer открывает папку по умолчанию |
//! | `.arg("/select,C:\a,b\x.mp4")` | `/select,C:\a,b\x.mp4` | кавычек нет, запятая режет путь, выделяется `C:\a` |
//! | `.args(["/select,", path])` | `/select, C:\a,b\x.mp4` | пробелы переживает, запятую — нет |
//! | **`raw_arg("/select,\"C:\a,b c\x.mp4\"")`** | **`/select,"C:\a,b c\x.mp4"`** | путь в кавычках, запятые и пробелы внутри защищены |
//!
//! Последняя строка — наш вариант ([`CommandArgs::WindowsCommandLine`],
//! `CommandExt::raw_arg`). Он читается одинаково при любом из двух
//! толкований кавычек, поэтому от догадок о парсере explorer не зависит:
//!
//! - `"` в пути Win32 недопустим. Такой путь отклоняется
//!   ([`PathRejection::ForbiddenChar`]), так что экранировать кавычку не
//!   нужно;
//! - обратная косая перед закрывающей кавычкой (`\"`) не возникает никогда.
//!   Имя файла на `\` не кончается. У папки хвостовые `\` срезаются, а корень
//!   диска `X:\` уходит без кавычек: пробелов и запятых в нём нет. Правило
//!   «`\` перед `"` — экранирование» из MSVCRT поэтому не срабатывает ни
//!   разу.
//!
//! Префикс `\\?\` explorer не понимает и снимается (`\\?\UNC\` → `\\`),
//! `/` приводятся к `\`. Абсолютность проверяется по тексту: `X:\` или
//! `\\сервер\…`. Ведущего дефиса опасаться нечего: путь начинается с буквы
//! диска или `\\` и стоит внутри кавычек после `/select,`.
//!
//! Папку Windows тоже **показывает** (`/select,"<папка>"`), как macOS
//! (`-R`), а не открывает. Без ключа explorer применяет к аргументу действие
//! по умолчанию. Если между `stat` папки и запуском на её месте окажется
//! файл, `explorer.exe "<путь>"` запустит его, а С-2 это запрещает. С
//! `/select,` explorer только выделяет элемент в родительской папке, что бы
//! там ни лежало. Цена та же, что на macOS: открывается родитель, в папку
//! назначения нужен ещё один двойной щелчок. Корень диска уходит как
//! `/select,X:\`.
//!
//! `explorer.exe` запускается по имени. Путь ищет сам `Command` (std 1.98,
//! `library/std/src/sys/process/windows.rs`, `search_paths`) в таком
//! порядке: каталог приложения → System32 → каталог Windows → `PATH`.
//! Первым шагом был бы `PATH` потомка, но только если его задали через
//! `Command::env`; мы его не задаём. Текущий каталог не просматривается.
//! Сам explorer лежит в каталоге Windows. Порядок взят из исходника std, а
//! не из прогона.
//!
//! **Не измерено** (Р-6 E1: машин под Windows в проекте нет). Первое —
//! поведение explorer в таблице выше; это сведения из отчётов, а не прогон.
//! Второе — код возврата: explorer, передав запрос уже работающему
//! экземпляру, по многочисленным отчётам возвращает 1 и при успехе. Поэтому
//! его код не считается отказом ([`LauncherKind::Explorer`]), отказ — только
//! если процесс не запустился.
//!
//! ## Linux: D-Bus, затем `xdg-open` на папку
//!
//! Все три утилиты — `dbus-send`, `gdbus`, `xdg-open` — ищет одна функция
//! ([`find_executable`], набор — [`LinuxTools`]): исполняемый файл в
//! **абсолютном** каталоге из `PATH`. Пустой или относительный элемент
//! `PATH` пропускается. libc (`execvp`) считает пустой элемент текущим
//! каталогом, поэтому запуск по имени подхватил бы `xdg-open`, лежащий
//! рядом с процессом. Найденная утилита запускается по абсолютному пути.
//! Если `xdg-open` не найден, план содержит [`PlanStep::NotFound`]: ничего не
//! запускается, а исход — отказ `NotStarted(NotFound)`.
//!
//! Признак «есть D-Bus» (Р-8) — найденный `dbus-send` или `gdbus`. Шину и
//! сеть поиск не трогает. Порядок такой:
//!
//! 1. если инструмент найден — `ShowItems` через него. Приоритет у
//!    `dbus-send`: пакет `dbus` есть почти везде, `gdbus` лежит в
//!    `libglib2.0-bin`, который ставят не всегда. Второй инструмент после
//!    отказа первого не пробуется: отказ `ServiceUnknown` (нет
//!    `FileManager1`) у обоих одинаков, и это только удвоило бы ожидание;
//! 2. если инструмента нет или вызов не удался — `xdg-open <папка>`, без
//!    выделения. Если нет и `xdg-open`, отказ говорит о нём: нечем открыть
//!    папку, и это полезнее ответа шины.
//!
//! `dbus-send` зовётся с `--print-reply`, иначе он не ждёт ответа и
//! возвращает 0, даже если метод не существует
//! (`dbus/tools/dbus-send.c`: без `print_reply` — `dbus_connection_send` и
//! `exit (0)`). С ним ошибка ответа даёт `exit (1)`.
//!
//! Путь уходит в D-Bus как URI `file://`, процентно-кодированный
//! ([`file_uri`]) по белому списку RFC 3986: латиница, цифры, `-._~` и `/`.
//! Всё остальное кодируется `%XX`. Это нужно не только ради корректного URI.
//! `dbus-send` режет `array:string:` по запятым (`strtok (dupval, ",")`), а
//! `gdbus` разбирает аргумент как текст GVariant `['…']`, где `'` и `\`
//! значимы. После кодирования ни `,`, ни `'`, ни `\` в URI не остаётся.
//!
//! `xdg-open` получает абсолютный путь **без** `--`. xdg-utils 1.1.3 (2018,
//! до сих пор в Ubuntu 22.04 и Debian 12) на `--` отвечает
//! `unexpected option '--'`, и понимать его научился только 1.2.0 (2024).
//! Защиту здесь даёт абсолютность: путь начинается с `/`, а не с `-`.
//!
//! # Потолок времени и процессы-сироты (Н-3)
//!
//! Запуск ждёт выхода утилиты, но не дольше потолка ([`Ceilings`]). Что
//! делать с тем, кто не уложился, зависит от утилиты ([`LauncherKind`]):
//!
//! - `open -R`, `dbus-send`, `gdbus` передают запрос и выходят. Задержка
//!   дольше потолка означает зависание, поэтому процесс убивается. Для
//!   `open` это отказ `TimedOut`, для D-Bus — переход к `xdg-open`.
//!   `dbus-send`/`gdbus` получают свой таймаут ответа
//!   ([`DBUS_REPLY_TIMEOUT`]) на секунду короче потолка, чтобы в штатном
//!   случае выйти сами;
//! - `xdg-open` в части окружений исполняет обработчик **на переднем
//!   плане** и живёт, пока открыто окно файлового менеджера. `explorer`
//!   остаётся жить, если оболочки Windows не было. Для них потолок — окно
//!   наблюдения ([`Ceilings::linger`]): успел выйти — судим по коду; не
//!   успел — запуск состоялся, процесс отпускается. Убить его значило бы
//!   закрыть пользователю окно.
//!
//! Реестр E1 (`sidecar::ChildRegistry`) здесь **не нужен и вреден**. Он
//! существует, чтобы работа sidecar не пережила приложение. Процесс показа
//! — наоборот, окно пользователя, и выход из приложения не должен его
//! закрывать (Ф-8). Что остаётся после запуска:
//!
//! - убитый процесс дожидается `wait()` сразу. Убивается **прямой
//!   потомок** (`Child::kill` — `SIGKILL` на его pid), а не группа
//!   процессов. По потолку убиваются только `open`, `dbus-send` и `gdbus`.
//!   Своих процессов они не порождают: Finder поднимает launchd, файловый
//!   менеджер — шина. Поэтому после них не остаётся ничего. Этот вывод верен
//!   только для таких утилит. Внук, которого породила бы утилита, пережил бы
//!   убийство и остался бы сиротой у init/launchd. По отказу ожидания
//!   (`WaitFailed`) убивается утилита любого рода, в том числе `xdg-open`, и
//!   её обработчик тоже переживёт убийство;
//! - внук, унаследовавший пайп stderr, держит его открытым до своего выхода.
//!   Запуск из-за этого не виснет: `finish` ждёт конца потока не дольше
//!   `DRAIN_GRACE`. Но поток чтения `os-reveal-stderr` живёт, пока жив внук,
//!   а в хвост попадает только прочитанное к этому моменту. После выхода
//!   приложения конец чтения закрывается, и внук на записи в stderr получит
//!   `SIGPIPE`;
//! - отпущенный дожидается в отдельном потоке, так что зомби за время
//!   жизни приложения не копятся. После выхода приложения его подбирает
//!   init/launchd — ровно так, как должно быть для окна пользователя;
//! - у отпускаемых утилит stderr направлен в `/dev/null`, а не в пайп.
//!   Пайп закрылся бы вместе с приложением, и обработчик переднего плана
//!   (GTK пишет в stderr предупреждения) получил бы `SIGPIPE`: `Command`
//!   возвращает дочернему процессу обработку сигнала по умолчанию, и окно
//!   умерло бы при выходе из приложения. Цена — у `xdg-open` нет хвоста
//!   stderr в «Подробнее», есть только код (1 — синтаксис, 2 — нет файла,
//!   3 — нечем открыть, 4 — действие не удалось).

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, Command, ExitStatus, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::sidecar::stderr_tail;

/// Абсолютный путь `open` на macOS (Ф-8).
pub const MACOS_OPEN: &str = "/usr/bin/open";
/// `explorer.exe` по имени. Порядок поиска у `Command` — в doc модуля,
/// «Windows».
pub const WINDOWS_EXPLORER: &str = "explorer.exe";
/// Имя `xdg-open`. Запускается не по имени, а по пути из
/// [`find_executable`].
pub const XDG_OPEN: &str = "xdg-open";

const FM1_NAME: &str = "org.freedesktop.FileManager1";
const FM1_PATH: &str = "/org/freedesktop/FileManager1";
const FM1_SHOW_ITEMS: &str = "org.freedesktop.FileManager1.ShowItems";

/// Таймаут ответа D-Bus, который получают `dbus-send`/`gdbus`.
///
/// Он короче [`Ceilings::exit`] на секунду: в штатном случае утилита
/// сдаётся сама, и убивать её не приходится. Четыре секунды покрывают
/// холодную D-Bus-активацию файлового менеджера.
pub const DBUS_REPLY_TIMEOUT: Duration = Duration::from_secs(4);

/// Сколько байт stderr держится в памяти, пока утилита работает. Экрану
/// нужен хвост в [`crate::sidecar::STDERR_TAIL_MAX_CHARS`] символов, так
/// что 64 КиБ хватает с запасом и на многобайтовый текст.
const STDERR_BUFFER_BYTES: usize = 64 * 1024;
/// Шаг опроса `try_wait`. Опрашивается дочерний процесс, а не бэкенд.
const POLL_STEP: Duration = Duration::from_millis(10);
/// Сколько ждать конца stderr после выхода утилиты.
const DRAIN_GRACE: Duration = Duration::from_millis(250);

/// Целевая ОС плана. Параметр, а не `cfg!`: так тесты на одной машине
/// проверяют все три ветки.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetOs {
    MacOs,
    Windows,
    /// Linux и прочие Unix с `xdg-open`.
    Linux,
}

impl TargetOs {
    /// ОС, под которую собрано приложение.
    pub const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Linux
        }
    }
}

/// Что показывать.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevealTarget {
    /// Файл на месте — показать его выделенным.
    SelectFile,
    /// Файла нет, папка есть — показать папку.
    OpenFolder,
}

/// Инструмент D-Bus, найденный в `PATH`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbusFlavor {
    DbusSend,
    Gdbus,
}

impl DbusFlavor {
    /// Порядок предпочтения — см. doc модуля, «Linux».
    pub const PREFERENCE: [Self; 2] = [Self::DbusSend, Self::Gdbus];

    pub const fn program_name(self) -> &'static str {
        match self {
            Self::DbusSend => "dbus-send",
            Self::Gdbus => "gdbus",
        }
    }
}

/// `dbus-send` или `gdbus` с абсолютным путём, по которому он найден.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbusTool {
    pub flavor: DbusFlavor,
    pub program: PathBuf,
}

/// Утилиты Linux, найденные в `PATH` одной функцией ([`find_executable`]).
/// На macOS и Windows не нужны: там набор пустой.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinuxTools {
    pub dbus: Option<DbusTool>,
    /// Абсолютный путь `xdg-open`.
    pub xdg_open: Option<PathBuf>,
}

impl LinuxTools {
    /// Ищет утилиты в абсолютных каталогах `path_var`.
    pub fn find(path_var: Option<&OsStr>) -> Self {
        Self {
            dbus: find_dbus_tool(path_var),
            xdg_open: find_executable(XDG_OPEN, path_var),
        }
    }
}

/// Какая утилита запускается. От этого зависят судьба кода возврата,
/// stderr и поведение по истечении потолка (doc модуля, «Потолок»).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LauncherKind {
    /// `open -R` на macOS.
    Finder,
    /// `explorer.exe` на Windows.
    Explorer,
    /// `dbus-send`/`gdbus` → `FileManager1.ShowItems`.
    DbusShowItems,
    /// `xdg-open` на папку.
    XdgOpen,
}

impl LauncherKind {
    /// Считается ли ненулевой код отказом. У explorer — нет (doc модуля).
    pub const fn exit_code_is_meaningful(self) -> bool {
        !matches!(self, Self::Explorer)
    }

    /// Может ли утилита штатно жить дольше потолка. Такую по потолку не
    /// убивают, а отпускают.
    pub const fn may_stay_running(self) -> bool {
        matches!(self, Self::Explorer | Self::XdgOpen)
    }

    /// Собирается ли stderr. У отпускаемых утилит — нет: пайп пережил бы
    /// приложение и уронил бы окно по `SIGPIPE` (doc модуля).
    pub const fn captures_stderr(self) -> bool {
        !self.may_stay_running()
    }
}

/// Аргументы команды.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandArgs {
    /// Отдельные элементы `argv` (macOS, Linux).
    Argv(Vec<OsString>),
    /// Хвост командной строки Windows как есть, для `raw_arg`. Экранирование
    /// `Command::arg` explorer не понимает (doc модуля, «Windows»).
    WindowsCommandLine(String),
}

/// Одна запускаемая команда: программа, аргументы, род утилиты.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevealCommand {
    pub program: OsString,
    pub args: CommandArgs,
    pub kind: LauncherKind,
}

/// Шаг плана.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanStep {
    /// Запустить команду.
    Run(RevealCommand),
    /// Утилиты с этим именем нет в абсолютных каталогах `PATH`. Ничего не
    /// запускается; исход шага — отказ `NotStarted(NotFound)`.
    NotFound(&'static str),
}

/// План показа: шаг и, на Linux, запасной шаг на его отказ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevealPlan {
    pub first: PlanStep,
    /// Исполняется, только если `first` отказал.
    pub fallback: Option<PlanStep>,
}

/// Почему путь не годится для показа. Проверка идёт до диска и процессов.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PathRejection {
    /// Не абсолютный для целевой ОС. На Unix это ещё и защита от ведущего
    /// дефиса: абсолютный путь начинается с `/`.
    #[error("путь не абсолютный для целевой ОС")]
    NotAbsolute,
    /// Путь Windows не в Юникоде: для explorer строка собирается текстом.
    /// История и так хранит папку и имя в UTF-8.
    #[error("путь не в Юникоде")]
    NotUnicode,
    /// Кавычка или управляющий символ в пути Windows. В Win32 их не бывает,
    /// а кавычку в строке для explorer не экранировать.
    #[error("в пути Windows недопустимый символ")]
    ForbiddenChar,
    /// У файла нет родительской папки (путь — корень).
    #[error("у пути нет родительской папки")]
    NoParent,
}

/// Строит план показа `path` на `os`. Чистая функция.
///
/// Для [`RevealTarget::SelectFile`] `path` — файл, для
/// [`RevealTarget::OpenFolder`] — папка. `tools` учитываются только на
/// Linux; D-Bus — только при выделении файла.
///
/// ```text
/// build_plan(MacOs, SelectFile, "/Users/u/-rf.mp4", &LinuxTools::default())
///   → /usr/bin/open ["-R", "--", "/Users/u/-rf.mp4"]
/// ```
pub fn build_plan(
    os: TargetOs,
    target: RevealTarget,
    path: &Path,
    tools: &LinuxTools,
) -> Result<RevealPlan, PathRejection> {
    match os {
        TargetOs::MacOs => {
            let path = unix_absolute(path)?;
            // Папка тоже через `-R`: `open <пакет>.app` запустил бы
            // приложение (doc модуля, «macOS»).
            let args = vec!["-R".into(), "--".into(), path.to_os_string()];
            Ok(RevealPlan {
                first: PlanStep::Run(RevealCommand {
                    program: MACOS_OPEN.into(),
                    args: CommandArgs::Argv(args),
                    kind: LauncherKind::Finder,
                }),
                fallback: None,
            })
        }
        TargetOs::Windows => {
            let path = windows_path(path)?;
            // Папка тоже через `/select,`: без ключа explorer применил бы к
            // пути действие по умолчанию (doc модуля, «Windows»).
            let line = format!("/select,{}", explorer_quoted(&path));
            Ok(RevealPlan {
                first: PlanStep::Run(RevealCommand {
                    program: WINDOWS_EXPLORER.into(),
                    args: CommandArgs::WindowsCommandLine(line),
                    kind: LauncherKind::Explorer,
                }),
                fallback: None,
            })
        }
        TargetOs::Linux => {
            let path = unix_absolute(path)?;
            let xdg_open = tools.xdg_open.as_deref();
            match target {
                RevealTarget::OpenFolder => Ok(RevealPlan {
                    first: xdg_open_step(xdg_open, path),
                    fallback: None,
                }),
                RevealTarget::SelectFile => {
                    let folder = Path::new(path)
                        .parent()
                        .ok_or(PathRejection::NoParent)?
                        .as_os_str();
                    let open_folder = xdg_open_step(xdg_open, folder);
                    match &tools.dbus {
                        None => Ok(RevealPlan {
                            first: open_folder,
                            fallback: None,
                        }),
                        Some(tool) => Ok(RevealPlan {
                            first: PlanStep::Run(dbus_show_items(
                                tool,
                                &file_uri(Path::new(path))?,
                            )),
                            fallback: Some(open_folder),
                        }),
                    }
                }
            }
        }
    }
}

/// URI `file://` для абсолютного Unix-пути, процентно-кодированный белым
/// списком RFC 3986: `A–Z a–z 0–9 - . _ ~` и `/` как есть, остальные
/// байты — `%XX` заглавными.
///
/// Кодируются байты пути, а не символы: имя не в UTF-8 (законно на Linux)
/// и NFD проходят без искажения. Нормализации нет — байты на диске те же.
pub fn file_uri(path: &Path) -> Result<String, PathRejection> {
    let bytes = unix_path_bytes(path)?;
    if bytes.first() != Some(&b'/') {
        return Err(PathRejection::NotAbsolute);
    }
    let mut uri = String::with_capacity("file://".len() + bytes.len() * 3);
    uri.push_str("file://");
    for &byte in bytes {
        if is_uri_path_safe(byte) {
            uri.push(char::from(byte));
        } else {
            uri.push('%');
            uri.push(hex_digit(byte >> 4));
            uri.push(hex_digit(byte & 0x0f));
        }
    }
    Ok(uri)
}

/// Байт, который путь URI несёт как есть.
pub const fn is_uri_path_safe(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/')
}

const fn hex_digit(nibble: u8) -> char {
    // `nibble` — всегда 0..=15: вызывается с `>> 4` и `& 0x0f` от байта.
    let digit = if nibble < 10 {
        b'0' + nibble
    } else {
        b'A' + nibble - 10
    };
    digit as char
}

/// Ищет `dbus-send`, затем `gdbus` через [`find_executable`] (признак «есть
/// D-Bus», Р-8). Шина не трогается.
pub fn find_dbus_tool(path_var: Option<&OsStr>) -> Option<DbusTool> {
    DbusFlavor::PREFERENCE.into_iter().find_map(|flavor| {
        find_executable(flavor.program_name(), path_var).map(|program| DbusTool { flavor, program })
    })
}

/// Первый исполняемый файл `name` в **абсолютных** каталогах `path_var`.
///
/// Пустой или относительный элемент `PATH` пропускается. libc считает
/// пустой элемент текущим каталогом, а оттуда программу не берём. Этой
/// функцией ищутся все утилиты Linux, и найденный путь уходит в запуск как
/// есть.
pub fn find_executable(name: &str, path_var: Option<&OsStr>) -> Option<PathBuf> {
    std::env::split_paths(path_var?)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable_file(candidate))
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|meta| meta.is_file())
}

fn unix_absolute(path: &Path) -> Result<&OsStr, PathRejection> {
    // Первый байт `/` одинаков в любой кодировке `OsStr`: и сырые байты
    // Unix, и WTF-8 совместимы с ASCII.
    if path.as_os_str().as_encoded_bytes().first() == Some(&b'/') {
        Ok(path.as_os_str())
    } else {
        Err(PathRejection::NotAbsolute)
    }
}

#[cfg(unix)]
fn unix_path_bytes(path: &Path) -> Result<&[u8], PathRejection> {
    use std::os::unix::ffi::OsStrExt;
    Ok(path.as_os_str().as_bytes())
}

#[cfg(not(unix))]
fn unix_path_bytes(path: &Path) -> Result<&[u8], PathRejection> {
    // Ветка Linux на хосте Windows не исполняется; строгость здесь дешевле
    // догадки о кодировке.
    path.to_str()
        .map(str::as_bytes)
        .ok_or(PathRejection::NotUnicode)
}

/// Путь Windows в форме, которую понимает explorer: `\`-разделители, без
/// `\\?\`, абсолютный, без `"` и управляющих символов.
fn windows_path(path: &Path) -> Result<String, PathRejection> {
    let raw = path.to_str().ok_or(PathRejection::NotUnicode)?;
    if raw.chars().any(|c| c == '"' || c.is_control()) {
        return Err(PathRejection::ForbiddenChar);
    }
    let slashed = raw.replace('/', "\\");
    let plain = if let Some(rest) = slashed.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = slashed.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        slashed
    };

    let bytes = plain.as_bytes();
    let drive =
        bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\';
    // `\\сервер\…`, но не `\\?\…` и не `\\.\…` (пространство устройств).
    let unc = plain
        .strip_prefix(r"\\")
        .and_then(|rest| rest.chars().next())
        .is_some_and(|first| !matches!(first, '\\' | '?' | '.'));
    if drive || unc {
        Ok(plain)
    } else {
        Err(PathRejection::NotAbsolute)
    }
}

/// Путь в кавычках для explorer, так что `\"` не возникает никогда
/// (doc модуля, «Windows»).
fn explorer_quoted(path: &str) -> String {
    let trimmed = path.trim_end_matches('\\');
    let bytes = trimmed.as_bytes();
    if bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        // Корень диска: `X:` без `\` значило бы «текущий каталог диска», а
        // `"X:\"` оставило бы `\` перед кавычкой. Пробелов и запятых в
        // `X:\` нет, кавычки не нужны.
        return format!("{trimmed}\\");
    }
    format!("\"{trimmed}\"")
}

/// `xdg-open <папка>` по найденному пути. Не нашёлся — шаг-отказ, а не
/// запуск по имени (doc модуля, «Linux»).
fn xdg_open_step(xdg_open: Option<&Path>, folder: &OsStr) -> PlanStep {
    match xdg_open {
        Some(program) => PlanStep::Run(RevealCommand {
            program: program.as_os_str().to_os_string(),
            // Без `--`: xdg-utils 1.1.3 его не понимает (doc модуля, «Linux»).
            // Путь абсолютный и начинается с `/`.
            args: CommandArgs::Argv(vec![folder.to_os_string()]),
            kind: LauncherKind::XdgOpen,
        }),
        None => PlanStep::NotFound(XDG_OPEN),
    }
}

fn dbus_show_items(tool: &DbusTool, uri: &str) -> RevealCommand {
    let args: Vec<OsString> = match tool.flavor {
        DbusFlavor::DbusSend => vec![
            "--session".into(),
            "--print-reply".into(),
            format!("--dest={FM1_NAME}").into(),
            "--type=method_call".into(),
            format!("--reply-timeout={}", DBUS_REPLY_TIMEOUT.as_millis()).into(),
            FM1_PATH.into(),
            FM1_SHOW_ITEMS.into(),
            format!("array:string:{uri}").into(),
            // Второй аргумент ShowItems — startup id, пустая строка.
            "string:".into(),
        ],
        DbusFlavor::Gdbus => vec![
            "call".into(),
            "--session".into(),
            "--dest".into(),
            FM1_NAME.into(),
            "--object-path".into(),
            FM1_PATH.into(),
            "--method".into(),
            FM1_SHOW_ITEMS.into(),
            "--timeout".into(),
            DBUS_REPLY_TIMEOUT.as_secs().to_string().into(),
            // Текст GVariant: массив из одной строки. В кодированном URI нет
            // ни `'`, ни `\`.
            format!("['{uri}']").into(),
            // Пустая строка GVariant — две кавычки, а не пустой аргумент.
            "''".into(),
        ],
    };
    RevealCommand {
        program: tool.program.clone().into_os_string(),
        args: CommandArgs::Argv(args),
        kind: LauncherKind::DbusShowItems,
    }
}

// ---------------------------------------------------------------------------
// Запуск
// ---------------------------------------------------------------------------

/// Почему утилита показа не справилась.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchCause {
    /// Процесс не запустился: утилиты нет, нет прав.
    NotStarted(io::ErrorKind),
    /// Завершился с ненулевым кодом или сигналом.
    ExitedWithError,
    /// Не завершился за потолок и убит.
    TimedOut(Duration),
    /// Ожидание процесса отказало на уровне ОС.
    WaitFailed(io::ErrorKind),
    /// Команда собрана для другой ОС: сырую строку Windows вне Windows не
    /// передать.
    Unsupported,
}

impl fmt::Display for LaunchCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotStarted(kind) => write!(f, "не запустилась ({kind})"),
            Self::ExitedWithError => f.write_str("завершилась с ошибкой"),
            Self::TimedOut(ceiling) => write!(f, "не ответила за {} мс", ceiling.as_millis()),
            Self::WaitFailed(kind) => write!(f, "ожидание процесса отказало ({kind})"),
            Self::Unsupported => f.write_str("команда собрана для другой ОС"),
        }
    }
}

/// Отказ утилиты показа. Поля `exit_code` и `stderr_tail` — ровно форма
/// контрактного `LauncherFailureDetails`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LauncherFailure {
    /// Имя программы без каталога — для лога, путь пользователя в нём не
    /// появляется (Н-1).
    pub program: String,
    pub cause: LaunchCause,
    pub exit_code: Option<i32>,
    /// Хвост stderr по конвенции [`stderr_tail`].
    pub stderr_tail: Option<String>,
}

impl fmt::Display for LauncherFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.program, self.cause)?;
        if let Some(code) = self.exit_code {
            write!(f, ", код {code}")?;
        }
        Ok(())
    }
}

impl std::error::Error for LauncherFailure {}

/// Запускатель команды. Подменяется в тестах трёх случаев Ф-8.
pub trait Launcher {
    fn launch(&self, command: &RevealCommand) -> Result<(), LauncherFailure>;
}

/// Потолки времени запуска (doc модуля, «Потолок»).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ceilings {
    /// Для утилит, которые обязаны выйти сами (`open`, D-Bus). По истечении
    /// процесс убивается.
    pub exit: Duration,
    /// Окно наблюдения для утилит, которые штатно остаются жить
    /// (`xdg-open`, explorer). По истечении процесс отпускается, это успех.
    pub linger: Duration,
}

impl Ceilings {
    /// Продакшен-значения. Худший случай на Linux: D-Bus завис (5 с), затем
    /// `xdg-open` (2 с) — 7 с. На macOS 5 с, на Windows 2 с.
    pub const PRODUCTION: Self = Self {
        exit: Duration::from_secs(5),
        linger: Duration::from_secs(2),
    };

    const fn for_kind(self, kind: LauncherKind) -> Duration {
        if kind.may_stay_running() {
            self.linger
        } else {
            self.exit
        }
    }
}

impl Default for Ceilings {
    fn default() -> Self {
        Self::PRODUCTION
    }
}

/// Настоящий запуск через `std::process::Command`, без shell.
///
/// Синхронный: ждёт не дольше потолка. Вызывать из блокирующего пула, не
/// из async-задачи.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemLauncher {
    pub ceilings: Ceilings,
}

impl Launcher for SystemLauncher {
    fn launch(&self, command: &RevealCommand) -> Result<(), LauncherFailure> {
        self.launch_prepared(command, |_| {})
    }
}

impl SystemLauncher {
    /// Запуск, в котором `prepare` правит `Command` перед `spawn`. Продакшен
    /// ничего не правит. Тесты задают **потомку** `PATH` и текущий каталог,
    /// не трогая их у своего процесса: параллельные тесты не гоняются.
    fn launch_prepared(
        &self,
        command: &RevealCommand,
        prepare: impl FnOnce(&mut Command),
    ) -> Result<(), LauncherFailure> {
        let fail = |cause, exit_code, stderr_tail| LauncherFailure {
            program: program_label(&command.program),
            cause,
            exit_code,
            stderr_tail,
        };

        let mut process = Command::new(&command.program);
        match &command.args {
            CommandArgs::Argv(args) => {
                process.args(args);
            }
            CommandArgs::WindowsCommandLine(line) => {
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    process.raw_arg(line);
                }
                #[cfg(not(windows))]
                {
                    let _ = line;
                    return Err(fail(LaunchCause::Unsupported, None, None));
                }
            }
        }

        prepare(&mut process);
        let kind = command.kind;
        process
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(if kind.captures_stderr() {
                Stdio::piped()
            } else {
                Stdio::null()
            });

        let mut child = process
            .spawn()
            .map_err(|err| fail(LaunchCause::NotStarted(err.kind()), None, None))?;
        let drain = child.stderr.take().map(StderrDrain::start);

        let ceiling = self.ceilings.for_kind(kind);
        let deadline = Instant::now() + ceiling;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let tail = drain.and_then(StderrDrain::finish);
                    return judge(kind, status)
                        .map_err(|exit_code| fail(LaunchCause::ExitedWithError, exit_code, tail));
                }
                Ok(None) if Instant::now() < deadline => thread::sleep(POLL_STEP),
                Ok(None) => break,
                Err(err) => {
                    kill_and_reap(&mut child);
                    let tail = drain.and_then(StderrDrain::finish);
                    return Err(fail(LaunchCause::WaitFailed(err.kind()), None, tail));
                }
            }
        }

        if kind.may_stay_running() {
            release(child);
            return Ok(());
        }
        kill_and_reap(&mut child);
        let tail = drain.and_then(StderrDrain::finish);
        Err(fail(LaunchCause::TimedOut(ceiling), None, tail))
    }
}

/// `Err(код)` — отказ; `None` внутри — завершение сигналом.
fn judge(kind: LauncherKind, status: ExitStatus) -> Result<(), Option<i32>> {
    if status.success() || !kind.exit_code_is_meaningful() {
        Ok(())
    } else {
        Err(status.code())
    }
}

fn program_label(program: &OsStr) -> String {
    Path::new(program)
        .file_name()
        .unwrap_or(program)
        .to_string_lossy()
        .into_owned()
}

fn kill_and_reap(child: &mut Child) {
    // Уже вышедший между `try_wait` и `kill` процесс — не ошибка.
    let _ = child.kill();
    let _ = child.wait();
}

/// Отпускает живой процесс: ждёт его в отдельном потоке, чтобы не копились
/// зомби, и не убивает никогда.
fn release(mut child: Child) {
    let spawned = thread::Builder::new()
        .name("os-reveal-reaper".into())
        .spawn(move || {
            let _ = child.wait();
        });
    // Поток не создался — процесс останется зомби до выхода приложения,
    // окно пользователя от этого не страдает.
    drop(spawned);
}

/// Фоновое чтение stderr: пайп не переполнится, пока утилита работает.
struct StderrDrain {
    buffer: Arc<Mutex<Vec<u8>>>,
    done: mpsc::Receiver<()>,
}

impl StderrDrain {
    fn start(mut pipe: ChildStderr) -> Self {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let (tx, done) = mpsc::channel();
        let sink = Arc::clone(&buffer);
        let spawned = thread::Builder::new()
            .name("os-reveal-stderr".into())
            .spawn(move || {
                let mut chunk = [0_u8; 4096];
                loop {
                    match pipe.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => {
                            let mut kept = sink.lock().unwrap_or_else(|p| p.into_inner());
                            kept.extend_from_slice(&chunk[..read]);
                            if kept.len() > STDERR_BUFFER_BYTES {
                                let excess = kept.len() - STDERR_BUFFER_BYTES;
                                kept.drain(..excess);
                            }
                        }
                    }
                }
                let _ = tx.send(());
            });
        // Без потока просто не будет хвоста; `finish` не ждёт дольше
        // `DRAIN_GRACE`.
        drop(spawned);
        Self { buffer, done }
    }

    fn finish(self) -> Option<String> {
        let _ = self.done.recv_timeout(DRAIN_GRACE);
        let kept = self.buffer.lock().unwrap_or_else(|p| p.into_inner());
        stderr_tail(&String::from_utf8_lossy(&kept))
    }
}

// ---------------------------------------------------------------------------
// Три случая Ф-8
// ---------------------------------------------------------------------------

/// Отказ «Показать в папке» в домене. Соответствие контрактному
/// `ShowInFolderErrorKind` (перевод — TL-90):
///
/// | здесь | контракт |
/// |---|---|
/// | `FileMissing` | `fileMissing` — папка **уже открыта** |
/// | `FolderMissing` | `folderMissing` — ничего не запускалось |
/// | `LauncherFailed` | `launcherFailed` с `exitCode`/`stderrTail` |
/// | `Rejected` | `launcherFailed` без деталей, причина в `message` |
///
/// `Rejected` через `HistoryRecord::file_path()` на macOS и Linux не
/// возникает: там путь уже абсолютный. На Windows он возможен для пути с
/// `"` или не в Юникоде. Показать такой путь нечем, это отказ механизма
/// показа, а не отсутствие файла.
///
/// **Отказ `stat` любого рода — это отсутствие.** `EACCES` на папке или на
/// любом её родителе, `EIO`, `ETIMEDOUT` отключённого сетевого тома
/// классифицируются так же, как «нет на месте», — это `FileMissing` или
/// `FolderMissing`. Так же считает статус файла в истории
/// (`storage::history`, `file_status`), и экран не может показать запись
/// «на месте», которую показ назовёт пропавшей, или наоборот. Если файл
/// недоступен, а папка доступна (например, у папки нет права поиска, а у
/// родителя есть), получается `FileMissing`, и папка открывается. Если
/// недоступна и папка, получается `FolderMissing`. Причина отказа `stat`
/// наружу не передаётся.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RevealError {
    /// Файла нет, папка есть и **открыта** без выделения.
    #[error("файла нет на месте, открыта его папка")]
    FileMissing,
    /// Нет ни файла, ни папки. Ничего не запускалось.
    #[error("нет ни файла, ни его папки")]
    FolderMissing,
    #[error("механизм показа отказал: {0}")]
    LauncherFailed(LauncherFailure),
    #[error("путь не годится для показа: {0}")]
    Rejected(#[from] PathRejection),
}

/// «Показать в папке» для текущей ОС настоящими утилитами.
///
/// Блокирующая: `stat` (сетевой том может задуматься) и ожидание утилиты до
/// потолка. TL-90 зовёт её через `spawn_blocking`.
pub fn reveal(file: &Path) -> Result<(), RevealError> {
    reveal_with_path(
        TargetOs::current(),
        file,
        std::env::var_os("PATH").as_deref(),
        &SystemLauncher::default(),
    )
}

/// [`reveal_with`], где утилиты Linux ищутся в `path_var`
/// ([`LinuxTools::find`]). На macOS и Windows `path_var` не читается.
pub fn reveal_with_path(
    os: TargetOs,
    file: &Path,
    path_var: Option<&OsStr>,
    launcher: &impl Launcher,
) -> Result<(), RevealError> {
    let tools = match os {
        TargetOs::Linux => LinuxTools::find(path_var),
        TargetOs::MacOs | TargetOs::Windows => LinuxTools::default(),
    };
    reveal_with(os, file, &tools, launcher)
}

/// Три случая Ф-8 с заданными ОС, утилитами Linux и запускателем.
///
/// - файл есть — показать его выделенным; `Ok(())`;
/// - файла нет, папка есть — показать папку и **затем** вернуть
///   [`RevealError::FileMissing`]. Если показ папки отказал —
///   [`RevealError::LauncherFailed`]: `FileMissing` обещает, что папка
///   открыта;
/// - нет и папки — [`RevealError::FolderMissing`], без запуска.
///
/// Оба плана строятся до обращения к диску: негодный путь отклоняется, не
/// тронув ни ФС, ни процессов. «Файл есть» значит, что `stat` удался и это
/// обычный файл, как у статуса файла в истории (`is_file`): каталог на месте
/// файла — не файл. Отказ `stat` любого рода (`EACCES` на папке или
/// родителе, `EIO`, `ETIMEDOUT` на сетевом томе) — отсутствие того, что
/// проверялось: `FileMissing` или `FolderMissing` (подробно — у
/// [`RevealError`]).
pub fn reveal_with(
    os: TargetOs,
    file: &Path,
    tools: &LinuxTools,
    launcher: &impl Launcher,
) -> Result<(), RevealError> {
    let select = build_plan(os, RevealTarget::SelectFile, file, tools)?;
    let folder = file.parent().ok_or(PathRejection::NoParent)?;
    let open_folder = build_plan(os, RevealTarget::OpenFolder, folder, tools)?;

    if std::fs::metadata(file).is_ok_and(|meta| meta.is_file()) {
        run_plan(&select, launcher).map_err(RevealError::LauncherFailed)
    } else if std::fs::metadata(folder).is_ok_and(|meta| meta.is_dir()) {
        run_plan(&open_folder, launcher).map_err(RevealError::LauncherFailed)?;
        Err(RevealError::FileMissing)
    } else {
        Err(RevealError::FolderMissing)
    }
}

/// Первый шаг, при его отказе — запасной. Отказ — последний.
fn run_plan(plan: &RevealPlan, launcher: &impl Launcher) -> Result<(), LauncherFailure> {
    match (run_step(&plan.first, launcher), &plan.fallback) {
        (Ok(()), _) => Ok(()),
        (Err(failure), None) => Err(failure),
        (Err(_), Some(fallback)) => run_step(fallback, launcher),
    }
}

fn run_step(step: &PlanStep, launcher: &impl Launcher) -> Result<(), LauncherFailure> {
    match step {
        PlanStep::Run(command) => launcher.launch(command),
        PlanStep::NotFound(program) => Err(LauncherFailure {
            program: (*program).to_string(),
            cause: LaunchCause::NotStarted(io::ErrorKind::NotFound),
            exit_code: None,
            stderr_tail: None,
        }),
    }
}

#[cfg(test)]
#[path = "os_reveal_tests.rs"]
mod tests;
