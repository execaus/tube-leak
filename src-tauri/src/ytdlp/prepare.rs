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
use super::layout::{self, Installed, Layout, RepairLog};
use super::unpack;
use crate::sidecar::{self, ChildRegistry, SidecarError};
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
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

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
const WARMUP_TIMEOUT: Duration = Duration::from_secs(120);

// Соотношение сторожится на этапе компиляции: проверка «тёплое ли дерево»
// обязана быть заметно короче прогрева, иначе она перестаёт отличать одно
// состояние от другого и превращается в удвоенный прогрев.
const _: () = assert!(PROBE_TIMEOUT.as_secs() * 10 <= WARMUP_TIMEOUT.as_secs());

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
}

impl Timeouts {
    const DEFAULT: Self = Self {
        probe: PROBE_TIMEOUT,
        warmup: WARMUP_TIMEOUT,
    };
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
/// события.
pub async fn prepare(
    archive_path: &Path,
    data_dir: &Path,
    registry: &ChildRegistry,
    sink: &dyn ProgressSink,
) -> Result<YtDlpPrepared, PrepareError> {
    prepare_with(archive_path, data_dir, registry, sink, Timeouts::DEFAULT).await
}

async fn prepare_with(
    archive_path: &Path,
    data_dir: &Path,
    registry: &ChildRegistry,
    sink: &dyn ProgressSink,
    timeouts: Timeouts,
) -> Result<YtDlpPrepared, PrepareError> {
    let started = Instant::now();
    let result = prepare_inner(archive_path, data_dir, registry, sink, started, timeouts).await;

    match &result {
        Ok(prepared) if prepared.prepared => sink.emit(YtDlpPrepareEvent {
            stage: YtDlpPrepareStage::Ready,
            percent: 100,
            eta_secs: Some(0),
            version: Some(prepared.version.clone()),
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
) -> Result<YtDlpPrepared, PrepareError> {
    let layout = Layout::new(data_dir);
    layout.create_root()?;
    let build_id = layout::bundled_build_id();

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

    let repair_path = layout.repair_path(&build_id);

    match layout::validate(&layout, &build_id) {
        Ok(installed) => {
            match probe(&installed, registry, timeouts.probe).await {
                Probe::Warm(version) => {
                    RepairLog::clear(&repair_path);
                    Ok(YtDlpPrepared {
                        version,
                        path: installed.executable.display().to_string(),
                        prepared: false,
                        duration_ms: elapsed_ms(started),
                    })
                }
                Probe::Cold => {
                    // Дерево на месте и цело, но ОС забыла результат
                    // проверки подписей — распаковывать заново незачем,
                    // достаточно прогреть.
                    let version = warm_up(
                        &installed,
                        registry,
                        sink,
                        count_tree_files(&installed),
                        0,
                        timeouts.warmup,
                    )
                    .await?;
                    RepairLog::clear(&repair_path);
                    Ok(YtDlpPrepared {
                        version,
                        path: installed.executable.display().to_string(),
                        prepared: true,
                        duration_ms: elapsed_ms(started),
                    })
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
                        &layout,
                        &build_id,
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
            let prepared = install_and_warm(
                archive_path,
                &layout,
                &build_id,
                registry,
                sink,
                started,
                timeouts,
            )
            .await?;
            RepairLog::clear(&repair_path);
            Ok(prepared)
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
    build_id: &str,
    registry: &ChildRegistry,
    sink: &dyn ProgressSink,
    started: Instant,
    timeouts: Timeouts,
) -> Result<YtDlpPrepared, PrepareError> {
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
        Ok(_) => RepairLog::clear(&repair_path),
        Err(PrepareError::WarmupFailed { .. }) => {}
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
    build_id: &str,
    registry: &ChildRegistry,
    sink: &dyn ProgressSink,
    started: Instant,
    timeouts: Timeouts,
) -> Result<YtDlpPrepared, PrepareError> {
    let installed = install(archive_path, layout, build_id, sink)?;
    let file_count = count_tree_files(&installed);
    let version = warm_up(
        &installed,
        registry,
        sink,
        file_count,
        UNPACK_PERCENT_SHARE,
        timeouts.warmup,
    )
    .await?;

    Ok(YtDlpPrepared {
        version,
        path: installed.executable.display().to_string(),
        prepared: true,
        duration_ms: elapsed_ms(started),
    })
}

/// Распаковка в `.staging-*`, атомарный перенос, запись манифеста.
fn install(
    archive_path: &Path,
    layout: &Layout,
    build_id: &str,
    sink: &dyn ProgressSink,
) -> Result<Installed, PrepareError> {
    let install_dir = layout.install_dir(build_id);
    let manifest_path = layout.manifest_path(build_id);

    // Каталог распаковки создаётся здесь и под непредсказуемым именем
    // (см. doc `super::layout`), поэтому «убрать прежний staging» не
    // требуется: своего у нас ещё нет, а чужой — не наш.
    let staging = layout.create_staging_dir(build_id)?;

    // Прежняя установка этого же build id могла остаться непригодной
    // (`validate` уже сказала, что она не годится) — переименование в
    // занятый путь не пройдёт, поэтому её надо убрать до распаковки.
    unpack::remove_dir_if_exists(&install_dir)?;
    let _ = std::fs::remove_file(&manifest_path);

    let unpack_started = Instant::now();
    let unpacked = unpack::unpack(archive_path, &staging, &mut |done, total| {
        sink.emit(unpacking_event(done, total, unpack_started));
    })
    .inspect_err(|_| {
        // Полураспакованное дерево не должно пережить неудачу — иначе
        // следующий запуск найдёт мусор на 124 МиБ и будет чистить его
        // «за прошлый раз».
        let _ = unpack::remove_dir_if_exists(&staging);
    })?;

    unpack::promote(&staging, &install_dir)?;

    let manifest = layout::manifest_for(&install_dir, &unpacked.executable)?;
    manifest.write_atomic(&manifest_path)?;

    Ok(Installed {
        dir: install_dir.clone(),
        executable: install_dir.join(&unpacked.executable),
        version: manifest.yt_dlp_version,
    })
}

/// Исход проверки «дерево уже тёплое?».
enum Probe {
    /// Отозвалось быстро; строка — разобранная версия.
    Warm(String),
    /// Не уложилось в [`PROBE_TIMEOUT`] — нужен прогрев.
    Cold,
    /// Не запускается или завершается с ошибкой.
    Broken(String),
}

async fn probe(installed: &Installed, registry: &ChildRegistry, timeout: Duration) -> Probe {
    match sidecar::run(&installed.executable, WARMUP_ARGS, timeout, registry).await {
        Ok(output) => Probe::Warm(parse_version(&output.stdout, &installed.version)),
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
) -> Result<String, PrepareError> {
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

    match output {
        Ok(output) => Ok(parse_version(&output.stdout, &installed.version)),
        Err(error) => Err(PrepareError::WarmupFailed {
            reason: error.to_string(),
        }),
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

/// Путь к исполняемому файлу готовой установки yt-dlp.
///
/// `Err` означает «подготовка не выполнена или дерево непригодно» — это
/// не сбой, а нормальное состояние до первого вызова `prepare_ytdlp`.
pub fn installed_executable(data_dir: &Path) -> Result<PathBuf, PrepareError> {
    let layout = Layout::new(data_dir);
    layout::validate(&layout, &layout::bundled_build_id())
        .map(|installed| installed.executable)
        .map_err(|invalid| PrepareError::LayoutUnexpected {
            reason: invalid.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
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
            prepare_with(
                &self.archive,
                &self.data_dir,
                &self.registry,
                sink,
                timeouts,
            )
            .await
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
            self.layout().repair_path(&layout::bundled_build_id())
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
            self.layout().install_dir(&layout::bundled_build_id())
        }

        fn manifest_path(&self) -> PathBuf {
            self.layout().manifest_path(&layout::bundled_build_id())
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
            layout::validate(&fixture.layout(), &layout::bundled_build_id()).is_ok(),
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
            .create_staging_dir("2000.01.01-deadbeefdead")
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

        installed_executable(&fixture.data_dir).expect_err("до подготовки резолвить нечего");

        fixture
            .prepare(&RecordingSink::default())
            .await
            .expect("подготовка");

        let path = installed_executable(&fixture.data_dir).expect("после подготовки путь есть");
        assert!(path.ends_with(EXECUTABLE_NAME));
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
}
