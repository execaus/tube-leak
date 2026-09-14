//! Подготовка yt-dlp к работе: распаковка дерева в каталог данных,
//! прогрев и события о ходе (TL-12).
//!
//! # Порядок действий
//!
//! 1. Убрать мусор `.staging-*` от прерванных подготовок.
//! 2. Проверить установку ([`super::layout::validate`]). Нет или не
//!    сходится — распаковать заново.
//! 3. Если дерево уже было на месте — **проверить его запуском**
//!    (`--version` с коротким таймаутом). Отозвалось быстро — подготовка не
//!    нужна, ни одного события не отправлено, экран подготовки не
//!    показывается.
//! 4. Иначе — прогреть: один прогон `--version` с длинным таймаутом.
//! 5. Если дерево сходится с манифестом, но не запускается — переустановить
//!    и прогреть заново, но не бесконечно: см. «Починка не повторяется вечно».
//!
//! # Почему «прогрев» вообще существует
//!
//! macOS берёт ~0,42 с за первую загрузку каждого только что созданного
//! Mach-O-файла: `dyld` при отображении сегментов регистрирует подпись
//! через `fcntl`, запрос обслуживает `syspolicyd`, результат кэшируется по
//! inode. В дереве yt-dlp 107 таких файлов, и первый запуск после
//! распаковки платит за все загруженные разом — измеренные 24,6–36,4 с.
//! Дальше те же inode стоят 0,30–0,35 с. Прогрев — это способ заплатить
//! эту цену один раз, на экране подготовки, а не при первом скачивании.
//!
//! Распараллеливать прогрев бесполезно: проверка сериализована в
//! `syspolicyd` (замер TL-12: 8 воркеров на 107 файлов — 63,58 с против
//! ~45 с последовательных). Поэтому прогон ровно один.
//!
//! # Почему проверка запуском на каждом старте, а не флаг в манифесте
//!
//! «Тёплое» состояние принадлежит не нам, а ОС, и наблюдать его снаружи
//! нельзя. Записанный флаг «уже прогрето» соврал бы в тот день, когда ОС
//! забудет свой кэш, и служебный экран получил бы тридцатисекундный
//! таймаут вместо честного этапа подготовки. Проверка запуском стоит
//! 0,30–0,35 с на обычном старте и самостоятельно возвращает приложение в
//! рабочее состояние в любом случае, когда кэш пропал.
//!
//! # Починка не повторяется вечно
//!
//! Дерево, которое сошлось с манифестом, но не запускается, лечится
//! переустановкой — она стоит 124 МиБ записи и ~35 с прогрева. Если
//! переустановка не помогла, повторять её на каждом запуске приложения
//! незачем: причина не в содержимом дерева, а во внешнем по отношению к
//! нему обстоятельстве (снятые права на каталог данных, чужой антивирус,
//! несовместимая ОС), и ещё один заход даст тот же исход, отняв те же
//! полминуты. Поэтому факт безуспешной попытки записывается рядом с
//! установкой ([`super::layout::RepairLog`]) и переживает перезапуск, а
//! после [`MAX_REPAIR_ATTEMPTS`] попыток подготовка отвечает отказом сразу
//! — за доли секунды вместо тридцати пяти.
//!
//! Терминальное состояние не вечно, иначе оно поймало бы в ловушку любого,
//! кто устранил причину: счётчик обнуляется удачным запуском, не
//! действует для другого build id (обновление yt-dlp или приложения) и
//! остывает сам через [`REPAIR_COOLDOWN`]. Отдельного `kind` у отказа нет
//! намеренно: снаружи это по-прежнему `warmupFailed` — «дерево на месте, но
//! yt-dlp не запускается», — и контракт с фронтендом не меняется, меняется
//! только цена повторного выяснения этого факта.
//!
//! # Медленная машина — не отказ (TL-21)
//!
//! Обратный случай: дерево цело и запускается, но прогрев на этой машине
//! не укладывается в [`WARMUP_TIMEOUT`] (очень медленный диск, антивирус,
//! проверяющий каждый загружаемый файл). Отказом это не объявляется ни
//! разу. Упёршийся в таймаут прогрев оставляет рядом с установкой отметку
//! ([`SlowWarmupMark`]), подготовка отвечает успехом — дерево по манифесту
//! цело и считается готовым, — а прогрев продолжается в фоне
//! ([`BackgroundWarmup`]) с отдельным, более длинным таймаутом. Следующий
//! старт, увидев отметку, не пробует дерево и не ждёт прогрева вовсе:
//! сразу отвечает успехом и снова отдаёт прогрев в фон. Удачный прогрев —
//! переднего плана или фоновый — отметку снимает.
//!
//! Цена названа. Пока фоновый прогрев идёт, первые запуски yt-dlp
//! (проверка служебного экрана, разбор ссылки) тоже холодные и могут не
//! уложиться в собственные таймауты — это видно пользователю строкой
//! «не отвечает» с кнопкой повтора, а не экраном подготовки на две минуты
//! при каждом запуске приложения. Фоновый прогрев, окончившийся не
//! таймаутом, а отказом запуска, отметку тоже снимает: это уже не
//! медленная машина, и следующий старт идёт обычным путём — с пробой и,
//! если нужно, переустановкой.
//!
//! # Что подготовка отдаёт служебному экрану (TL-23)
//!
//! Проба тёплого дерева — это `--version`, ровно тот запуск, который
//! служебный экран сделал бы секундой позже. Поэтому её вывод, штамп и
//! длительность уходят в итог ([`PrepareOutcome::warm_launch`]), и
//! `check_sidecar` строит из них строку yt-dlp вместо второго запуска
//! (распорядок — `super::session`). Отдаётся только проба: прогрев тоже
//! печатает версию, но длится десятки секунд, и `durationMs` на экране
//! перестал бы значить то, что значит для отдельного запуска.
//!
//! # Замеры на собранном дистрибутиве
//!
//! Всё ниже снято на `.dmg`, смонтированном `hdiutil attach`, запуском
//! `tube-leak.app/Contents/MacOS/tube-leak` (Apple Silicon, macOS 26.6);
//! отсчёт — от запуска приложения:
//!
//! | сценарий                                        | что заняло          |
//! |-------------------------------------------------|---------------------|
//! | первый запуск, каталог данных пуст                | распаковка 1,27 с, прогрев 36,2 с |
//! | второй запуск                                     | проверка yt-dlp завершилась к 1,03 с |
//! | после подготовки, прерванной на распаковке        | распаковка 1,0 с, прогрев 25,1 с |
//! | прямой запуск установленного дерева, 5 раз        | 0,40–0,51 с          |
//!
//! Прерванная распаковка при этом оставила `.staging-*` на 57 МиБ и ни
//! одного каталога установки — следующий запуск убрал остаток и
//! распаковал заново. Прерванный прогрев оставил целое дерево с
//! манифестом: следующий запуск распаковку не повторял, только прогрел.
//!
//! Замер (TL-12, Apple Silicon, macOS 26.6): результат проверки подписи
//! переживает размонтирование и повторное монтирование тома — после
//! `hdiutil detach`/`attach` запуск занял 1,27 с, а не 36 с, и системная
//! база `/var/db/SystemPolicyConfiguration/ExecPolicy` в момент холодных
//! запусков менялась. То есть кэш не живёт в памяти vnode, а лежит на
//! диске, и переживает перезагрузку с большой вероятностью — но сама
//! перезагрузка в рамках задачи не выполнялась, и код на это не
//! опирается.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter};

use super::error::PrepareError;
use super::layout::{self, BuildId, Installed, Layout, RepairLog, SlowWarmupMark};
use super::state::{self, InUse, InUseGuard, InstallEntry, InstallState};
use super::unpack;
use crate::sidecar::{self, ChildRegistry, RunOutput, SidecarError};
use crate::types::{YtDlpPrepareEvent, YtDlpPrepareStage, YtDlpPrepared};

/// Имя Tauri-события с ходом подготовки. Полезная нагрузка —
/// [`YtDlpPrepareEvent`].
pub const PREPARE_EVENT: &str = "ytdlp://prepare";

/// Аргументы прогона, которым дерево прогревается и одновременно
/// сообщает свою версию.
///
/// Прогон ровно один. Проверено замером (TL-12): после прогрева
/// `--version` первое «настоящее» обращение — `--simulate` по URL с
/// недостижимым хостом, то есть полный путь с загрузкой сетевого стека и
/// машинерии extractor'ов — заняло 0,75 с, повторное 0,68 с, а
/// `--list-extractors` 0,36 с. Значит `--version` уже прогревает
/// практически все файлы, которые нужны настоящей работе, и добавлять к
/// прогреву новые команды не за чем: остаток холодной цены не превышает
/// одного файла (~0,42 с).
const WARMUP_ARGS: &[&str] = &["--version"];

/// Таймаут проверки «дерево уже тёплое?».
///
/// Замеры TL-12 (Apple Silicon, macOS 26.6, APFS/NVMe, `/usr/bin/time -p`,
/// `real`), onedir-дерево в каталоге данных:
///
/// | состояние дерева                                   | `--version`  |
/// |----------------------------------------------------|--------------|
/// | тёплое, подряд 11 запусков                          | 0,30–0,35 с  |
/// | тёплое, после detach/attach тома (кэш vnode сброшен)| 1,27 с       |
/// | тёплое, следующий запуск после того же              | 0,35 с       |
/// | холодное (только что распаковано)                   | 24,6–36,4 с  |
///
/// Значение 5 с — почти 4× к худшему тёплому замеру (1,27 с) и на порядок
/// меньше любого холодного. Промахнуться в сторону «решили, что холодное»
/// стоит одного лишнего прогрева; промахнуться в другую сторону
/// невозможно: холодное дерево не отвечает и за 20 с.
///
/// Публичен ради одного сторожа: запуск пробы заменяет запуск служебного
/// экрана (TL-23), и `crate::commands::sidecar` на этапе компиляции
/// проверяет, что проба не дольше его таймаута — иначе экран показал бы
/// `ok` там, где его собственный запуск упёрся бы в «не отвечает».
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Таймаут прогрева.
///
/// Арифметический потолок цены: 107 Mach-O-файлов × 0,42 с ≈ 45 с — это
/// если бы загрузились все. Фактические замеры первого запуска после
/// распаковки: 24,58 / 25,33 / 36,37 с (TL-12) и 27,8–39,1 с (диагностика
/// в issue #13). Значение 120 с — 2,7× к арифметическому потолку и ~3× к
/// худшему измеренному, то есть покрывает машину втрое медленнее
/// эталонной.
///
/// Таймаут здесь не выводится из бюджета служебного экрана (Н-2, 10 с):
/// экран подготовки — отдельный этап, он показывает ход и не обязан
/// уложиться в бюджет запуска. Служебный экран после подготовки видит уже
/// тёплое дерево, и его таймауты живут в `crate::commands::sidecar`.
pub(super) const WARMUP_TIMEOUT: Duration = Duration::from_secs(120);

// Соотношение сторожится на этапе компиляции: проверка «тёплое ли дерево»
// обязана быть заметно короче прогрева, иначе она перестаёт отличать одно
// состояние от другого и превращается в удвоенный прогрев.
const _: () = assert!(PROBE_TIMEOUT.as_secs() * 10 <= WARMUP_TIMEOUT.as_secs());

/// Таймаут прогрева, продолжающегося в фоне (TL-21).
///
/// Фоновый прогрев никто не ждёт, поэтому таймаут здесь не бюджет
/// ожидания, а граница, за которой зависший процесс снимается. Из замера
/// значение не выведено и вывести его не из чего: машины, где прогрев не
/// укладывается в [`WARMUP_TIMEOUT`], не измерял никто. Десять минут — 5×
/// к таймауту переднего плана и ~16× к худшему измеренному прогреву
/// (36,4 с). Упрётся фоновый прогрев и в них — отметка остаётся, и
/// следующий старт снова отдаст прогрев в фон; сохраняет ли убитый процесс
/// часть проверенных ОС подписей, не измерялось, и код на это не опирается.
const BACKGROUND_WARMUP_TIMEOUT: Duration = Duration::from_secs(10 * 60);

// Фоновый прогрев обязан быть не короче переднего плана: иначе отметка,
// записанная как раз потому, что прогрев не уложился, заставляла бы
// следующий старт пробовать то же самое с ещё меньшим шансом.
const _: () = assert!(WARMUP_TIMEOUT.as_secs() < BACKGROUND_WARMUP_TIMEOUT.as_secs());

/// Доля общего прогресса, отданная распаковке. Распаковка занимает
/// 1,4–1,5 с против 25–36 с прогрева (замеры TL-12), то есть около 5 %
/// времени; 10 % взяты с запасом, чтобы на медленном диске полоса не
/// упиралась в потолок этапа задолго до его конца.
const UNPACK_PERCENT_SHARE: u8 = 10;

/// Ожидаемая цена прогрева на один файл дерева, в миллисекундах.
///
/// Не цена загрузки одного Mach-O (та ~420 мс), а именно средняя по всему
/// дереву: грузится примерно шестьдесят процентов файлов. Получено
/// делением измеренного времени прогрева на число файлов: 24,58 / 25,33 /
/// 36,37 с на 134 файла — 183 / 189 / 271 мс. Взято 250 мс, ближе к
/// худшему: оценка «осталось» должна убывать до нуля, а не застревать на
/// нуле, пока пользователь ждёт.
const WARMUP_MS_PER_FILE: u64 = 250;

/// Как часто отправлять событие с оценкой во время прогрева. Полсекунды —
/// достаточно, чтобы обратный отсчёт выглядел живым, и достаточно редко,
/// чтобы не занимать WebView отрисовкой вместо ожидания.
const WARMUP_TICK: Duration = Duration::from_millis(500);

/// Сколько раз переустанавливать дерево, которое сходится с манифестом, но
/// не запускается.
///
/// Два — это «один настоящий шанс и один на всякий случай»: первая
/// переустановка чинит порчу, которую не поймала дешёвая сверка с
/// манифестом, вторая покрывает случай, когда первая сама попала на
/// временную помеху. Третья и дальше — уже гарантированные тридцать пять
/// секунд впустую при каждом старте.
const MAX_REPAIR_ATTEMPTS: u32 = 2;

/// Через сколько счётчик безуспешных переустановок остывает.
///
/// Сутки выбраны как срок, за который обстоятельство снаружи дерева могло
/// измениться (обновление ОС, возвращённые права, отключённый антивирус), а
/// цена ошибки — один лишний прогрев в сутки, а не при каждом запуске.
const REPAIR_COOLDOWN: Duration = Duration::from_secs(24 * 60 * 60);

/// Таймауты подготовки одним значением.
///
/// Существует ради тестов: ветку «дерево цело, но ОС забыла кэш» иначе
/// пришлось бы воспроизводить пятисекундным ожиданием в каждом прогоне
/// `cargo test`. Боевой код всегда берёт [`Timeouts::DEFAULT`], то есть
/// константы выше.
#[derive(Debug, Clone, Copy)]
struct Timeouts {
    probe: Duration,
    warmup: Duration,
    background: Duration,
}

impl Timeouts {
    const DEFAULT: Self = Self {
        probe: PROBE_TIMEOUT,
        warmup: WARMUP_TIMEOUT,
        background: BACKGROUND_WARMUP_TIMEOUT,
    };
}

/// Итог подготовки вместе с тем, что нужно сеансу (TL-23, TL-21).
#[derive(Debug)]
pub struct PrepareOutcome {
    /// Итог для фронтенда — контракт команды `prepare_ytdlp`.
    pub prepared: YtDlpPrepared,
    /// Запуск, которым проба застала дерево тёплым; служебный экран
    /// строит из него строку yt-dlp вместо своего запуска. `None` — пробы
    /// не было или дерево пришлось греть.
    pub warm_launch: Option<WarmLaunch>,
    /// Прогрев, который подготовка не стала ждать и который вызывающий
    /// обязан продолжить в фоне.
    pub background: Option<BackgroundWarmup>,
}

impl PrepareOutcome {
    /// Проба застала дерево тёплым: работы не было, запуск есть.
    fn warm(installed: &Installed, launch: WarmLaunch, started: Instant) -> Self {
        Self {
            prepared: YtDlpPrepared {
                version: parse_version(&launch.output.stdout, &installed.version),
                path: installed.executable.display().to_string(),
                prepared: false,
                duration_ms: elapsed_ms(started),
            },
            warm_launch: Some(launch),
            background: None,
        }
    }

    /// Дерево распаковано и/или прогрето в переднем плане.
    fn worked(installed: &Installed, version: String, started: Instant) -> Self {
        Self {
            prepared: YtDlpPrepared {
                version,
                path: installed.executable.display().to_string(),
                prepared: true,
                duration_ms: elapsed_ms(started),
            },
            warm_launch: None,
            background: None,
        }
    }

    /// Дерево цело по манифесту и считается готовым, прогрев уходит в фон
    /// (TL-21).
    ///
    /// Версия — из манифеста: запуска, который её сообщил бы, ещё не было.
    /// `worked` — показывал ли этот вызов ход работы событиями: тогда
    /// экран подготовки уже поднят и обязан получить терминальное `ready`.
    fn in_background(
        installed: &Installed,
        layout: &Layout,
        build_id: &BuildId,
        worked: bool,
        started: Instant,
        timeouts: Timeouts,
    ) -> Self {
        Self {
            prepared: YtDlpPrepared {
                version: installed.version.clone(),
                path: installed.executable.display().to_string(),
                prepared: worked,
                duration_ms: elapsed_ms(started),
            },
            warm_launch: None,
            background: Some(BackgroundWarmup {
                build_id: build_id.clone(),
                executable: installed.executable.clone(),
                mark_path: layout.slow_warmup_path(build_id),
                repair_path: layout.repair_path(build_id),
                timeout: timeouts.background,
            }),
        }
    }
}

/// Запуск `--version`, которым проба застала дерево тёплым (TL-23).
///
/// Поля — ровно то, из чего служебный экран собирает строку после своего
/// запуска: путь, вывод, штамп начала и длительность самого запуска.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarmLaunch {
    pub executable: PathBuf,
    pub output: RunOutput,
    pub checked_at: String,
    pub duration_ms: u64,
}

/// Прогрев, продолжающийся в фоне (TL-21).
#[derive(Debug)]
pub struct BackgroundWarmup {
    build_id: BuildId,
    executable: PathBuf,
    mark_path: PathBuf,
    repair_path: PathBuf,
    timeout: Duration,
}

/// Чем кончился фоновый прогрев.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackgroundOutcome {
    /// Уложился: отметка снята.
    Warmed,
    /// Снова не уложился: отметка обновлена, следующий старт тоже не ждёт.
    TimedOut,
    /// Не запустился: отметка снята, следующий старт проверит дерево
    /// обычным путём.
    Failed(String),
}

impl BackgroundWarmup {
    /// Установка, которую греет прогрев, — её держат занятой на время
    /// прогрева (Ф-7).
    pub fn build_id(&self) -> &BuildId {
        &self.build_id
    }

    /// Выполняет прогрев. Событий в `ytdlp://prepare` не шлёт: на этом
    /// канале поднимается блокирующий экран подготовки.
    pub async fn run(self, registry: &ChildRegistry) -> BackgroundOutcome {
        match sidecar::run(&self.executable, WARMUP_ARGS, self.timeout, registry).await {
            Ok(_) => {
                SlowWarmupMark::clear(&self.mark_path);
                // Удачный запуск обнуляет и счётчик починок — то же правило,
                // что у пробы переднего плана.
                RepairLog::clear(&self.repair_path);
                eprintln!(
                    "yt-dlp: фоновый прогрев {} завершился — отметка медленного прогрева снята",
                    self.build_id
                );
                BackgroundOutcome::Warmed
            }
            Err(SidecarError::Timeout { ms, .. }) => {
                record_slow_warmup(&self.mark_path, &self.build_id, ms);
                BackgroundOutcome::TimedOut
            }
            Err(error) => {
                SlowWarmupMark::clear(&self.mark_path);
                eprintln!(
                    "yt-dlp: фоновый прогрев {} не удался ({error}) — это не медленная машина, \
                     отметка снята: следующий старт проверит дерево запуском и при нужде \
                     переустановит",
                    self.build_id
                );
                BackgroundOutcome::Failed(error.to_string())
            }
        }
    }
}

/// Куда уходят события хода подготовки.
///
/// Абстракция ровно ради тестов: домейн-логику подготовки нельзя
/// проверить, поднимая настоящее Tauri-приложение, а проверить её надо —
/// именно она распаковывает, прогревает и решает, что делать с
/// повреждённым деревом.
///
/// `Send + Sync` — требование Tauri: future асинхронной команды обязан
/// быть `Send`, а приёмник живёт поперёк `await`.
pub trait ProgressSink: Send + Sync {
    fn emit(&self, event: YtDlpPrepareEvent);
}

/// Боевой приёмник: Tauri-событие [`PREPARE_EVENT`] всем окнам.
pub struct AppSink<'a>(pub &'a AppHandle);

impl ProgressSink for AppSink<'_> {
    fn emit(&self, event: YtDlpPrepareEvent) {
        // Отказ отправки события не повод прерывать подготовку: работа
        // важнее индикатора, а результат вернётся вызовом команды в любом
        // случае.
        if let Err(err) = self.0.emit(PREPARE_EVENT, event) {
            eprintln!("yt-dlp: не удалось отправить событие подготовки: {err}");
        }
    }
}

/// Готовит yt-dlp к работе и возвращает итог.
///
/// Идемпотентна: на уже подготовленном дереве только проверяет его
/// запуском и возвращает `prepared = false`, не отправив ни одного
/// события. Сеанс приложения зовёт её через `super::session::Session`,
/// которая помнит итог и не пробует дерево второй раз.
pub async fn prepare(
    archive_path: &Path,
    data_dir: &Path,
    registry: &ChildRegistry,
    sink: &dyn ProgressSink,
) -> Result<PrepareOutcome, PrepareError> {
    prepare_with(archive_path, data_dir, registry, sink, Timeouts::DEFAULT).await
}

async fn prepare_with(
    archive_path: &Path,
    data_dir: &Path,
    registry: &ChildRegistry,
    sink: &dyn ProgressSink,
    timeouts: Timeouts,
) -> Result<PrepareOutcome, PrepareError> {
    let started = Instant::now();
    let result = prepare_inner(archive_path, data_dir, registry, sink, started, timeouts).await;

    match &result {
        Ok(outcome) if outcome.prepared.prepared => sink.emit(YtDlpPrepareEvent {
            stage: YtDlpPrepareStage::Ready,
            percent: 100,
            eta_secs: Some(0),
            version: Some(outcome.prepared.version.clone()),
            error: None,
        }),
        Ok(_) => {}
        Err(error) => sink.emit(YtDlpPrepareEvent {
            stage: YtDlpPrepareStage::Failed,
            percent: 100,
            eta_secs: None,
            version: None,
            error: Some(error.to_contract()),
        }),
    }

    result
}

async fn prepare_inner(
    archive_path: &Path,
    data_dir: &Path,
    registry: &ChildRegistry,
    sink: &dyn ProgressSink,
    started: Instant,
    timeouts: Timeouts,
) -> Result<PrepareOutcome, PrepareError> {
    let layout = Layout::new(data_dir);
    layout.create_root()?;
    // Пин сразу в форме записи Ф-5: из неё же берётся его build id, то
    // есть проверка идентификатора остаётся одна на оба применения.
    let pinned = InstallEntry::for_identity(layout::ArchiveIdentity::bundled())?;
    let build_id = pinned.build_id().clone();

    // Мусор от подготовок, прерванных на середине: полураспакованное
    // дерево под именем `.staging-*`. Оно никогда не считается установкой
    // (установка появляется только переименованием), но занимает до
    // 124 МиБ и должно уйти.
    for stale in unpack::stale_staging_dirs(layout.root()) {
        eprintln!(
            "yt-dlp: убираю остаток прерванной подготовки {}",
            stale.display()
        );
        unpack::remove_dir_if_exists(&stale)?;
    }

    // То же самое для недокачанного архива обновления (С-7): прерванное
    // скачивание оставляет `.download-*` до шестидесяти мегабайт, и
    // убирать его надо там же, где убирается `.staging-*`, — на старте,
    // единственном моменте, когда заведомо никто не пишет в каталог
    // данных. Отказ уборки здесь не роняет подготовку: остаток мешает
    // месту на диске, а не работе.
    for stale in layout.stale_downloads() {
        eprintln!(
            "yt-dlp: убираю недокачанный архив обновления {}",
            stale.display()
        );
        if let Err(err) = std::fs::remove_file(&stale) {
            eprintln!("yt-dlp: не удалось убрать {}: {err}", stale.display());
        }
    }

    // Кого готовить, решает запись Ф-5, а не константа пина: контур
    // обновления (E6) мог переключить активную установку на скачанную из
    // сети, и готовить вместо неё пин значило бы греть дерево, которого
    // никто потом не запустит, — а при исчерпанной уборке (Ф-8 держит на
    // диске две установки, и пина среди них может уже не быть) ещё и
    // распаковывать его заново на каждом старте.
    let mut state = InstallState::load(&layout);
    if let Some(outcome) = prepare_active(
        &layout, &state, &build_id, registry, sink, started, timeouts,
    )
    .await
    {
        return Ok(outcome);
    }

    let outcome = prepare_pinned(
        archive_path,
        &layout,
        &build_id,
        registry,
        sink,
        started,
        timeouts,
    )
    .await?;

    // Пин работает — значит он и есть активная установка. Сюда приходят
    // два случая: первый запуск вообще (записи ещё нет) и отказ активной
    // установки, после которого вложенный архив сработал резервом (Ф-5).
    // Отказ записи не отменяет подготовку: приложение работоспособно и
    // без неё — резолв найдёт пин третьей попыткой, — а следующий запуск
    // попробует записать снова.
    match state.activate(&layout, pinned) {
        Ok(true) => eprintln!("yt-dlp: активной установкой записан {build_id}"),
        Ok(false) => {}
        Err(err) => eprintln!("yt-dlp: не удалось записать активную установку: {err}"),
    }

    Ok(outcome)
}

/// Готовит установку, которую называет активной запись Ф-5, — но только
/// если это не пин: пином занимается [`prepare_pinned`], у которого есть
/// чем переустановить дерево.
///
/// `None` означает «этой дорогой не вышло, работай вложенным архивом».
/// Причин ровно три, и все три — про негодность активной установки:
/// записи нет, дерево не прошло [`layout::validate`], дерево не
/// запускается. Переустановить активную установку здесь нечем: в бандле
/// лежит архив пина, а не её, и распаковка его под чужим идентификатором
/// собрала бы установку, врущую о своём содержимом. Поэтому исход один —
/// вложенный архив как резерв (Ф-5), и это **не** откат (Р-3): запись не
/// меняется, а сменит её [`prepare_pinned`] только после того, как пин
/// действительно заработает.
async fn prepare_active(
    layout: &Layout,
    state: &InstallState,
    pinned: &layout::BuildId,
    registry: &ChildRegistry,
    sink: &dyn ProgressSink,
    started: Instant,
    timeouts: Timeouts,
) -> Option<PrepareOutcome> {
    let active = state.active()?;
    if active.build_id() == pinned {
        return None;
    }
    let build_id = active.build_id();

    let installed = match layout::validate(layout, build_id) {
        Ok(installed) => installed,
        Err(invalid) => {
            eprintln!(
                "yt-dlp: активная установка {build_id} непригодна ({invalid}), \
                 беру вложенный в бандл архив как резерв"
            );
            return None;
        }
    };

    // Медленная активная установка — не негодная: на пин она не меняется
    // (TL-21).
    if slow_warmup_recorded(layout, build_id) {
        return Some(PrepareOutcome::in_background(
            &installed, layout, build_id, false, started, timeouts,
        ));
    }

    match probe(&installed, registry, timeouts.probe).await {
        Probe::Warm(launch) => {
            SlowWarmupMark::clear(&layout.slow_warmup_path(build_id));
            Some(PrepareOutcome::warm(&installed, launch, started))
        }
        Probe::Cold => {
            // Дерево цело, но ОС забыла результат проверки подписей.
            let file_count = count_tree_files(&installed);
            let warmed = warm_up(&installed, registry, sink, file_count, 0, timeouts.warmup).await;
            match after_warm_up(warmed, &installed, layout, build_id, started, timeouts) {
                Ok(outcome) => Some(outcome),
                Err(err) => {
                    eprintln!(
                        "yt-dlp: активная установка {build_id} не прогрелась ({err}), \
                         беру вложенный в бандл архив как резерв"
                    );
                    None
                }
            }
        }
        Probe::Broken(reason) => {
            eprintln!(
                "yt-dlp: активная установка {build_id} не запускается ({reason}), \
                 беру вложенный в бандл архив как резерв"
            );
            None
        }
    }
}

/// Подготовка вложенной в бандл установки — тот же путь, что был до E6:
/// проверка дерева, проба, прогрев, а при негодном дереве —
/// переустановка со счётчиком починок.
#[allow(clippy::too_many_arguments)]
async fn prepare_pinned(
    archive_path: &Path,
    layout: &Layout,
    build_id: &layout::BuildId,
    registry: &ChildRegistry,
    sink: &dyn ProgressSink,
    started: Instant,
    timeouts: Timeouts,
) -> Result<PrepareOutcome, PrepareError> {
    let repair_path = layout.repair_path(build_id);

    match layout::validate(layout, build_id) {
        Ok(installed) => {
            // Прогрев этого дерева уже упирался в таймаут: не пробуем и не
            // ждём, дерево цело по манифесту (TL-21).
            if slow_warmup_recorded(layout, build_id) {
                return Ok(PrepareOutcome::in_background(
                    &installed, layout, build_id, false, started, timeouts,
                ));
            }

            match probe(&installed, registry, timeouts.probe).await {
                Probe::Warm(launch) => {
                    RepairLog::clear(&repair_path);
                    SlowWarmupMark::clear(&layout.slow_warmup_path(build_id));
                    Ok(PrepareOutcome::warm(&installed, launch, started))
                }
                Probe::Cold => {
                    // Дерево на месте и цело, но ОС забыла результат
                    // проверки подписей — распаковывать заново незачем,
                    // достаточно прогреть.
                    let warmed = warm_up(
                        &installed,
                        registry,
                        sink,
                        count_tree_files(&installed),
                        0,
                        timeouts.warmup,
                    )
                    .await;
                    let outcome =
                        after_warm_up(warmed, &installed, layout, build_id, started, timeouts)?;
                    if outcome.background.is_none() {
                        RepairLog::clear(&repair_path);
                    }
                    Ok(outcome)
                }
                Probe::Broken(reason) => {
                    // Дерево прошло сверку с манифестом, но не
                    // запускается. Сверка дешёвая и не ловит порчу «файл
                    // того же размера», поэтому осмысленное действие —
                    // переустановить и попробовать ещё раз. Повторов
                    // внутри одного вызова не бывает, а между запусками их
                    // считает `repair`.
                    repair(
                        &reason,
                        archive_path,
                        layout,
                        build_id,
                        registry,
                        sink,
                        started,
                        timeouts,
                    )
                    .await
                }
            }
        }
        Err(invalid) => {
            eprintln!("yt-dlp: установка непригодна ({invalid}), распаковываю заново");
            let outcome = install_and_warm(
                archive_path,
                layout,
                build_id,
                registry,
                sink,
                started,
                timeouts,
            )
            .await?;
            if outcome.background.is_none() {
                RepairLog::clear(&repair_path);
            }
            Ok(outcome)
        }
    }
}

/// Переустановка дерева, которое сходится с манифестом, но не запускается —
/// с памятью о том, что это уже пробовали.
///
/// Порядок именно такой: сначала проверить исчерпание, потом **записать
/// попытку**, и только потом работать. Запись до работы, а не после,
/// потому что самый неприятный исход — не отказ, а зависание: если
/// приложение убьют посреди тридцатипятисекундного прогрева, попытка всё
/// равно должна оказаться засчитанной, иначе цикл «запустил — не дождался —
/// убил» повторяется вечно.
///
/// Откат счётчика при не-прогревной ошибке — не педантизм: «на диске нет
/// места» и «дерево не запускается» приводят к разным решениям
/// пользователя, и первое не должно приближать нас к отказу чинить второе.
// Восемь аргументов — это ровно то, что нужно переустановке, плюс причина,
// по которой она затеяна: заворачивать их в структуру-контекст значило бы
// переписать соседние функции ради формы, а не ради смысла.
#[allow(clippy::too_many_arguments)]
async fn repair(
    reason: &str,
    archive_path: &Path,
    layout: &Layout,
    build_id: &layout::BuildId,
    registry: &ChildRegistry,
    sink: &dyn ProgressSink,
    started: Instant,
    timeouts: Timeouts,
) -> Result<PrepareOutcome, PrepareError> {
    let repair_path = layout.repair_path(build_id);
    let history = RepairLog::read(&repair_path);
    let now = crate::clock::now_unix_secs();

    if repair_exhausted(&history, now) {
        eprintln!(
            "yt-dlp: установка не запускается ({reason}), но переустановка уже \
             выполнялась {} раз(а) и не помогла — отказываюсь повторять",
            history.attempts
        );
        return Err(PrepareError::WarmupFailed {
            reason: format!(
                "yt-dlp не запускается ({reason}); переустановка дерева выполнялась {} раз(а) \
                 и не помогла, последняя — {}. Повторять её бессмысленно: переустановите \
                 приложение или удалите каталог {}",
                history.attempts,
                history.last_attempt_at,
                layout.root().display()
            ),
        });
    }

    eprintln!("yt-dlp: установка не запускается ({reason}), переустанавливаю");
    let attempted = history.with_attempt(reason, now);
    if let Err(err) = attempted.write_atomic(&repair_path) {
        // Счётчик — страховка, а не условие работы: не записался — чиним
        // всё равно, просто в следующий раз посчитаем заново.
        eprintln!("yt-dlp: не удалось записать историю починки: {err}");
    }

    let outcome = install_and_warm(
        archive_path,
        layout,
        build_id,
        registry,
        sink,
        started,
        timeouts,
    )
    .await;

    match &outcome {
        Ok(done) if done.background.is_none() => RepairLog::clear(&repair_path),
        // Переустановка прошла, а прогрев ушёл в фон (TL-21): запускается ли
        // дерево, ещё не известно, и попытка остаётся засчитанной — снимет
        // её удачный фоновый прогрев.
        Ok(_) | Err(PrepareError::WarmupFailed { .. }) => {}
        Err(_) => {
            if history.attempts == 0 {
                RepairLog::clear(&repair_path);
            } else if let Err(err) = history.write_atomic(&repair_path) {
                eprintln!("yt-dlp: не удалось откатить историю починки: {err}");
            }
        }
    }

    outcome
}

/// Исчерпаны ли попытки починки на момент `now_unix`.
fn repair_exhausted(history: &RepairLog, now_unix: u64) -> bool {
    if history.attempts < MAX_REPAIR_ATTEMPTS {
        return false;
    }

    // Часы могли отойти назад (правка времени, переезд через часовой
    // пояс в BIOS): отрицательного «прошло времени» не бывает, и такой
    // случай считается «только что», то есть отказ сохраняется. Ошибка в
    // эту сторону стоит пользователю одного явного сообщения, в обратную —
    // тридцати пяти секунд на каждом старте.
    now_unix.saturating_sub(history.last_attempt_unix) < REPAIR_COOLDOWN.as_secs()
}

/// Распаковывает дерево заново и прогревает его.
async fn install_and_warm(
    archive_path: &Path,
    layout: &Layout,
    build_id: &BuildId,
    registry: &ChildRegistry,
    sink: &dyn ProgressSink,
    started: Instant,
    timeouts: Timeouts,
) -> Result<PrepareOutcome, PrepareError> {
    let unpack_started = Instant::now();
    let installed = install(
        archive_path,
        layout,
        layout::ArchiveIdentity::bundled(),
        &mut |done, total| sink.emit(unpacking_event(done, total, unpack_started)),
    )?;
    let file_count = count_tree_files(&installed);
    let warmed = warm_up(
        &installed,
        registry,
        sink,
        file_count,
        UNPACK_PERCENT_SHARE,
        timeouts.warmup,
    )
    .await;

    after_warm_up(warmed, &installed, layout, build_id, started, timeouts)
}

/// Распаковка в `.staging-*`, атомарный перенос, запись манифеста.
///
/// Единственная реализация установки на весь проект: обновление yt-dlp
/// (TL-56) вызывает **эту** функцию, а не свою копию (Ф-4). Отсюда два
/// её свойства, которых у неё не было до E6.
///
/// Первое — `identity` вместо констант пина: build id, каталог и манифест
/// адресуются тем, чем архив себя называет, а вложен он в бандл или
/// скачан из сети, установке безразлично.
///
/// Второе — `on_progress` вместо [`ProgressSink`]. Здесь сменился не
/// стиль, а адресат: подготовка первого запуска эмитит ход в
/// `ytdlp://prepare`, а фоновое обновление обязано **не** эмитить туда
/// ничего — на этом канале поднимается полноэкранный блокирующий
/// `YtDlpPrepareScreen`, и его появление посреди фоновой работы было бы
/// прямой регрессией Р-1 («никаких прерывающих уведомлений»). Пока сюда
/// передавался `&dyn ProgressSink`, единственным способом это соблюсти
/// была дисциплина вызывающего; с замыканием канал выбирает тот, кто
/// установку затеял, и выбрать чужой не может.
///
/// # Порядок шагов: распаковать, и только потом сносить прежнее
///
/// Прежний каталог установки и его манифест удаляются **после** того,
/// как распаковка в `.staging-*` дошла до конца, а не до неё. Разница
/// не косметическая: до TL-58 снос стоял первым, и отказ распаковки —
/// битый архив, потолок объёма, «не ровно один исполняемый в корне» —
/// уносил рабочую установку с тем же идентификатором. Ревью TL-56
/// показало это запуском: после отказа на битом архиве не оставалось ни
/// каталога, ни исполняемого файла, ни манифеста.
///
/// Отсюда гарантия С-4 («активная установка не тронута ничем»)
/// становится свойством конструкции, а не договорённостью о том, с
/// каким идентификатором сюда можно звать. Незащищённым остаётся одно
/// окно — между сносом и `rename`, то есть между двумя операциями
/// файловой системы; прежде оно длилось всю распаковку.
///
/// Цена названа: в момент промоушена на диске лежат оба дерева сразу,
/// пик расхода — на одну установку (около 124 МиБ) больше прежнего.
/// Н-4 этот пик уже закладывает («активная + известно-хорошая +
/// подготавливаемая + архив»).
pub(super) fn install(
    archive_path: &Path,
    layout: &Layout,
    identity: layout::ArchiveIdentity<'_>,
    on_progress: &mut dyn FnMut(u64, u64),
) -> Result<Installed, PrepareError> {
    let build_id = identity.build_id()?;
    let install_dir = layout.install_dir(&build_id);
    let manifest_path = layout.manifest_path(&build_id);

    // Каталог распаковки создаётся здесь и под непредсказуемым именем
    // (см. doc `super::layout`), поэтому «убрать прежний staging» не
    // требуется: своего у нас ещё нет, а чужой — не наш.
    let staging = layout.create_staging_dir(&build_id)?;

    let unpacked = unpack::unpack(archive_path, &staging, on_progress).inspect_err(|_| {
        // Полураспакованное дерево не должно пережить неудачу — иначе
        // следующий запуск найдёт мусор на 124 МиБ и будет чистить его
        // «за прошлый раз».
        let _ = unpack::remove_dir_if_exists(&staging);
    })?;

    // Прежняя установка этого же build id могла остаться непригодной
    // (`validate` уже сказала, что она не годится) — переименование в
    // занятый путь не пройдёт, поэтому её надо убрать. Убирается она
    // **после** распаковки, и порядок здесь несущий — см. «Порядок
    // шагов» в doc функции. Отказ сноса оставляет `.staging-*` на диске:
    // его уберёт следующий запуск (`prepare_inner` чистит остатки), а
    // рабочее дерево при этом цело.
    unpack::remove_dir_if_exists(&install_dir).inspect_err(|_| {
        let _ = unpack::remove_dir_if_exists(&staging);
    })?;
    let _ = std::fs::remove_file(&manifest_path);

    unpack::promote(&staging, &install_dir)?;

    let manifest = layout::manifest_for(&install_dir, &unpacked.executable, identity)?;
    manifest.write_atomic(&manifest_path)?;

    Ok(Installed {
        dir: install_dir.clone(),
        executable: install_dir.join(&unpacked.executable),
        version: manifest.yt_dlp_version,
    })
}

/// Исход проверки «дерево уже тёплое?».
enum Probe {
    /// Отозвалось быстро; запуск уходит служебному экрану (TL-23).
    Warm(WarmLaunch),
    /// Не уложилось в [`PROBE_TIMEOUT`] — нужен прогрев.
    Cold,
    /// Не запускается или завершается с ошибкой.
    Broken(String),
}

async fn probe(installed: &Installed, registry: &ChildRegistry, timeout: Duration) -> Probe {
    // Штамп — до запуска, длительность — самого запуска: так же их берёт
    // проверка служебного экрана, и в «Подробнее» они обязаны значить то
    // же самое, чей бы запуск ни был.
    let checked_at = crate::clock::now_iso8601();
    let launched = Instant::now();
    match sidecar::run(&installed.executable, WARMUP_ARGS, timeout, registry).await {
        Ok(output) => Probe::Warm(WarmLaunch {
            executable: installed.executable.clone(),
            output,
            checked_at,
            duration_ms: elapsed_ms(launched),
        }),
        Err(SidecarError::Timeout { .. }) => Probe::Cold,
        Err(error) => Probe::Broken(error.to_string()),
    }
}

/// Один прогон, оплачивающий проверку подписей всего дерева.
///
/// `base_percent` — сколько процентов общего прогресса уже позади
/// (распаковка), `file_count` — размер дерева, из которого выводится
/// оценка оставшегося времени.
async fn warm_up(
    installed: &Installed,
    registry: &ChildRegistry,
    sink: &dyn ProgressSink,
    file_count: u64,
    base_percent: u8,
    timeout: Duration,
) -> Result<String, SidecarError> {
    let expected = Duration::from_millis(file_count.saturating_mul(WARMUP_MS_PER_FILE));
    let started = Instant::now();
    sink.emit(warming_event(started, expected, base_percent));

    let run = sidecar::run(&installed.executable, WARMUP_ARGS, timeout, registry);
    tokio::pin!(run);

    let mut ticker = tokio::time::interval(WARMUP_TICK);
    ticker.tick().await; // первый тик у `interval` мгновенный

    let output = loop {
        tokio::select! {
            result = &mut run => break result,
            _ = ticker.tick() => sink.emit(warming_event(started, expected, base_percent)),
        }
    };

    output.map(|output| parse_version(&output.stdout, &installed.version))
}

/// Исход прогрева переднего плана: удача, медленная машина или отказ.
///
/// Таймаут отказом не считается (TL-21): отметка записывается рядом с
/// установкой, а прогрев уходит в фон. Отказом остаётся только то, что
/// говорит о неработоспособности дерева, — ненулевой код, отказ запуска.
fn after_warm_up(
    warmed: Result<String, SidecarError>,
    installed: &Installed,
    layout: &Layout,
    build_id: &BuildId,
    started: Instant,
    timeouts: Timeouts,
) -> Result<PrepareOutcome, PrepareError> {
    match warmed {
        Ok(version) => {
            SlowWarmupMark::clear(&layout.slow_warmup_path(build_id));
            Ok(PrepareOutcome::worked(installed, version, started))
        }
        Err(SidecarError::Timeout { ms, .. }) => {
            record_slow_warmup(&layout.slow_warmup_path(build_id), build_id, ms);
            Ok(PrepareOutcome::in_background(
                installed, layout, build_id, true, started, timeouts,
            ))
        }
        Err(error) => Err(PrepareError::WarmupFailed {
            reason: error.to_string(),
        }),
    }
}

/// Упирался ли прогрев этой установки в таймаут (TL-21).
///
/// Испорченная отметка — то же, что отсутствующая (прогрев идёт как
/// обычно: медленнее, но честно), и причина уходит в лог.
fn slow_warmup_recorded(layout: &Layout, build_id: &BuildId) -> bool {
    let path = layout.slow_warmup_path(build_id);
    match SlowWarmupMark::read(&path) {
        Ok(Some(mark)) => {
            eprintln!(
                "yt-dlp: прогрев {build_id} не укладывался в таймаут (подряд: {}, последний — \
                 {}) — дерево цело по манифесту, экран его не ждёт, прогрев идёт в фоне",
                mark.timeouts, mark.last_timeout_at
            );
            true
        }
        Ok(None) => false,
        Err(reason) => {
            eprintln!(
                "yt-dlp: отметка медленного прогрева {} {reason} — считаю, что её нет, и \
                 проверяю дерево обычным путём",
                path.display()
            );
            false
        }
    }
}

/// Записывает (или обновляет) отметку медленного прогрева.
///
/// Отказ записи не роняет подготовку: отметка — страховка от повторной
/// оплаты таймаута, а не условие работы; не записалась — следующий старт
/// просто подождёт ещё раз.
fn record_slow_warmup(path: &Path, build_id: &BuildId, timeout_ms: u64) {
    let previous = SlowWarmupMark::read(path).ok().flatten();
    let mark =
        SlowWarmupMark::recorded(previous.as_ref(), timeout_ms, crate::clock::now_unix_secs());
    eprintln!(
        "yt-dlp: прогрев {build_id} не уложился в {timeout_ms} мс (подряд: {}) — это медленная \
         машина, а не отказ: экран этот прогрев больше не ждёт",
        mark.timeouts
    );
    if let Err(err) = mark.write_atomic(path) {
        eprintln!("yt-dlp: не удалось записать отметку медленного прогрева: {err}");
    }
}

/// Число файлов дерева — основа оценки времени прогрева.
///
/// Берётся не из манифеста, а обходом: манифест мог быть записан другой
/// сборкой приложения, а оценка должна соответствовать тому, что лежит на
/// диске прямо сейчас. Ошибка обхода не важна — тогда оценка строится по
/// пустому дереву и просто оказывается нулевой.
fn count_tree_files(installed: &Installed) -> u64 {
    fn count(dir: &Path) -> u64 {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return 0;
        };
        entries
            .filter_map(Result::ok)
            .map(|entry| match entry.metadata() {
                Ok(metadata) if metadata.is_dir() => count(&entry.path()),
                Ok(_) => 1,
                Err(_) => 0,
            })
            .sum()
    }

    count(&installed.dir)
}

fn unpacking_event(done: u64, total: u64, started: Instant) -> YtDlpPrepareEvent {
    let fraction = if total == 0 {
        1.0
    } else {
        done as f64 / total as f64
    };
    let percent = (fraction * f64::from(UNPACK_PERCENT_SHARE)).round() as u8;

    // Оценка по фактической скорости записи: она известна с первого
    // мегабайта, в отличие от прогрева, где до конца прогона ничего не
    // видно.
    let eta_secs = if done == 0 {
        None
    } else {
        let elapsed = started.elapsed().as_secs_f64();
        let remaining = (total.saturating_sub(done)) as f64 * elapsed / done as f64;
        Some(remaining.ceil() as u64)
    };

    YtDlpPrepareEvent {
        stage: YtDlpPrepareStage::Unpacking,
        percent: percent.min(UNPACK_PERCENT_SHARE),
        eta_secs,
        version: None,
        error: None,
    }
}

fn warming_event(started: Instant, expected: Duration, base_percent: u8) -> YtDlpPrepareEvent {
    let elapsed = started.elapsed();
    let fraction = if expected.is_zero() {
        1.0
    } else {
        (elapsed.as_secs_f64() / expected.as_secs_f64()).min(1.0)
    };

    let span = f64::from(100 - base_percent);
    // Потолок 99: сотню показывает только терминальное событие `ready`,
    // иначе полоса упирается в конец и стоит там, пока процесс ещё идёт.
    let percent = (f64::from(base_percent) + fraction * span)
        .round()
        .min(99.0) as u8;

    YtDlpPrepareEvent {
        stage: YtDlpPrepareStage::WarmingUp,
        percent,
        // Округление вверх, а не отбрасывание дробной части: «осталось 29 с»
        // сразу после старта тридцатисекундной оценки выглядит как потеря
        // секунды, которой не было.
        eta_secs: Some(expected.saturating_sub(elapsed).as_secs_f64().ceil() as u64),
        version: None,
        error: None,
    }
}

/// Версия по выводу запуска; если разобрать не удалось — та, что записана
/// в манифесте (из пина). Показать пину лучше, чем показать пустоту.
fn parse_version(stdout: &str, fallback: &str) -> String {
    sidecar::parse_ytdlp_version(stdout)
        .map(|parsed| parsed.display)
        .unwrap_or_else(|| {
            let trimmed = stdout.trim();
            if trimmed.is_empty() {
                fallback.to_string()
            } else {
                trimmed.to_string()
            }
        })
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Путь к исполняемому файлу готовой установки yt-dlp — единственный
/// резолв на всех потребителей (служебный экран, разбор ссылки,
/// скачивание).
///
/// Идёт через явную запись Ф-5, а не через константу пина: после первого
/// же обновления yt-dlp пин перестаёт быть тем, чем работает приложение,
/// а после второго его каталога может уже не быть на диске (Ф-8 держит
/// две установки). Что именно ответило — активная запись, известно-хорошая
/// или вложенный в бандл пин, — решает [`state::resolve`].
///
/// `Err` означает «подготовка не выполнена или дерево непригодно» — это
/// не сбой, а нормальное состояние до первого вызова `prepare_ytdlp`.
///
/// # Почему вместе с путём отдаётся страж
///
/// Ф-7 требует безусловного: установка, из которой запущен работающий
/// процесс, не удаляется — даже если она уже не активна и не
/// известно-хорошая. Такое состояние не экзотика, а прямое следствие
/// Р-2: контур переключает активную запись на границе задач, а
/// работающая задача продолжает жить в прежнем дереве. Пока
/// [`InUseGuard`] жив, уборка ([`state::cleanup`]) это дерево не
/// трогает; когда он уронен, защита снимается сама.
pub fn installed_executable<'a>(
    data_dir: &Path,
    in_use: &'a InUse,
) -> Result<(PathBuf, InUseGuard<'a>), PrepareError> {
    let layout = Layout::new(data_dir);
    let state = InstallState::load(&layout);
    let resolved = state::resolve(&layout, &state)?;

    // Резерв работает молча для пользователя, но не для лога: по этой
    // строке владелец отличает «работаем на том, что записано» от
    // «активная установка испортилась, и мы работаем на запасной».
    // Молчание про `Bundled` без записи — не пропуск: до первого
    // переключения пин и есть активная установка (С-10), и сообщать там
    // не о чем.
    if resolved.slot != state::Slot::Active && state.active().is_some() {
        eprintln!(
            "yt-dlp: активная установка недоступна, работаю на резерве ({})",
            resolved.build_id
        );
    }

    // Отметка занятости ставится здесь, и это единственное место, где
    // путь к yt-dlp вообще берётся (Ф-5). Совпадение не случайное:
    // правило «кто получил путь, тот и держит установку» верно ровно
    // тогда, когда получить путь мимо этой функции нельзя. Разъедини их
    // — и появится вызывающий, который отметить забыл, а следом уборка,
    // снёсшая дерево под работающим процессом (Ф-7, Ф-8).
    //
    // Страж живёт столько, сколько его держит вызывающий: у команды
    // служебного экрана — на время одного запуска, у воркера задачи
    // скачивания — на всю задачу, включая повторы (Р-2: «задача доходит
    // до конца на той версии, на которой началась»).
    let guard = in_use.mark(&resolved.build_id);

    Ok((resolved.installed.executable, guard))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Идентификатор вложенной сборки для тестов. Отдельный хелпер, а не
    /// `unwrap` по месту: пин обязан проходить ту же проверку, что и
    /// кандидат из сети, и «обязан» здесь означает падение теста, а не
    /// молчаливое `unwrap_or`.
    fn pinned_build_id() -> layout::BuildId {
        layout::bundled_build_id().expect("пин обязан проходить проверку идентификатора")
    }

    /// Идентификатор чужой установки — той, что не совпадает с пином.
    /// Нужен там, где предмет проверки — остаток от **другой** сборки.
    fn other_build_id() -> layout::BuildId {
        layout::BuildId::new("2000.01.01", &"de".repeat(32))
            .expect("образец обязан проходить проверку")
    }

    /// Установка, которой в бандле нет и быть не может: так выглядит
    /// yt-dlp, скачанный контуром обновления (E6) уже после выпуска этой
    /// сборки приложения. Версия заведомо новее пина — не потому, что
    /// код где-то их сравнивает (он не сравнивает нигде), а чтобы
    /// читатель теста не гадал, кто из двух кому предшественник.
    fn updated_identity() -> layout::ArchiveIdentity<'static> {
        layout::ArchiveIdentity {
            version: "2030.01.01",
            sha256: UPDATED_SHA,
        }
    }

    const UPDATED_SHA: &str = "beef0123456789abcdef0123456789abcdef0123456789abcdef0123456789ab";

    fn updated_entry() -> InstallEntry {
        InstallEntry::for_identity(updated_identity()).expect("образец обязан проходить проверку")
    }
    use std::fs::{self, File};
    use std::io::Write;
    use std::sync::Mutex;

    use tempfile::{tempdir, TempDir};
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    #[derive(Default)]
    struct RecordingSink(Mutex<Vec<YtDlpPrepareEvent>>);

    impl RecordingSink {
        fn events(&self) -> Vec<YtDlpPrepareEvent> {
            self.0.lock().expect("mutex").clone()
        }

        fn stages(&self) -> Vec<YtDlpPrepareStage> {
            self.events().into_iter().map(|event| event.stage).collect()
        }
    }

    impl ProgressSink for RecordingSink {
        fn emit(&self, event: YtDlpPrepareEvent) {
            self.0.lock().expect("mutex").push(event);
        }
    }

    /// Архив с «yt-dlp», который печатает версию — форма та же, что у
    /// апстримного onedir-ассета: исполняемый файл в корне плюс `_internal`.
    fn write_fake_ytdlp_zip(path: &Path, body: &str) {
        let file = File::create(path).expect("create archive");
        let mut zip = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);

        zip.start_file(EXECUTABLE_NAME, options.unix_permissions(0o755))
            .expect("start_file");
        zip.write_all(body.as_bytes()).expect("write");

        zip.add_directory("_internal/", options.unix_permissions(0o755))
            .expect("add_directory");
        zip.start_file("_internal/lib.so", options.unix_permissions(0o755))
            .expect("start_file");
        zip.write_all(b"pretend-shared-library").expect("write");

        zip.finish().expect("finish");
    }

    const EXECUTABLE_NAME: &str = "yt-dlp_fake";
    const PRINTS_VERSION: &str = "#!/bin/sh\necho 2026.08.19\n";

    /// «yt-dlp», который не запускается, **того же размера**, что рабочий.
    ///
    /// Размер совпадает не для красоты: дешёвая сверка с манифестом
    /// (`layout::validate`) считает файлы и байты, и подмена другой длины
    /// была бы поймана ею, не дойдя до запуска. Именно эта — «дерево цело
    /// по манифесту, но не работает» — и есть ветка `Probe::Broken`.
    const EXITS_NONZERO: &str = "#!/bin/sh\nexit 3         \n";

    struct Fixture {
        _dir: TempDir,
        archive: PathBuf,
        data_dir: PathBuf,
        registry: ChildRegistry,
    }

    fn fixture(body: &str) -> Fixture {
        let dir = tempdir().expect("tempdir");
        let archive = dir.path().join("yt-dlp.zip");
        write_fake_ytdlp_zip(&archive, body);
        let data_dir = dir.path().join("app-data");

        Fixture {
            _dir: dir,
            archive,
            data_dir,
            registry: ChildRegistry::new(),
        }
    }

    impl Fixture {
        async fn prepare(&self, sink: &RecordingSink) -> Result<YtDlpPrepared, PrepareError> {
            self.prepare_with(sink, Timeouts::DEFAULT).await
        }

        async fn prepare_with(
            &self,
            sink: &RecordingSink,
            timeouts: Timeouts,
        ) -> Result<YtDlpPrepared, PrepareError> {
            self.outcome_with(sink, timeouts)
                .await
                .map(|outcome| outcome.prepared)
        }

        async fn outcome_with(
            &self,
            sink: &RecordingSink,
            timeouts: Timeouts,
        ) -> Result<PrepareOutcome, PrepareError> {
            prepare_with(
                &self.archive,
                &self.data_dir,
                &self.registry,
                sink,
                timeouts,
            )
            .await
        }

        fn mark_path(&self) -> PathBuf {
            self.layout().slow_warmup_path(&pinned_build_id())
        }

        /// Портит установленный исполняемый файл, сохраняя размер дерева.
        fn break_installed_executable(&self) {
            let path = self.install_dir().join(EXECUTABLE_NAME);
            // `write` усекает файл, но не трогает права: бит выполнения
            // остаётся, иначе `validate` отвергла бы дерево раньше пробы.
            fs::write(&path, EXITS_NONZERO).expect("подменить исполняемый файл");
        }

        /// Заменяет вложенный архив на такой же по форме, но с неработающим
        /// yt-dlp внутри: так выглядит починка, которой нечем чинить.
        fn replace_archive(&self, body: &str) {
            write_fake_ytdlp_zip(&self.archive, body);
        }

        fn repair_path(&self) -> PathBuf {
            self.layout().repair_path(&pinned_build_id())
        }

        fn manifest_modified(&self) -> std::time::SystemTime {
            fs::metadata(self.manifest_path())
                .expect("манифест обязан существовать")
                .modified()
                .expect("время изменения обязано быть доступно")
        }

        fn layout(&self) -> Layout {
            Layout::new(&self.data_dir)
        }

        fn install_dir(&self) -> PathBuf {
            self.layout().install_dir(&pinned_build_id())
        }

        fn manifest_path(&self) -> PathBuf {
            self.layout().manifest_path(&pinned_build_id())
        }

        /// Раскладывает готовую установку, которой нет в бандле, и делает
        /// её активной по записи — так выглядит машина, на которой контур
        /// обновления уже отработал.
        ///
        /// Дерево пишется руками, а не распаковкой: архива этой версии у
        /// приложения нет и взяться ему неоткуда — в этом весь смысл
        /// сценария.
        fn install_and_activate(&self, identity: layout::ArchiveIdentity<'_>, body: &str) {
            let layout = self.layout();
            layout.create_root().expect("создать корень");
            let entry = InstallEntry::for_identity(identity).expect("проверенный идентификатор");
            let dir = layout.install_dir(entry.build_id());

            fs::create_dir_all(dir.join("_internal")).expect("создать дерево");
            fs::write(dir.join(EXECUTABLE_NAME), body).expect("записать исполняемый файл");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(dir.join(EXECUTABLE_NAME), fs::Permissions::from_mode(0o755))
                    .expect("бит выполнения");
            }
            fs::write(dir.join("_internal/lib.so"), b"pretend-shared-library")
                .expect("записать библиотеку");

            layout::manifest_for(&dir, EXECUTABLE_NAME, identity)
                .expect("манифест обязан собираться")
                .write_atomic(&layout.manifest_path(entry.build_id()))
                .expect("манифест обязан записываться");

            InstallState::default()
                .activate(&layout, entry)
                .expect("запись обязана сохраняться");
        }

        fn state(&self) -> InstallState {
            InstallState::load(&self.layout())
        }
    }

    #[tokio::test]
    async fn first_run_unpacks_warms_up_and_reports_progress() {
        let fixture = fixture(PRINTS_VERSION);
        let sink = RecordingSink::default();

        let prepared = fixture
            .prepare(&sink)
            .await
            .expect("подготовка обязана пройти");

        assert!(prepared.prepared, "первый запуск обязан выполнить работу");
        assert_eq!(prepared.version, "2026.08.19");
        assert!(prepared.path.ends_with(EXECUTABLE_NAME));
        assert!(fixture.install_dir().join("_internal/lib.so").exists());
        assert!(fixture.manifest_path().exists());

        let stages = sink.stages();
        assert_eq!(stages.first(), Some(&YtDlpPrepareStage::Unpacking));
        assert_eq!(stages.last(), Some(&YtDlpPrepareStage::Ready));
        assert!(stages.contains(&YtDlpPrepareStage::WarmingUp));

        let percents: Vec<u8> = sink.events().iter().map(|event| event.percent).collect();
        assert!(
            percents.windows(2).all(|pair| pair[0] <= pair[1]),
            "прогресс обязан только расти: {percents:?}"
        );
        assert_eq!(percents.last(), Some(&100));
    }

    #[tokio::test]
    async fn second_run_does_nothing_and_stays_silent() {
        let fixture = fixture(PRINTS_VERSION);
        fixture
            .prepare(&RecordingSink::default())
            .await
            .expect("первая подготовка");

        let sink = RecordingSink::default();
        let prepared = fixture.prepare(&sink).await.expect("вторая подготовка");

        assert!(!prepared.prepared, "готовое дерево не надо готовить снова");
        assert_eq!(prepared.version, "2026.08.19");
        assert!(
            sink.events().is_empty(),
            "на тёплом запуске экран подготовки показывать нечему: {:?}",
            sink.stages()
        );
    }

    #[tokio::test]
    async fn a_cold_tree_is_only_warmed_up_and_never_unpacked_again() {
        // Ветка, ради которой перезагрузка машины стоит секунд, а не
        // минут: дерево на месте и цело, но ОС забыла результат проверки
        // подписей. Проба этого не переживает — она упирается в таймаут, —
        // а распаковывать заново нечего.
        let fixture = fixture(PRINTS_VERSION);
        fixture
            .prepare(&RecordingSink::default())
            .await
            .expect("первая подготовка");
        let manifest_written_at = fixture.manifest_modified();

        let sink = RecordingSink::default();
        let prepared = fixture
            .prepare_with(
                &sink,
                Timeouts {
                    // «ОС забыла кэш» на настоящем дереве выглядит как
                    // проба, не уложившаяся в отведённое время; здесь то же
                    // самое достигается заведомо коротким таймаутом, чтобы
                    // не ждать пять секунд в каждом прогоне тестов.
                    probe: Duration::from_millis(1),
                    ..Timeouts::DEFAULT
                },
            )
            .await
            .expect("прогрев обязан пройти");

        assert!(
            prepared.prepared,
            "прогрев — это работа, а не её отсутствие"
        );
        assert_eq!(prepared.version, "2026.08.19");

        let stages = sink.stages();
        assert_eq!(
            stages.first(),
            Some(&YtDlpPrepareStage::WarmingUp),
            "у этой ветки другой порядок событий: сразу warmingUp, без unpacking — \
             экран подготовки (TL-17) обязан это учитывать: {stages:?}"
        );
        assert!(
            !stages.contains(&YtDlpPrepareStage::Unpacking),
            "целое дерево распаковывать заново незачем: {stages:?}"
        );
        assert_eq!(stages.last(), Some(&YtDlpPrepareStage::Ready));

        let percents: Vec<u8> = sink.events().iter().map(|event| event.percent).collect();
        assert_eq!(
            percents.first(),
            Some(&0),
            "без распаковки прогрев занимает всю шкалу с нуля: {percents:?}"
        );
        assert!(percents.windows(2).all(|pair| pair[0] <= pair[1]));

        assert_eq!(
            fixture.manifest_modified(),
            manifest_written_at,
            "манифест переписан — значит дерево всё-таки переустановили"
        );
        assert!(
            unpack::stale_staging_dirs(fixture.layout().root()).is_empty(),
            "распаковки не было, каталогов распаковки быть не может"
        );
    }

    #[tokio::test]
    async fn a_tree_that_passes_the_manifest_but_does_not_run_is_reinstalled() {
        let fixture = fixture(PRINTS_VERSION);
        fixture
            .prepare(&RecordingSink::default())
            .await
            .expect("первая подготовка");
        fixture.break_installed_executable();

        // Дешёвая сверка такой порчи не видит — иначе ветки `Broken` не
        // существовало бы вовсе.
        assert!(
            layout::validate(&fixture.layout(), &pinned_build_id()).is_ok(),
            "подмена обязана быть незаметной для сверки с манифестом"
        );

        let sink = RecordingSink::default();
        let prepared = fixture
            .prepare(&sink)
            .await
            .expect("переустановка обязана вылечить дерево");

        assert!(prepared.prepared);
        assert_eq!(prepared.version, "2026.08.19");
        assert!(
            sink.stages().contains(&YtDlpPrepareStage::Unpacking),
            "лечение непригодного дерева — это распаковка заново: {:?}",
            sink.stages()
        );
        assert_eq!(sink.stages().last(), Some(&YtDlpPrepareStage::Ready));
        assert!(
            !fixture.repair_path().exists(),
            "удачная подготовка обязана забыть, что дерево чинили"
        );
    }

    #[tokio::test]
    async fn a_reinstall_that_does_not_help_is_not_repeated_at_every_start() {
        // Дерево не запускается, и переустанавливать его нечем: в архиве
        // такой же нерабочий yt-dlp. Без счётчика это 124 МиБ записи и
        // полминуты прогрева при каждом запуске приложения, всегда с тем же
        // исходом.
        let fixture = fixture(PRINTS_VERSION);
        fixture
            .prepare(&RecordingSink::default())
            .await
            .expect("первая подготовка");
        fixture.replace_archive(EXITS_NONZERO);
        fixture.break_installed_executable();

        for attempt in 1..=MAX_REPAIR_ATTEMPTS {
            let sink = RecordingSink::default();
            let error = fixture
                .prepare(&sink)
                .await
                .expect_err("нерабочее дерево не может подготовиться");

            assert!(
                matches!(error, PrepareError::WarmupFailed { .. }),
                "попытка {attempt}: {error}"
            );
            assert!(
                sink.stages().contains(&YtDlpPrepareStage::Unpacking),
                "попытка {attempt} обязана быть настоящей переустановкой: {:?}",
                sink.stages()
            );
            assert_eq!(
                RepairLog::read(&fixture.repair_path()).attempts,
                attempt,
                "попытка {attempt} обязана быть записана рядом с установкой"
            );
        }

        let sink = RecordingSink::default();
        let error = fixture
            .prepare(&sink)
            .await
            .expect_err("исчерпав попытки, подготовка обязана отказать");

        assert!(
            matches!(error, PrepareError::WarmupFailed { .. }),
            "отказ остаётся тем же по типу, меняется только его цена: {error}"
        );
        assert!(
            error.to_string().contains("переустановка"),
            "сообщение обязано объяснять, почему попытки прекращены: {error}"
        );
        assert!(
            !sink.stages().contains(&YtDlpPrepareStage::Unpacking),
            "переустановки быть не должно: {:?}",
            sink.stages()
        );
        assert_eq!(sink.stages().last(), Some(&YtDlpPrepareStage::Failed));
    }

    #[tokio::test]
    async fn the_memory_of_failed_repairs_survives_a_restart_and_a_success_clears_it() {
        let fixture = fixture(PRINTS_VERSION);
        fixture
            .prepare(&RecordingSink::default())
            .await
            .expect("первая подготовка");
        fixture.replace_archive(EXITS_NONZERO);
        fixture.break_installed_executable();
        fixture
            .prepare(&RecordingSink::default())
            .await
            .expect_err("нерабочее дерево не может подготовиться");

        // Ничего, кроме файла в каталоге данных, между «запусками
        // приложения» не переживает: `prepare` состояния в памяти не
        // держит, а `Fixture` — только пути.
        assert_eq!(RepairLog::read(&fixture.repair_path()).attempts, 1);

        fixture.replace_archive(PRINTS_VERSION);
        let prepared = fixture
            .prepare(&RecordingSink::default())
            .await
            .expect("рабочий архив обязан вылечить дерево");

        assert!(prepared.prepared);
        assert_eq!(
            RepairLog::read(&fixture.repair_path()).attempts,
            0,
            "удачный запуск обязан сбрасывать счётчик"
        );
        assert!(!fixture.repair_path().exists());
    }

    #[tokio::test]
    async fn a_failure_that_is_not_about_the_tree_does_not_count_as_a_repair_attempt() {
        // «Архив непригоден» и «дерево не запускается» — разные беды с
        // разными действиями пользователя, и первая не должна приближать
        // отказ чинить вторую.
        let fixture = fixture(PRINTS_VERSION);
        fixture
            .prepare(&RecordingSink::default())
            .await
            .expect("первая подготовка");
        fixture.break_installed_executable();
        fs::write(&fixture.archive, b"not a zip at all").expect("испортить архив");

        let error = fixture
            .prepare(&RecordingSink::default())
            .await
            .expect_err("чинить нечем");

        assert!(
            matches!(error, PrepareError::ArchiveCorrupted { .. }),
            "{error}"
        );
        assert_eq!(
            RepairLog::read(&fixture.repair_path()).attempts,
            0,
            "попытка, сорвавшаяся не на дереве, обязана быть откачена"
        );
    }

    #[tokio::test]
    async fn two_preparations_started_at_once_do_the_work_once() {
        // Автозапуск при старте приложения и вызов команды с фронтенда —
        // две двери в одну и ту же подготовку, и разводит их единственный
        // мьютекс (`crate::commands::ytdlp::PreparationLock`). Без него оба
        // входа распаковывали бы дерево одновременно и дрались за
        // переименование в один и тот же каталог.
        let fixture = fixture(PRINTS_VERSION);
        let lock = crate::commands::PreparationLock::new();
        let first_sink = RecordingSink::default();
        let second_sink = RecordingSink::default();

        let (first, second) = tokio::join!(
            async {
                let _guard = lock.acquire().await;
                fixture.prepare(&first_sink).await
            },
            async {
                let _guard = lock.acquire().await;
                fixture.prepare(&second_sink).await
            }
        );

        let first = first.expect("первый вход обязан завершиться");
        let second = second.expect("второй вход обязан завершиться");

        assert_eq!(first.version, second.version);
        assert_eq!(first.path, second.path);
        assert_ne!(
            first.prepared, second.prepared,
            "работу обязан выполнить ровно один вход, второй — застать готовое"
        );

        let (worked, idle) = if first.prepared {
            (&first_sink, &second_sink)
        } else {
            (&second_sink, &first_sink)
        };
        assert!(
            worked.stages().contains(&YtDlpPrepareStage::Unpacking),
            "кто-то один обязан был распаковать дерево: {:?}",
            worked.stages()
        );
        assert!(
            idle.events().is_empty(),
            "вошедшему вторым показывать нечего: {:?}",
            idle.stages()
        );
        assert!(
            unpack::stale_staging_dirs(fixture.layout().root()).is_empty(),
            "второй вход не должен оставить своего каталога распаковки"
        );
    }

    #[tokio::test]
    async fn an_interrupted_unpack_is_detected_and_redone() {
        // Так выглядит подготовка, прерванная между переименованием дерева
        // и записью манифеста: дерево есть, манифеста нет.
        let fixture = fixture(PRINTS_VERSION);
        fixture
            .prepare(&RecordingSink::default())
            .await
            .expect("первая подготовка");
        fs::remove_file(fixture.manifest_path()).expect("убрать манифест");

        let sink = RecordingSink::default();
        let prepared = fixture
            .prepare(&sink)
            .await
            .expect("подготовка обязана пройти");

        assert!(prepared.prepared);
        assert!(
            sink.stages().contains(&YtDlpPrepareStage::Unpacking),
            "дерево без манифеста обязано быть распаковано заново"
        );
        assert!(fixture.manifest_path().exists());
    }

    #[tokio::test]
    async fn a_damaged_tree_is_never_used_as_is() {
        let fixture = fixture(PRINTS_VERSION);
        fixture
            .prepare(&RecordingSink::default())
            .await
            .expect("первая подготовка");
        // Порча, которую ловит дешёвая сверка: файл потерян.
        fs::remove_file(fixture.install_dir().join("_internal/lib.so")).expect("убрать файл");

        let sink = RecordingSink::default();
        fixture
            .prepare(&sink)
            .await
            .expect("подготовка обязана пройти");

        assert!(sink.stages().contains(&YtDlpPrepareStage::Unpacking));
        assert!(
            fixture.install_dir().join("_internal/lib.so").exists(),
            "дерево обязано быть восстановлено целиком"
        );
    }

    #[tokio::test]
    async fn leftovers_of_an_interrupted_unpack_are_removed() {
        let fixture = fixture(PRINTS_VERSION);
        let layout = fixture.layout();
        layout.create_root().expect("создать корень");
        let stale = layout
            .create_staging_dir(&other_build_id())
            .expect("создать каталог распаковки");
        fs::create_dir_all(stale.join("_internal")).expect("создать мусор");
        fs::write(stale.join("half-written"), b"...").expect("записать мусор");

        fixture
            .prepare(&RecordingSink::default())
            .await
            .expect("подготовка обязана пройти");

        assert!(
            !stale.exists(),
            "мусор прерванной подготовки обязан исчезнуть"
        );
    }

    #[tokio::test]
    async fn a_half_downloaded_update_archive_is_removed_too() {
        // С-7: обрыв во время скачивания обновления оставляет
        // `.download-*` — до шестидесяти мегабайт, — и убирать его надо
        // там же, где `.staging-*`: на старте, единственном моменте,
        // когда в каталог данных заведомо никто не пишет. До TL-56
        // уборка знала только про каталоги распаковки, и остаток
        // скачивания пережил бы сколько угодно запусков.
        let fixture = fixture(PRINTS_VERSION);
        let layout = fixture.layout();
        layout.create_root().expect("создать корень");
        let (partial, file) = layout
            .create_download_file(&other_build_id())
            .expect("создать файл приёма");
        drop(file);
        fs::write(&partial, vec![0_u8; 4096]).expect("записать недокачанное");

        assert_eq!(
            layout.stale_downloads(),
            vec![partial.clone()],
            "остаток обязан быть виден уборке до подготовки"
        );

        fixture
            .prepare(&RecordingSink::default())
            .await
            .expect("подготовка обязана пройти");

        assert!(
            !partial.exists(),
            "недокачанный архив обновления обязан исчезнуть"
        );
        assert!(
            fixture.install_dir().exists(),
            "уборка чужого остатка не должна мешать самой подготовке"
        );
    }

    #[tokio::test]
    async fn a_broken_archive_fails_without_leaving_anything_behind() {
        let fixture = fixture(PRINTS_VERSION);
        fs::write(&fixture.archive, b"not a zip at all").expect("испортить архив");

        let sink = RecordingSink::default();
        let error = fixture
            .prepare(&sink)
            .await
            .expect_err("битый архив не может подготовиться");

        assert!(
            matches!(error, PrepareError::ArchiveCorrupted { .. }),
            "{error}"
        );
        assert_eq!(sink.stages().last(), Some(&YtDlpPrepareStage::Failed));
        assert!(!fixture.install_dir().exists());
        assert!(
            unpack::stale_staging_dirs(fixture.layout().root()).is_empty(),
            "неудачная распаковка не должна оставлять .staging-*"
        );
    }

    #[tokio::test]
    async fn a_missing_archive_is_reported_as_such() {
        let fixture = fixture(PRINTS_VERSION);
        fs::remove_file(&fixture.archive).expect("убрать архив");

        let sink = RecordingSink::default();
        let error = fixture.prepare(&sink).await.expect_err("архива нет");

        assert!(
            matches!(error, PrepareError::ArchiveMissing { .. }),
            "{error}"
        );
        let failed = sink
            .events()
            .into_iter()
            .find(|event| event.stage == YtDlpPrepareStage::Failed)
            .expect("отказ обязан быть событием");
        assert_eq!(
            failed.error.map(|error| error.kind),
            Some(crate::types::YtDlpPrepareErrorKind::ArchiveMissing)
        );
    }

    #[tokio::test]
    async fn a_tree_that_does_not_run_reports_a_warmup_failure() {
        let fixture = fixture("#!/bin/sh\nexit 3\n");

        let sink = RecordingSink::default();
        let error = fixture
            .prepare(&sink)
            .await
            .expect_err("не запускающийся yt-dlp — отказ подготовки");

        assert!(
            matches!(error, PrepareError::WarmupFailed { .. }),
            "{error}"
        );
        assert_eq!(sink.stages().last(), Some(&YtDlpPrepareStage::Failed));
    }

    #[tokio::test]
    async fn resolving_the_executable_requires_a_finished_preparation() {
        let fixture = fixture(PRINTS_VERSION);
        let in_use = InUse::new();

        installed_executable(&fixture.data_dir, &in_use)
            .expect_err("до подготовки резолвить нечего");

        fixture
            .prepare(&RecordingSink::default())
            .await
            .expect("подготовка");

        let (path, _guard) =
            installed_executable(&fixture.data_dir, &in_use).expect("после подготовки путь есть");
        assert!(path.ends_with(EXECUTABLE_NAME));
    }

    #[test]
    fn a_failed_unpack_leaves_the_previous_installation_of_the_same_id_alone() {
        // Гарантия С-4 в её конструктивной форме (TL-58): распаковка идёт
        // в `.staging-*` целиком, и прежнее дерево сносится только после
        // её успеха. До этой правки порядок был обратным, и ревью TL-56
        // показало запуском, что отказ на битом архиве не оставлял ни
        // каталога, ни исполняемого файла, ни манифеста — то есть уносил
        // рабочую установку.
        //
        // Тест зовёт `install` напрямую: через контур этот путь закрыт
        // отдельной проверкой («ставить поверх активной нечего»), а
        // предмет здесь — сама установка, у которой два вызывающих.
        let dir = tempdir().expect("tempdir");
        let data_dir = dir.path().join("app-data");
        let layout = Layout::new(&data_dir);
        layout.create_root().expect("корень обязан создаваться");

        let identity = layout::ArchiveIdentity::bundled();
        let build_id = pinned_build_id();
        let install_dir = layout.install_dir(&build_id);
        let manifest_path = layout.manifest_path(&build_id);

        // Рабочая установка того же build id — та, которую отказ не имеет
        // права тронуть.
        let good = dir.path().join("good.zip");
        write_fake_ytdlp_zip(&good, PRINTS_VERSION);
        install(&good, &layout, identity, &mut |_, _| {}).expect("первая установка");
        assert!(install_dir.join(EXECUTABLE_NAME).exists());
        let before = fs::read(install_dir.join(EXECUTABLE_NAME)).expect("файл читается");

        // Архив, который не открывается как zip: отказ приходит из
        // распаковки, то есть ровно оттуда, где раньше стоял снос.
        let broken = dir.path().join("broken.zip");
        fs::write(&broken, b"not a zip at all").expect("битый архив");

        let error = install(&broken, &layout, identity, &mut |_, _| {})
            .expect_err("битый архив обязан быть отвергнут");

        assert!(
            install_dir.exists(),
            "каталог рабочей установки обязан пережить отказ ({error})"
        );
        assert!(
            manifest_path.exists(),
            "манифест рабочей установки обязан пережить отказ ({error})"
        );
        assert_eq!(
            fs::read(install_dir.join(EXECUTABLE_NAME)).expect("файл читается"),
            before,
            "исполняемый файл обязан остаться тем же байт в байт"
        );
        assert!(
            layout::validate(&layout, &build_id).is_ok(),
            "установка обязана остаться пригодной к запуску"
        );

        // И ни одного полураспакованного дерева рядом: неудача не
        // оставляет за собой 124 МиБ.
        for entry in fs::read_dir(layout.root()).expect("корень читается") {
            let name = entry.expect("запись").file_name();
            assert!(
                !name.to_string_lossy().starts_with(".staging-"),
                "остался каталог распаковки {name:?}"
            );
        }
    }

    #[tokio::test]
    async fn resolving_the_executable_marks_the_installation_as_in_use() {
        // Ф-7: установка, из которой запущен процесс, не удаляется. Эта
        // функция отвечает за половину утверждения — «контур знает, какая
        // установка занята», — и знает он ровно потому, что путь берётся
        // только здесь. Вторая половина (уборка отметки уважает) живёт и
        // проверена в `super::state`, и повторять её здесь нечем: тест,
        // который сносит дерево, проверял бы чужую гарантию, а не эту.
        let fixture = fixture(PRINTS_VERSION);
        let in_use = InUse::new();
        fixture
            .prepare(&RecordingSink::default())
            .await
            .expect("подготовка");

        assert!(
            in_use.snapshot().is_empty(),
            "до резолва занятых установок нет"
        );

        let (_path, guard) =
            installed_executable(&fixture.data_dir, &in_use).expect("путь обязан находиться");
        assert_eq!(
            in_use.snapshot(),
            BTreeSet::from([pinned_build_id()]),
            "отмеченной обязана быть ровно та установка, путь к которой отдан"
        );

        drop(guard);
        assert!(
            in_use.snapshot().is_empty(),
            "отметка держится стражем и снимается вместе с ним"
        );
    }

    #[tokio::test]
    async fn the_first_run_records_the_pin_as_the_active_installation() {
        // Ф-5: активная установка — явная запись, и завести её обязана
        // подготовка первого запуска. Без этого запись появлялась бы
        // только после первого обновления, а до него резолв работал бы
        // догадкой.
        let fixture = fixture(PRINTS_VERSION);
        assert_eq!(
            fixture.state(),
            InstallState::default(),
            "до подготовки записи нет"
        );

        fixture
            .prepare(&RecordingSink::default())
            .await
            .expect("подготовка обязана пройти");

        let state = fixture.state();
        assert_eq!(
            state.active().map(InstallEntry::build_id),
            Some(&pinned_build_id())
        );
        assert_eq!(
            state.known_good(),
            None,
            "первому запуску не от чего откатываться"
        );

        // Второй запуск ничего не переписывает: активная и так та же.
        let before = fs::metadata(fixture.layout().state_path())
            .and_then(|meta| meta.modified())
            .expect("время изменения записи");
        fixture
            .prepare(&RecordingSink::default())
            .await
            .expect("второй запуск обязан пройти");
        assert_eq!(
            fs::metadata(fixture.layout().state_path())
                .and_then(|meta| meta.modified())
                .expect("время изменения записи"),
            before,
            "повторная активация той же установки не должна переписывать файл"
        );
    }

    #[tokio::test]
    async fn preparation_and_resolution_follow_the_record_not_the_pin() {
        // Машина, на которой контур обновления уже отработал: активна
        // версия, которой в бандле нет. Подготовка обязана греть её, а не
        // распаковывать пин, — иначе после уборки (Ф-8 держит на диске
        // две установки) каждый старт заново разворачивал бы 124 МиБ
        // дерева, которое потом никто не запустит.
        let fixture = fixture(PRINTS_VERSION);
        fixture.install_and_activate(updated_identity(), "#!/bin/sh\necho 2030.01.01\n");

        let prepared = fixture
            .prepare(&RecordingSink::default())
            .await
            .expect("подготовка обязана пройти");

        assert_eq!(prepared.version, "2030.01.01");
        assert!(
            !prepared.prepared,
            "готовая установка не требует ни распаковки, ни прогрева"
        );
        assert!(
            !fixture.install_dir().exists(),
            "вложенный в бандл архив не должен распаковываться, пока активная \
             установка работает"
        );
        assert_eq!(
            fixture.state().active().map(InstallEntry::build_id),
            Some(updated_entry().build_id()),
            "подготовка не должна переписывать запись, которую не она завела"
        );

        // И то же самое для всех потребителей пути (служебный экран,
        // разбор ссылки, скачивание): они ходят через тот же резолв.
        let in_use = InUse::new();
        let (resolved, _guard) =
            installed_executable(&fixture.data_dir, &in_use).expect("путь обязан находиться");
        assert_eq!(
            resolved,
            fixture
                .layout()
                .install_dir(updated_entry().build_id())
                .join(EXECUTABLE_NAME)
        );
    }

    #[tokio::test]
    async fn a_broken_active_installation_falls_back_to_the_bundled_archive() {
        // Ф-5, «вшитый архив — резерв»: активная установка не запускается,
        // а починить её нечем — архива этой версии в бандле нет. Приложение
        // обязано остаться работоспособным на пине, и запись обязана
        // сказать об этом честно.
        let fixture = fixture(PRINTS_VERSION);
        fixture.install_and_activate(updated_identity(), EXITS_NONZERO);

        let prepared = fixture
            .prepare(&RecordingSink::default())
            .await
            .expect("резерв обязан вытащить подготовку");

        assert_eq!(prepared.version, "2026.08.19");
        assert!(
            prepared
                .path
                .starts_with(&fixture.install_dir().display().to_string()),
            "работать обязан пин: {}",
            prepared.path
        );

        let state = fixture.state();
        assert_eq!(
            state.active().map(InstallEntry::build_id),
            Some(&pinned_build_id())
        );
        assert_eq!(
            state.known_good().map(InstallEntry::build_id),
            Some(updated_entry().build_id()),
            "смещённая установка становится известно-хорошей — годна ли она к \
             запуску, решает тот, кто будет откатываться (TL-58)"
        );
    }

    #[test]
    fn unpacking_progress_never_exceeds_its_share_of_the_bar() {
        let started = Instant::now();
        for done in [0_u64, 1, 500, 999, 1000] {
            let event = unpacking_event(done, 1000, started);
            assert_eq!(event.stage, YtDlpPrepareStage::Unpacking);
            assert!(
                event.percent <= UNPACK_PERCENT_SHARE,
                "распаковка не может занимать больше своей доли: {}",
                event.percent
            );
        }
    }

    #[test]
    fn warmup_progress_starts_at_the_unpacking_share_and_stops_below_a_hundred() {
        let started = Instant::now();
        let event = warming_event(started, Duration::from_secs(30), UNPACK_PERCENT_SHARE);
        assert_eq!(event.percent, UNPACK_PERCENT_SHARE);
        assert_eq!(event.eta_secs, Some(30));

        // Прогрев затянулся вдвое против оценки: полоса упирается в 99, а
        // оценка оставшегося обнуляется, но событие остаётся честным —
        // stage всё ещё warmingUp.
        let overdue = warming_event(started, Duration::from_nanos(1), UNPACK_PERCENT_SHARE);
        assert_eq!(overdue.percent, 99);
        assert_eq!(overdue.eta_secs, Some(0));
        assert_eq!(overdue.stage, YtDlpPrepareStage::WarmingUp);
    }

    #[test]
    fn repair_attempts_are_exhausted_only_until_they_cool_down() {
        let exhausted = (0..MAX_REPAIR_ATTEMPTS).fold(RepairLog::empty(), |log, _| {
            log.with_attempt("не запускается", 1_000)
        });

        assert!(
            !repair_exhausted(&RepairLog::empty(), 1_000),
            "первую попытку никто не отменял"
        );
        assert!(!repair_exhausted(
            &RepairLog::empty().with_attempt("не запускается", 1_000),
            1_000
        ));
        assert!(repair_exhausted(&exhausted, 1_000));
        assert!(repair_exhausted(
            &exhausted,
            1_000 + REPAIR_COOLDOWN.as_secs() - 1
        ));
        assert!(
            !repair_exhausted(&exhausted, 1_000 + REPAIR_COOLDOWN.as_secs()),
            "счётчик обязан остывать: иначе устранённая причина никогда не \
             выпустит пользователя из отказа"
        );
        assert!(
            repair_exhausted(&exhausted, 0),
            "часы, отошедшие назад, не повод считать попытку давней"
        );
    }

    #[test]
    fn version_falls_back_to_the_pinned_one_when_the_run_prints_nothing() {
        assert_eq!(parse_version("2026.08.19\n", "pin"), "2026.08.19");
        assert_eq!(parse_version("   \n", "pin"), "pin");
    }

    // ─────────────── медленная машина — не отказ (TL-21) ───────────────

    use crate::ytdlp::testing::Control;

    /// «Прогрев не укладывается»: фикстура висит, пока её не отпустят, а
    /// таймауты заведомо короткие. Исход от времени не зависит — висящий
    /// процесс не завершится ни за 50 мс, ни за сколько угодно.
    const HUNG: Timeouts = Timeouts {
        probe: Duration::from_millis(1),
        warmup: Duration::from_millis(50),
        background: Duration::from_millis(50),
    };

    /// Таймауты прогрева, которых тест не дождётся: подготовка, которая
    /// всё-таки ждёт прогрев, упрётся в страховку [`SAFETY_NET`].
    const UNREACHABLE: Timeouts = Timeouts {
        probe: PROBE_TIMEOUT,
        warmup: Duration::from_secs(3600),
        background: Duration::from_secs(3600),
    };

    /// Страховка от вечного зависания красного прогона, а не утверждение о
    /// длительности: зелёный исход не зависит от неё никак.
    const SAFETY_NET: Duration = Duration::from_secs(60);

    /// Фикстура, чей yt-dlp считает запуски и зависает по команде.
    fn controlled_fixture() -> (Fixture, Control) {
        let dir = tempdir().expect("tempdir");
        let control = Control::new(&dir.path().join("control"));
        let archive = dir.path().join("yt-dlp.zip");
        write_fake_ytdlp_zip(&archive, &control.script("2026.08.19"));
        let data_dir = dir.path().join("app-data");

        (
            Fixture {
                _dir: dir,
                archive,
                data_dir,
                registry: ChildRegistry::new(),
            },
            control,
        )
    }

    fn write_mark(path: &Path) {
        SlowWarmupMark::recorded(None, 120_000, crate::clock::now_unix_secs())
            .write_atomic(path)
            .expect("отметка обязана записываться");
    }

    #[tokio::test]
    async fn a_warm_up_that_hits_its_timeout_is_recorded_and_the_start_is_not_refused() {
        let (fixture, control) = controlled_fixture();
        fixture
            .prepare(&RecordingSink::default())
            .await
            .expect("первая подготовка");
        control.hang();

        let sink = RecordingSink::default();
        let outcome = fixture
            .outcome_with(&sink, HUNG)
            .await
            .expect("медленная машина — не отказ");
        control.release();

        assert!(
            outcome.background.is_some(),
            "прогрев обязан продолжиться в фоне"
        );
        assert!(
            outcome.warm_launch.is_none(),
            "у недошедшего прогрева нет запуска для экрана"
        );
        assert!(
            outcome.prepared.prepared,
            "экран подготовки показывал прогрев и обязан получить ready"
        );
        assert_eq!(outcome.prepared.version, layout::BUNDLED_VERSION);
        let stages = sink.stages();
        assert_eq!(stages.last(), Some(&YtDlpPrepareStage::Ready), "{stages:?}");
        assert!(!stages.contains(&YtDlpPrepareStage::Failed), "{stages:?}");

        let mark = SlowWarmupMark::read(&fixture.mark_path())
            .expect("отметка читается")
            .expect("отметка обязана быть записана рядом с установкой");
        assert_eq!(mark.timeouts, 1);
        assert_eq!(mark.timeout_ms, 50);
    }

    #[tokio::test]
    async fn a_first_run_whose_warm_up_hits_its_timeout_is_not_refused_either() {
        let (fixture, control) = controlled_fixture();
        control.hang();

        let sink = RecordingSink::default();
        let outcome = fixture
            .outcome_with(&sink, HUNG)
            .await
            .expect("распаковка прошла, прогрев медленный — это не отказ");
        control.release();

        assert!(outcome.background.is_some());
        assert!(sink.stages().contains(&YtDlpPrepareStage::Unpacking));
        assert_eq!(sink.stages().last(), Some(&YtDlpPrepareStage::Ready));
        assert!(fixture.mark_path().exists());
    }

    #[tokio::test]
    async fn the_next_start_with_the_mark_does_not_wait_and_a_finished_background_warm_up_clears_it(
    ) {
        let (fixture, control) = controlled_fixture();
        fixture
            .prepare(&RecordingSink::default())
            .await
            .expect("первая подготовка");
        control.hang();
        fixture
            .outcome_with(&RecordingSink::default(), HUNG)
            .await
            .expect("прогрев упёрся в таймаут");
        assert!(fixture.mark_path().exists(), "предусловие: отметка есть");

        // Следующий старт приложения.
        control.hang();
        let launches = control.launches();
        let sink = RecordingSink::default();
        let outcome = tokio::time::timeout(SAFETY_NET, fixture.outcome_with(&sink, UNREACHABLE))
            .await
            .expect("подготовка с отметкой обязана вернуться, не дожидаясь прогрева")
            .expect("и вернуться успехом");

        assert_eq!(
            control.launches(),
            launches,
            "с отметкой подготовка сама ничего не запускает — ни пробы, ни прогрева"
        );
        assert!(!outcome.prepared.prepared);
        assert!(
            sink.events().is_empty(),
            "экран подготовки на этом старте не поднимается: {:?}",
            sink.stages()
        );
        let background = outcome.background.expect("прогрев уходит в фон");

        let (result, ()) = tokio::join!(background.run(&fixture.registry), async {
            control.wait_until_hanging().await;
            assert!(
                fixture.mark_path().exists(),
                "пока прогрев идёт, отметка на месте"
            );
            control.release();
        });

        assert_eq!(result, BackgroundOutcome::Warmed);
        assert!(
            !fixture.mark_path().exists(),
            "удачный фоновый прогрев снимает отметку"
        );

        let outcome = fixture
            .outcome_with(&RecordingSink::default(), Timeouts::DEFAULT)
            .await
            .expect("дальше старт обычный");
        assert!(outcome.background.is_none());
        assert!(outcome.warm_launch.is_some(), "проба застаёт тёплое дерево");
    }

    #[tokio::test]
    async fn a_background_warm_up_that_times_out_again_keeps_the_mark() {
        let (fixture, control) = controlled_fixture();
        fixture
            .prepare(&RecordingSink::default())
            .await
            .expect("первая подготовка");
        control.hang();
        fixture
            .outcome_with(&RecordingSink::default(), HUNG)
            .await
            .expect("прогрев упёрся в таймаут");

        control.hang();
        let outcome = fixture
            .outcome_with(&RecordingSink::default(), HUNG)
            .await
            .expect("с отметкой — успех");
        let result = outcome
            .background
            .expect("прогрев уходит в фон")
            .run(&fixture.registry)
            .await;
        control.release();

        assert_eq!(result, BackgroundOutcome::TimedOut);
        let mark = SlowWarmupMark::read(&fixture.mark_path())
            .expect("читается")
            .expect("отметка остаётся");
        assert_eq!(mark.timeouts, 2, "второй таймаут подряд учтён");
    }

    #[tokio::test]
    async fn a_background_warm_up_that_fails_outright_drops_the_mark_and_the_next_start_repairs() {
        let fixture = fixture(PRINTS_VERSION);
        fixture
            .prepare(&RecordingSink::default())
            .await
            .expect("первая подготовка");
        write_mark(&fixture.mark_path());
        fixture.break_installed_executable();

        let outcome = fixture
            .outcome_with(&RecordingSink::default(), Timeouts::DEFAULT)
            .await
            .expect("с отметкой дерево считается готовым");
        let result = outcome
            .background
            .expect("прогрев уходит в фон")
            .run(&fixture.registry)
            .await;

        assert!(matches!(result, BackgroundOutcome::Failed(_)), "{result:?}");
        assert!(
            !fixture.mark_path().exists(),
            "отказ запуска — не медленная машина: отметка снимается"
        );

        let sink = RecordingSink::default();
        let prepared = fixture
            .prepare(&sink)
            .await
            .expect("следующий старт обязан вылечить дерево переустановкой");
        assert!(prepared.prepared);
        assert!(sink.stages().contains(&YtDlpPrepareStage::Unpacking));
    }

    #[tokio::test]
    async fn an_unreadable_mark_is_treated_as_absent() {
        let (fixture, control) = controlled_fixture();
        fixture
            .prepare(&RecordingSink::default())
            .await
            .expect("первая подготовка");

        let foreign_schema = br#"{"schemaVersion":99,"timeouts":1,"lastTimeoutUnix":0,"lastTimeoutAt":"","timeoutMs":1}"#;
        for garbage in [b"{ not json".as_slice(), foreign_schema.as_slice()] {
            fs::write(fixture.mark_path(), garbage).expect("испортить отметку");
            assert!(
                SlowWarmupMark::read(&fixture.mark_path()).is_err(),
                "испорченная отметка отличается от отсутствующей — ради строки в логе"
            );

            let launches = control.launches();
            let outcome = fixture
                .outcome_with(&RecordingSink::default(), Timeouts::DEFAULT)
                .await
                .expect("испорченная отметка не мешает старту");

            assert!(
                outcome.background.is_none(),
                "испорченная отметка — как отсутствующая: дерево проверяется пробой"
            );
            assert!(outcome.warm_launch.is_some());
            assert_eq!(control.launches(), launches + 1);
            assert!(
                !fixture.mark_path().exists(),
                "тёплое дерево снимает и испорченную отметку"
            );
        }

        assert_eq!(
            SlowWarmupMark::read(&fixture.data_dir.join("absent")),
            Ok(None),
            "отсутствие — не ошибка"
        );
    }

    #[tokio::test]
    async fn a_slow_active_installation_is_kept_and_not_replaced_by_the_pin() {
        let (fixture, control) = controlled_fixture();
        fixture.install_and_activate(updated_identity(), &control.script("2030.01.01"));
        control.hang();

        let outcome = fixture
            .outcome_with(&RecordingSink::default(), HUNG)
            .await
            .expect("медленная активная установка — не отказ");
        control.release();

        let mark_path = fixture
            .layout()
            .slow_warmup_path(updated_entry().build_id());
        assert!(outcome.background.is_some());
        assert_eq!(outcome.prepared.version, "2030.01.01");
        assert!(mark_path.exists(), "отметка — у активной установки");
        assert!(
            !fixture.install_dir().exists(),
            "медленную активную установку пин не вытесняет"
        );

        // Следующий старт с отметкой — снова активная, без пробы.
        let launches = control.launches();
        let outcome = fixture
            .outcome_with(&RecordingSink::default(), UNREACHABLE)
            .await
            .expect("успех");
        assert_eq!(control.launches(), launches);
        assert_eq!(outcome.prepared.version, "2030.01.01");
        assert!(outcome.background.is_some());
    }
}
