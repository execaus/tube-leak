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
use super::layout::{self, Installed, Layout};
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
    let started = Instant::now();
    let result = prepare_inner(archive_path, data_dir, registry, sink, started).await;

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

    match layout::validate(&layout, &build_id) {
        Ok(installed) => {
            match probe(&installed, registry).await {
                Probe::Warm(version) => Ok(YtDlpPrepared {
                    version,
                    path: installed.executable.display().to_string(),
                    prepared: false,
                    duration_ms: elapsed_ms(started),
                }),
                Probe::Cold => {
                    // Дерево на месте и цело, но ОС забыла результат
                    // проверки подписей — распаковывать заново незачем,
                    // достаточно прогреть.
                    let version =
                        warm_up(&installed, registry, sink, count_tree_files(&installed), 0)
                            .await?;
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
                    // того же размера», поэтому единственное осмысленное
                    // действие — переустановить и попробовать ещё раз.
                    // Повторов внутри одного вызова не бывает: если после
                    // переустановки не заработало, ошибка уходит наверх.
                    eprintln!("yt-dlp: установка не запускается ({reason}), переустанавливаю");
                    install_and_warm(archive_path, &layout, &build_id, registry, sink, started)
                        .await
                }
            }
        }
        Err(invalid) => {
            eprintln!("yt-dlp: установка непригодна ({invalid}), распаковываю заново");
            install_and_warm(archive_path, &layout, &build_id, registry, sink, started).await
        }
    }
}

/// Распаковывает дерево заново и прогревает его.
async fn install_and_warm(
    archive_path: &Path,
    layout: &Layout,
    build_id: &str,
    registry: &ChildRegistry,
    sink: &dyn ProgressSink,
    started: Instant,
) -> Result<YtDlpPrepared, PrepareError> {
    let installed = install(archive_path, layout, build_id, sink)?;
    let file_count = count_tree_files(&installed);
    let version = warm_up(&installed, registry, sink, file_count, UNPACK_PERCENT_SHARE).await?;

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
    let staging = layout.staging_dir(build_id);
    let install_dir = layout.install_dir(build_id);
    let manifest_path = layout.manifest_path(build_id);

    // Прежняя установка этого же build id могла остаться непригодной
    // (`validate` уже сказала, что она не годится) — переименование в
    // занятый путь не пройдёт, поэтому её надо убрать до распаковки.
    unpack::remove_dir_if_exists(&staging)?;
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

async fn probe(installed: &Installed, registry: &ChildRegistry) -> Probe {
    match sidecar::run(&installed.executable, WARMUP_ARGS, PROBE_TIMEOUT, registry).await {
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
) -> Result<String, PrepareError> {
    let expected = Duration::from_millis(file_count.saturating_mul(WARMUP_MS_PER_FILE));
    let started = Instant::now();
    sink.emit(warming_event(started, expected, base_percent));

    let run = sidecar::run(&installed.executable, WARMUP_ARGS, WARMUP_TIMEOUT, registry);
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
            prepare(&self.archive, &self.data_dir, &self.registry, sink).await
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
        let stale = layout.staging_dir("2000.01.01-deadbeefdead");
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
    fn version_falls_back_to_the_pinned_one_when_the_run_prints_nothing() {
        assert_eq!(parse_version("2026.08.19\n", "pin"), "2026.08.19");
        assert_eq!(parse_version("   \n", "pin"), "pin");
    }
}
