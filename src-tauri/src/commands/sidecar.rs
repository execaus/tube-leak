//! `#[tauri::command]` для служебного экрана: проверка исполняемых
//! файлов yt-dlp, ffmpeg и deno (Ф-9, Н-6 эпика E1; deno — TL-110).
//!
//! Тонкий слой над `crate::sidecar`: резолвит путь к каждому бинарнику,
//! запускает его с аргументом версии и конвертирует
//! [`crate::sidecar::SidecarError`] в контрактный
//! [`crate::types::SidecarCheckResult`]. Все три проверки идут параллельно
//! (`tokio::join!`, Н-6 «Экономия вызовов» — общее время ожидания близко к
//! максимуму из проверок, а не к их сумме).
//!
//! Резолв у бинарников разный (TL-12): ffmpeg и deno — sidecar рядом с
//! исполняемым файлом приложения, yt-dlp — распакованное дерево в каталоге
//! данных, см. [`crate::ytdlp`]. Отсюда предусловие: команда `prepare_ytdlp`
//! должна отработать раньше, иначе yt-dlp честно окажется `notFound` —
//! дерева ещё нет.
//!
//! Проверка deno — заодно и его прогрев на старте: служебный экран зовёт
//! команду при каждом запуске приложения, и холодную надбавку первой
//! загрузки 77-МиБ бинарника платит эта проверка, а не первый разбор ролика
//! внутри его таймаута. deno запускается с окружением [`DenoEnv`]: без
//! проверки обновлений (сети проверка версии не касается) и с кэшем в
//! каталоге данных приложения, а не в домашнем каталоге пользователя.

use std::ffi::OsStr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager, State};

use crate::clock::now_iso8601;
use crate::sidecar::{self, stderr_tail, ChildRegistry, DenoEnv, DenoLaunch, SidecarError};
use crate::types::{LaunchFailedReason, SidecarCheckReport, SidecarCheckResult, SidecarStatus};
use crate::ytdlp::{self, InUse, InUseGuard, Session, WarmLaunch};

/// Верхняя граница времени служебного экрана по Н-2: версия должна
/// появиться не позже, чем через 10 секунд после старта. Обе проверки идут
/// параллельно (`tokio::join!`), поэтому экран ждёт `max` таймаутов, а не их
/// сумму — значит ни один отдельный таймаут не может быть больше этого
/// бюджета, иначе требование нарушается самой конструкцией, независимо от
/// того, как быстро работают бинарники.
///
/// Единица измерения — секунды; используется только внутри этого модуля —
/// для вывода [`CHECK_TIMEOUT_SECS`] и для `const`-проверки, сторожащей
/// это соотношение на этапе компиляции.
const SERVICE_SCREEN_BUDGET_SECS: u64 = 10;

/// Запас бюджета Н-2, который не отдаётся под ожидание процесса: время на
/// invoke-round-trip, сериализацию отчёта и отрисовку экрана. Одна секунда
/// на порядок больше фактического round-trip (`invoke` на локальном IPC —
/// единицы миллисекунд) и покрывает медленный первый рендер WebView.
const SERVICE_SCREEN_RESERVE_SECS: u64 = 1;

/// Таймаут одной проверки: бюджет Н-2 минус запас. Оба sidecar-бинарника
/// проверяются с одним и тем же значением — экран всё равно ждёт максимум
/// из двух параллельных проверок, поэтому индивидуально более короткий
/// таймаут ничего не экономит, а только повышает шанс ложного «не
/// отвечает» на машине медленнее той, на которой снимались замеры.
const CHECK_TIMEOUT_SECS: u64 = SERVICE_SCREEN_BUDGET_SECS - SERVICE_SCREEN_RESERVE_SECS;

// Сторожит калибровку К-4 на этапе компиляции: таймаут отдельной проверки
// не может ни выродиться в ноль (мгновенный таймаут), ни превысить бюджет
// служебного экрана — второе нарушало бы Н-2 самой конструкцией, ещё до
// того, как что-то запустится.
const _: () = assert!(CHECK_TIMEOUT_SECS > 0 && CHECK_TIMEOUT_SECS <= SERVICE_SCREEN_BUDGET_SECS);

/// Таймаут проверки yt-dlp. Калибровка К-4 по фактическим замерам TL-12
/// (Apple Silicon, macOS 26.6, APFS/NVMe; `/usr/bin/time -p`, `real`).
///
/// Служебный экран видит yt-dlp **после** подготовки
/// (`prepare_ytdlp`, см. [`crate::ytdlp`]), то есть уже распакованное и
/// прогретое onedir-дерево в каталоге данных:
///
/// | состояние дерева                                      | `--version`  |
/// |-------------------------------------------------------|--------------|
/// | прогретое, 11 запусков подряд                          | 0,30–0,35 с  |
/// | прогретое, сразу после detach/attach тома              | 1,27 с       |
/// | холодное — на экран не попадает, его берёт подготовка   | 24,6–36,4 с  |
///
/// Прежняя однофайловая поставка платила 25–39 с на **каждом** запуске:
/// бутлоадер PyInstaller распаковывал 130 файлов в новый `$TMPDIR/_MEI…`,
/// а macOS берёт ~0,42 с за первую загрузку каждого только что созданного
/// Mach-O (регистрация подписи `dyld4::Loader::mapSegments` → `fcntl`,
/// обслуживает `syspolicyd`, кэш по inode). Именно поэтому поставка
/// сменилась на onedir в каталоге данных — таймаут эту цену не лечил и не
/// мог вылечить.
///
/// Значение — [`CHECK_TIMEOUT_SECS`], то есть бюджет Н-2 минус запас, а не
/// «замер плюс коэффициент». К худшему измеренному тёплому запуску
/// (1,27 с) это запас в семь раз. Брать меньше нечего: экран всё равно
/// ждёт максимум из двух параллельных проверок, и более короткий таймаут
/// у yt-dlp не ускорил бы экран ни на миллисекунду, а на машине медленнее
/// эталонной добавил бы ложное «не отвечает».
const YT_DLP_TIMEOUT: Duration = Duration::from_secs(CHECK_TIMEOUT_SECS);

// Строку yt-dlp на обычном старте собирает запуск пробы подготовки, а не
// свой (TL-23). Подменять один запуск другим честно, только пока проба не
// дольше проверки: проба, признавшая дерево тёплым, уложилась в
// `PROBE_TIMEOUT`, значит и собственный запуск экрана уложился бы в свой
// таймаут. Разойдись они — экран показал бы `ok` там, где его запуск
// ответил бы «не отвечает».
const _: () = assert!(ytdlp::PROBE_TIMEOUT.as_secs() < YT_DLP_TIMEOUT.as_secs());

/// Таймаут проверки ffmpeg. Калибровка К-4 по фактическим замерам TL-12 на
/// нативной arm64-сборке, поставленной в TL-11 (единый файл 66 МиБ):
/// холодный запуск (файл только что создан, inode новый) 1,58–2,52 с,
/// повторные 0,03–0,08 с. Природа холодной надбавки та же, что у yt-dlp,
/// но платится один раз на файл, а не на каждый запуск.
///
/// Значение — то же [`CHECK_TIMEOUT_SECS`], что и у [`YT_DLP_TIMEOUT`]
/// (обоснование единой величины — там же); к худшему измеренному
/// холодному запуску это запас 3,6×. Прежние 5 с тоже покрывали замер, но
/// были взяты из дизайна, а не из него.
const FFMPEG_TIMEOUT: Duration = Duration::from_secs(CHECK_TIMEOUT_SECS);

/// Таймаут проверки deno (TL-110). Замер на вложенном бинарнике пина
/// 2.9.6 (Apple Silicon, `/usr/bin/time -p`, `real`, окружение
/// [`DenoEnv`]): первый запуск только что доставленного файла 1,74 с,
/// копия с новым inode 1,51 с, повторный 0,01 с. Природа надбавки та же,
/// что у ffmpeg: платится один раз на файл.
///
/// Худший замер — не одиночный запуск, а тот, что бывает на экране:
/// холодный старт deno параллельно с холодным ffmpeg (проверки идут
/// `tokio::join!`), **3,07 с**. В `.app` холодный старт не мерился
/// (исследование TL-107, B4); значение — то же [`CHECK_TIMEOUT_SECS`],
/// обоснование единой величины — у [`YT_DLP_TIMEOUT`]. К худшему замеру
/// это запас ≈3× (9 с / 3,07 с), а не 5×, как считалось по одиночному.
const DENO_TIMEOUT: Duration = Duration::from_secs(CHECK_TIMEOUT_SECS);

/// Предел длины `versionRaw` в Unicode-символах, не считая `…` обрезки
/// (TL-15).
///
/// Самая длинная известная настоящая строка — ffmpeg от gyan.dev
/// (`ffmpeg version 9.0.1-essentials_build-www.gyan.dev Copyright (c)
/// 2000-2026 the FFmpeg developers`) — 97 символов, то есть предел вдвое
/// больше неё: любая настоящая строка версии видна целиком. Предел нужен не
/// им, а бинарнику, подменённому под именем sidecar: одна строка его вывода
/// может весить мегабайт, а уходит она через IPC прямо в «Подробнее».
const VERSION_RAW_MAX_CHARS: usize = 200;

/// Символы, которые меняют порядок отрисовки текста, оставаясь невидимыми
/// (Unicode Bidi: ALM, LRM, RLM, встраивания и переопределения
/// U+202A–U+202E, изоляты U+2066–U+2069). `char::is_control` их не ловит:
/// это категория `Cf`, а не `Cc`.
fn is_bidi_formatting(symbol: char) -> bool {
    matches!(
        symbol,
        '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}

/// Заменяет управляющие и bidi-форматирующие символы на U+FFFD.
///
/// Строка приходит из вывода бинарника, подменённого под именем sidecar, и
/// уходит через IPC прямо в текст экрана: ESC-последовательность, NUL или
/// RLO (`deno 2.9.6 \x1b[31m\u{202E}lave\x07\0x` — воспроизведение ревью
/// TL-15) иначе доехали бы до пользователя как есть — RLO разворачивает
/// отрисовку всего, что стоит за ним. Замена, а не удаление: «здесь был
/// непечатаемый символ» — сведение, которое «Подробнее» обязано показать.
fn replace_unprintable(line: &str) -> String {
    line.chars()
        .map(|symbol| {
            if symbol.is_control() || is_bidi_formatting(symbol) {
                char::REPLACEMENT_CHARACTER
            } else {
                symbol
            }
        })
        .collect()
}

/// Строка для `versionRaw` (и для `version`, когда версии в выводе не
/// нашлось): непечатаемые символы заменены ([`replace_unprintable`]), без
/// краевых пробелов и не длиннее [`VERSION_RAW_MAX_CHARS`] символов, при
/// обрезке — с `…` в конце. Режет по символам, а не по байтам: срез UTF-8
/// посередине символа — паника.
fn clip_version_raw(line: &str) -> String {
    let line = replace_unprintable(line);
    let line = line.trim();
    if line.chars().count() <= VERSION_RAW_MAX_CHARS {
        return line.to_string();
    }

    let head: String = line.chars().take(VERSION_RAW_MAX_CHARS).collect();
    format!("{head}…")
}

/// Возвращает результат проверки sidecar-бинарников (yt-dlp, ffmpeg, deno).
///
/// `registry` — реестр PID выполняющихся процессов (TL-10), внедряется
/// Tauri автоматически из состояния, управляемого в `main.rs`
/// (`app.manage(ChildRegistry::new())`); не часть JS-видимого контракта
/// команды — фронтенд по-прежнему вызывает `invoke("check_sidecar")` без
/// аргументов.
///
/// Возвращает `Result`, хотя сама проверка не может завершиться ошибкой на
/// этом уровне (все ошибки уже
/// конвертированы в поля [`SidecarCheckReport`], `Err` здесь никогда не
/// конструируется) — это требование самого Tauri: async-команда,
/// принимающая ссылки (`State<'_, T>`), обязана возвращать `Result`,
/// иначе сгенерированный future не может быть `'static`
/// (`AsyncCommandMustReturnResult`). На JS-стороне это не меняет поведение:
/// промис `invoke("check_sidecar")` всегда резолвится тем же отчётом, что
/// и раньше, и никогда не реджектится.
#[tauri::command]
pub async fn check_sidecar(
    app: AppHandle,
    registry: State<'_, ChildRegistry>,
) -> Result<SidecarCheckReport, ()> {
    // Страж занятости живёт до конца проверки: между резолвом и запуском
    // `--version` уборка контура обновления не должна унести дерево,
    // путь к которому мы только что отдали (Ф-7).
    let (yt_dlp_path, _in_use) = match resolve_ytdlp_path(&app) {
        Ok((path, guard)) => (Ok(path), Some(guard)),
        Err(error) => (Err(error), None),
    };

    let yt_dlp = yt_dlp_input(app.state::<Session>().inner(), yt_dlp_path);

    let deno = DenoLaunch::for_app(&app).inspect_err(|err| {
        eprintln!(
            "deno: запуск невозможен: {}",
            DenoLaunch::failure_reason(err)
        );
    });

    Ok(check_report_with(
        yt_dlp,
        sidecar::resolve_sidecar_path("ffmpeg"),
        deno,
        &registry,
    )
    .await)
}

/// Откуда берётся строка yt-dlp (TL-23).
enum YtDlpCheck {
    /// Запустить бинарник по резолвленному пути — или сообщить, почему пути
    /// нет.
    Launch(Result<PathBuf, SidecarError>),
    /// Собрать строку из запуска, которым проба подготовки уже застала
    /// дерево тёплым.
    Remembered(WarmLaunch),
}

/// Строка yt-dlp на обычном старте берётся из запуска пробы подготовки, а
/// любая следующая проверка запускает бинарник сама.
///
/// Распорядок — у [`Session`]: запуск пробы отдаётся один раз и только
/// проверке того же пути. Отсюда поведение «Повторить проверку»: подготовка
/// отвечает из памяти сеанса, запуск пробы уже отдан, и yt-dlp запускается
/// заново — иначе повтор ничего не проверял бы. ffmpeg и deno проверяются
/// своим запуском всегда.
fn yt_dlp_input(session: &Session, resolved: Result<PathBuf, SidecarError>) -> YtDlpCheck {
    match resolved {
        Ok(path) => match session.take_warm_launch(&path) {
            Some(launch) => YtDlpCheck::Remembered(launch),
            None => YtDlpCheck::Launch(Ok(path)),
        },
        Err(error) => YtDlpCheck::Launch(Err(error)),
    }
}

/// Проверка yt-dlp: аргументы, таймаут и политика вывода. Одна на оба
/// источника строки, поэтому строка из запуска пробы и строка из своего
/// запуска собираются одними и теми же правилами.
fn yt_dlp_version_check() -> VersionCheck<'static> {
    VersionCheck {
        name: "yt-dlp",
        args: &["--version"],
        env: &[],
        timeout: YT_DLP_TIMEOUT,
        parse_version: sidecar::parse_ytdlp_version,
        unrecognized: UnrecognizedOutput::ShowAsIs,
    }
}

/// Путь к yt-dlp — в каталоге данных, а не рядом с приложением (TL-12),
/// вместе со стражем занятости этой установки.
///
/// # Зачем страж и почему он неотделим от пути
///
/// Контур обновления (E6) держит на диске две установки и убирает всё
/// прочее, а переключается на границе задач (Р-2) — значит бывают
/// моменты, когда работающий процесс запущен из установки, которую
/// запись уже не называет ни активной, ни известно-хорошей. Ф-7 и Ф-8
/// требуют безусловного: такую установку уборка не трогает. Держится
/// это отметкой, которую ставит **резолв**: получить путь мимо него
/// нельзя, поэтому «кто получил путь — тот и держит установку» —
/// свойство кода, а не договорённость.
///
/// Вызывающий обязан держать [`InUseGuard`] всё время, пока может
/// запуститься процесс: у команды служебного экрана — на время
/// проверки, у воркера задачи скачивания — на всю задачу вместе с
/// повторами.
///
/// Любая причина, по которой готовой установки нет (подготовка ещё не
/// выполнялась, дерево не сошлось с манифестом, каталог данных
/// недоступен), для служебного экрана означает одно и то же: запускать
/// нечего. Поэтому все они схлопываются в [`SidecarError::NotFound`] —
/// тот же статус, что у отсутствующего sidecar-файла, с той же подсказкой
/// пользователю. Подробную причину знает и показывает экран подготовки
/// (`prepare_ytdlp`), дублировать её здесь незачем.
pub(super) fn resolve_ytdlp_path(
    app: &AppHandle,
) -> Result<(PathBuf, InUseGuard<'_>), SidecarError> {
    let data_dir = app.path().app_data_dir().map_err(|err| {
        eprintln!("yt-dlp: каталог данных приложения не определяется: {err}");
        SidecarError::NotFound
    })?;

    let in_use = app.state::<InUse>().inner();

    ytdlp::installed_executable(&data_dir, in_use).map_err(|err| {
        eprintln!("yt-dlp: готовой установки нет: {err}");
        SidecarError::NotFound
    })
}

/// Собирает отчёт по уже резолвленным (или неуспешно резолвленным) путям —
/// вынесено из [`check_sidecar`] отдельно от резолва, чтобы тесты могли
/// подставлять пути к фикстурным скриптам вместо реальных sidecar-бинарников
/// (см. `crate::sidecar::process` тесты TL-4).
// Боевой путь зовёт `check_report_with` (TL-23); эта форма осталась тестам,
// где строка yt-dlp всегда из своего запуска.
#[cfg(test)]
async fn check_report(
    yt_dlp_path: Result<PathBuf, SidecarError>,
    ffmpeg_path: Result<PathBuf, SidecarError>,
    deno: Result<DenoLaunch, SidecarError>,
    registry: &ChildRegistry,
) -> SidecarCheckReport {
    check_report_with(YtDlpCheck::Launch(yt_dlp_path), ffmpeg_path, deno, registry).await
}

/// [`check_report`], у которого строка yt-dlp может прийти из запуска
/// пробы подготовки ([`YtDlpCheck::Remembered`], TL-23).
async fn check_report_with(
    yt_dlp: YtDlpCheck,
    ffmpeg_path: Result<PathBuf, SidecarError>,
    deno: Result<DenoLaunch, SidecarError>,
    registry: &ChildRegistry,
) -> SidecarCheckReport {
    let (deno_path, deno_env) = match deno {
        Ok(launch) => (Ok(launch.path().to_path_buf()), Some(launch.env().clone())),
        Err(error) => (Err(error), None),
    };
    let deno_vars = deno_env.as_ref().map(DenoEnv::vars);
    let deno_check = VersionCheck {
        name: "deno",
        args: &["--version"],
        env: deno_vars.as_ref().map_or(&[][..], |vars| &vars[..]),
        timeout: DENO_TIMEOUT,
        parse_version: sidecar::parse_deno_version,
        unrecognized: UnrecognizedOutput::Refuse,
    };

    let yt_dlp_check = yt_dlp_version_check();
    let yt_dlp_row = async {
        match yt_dlp {
            YtDlpCheck::Launch(path) => run_check(&yt_dlp_check, path, registry).await,
            YtDlpCheck::Remembered(launch) => completed_run_result(
                &yt_dlp_check,
                launch.executable.display().to_string(),
                launch.checked_at,
                launch.duration_ms,
                launch.output,
            ),
        }
    };

    let (yt_dlp, ffmpeg, deno) = tokio::join!(
        yt_dlp_row,
        check_binary(
            "ffmpeg",
            ffmpeg_path,
            &["-version"],
            FFMPEG_TIMEOUT,
            sidecar::parse_ffmpeg_version,
            registry,
        ),
        run_check(&deno_check, deno_path, registry),
    );

    SidecarCheckReport {
        yt_dlp,
        ffmpeg,
        deno,
    }
}

/// Проверяет один sidecar-бинарник: запускает `program` с аргументом
/// версии `args`, ждёт не дольше `timeout`, разбирает версию `parse_version`
/// и конвертирует итог в [`SidecarCheckResult`].
///
/// `resolved_path` уже несёт исход резолва (`Ok` — путь; `Err` — почему не
/// удалось определить путь, см. `crate::sidecar::resolve_sidecar_path`) —
/// если резолв не удался, дальше запускать нечего, ошибка конвертируется
/// напрямую, а `path` в результате — сам запрошенный `name` (место, где
/// физически не смогли даже начать искать, единственное осмысленное «здесь
/// искали» в этом случае).
///
/// Запускается без добавочного окружения, а вывод без версии показывается
/// как есть — поведение yt-dlp и ffmpeg со времён E1. Проверка с
/// окружением и отказом на нераспознанный вывод — [`run_check`].
async fn check_binary(
    name: &str,
    resolved_path: Result<PathBuf, SidecarError>,
    args: &[&str],
    timeout: Duration,
    parse_version: fn(&str) -> Option<sidecar::SidecarVersion>,
    registry: &ChildRegistry,
) -> SidecarCheckResult {
    let check = VersionCheck {
        name,
        args,
        env: &[],
        timeout,
        parse_version,
        unrecognized: UnrecognizedOutput::ShowAsIs,
    };
    run_check(&check, resolved_path, registry).await
}

/// Что делать, если бинарник завершился успешно, а версии в выводе нет.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnrecognizedOutput {
    /// Показать вывод вместо версии со статусом `ok` — поведение yt-dlp и
    /// ffmpeg со времён E1.
    ShowAsIs,
    /// Отказ [`LaunchFailedReason::UnrecognizedOutput`] с нераспознанным
    /// выводом в `stderrTail` — поведение deno (TL-110; своя причина, а не
    /// `other`, — с TL-109: процесс запустился, и кода ОС у отказа нет). Под именем deno может
    /// оказаться что угодно исполняемое, и `ok` с чужой строкой на экране
    /// прятал бы ровно ту поломку, ради которой строка на экране существует.
    ///
    /// Проверяется только то, что версия **распознана**, а не то, что она
    /// подходит yt-dlp: минимальную версию (у yt-dlp 2026.08.19 — 2.3.0)
    /// экран не сверяет. Слишком старый deno покажется `ok` со своей
    /// версией, а yt-dlp сам пометит его `(unsupported)` и не возьмёт.
    Refuse,
}

/// Описание проверки одного бинарника для [`run_check`].
struct VersionCheck<'a> {
    name: &'a str,
    args: &'a [&'a str],
    /// Добавочное окружение процесса (см. `crate::sidecar::run_with_env`).
    env: &'a [(&'a str, &'a OsStr)],
    timeout: Duration,
    parse_version: fn(&str) -> Option<sidecar::SidecarVersion>,
    unrecognized: UnrecognizedOutput,
}

/// Тело [`check_binary`] — см. её doc; `check` несёт ещё окружение и
/// политику нераспознанного вывода.
async fn run_check(
    check: &VersionCheck<'_>,
    resolved_path: Result<PathBuf, SidecarError>,
    registry: &ChildRegistry,
) -> SidecarCheckResult {
    let name = check.name;
    let checked_at = now_iso8601();
    let started = Instant::now();

    let path = match resolved_path {
        Ok(path) => path,
        Err(error) => {
            let duration_ms = elapsed_ms(started);
            return error_to_result(name, name.to_string(), checked_at, duration_ms, error);
        }
    };
    let path_string = path.display().to_string();

    let run_result: Result<sidecar::RunOutput, SidecarError> =
        sidecar::run_with_env(&path, check.args, check.env, check.timeout, registry).await;
    let duration_ms = elapsed_ms(started);

    match run_result {
        Ok(output) => completed_run_result(check, path_string, checked_at, duration_ms, output),
        Err(error) => error_to_result(name, path_string, checked_at, duration_ms, error),
    }
}

/// Строка отчёта по завершившемуся с кодом 0 запуску — чьему угодно: своему
/// ([`run_check`]) или пробы подготовки ([`YtDlpCheck::Remembered`]). Одна
/// функция на оба, чтобы строки не могли разойтись ни в одном поле.
fn completed_run_result(
    check: &VersionCheck<'_>,
    path_string: String,
    checked_at: String,
    duration_ms: u64,
    output: sidecar::RunOutput,
) -> SidecarCheckResult {
    let name = check.name;
    let (version, version_raw) = match (check.parse_version)(&output.stdout) {
        Some(parsed) => {
            // На экране — нормализованный semver (`version`), полная
            // первая строка вывода уходит в `versionRaw` для
            // «Подробнее» (TL-15): у собранного `.app` stderr не
            // виден, и лог ниже доступен только разработчику.
            if parsed.is_normalized() {
                eprintln!(
                            "sidecar {name}: версия сборки {raw}, на служебном экране показывается {display}",
                            name = name,
                            raw = parsed.raw,
                            display = parsed.display,
                        );
            }
            (parsed.display, clip_version_raw(&parsed.line))
        }
        None => match check.unrecognized {
            // Разобрать нечего: вместо версии показывается первая
            // непустая строка вывода, и `versionRaw` приходит с
            // `version` всегда — это та же строка. Не весь stdout:
            // `version` — заголовок строки экрана, и мегабайт с
            // внутренними `\r\n` от подменённого бинарника уехал бы
            // туда целиком (остаток ревью TL-15).
            UnrecognizedOutput::ShowAsIs => {
                let shown = clip_version_raw(first_non_empty_line(&output.stdout));
                (shown.clone(), shown)
            }
            UnrecognizedOutput::Refuse => {
                eprintln!("{}", unrecognized_output_log_line(name, &output.stdout));
                let error = SidecarError::LaunchFailed {
                    reason: LaunchFailedReason::UnrecognizedOutput,
                    stderr: format!("{}\n{}", output.stdout.trim(), output.stderr.trim()),
                };
                return error_to_result(name, path_string, checked_at, duration_ms, error);
            }
        },
    };

    SidecarCheckResult {
        name: name.to_string(),
        path: path_string,
        status: SidecarStatus::Ok,
        version: Some(version),
        version_raw: Some(version_raw),
        reason: None,
        exit_code: None,
        os_error_code: None,
        stderr_tail: None,
        timeout_ms: None,
        checked_at: Some(checked_at),
        duration_ms: Some(duration_ms),
    }
}

/// Первая строка вывода, в которой есть что-то кроме пробелов; пустая
/// строка — если такой нет.
fn first_non_empty_line(stdout: &str) -> &str {
    stdout
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
}

/// Строка лога для вывода без версии: хвост stdout в пределе
/// [`stderr_tail`], а не весь поток — под именем deno может оказаться
/// бинарник, печатающий мегабайты, и лог приложения им не засоряется.
fn unrecognized_output_log_line(name: &str, stdout: &str) -> String {
    format!(
        "sidecar {name}: версия в выводе не найдена, хвост stdout: {tail:?}",
        tail = stderr_tail(stdout).unwrap_or_default(),
    )
}

/// Конвертирует [`SidecarError`] в [`SidecarCheckResult`] по контракту
/// TL-1: соответствие `status`/`osErrorCode` для каждого варианта
/// зафиксировано в дизайне эпика E1 (см. `epics/E1-karkas-i-sidecar.md`).
fn error_to_result(
    name: &str,
    path: String,
    checked_at: String,
    duration_ms: u64,
    error: SidecarError,
) -> SidecarCheckResult {
    let (status, reason, exit_code, os_error_code, stderr_tail, timeout_ms) = match error {
        SidecarError::NotFound => (
            SidecarStatus::NotFound,
            None,
            None,
            Some("ENOENT".to_string()),
            None,
            None,
        ),
        SidecarError::LaunchFailed { reason, stderr } => {
            let os_error_code = match reason {
                LaunchFailedReason::PermissionDenied => Some("EACCES".to_string()),
                // ENOEXEC — см. doc `crate::sidecar::process`: ядро отказывается
                // выполнить файл неопознанного формата; на Unix это и есть код
                // ошибки, которым он в итоге проявляется (через shell-фолбэк,
                // код выхода 126, классифицированный как `Corrupted`).
                LaunchFailedReason::Corrupted => Some("ENOEXEC".to_string()),
                // Процесс стартовал и завершился успешно — ошибки ОС нет.
                LaunchFailedReason::Other | LaunchFailedReason::UnrecognizedOutput => None,
            };
            (
                SidecarStatus::LaunchFailed,
                Some(reason),
                None,
                os_error_code,
                stderr_tail(&stderr),
                None,
            )
        }
        SidecarError::NonZeroExit { code, stderr } => (
            SidecarStatus::NonZeroExit,
            None,
            Some(code),
            None,
            stderr_tail(&stderr),
            None,
        ),
        SidecarError::Timeout { ms, stderr } => (
            SidecarStatus::Timeout,
            None,
            None,
            None,
            stderr_tail(&stderr),
            Some(ms),
        ),
    };

    SidecarCheckResult {
        name: name.to_string(),
        path,
        status,
        version: None,
        version_raw: None,
        reason,
        exit_code,
        os_error_code,
        stderr_tail,
        timeout_ms,
        checked_at: Some(checked_at),
        duration_ms: Some(duration_ms),
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sidecar::STDERR_TAIL_MAX_CHARS;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use tempfile::tempdir;

    fn write_script(dir: &tempfile::TempDir, name: &str, contents: &str, mode: u32) -> PathBuf {
        let path = dir.path().join(name);
        fs::write(&path, contents).expect("failed to write fixture script");
        fs::set_permissions(&path, fs::Permissions::from_mode(mode))
            .expect("failed to chmod fixture script");
        path
    }

    #[tokio::test]
    async fn converts_a_successful_run_into_ok_status_with_parsed_version() {
        let dir = tempdir().expect("failed to create temp dir");
        let script = write_script(
            &dir,
            "yt-dlp.sh",
            "#!/bin/sh\necho 2026.08.19\nexit 0\n",
            0o755,
        );

        let registry = ChildRegistry::new();
        let result = check_binary(
            "yt-dlp",
            Ok(script.clone()),
            &["--version"],
            Duration::from_secs(5),
            sidecar::parse_ytdlp_version,
            &registry,
        )
        .await;

        assert_eq!(result.name, "yt-dlp");
        assert_eq!(result.path, script.display().to_string());
        assert_eq!(result.status, SidecarStatus::Ok);
        assert_eq!(result.version.as_deref(), Some("2026.08.19"));
        assert!(result.reason.is_none());
        assert!(result.exit_code.is_none());
        assert!(result.os_error_code.is_none());
        assert!(result.stderr_tail.is_none());
        assert!(result.timeout_ms.is_none());
        assert!(result.checked_at.is_some());
        assert!(result.duration_ms.is_some());
    }

    #[tokio::test]
    async fn converts_a_missing_binary_into_not_found_status_with_enoent() {
        let dir = tempdir().expect("failed to create temp dir");
        let missing = dir.path().join("does-not-exist");

        let registry = ChildRegistry::new();
        let result = check_binary(
            "ffmpeg",
            Ok(missing.clone()),
            &["-version"],
            Duration::from_secs(5),
            sidecar::parse_ffmpeg_version,
            &registry,
        )
        .await;

        assert_eq!(result.status, SidecarStatus::NotFound);
        assert_eq!(result.path, missing.display().to_string());
        assert_eq!(result.os_error_code.as_deref(), Some("ENOENT"));
        assert!(result.version.is_none());
    }

    #[tokio::test]
    async fn converts_a_non_executable_binary_into_launch_failed_status_with_eacces() {
        let dir = tempdir().expect("failed to create temp dir");
        let script = write_script(&dir, "not-executable.sh", "#!/bin/sh\nexit 0\n", 0o644);

        let registry = ChildRegistry::new();
        let result = check_binary(
            "yt-dlp",
            Ok(script),
            &["--version"],
            Duration::from_secs(5),
            sidecar::parse_ytdlp_version,
            &registry,
        )
        .await;

        assert_eq!(result.status, SidecarStatus::LaunchFailed);
        assert_eq!(result.reason, Some(LaunchFailedReason::PermissionDenied));
        assert_eq!(result.os_error_code.as_deref(), Some("EACCES"));
        // Отказ на уровне spawn — процесс не стартовал, stderr недостижим.
        assert!(result.stderr_tail.is_none());
    }

    #[tokio::test]
    async fn converts_a_corrupted_binary_into_launch_failed_status_with_enoexec() {
        let dir = tempdir().expect("failed to create temp dir");
        let script = write_script(&dir, "garbage", "not a real executable\x00\x01\x02", 0o755);

        let registry = ChildRegistry::new();
        let result = check_binary(
            "ffmpeg",
            Ok(script),
            &["-version"],
            Duration::from_secs(5),
            sidecar::parse_ffmpeg_version,
            &registry,
        )
        .await;

        assert_eq!(result.status, SidecarStatus::LaunchFailed);
        assert_eq!(result.reason, Some(LaunchFailedReason::Corrupted));
        assert_eq!(result.os_error_code.as_deref(), Some("ENOEXEC"));
    }

    #[tokio::test]
    async fn converts_a_non_zero_exit_into_non_zero_exit_status_with_exit_code_and_stderr_tail() {
        let dir = tempdir().expect("failed to create temp dir");
        let script = write_script(
            &dir,
            "fail.sh",
            "#!/bin/sh\necho 'error: unsupported URL' >&2\nexit 1\n",
            0o755,
        );

        let registry = ChildRegistry::new();
        let result = check_binary(
            "yt-dlp",
            Ok(script),
            &["--version"],
            Duration::from_secs(5),
            sidecar::parse_ytdlp_version,
            &registry,
        )
        .await;

        assert_eq!(result.status, SidecarStatus::NonZeroExit);
        assert_eq!(result.exit_code, Some(1));
        assert_eq!(
            result.stderr_tail.as_deref(),
            Some("error: unsupported URL")
        );
        assert!(result.version.is_none());
    }

    #[tokio::test]
    async fn converts_a_slow_process_into_timeout_status_with_timeout_ms() {
        let dir = tempdir().expect("failed to create temp dir");
        let script = write_script(&dir, "slow.sh", "#!/bin/sh\nsleep 5\n", 0o755);

        let registry = ChildRegistry::new();
        let result = check_binary(
            "ffmpeg",
            Ok(script),
            &["-version"],
            Duration::from_millis(150),
            sidecar::parse_ffmpeg_version,
            &registry,
        )
        .await;

        assert_eq!(result.status, SidecarStatus::Timeout);
        assert_eq!(result.timeout_ms, Some(150));
        assert!(result.version.is_none());
        assert!(
            registry.is_empty(),
            "check_binary must not leave a pid registered after the timeout kill"
        );
    }

    #[tokio::test]
    async fn converts_a_resolve_failure_into_launch_failed_status_using_the_binary_name_as_path() {
        let registry = ChildRegistry::new();
        let result = check_binary(
            "yt-dlp",
            Err(SidecarError::LaunchFailed {
                reason: LaunchFailedReason::Other,
                stderr: String::new(),
            }),
            &["--version"],
            Duration::from_secs(5),
            sidecar::parse_ytdlp_version,
            &registry,
        )
        .await;

        assert_eq!(result.status, SidecarStatus::LaunchFailed);
        assert_eq!(result.reason, Some(LaunchFailedReason::Other));
        assert!(result.os_error_code.is_none());
        // Резолв не дал пути — единственное осмысленное «здесь искали» это
        // само запрошенное имя.
        assert_eq!(result.path, "yt-dlp");
    }

    #[tokio::test]
    async fn truncates_a_long_stderr_tail_to_the_configured_character_limit() {
        let dir = tempdir().expect("failed to create temp dir");
        // Печатает заметно больше STDERR_TAIL_MAX_CHARS символов на stderr.
        let script = write_script(
            &dir,
            "verbose-fail.sh",
            "#!/bin/sh\ni=0\nwhile [ $i -lt 2000 ]; do printf 'x' >&2; i=$((i + 1)); done\nexit 1\n",
            0o755,
        );

        let registry = ChildRegistry::new();
        let result = check_binary(
            "yt-dlp",
            Ok(script),
            &["--version"],
            Duration::from_secs(5),
            sidecar::parse_ytdlp_version,
            &registry,
        )
        .await;

        assert_eq!(result.status, SidecarStatus::NonZeroExit);
        let tail = result.stderr_tail.expect("stderr tail must be present");
        assert_eq!(tail.chars().count(), STDERR_TAIL_MAX_CHARS);
        assert!(tail.chars().all(|c| c == 'x'));
    }

    /// Прогревает фикстурный скрипт синхронным холостым запуском вне
    /// измеряемого окна теста — самый первый запуск нового исполняемого
    /// файла на некоторых машинах/песочницах несёт одноразовый
    /// фиксированный оверхед (например, проверка Gatekeeper на macOS для
    /// файла, который ещё не запускался). Второй и последующие запуски
    /// того же файла эту надбавку уже не платят.
    ///
    /// В тесте параллельности этот оверхед достался бы бюджету рандеву
    /// ([`RENDEZVOUS_POLL_LIMIT`]): первая фикстура ждёт, пока стартует
    /// вторая, а стартовать та начинает именно с этой надбавки. Прогрев
    /// выносит её за пределы ожидания. Одиночный запуск в прогреве
    /// никого не ждёт: рандеву вооружается файлом `armed` уже после
    /// прогрева, см. [`rendezvous_script`].
    async fn warm_up(script: &std::path::Path) {
        let registry = ChildRegistry::new();
        let _ = check_binary(
            "warm-up",
            Ok(script.to_path_buf()),
            &["--version"],
            Duration::from_secs(20),
            sidecar::parse_ytdlp_version,
            &registry,
        )
        .await;
    }

    /// Шаг опроса рандеву, в секундах — подставляется в текст фикстуры
    /// аргументом `sleep`, поэтому строка, а не [`Duration`].
    const RENDEZVOUS_POLL_STEP_SECS: &str = "0.1";

    /// Предел опросов рандеву: 50 шагов по 100 мс, то есть не меньше
    /// 5 секунд ожидания второй фикстуры. Тот же бюджет и по той же
    /// причине, что у ожидания маркера грандчайлда в тестах
    /// `crate::sidecar::registry`: набор тестов идёт параллельно, и запуск
    /// процесса на занятой машине стоит заметно дороже, чем в покое.
    ///
    /// Пять секунд — не «щедрее прежних 350 мс», а величина другого рода.
    /// Прежний порог сравнивал ~200 мс работы фикстуры с 350 мс
    /// допустимыми: запас 1,75×, который занятый раннер и съел (измерено
    /// 359 мс, прогон 32944919394). Здесь ожидание не покрывает работу
    /// процессов вовсе — только промежуток между стартом первого и стартом
    /// второго. `run_cancellable` доходит до `spawn` без единой точки
    /// `await`, поэтому в этот промежуток попадает лишь один опрос второго
    /// future плюс `fork`/`exec` уже прогретого файла (прогрев —
    /// [`warm_up`]). Замер на этой машине: в покое расходуется 0 опросов
    /// из 50, под нагрузкой в 20 занятых потоков на 8 ядрах (load average
    /// 11,6–16,4) — 0–1 опрос, то есть промежуток укладывается в один шаг
    /// и при шаге 20 мс тоже. Запас к бюджету — не меньше 50×, и он не
    /// сжимается на медленной машине: медленная машина дольше выполняет
    /// фикстуры, а ждут здесь не выполнения.
    ///
    /// Верхняя граница выбрана из внятности красного прогона: исчерпанный
    /// бюджет (замер — 6,6 с на весь тест: 5 с сна плюс 50 форков `sleep`
    /// и прогрев) меньше [`CHECK_TIMEOUT_SECS`] (9 с), с которым
    /// `check_binary` запускает фикстуру, поэтому первой срабатывает
    /// выдержка самого скрипта и в отчёт попадает `nonZeroExit` с текстом
    /// причины, а не безымянный `timeout`. Если на совсем медленной машине
    /// бюджет всё же перевалит за 9 секунд, тест останется красным —
    /// сменится только текст диагностики.
    const RENDEZVOUS_POLL_LIMIT: u32 = 50;

    /// Текст фикстуры «рандеву»: скрипт отмечается файлом
    /// `<own>.started` в `dir` и не завершается успехом, пока рядом не
    /// появится `<peer>.started`.
    ///
    /// Так факт параллельности проверяется наблюдаемым порядком событий, а
    /// не измерением: успех обеих фикстур возможен только если оба
    /// процесса были живы одновременно, и невозможен, если второй
    /// запускается после завершения первого. Никакого утверждения о
    /// длительности при этом не делается.
    ///
    /// Пока в `dir` нет файла `armed`, скрипт печатает версию и выходит,
    /// не ожидая никого, — этот режим нужен прогреву ([`warm_up`]), где
    /// фикстура запускается в одиночку.
    fn rendezvous_script(
        dir: &std::path::Path,
        own: &str,
        peer: &str,
        version_line: &str,
    ) -> String {
        let dir = dir.display();
        format!(
            "#!/bin/sh\n\
             if [ ! -f \"{dir}/armed\" ]; then\n\
             \x20 echo \"{version_line}\"\n\
             \x20 exit 0\n\
             fi\n\
             : > \"{dir}/{own}.started\"\n\
             waited=0\n\
             while [ ! -f \"{dir}/{peer}.started\" ]; do\n\
             \x20 if [ \"$waited\" -ge {limit} ]; then\n\
             \x20   echo 'rendezvous timed out: {peer} never started' >&2\n\
             \x20   exit 1\n\
             \x20 fi\n\
             \x20 waited=$((waited + 1))\n\
             \x20 sleep {step}\n\
             done\n\
             echo \"{version_line}\"\n\
             exit 0\n",
            limit = RENDEZVOUS_POLL_LIMIT,
            step = RENDEZVOUS_POLL_STEP_SECS,
        )
    }

    /// Первая строка настоящего `deno --version` пина 2.9.6 (проверено
    /// живьём, см. тест разбора в `crate::sidecar::version`).
    const DENO_VERSION_LINE: &str = "deno 2.9.6 (stable, release, aarch64-apple-darwin)";

    const YT_DLP_VERSION_LINE: &str = "2026.08.19";

    const FFMPEG_VERSION_LINE: &str =
        "ffmpeg version 9.0.1 Copyright (c) 2000-2026 the FFmpeg developers";

    /// Запуск deno из фикстурного скрипта с окружением, построенным от
    /// `data_dir`, — то, что в продакшене собирает `DenoLaunch::for_app`.
    fn deno_launch(path: PathBuf, data_dir: &std::path::Path) -> Result<DenoLaunch, SidecarError> {
        Ok(DenoLaunch::new(path, data_dir))
    }

    // ────────────── один запуск yt-dlp на тёплом старте (TL-23) ──────────────

    use crate::ytdlp::testing::{Control, SilentSink, EXECUTABLE};
    use crate::ytdlp::Session;

    /// Каталог данных, в котором «прошлый запуск приложения» уже распаковал
    /// и прогрел yt-dlp, считающий свои запуски.
    struct WarmStart {
        _dir: tempfile::TempDir,
        archive: PathBuf,
        data_dir: PathBuf,
        control: Control,
        registry: ChildRegistry,
        in_use: InUse,
    }

    async fn warm_start() -> WarmStart {
        let dir = tempdir().expect("failed to create temp dir");
        let control = Control::new(&dir.path().join("control"));
        let archive = dir.path().join("yt-dlp.zip");
        crate::ytdlp::testing::write_onedir_zip(
            &archive,
            EXECUTABLE,
            &control.script(YT_DLP_VERSION_LINE),
        );
        let data_dir = dir.path().join("app-data");
        let registry = ChildRegistry::new();

        Session::new()
            .prepare(&archive, &data_dir, &registry, &SilentSink)
            .await
            .expect("первый запуск приложения обязан подготовить yt-dlp");

        WarmStart {
            _dir: dir,
            archive,
            data_dir,
            control,
            registry,
            in_use: InUse::new(),
        }
    }

    impl WarmStart {
        /// Одна дверь в подготовку — то, что делает `prepare_now` под
        /// мьютексом.
        async fn prepare(&self, session: &Session) -> crate::types::YtDlpPrepared {
            let (prepared, background) = session
                .prepare(&self.archive, &self.data_dir, &self.registry, &SilentSink)
                .await
                .expect("подготовка обязана пройти");
            assert!(
                background.is_none(),
                "у тёплого дерева фонового прогрева нет"
            );
            prepared
        }

        /// Строка yt-dlp так, как её собирает `check_sidecar`: резолв,
        /// память сеанса, отчёт. ffmpeg и deno не запускаются — предмет
        /// здесь yt-dlp.
        async fn check(&self, session: &Session) -> SidecarCheckResult {
            let (path, _guard) = ytdlp::installed_executable(&self.data_dir, &self.in_use)
                .expect("путь к установке обязан находиться");
            check_report_with(
                yt_dlp_input(session, Ok(path)),
                Err(SidecarError::NotFound),
                Err(SidecarError::NotFound),
                &self.registry,
            )
            .await
            .yt_dlp
        }
    }

    /// Результат так, как его видит фронтенд, без полей времени: они
    /// принадлежат конкретному запуску и совпасть у двух запусков не
    /// обязаны.
    fn without_timing(result: &SidecarCheckResult) -> serde_json::Value {
        let mut value = serde_json::to_value(result).expect("результат сериализуется");
        let object = value.as_object_mut().expect("результат — объект");
        assert!(object.remove("checkedAt").is_some(), "{object:?}");
        assert!(object.remove("durationMs").is_some(), "{object:?}");
        value
    }

    #[tokio::test]
    async fn a_warm_start_launches_yt_dlp_once_and_reports_what_a_launch_would() {
        let start = warm_start().await;
        let before = start.control.launches();
        // Новый процесс приложения — новый сеанс.
        let session = Session::new();

        let setup = start.prepare(&session).await;
        let frontend = start.prepare(&session).await;
        let remembered = start.check(&session).await;

        assert_eq!(
            start.control.launches() - before,
            1,
            "тёплый старт — один запуск yt-dlp на обе двери подготовки и проверку экрана"
        );
        assert!(!setup.prepared && !frontend.prepared);

        // Тот же путь, проверенный отдельным запуском, — эталон строки.
        let (path, _guard) =
            ytdlp::installed_executable(&start.data_dir, &start.in_use).expect("путь");
        let launched = run_check(&yt_dlp_version_check(), Ok(path), &start.registry).await;
        assert_eq!(start.control.launches() - before, 2);

        assert_eq!(
            without_timing(&remembered),
            without_timing(&launched),
            "строка из запуска пробы обязана совпадать с отдельным запуском во всех полях"
        );
        assert_eq!(remembered.status, SidecarStatus::Ok);
        assert_eq!(remembered.version.as_deref(), Some(YT_DLP_VERSION_LINE));
        assert_eq!(remembered.version_raw.as_deref(), Some(YT_DLP_VERSION_LINE));
        assert_eq!(remembered.path, setup.path);
        assert!(remembered.checked_at.is_some() && remembered.duration_ms.is_some());
    }

    #[tokio::test]
    async fn repeating_the_check_launches_yt_dlp_again() {
        let start = warm_start().await;
        let session = Session::new();
        start.prepare(&session).await;
        start.check(&session).await;
        let before = start.control.launches();

        // «Повторить проверку»: фронтенд зовёт подготовку и проверку снова.
        start.prepare(&session).await;
        let again = start.check(&session).await;

        assert_eq!(
            start.control.launches() - before,
            1,
            "повтор обязан запустить yt-dlp — иначе он ничего не проверяет"
        );
        assert_eq!(again.status, SidecarStatus::Ok);
    }

    #[tokio::test]
    async fn after_the_session_is_forgotten_the_check_launches_yt_dlp_itself() {
        // Сброс делает контур обновления (установка, переключение, откат);
        // сам сброс в оркестрации проверен в `crate::ytdlp::orchestrate`.
        let start = warm_start().await;
        let session = Session::new();
        start.prepare(&session).await;
        let before = start.control.launches();

        session.invalidate();
        let row = start.check(&session).await;

        assert_eq!(
            start.control.launches() - before,
            1,
            "после сброса строка yt-dlp — только из своего запуска"
        );
        assert_eq!(row.status, SidecarStatus::Ok);
    }

    #[test]
    fn the_log_of_unrecognized_output_is_bounded_by_the_tail_limit() {
        // Остаток ревью TL-110: в лог уходил весь stdout.
        let stdout = "x".repeat(STDERR_TAIL_MAX_CHARS * 5);

        let line = unrecognized_output_log_line("deno", &stdout);

        assert!(line.starts_with("sidecar deno: "), "{line}");
        assert_eq!(
            line.chars().filter(|symbol| *symbol == 'x').count(),
            STDERR_TAIL_MAX_CHARS,
            "в лог уходит хвост в пределе, а не весь поток"
        );
    }

    /// Скрипт, который печатает строку версии и выходит успешно.
    fn version_script(dir: &tempfile::TempDir, name: &str, line: &str) -> PathBuf {
        write_script(
            dir,
            name,
            &format!("#!/bin/sh\necho \"{line}\"\nexit 0\n"),
            0o755,
        )
    }

    /// Отчёт, где yt-dlp и ffmpeg в порядке, а deno — то, что дали.
    async fn report_with_deno(
        dir: &tempfile::TempDir,
        deno: Result<DenoLaunch, SidecarError>,
    ) -> SidecarCheckReport {
        let yt_dlp = version_script(dir, "yt-dlp.sh", YT_DLP_VERSION_LINE);
        let ffmpeg = version_script(dir, "ffmpeg.sh", FFMPEG_VERSION_LINE);
        let registry = ChildRegistry::new();
        let report = check_report(Ok(yt_dlp), Ok(ffmpeg), deno, &registry).await;
        assert!(
            registry.is_empty(),
            "проверка не должна оставлять pid в реестре"
        );
        report
    }

    #[tokio::test]
    async fn reports_the_deno_version_next_to_yt_dlp_and_ffmpeg() {
        let dir = tempdir().expect("failed to create temp dir");
        // Весь трёхстрочный вывод, а не одна строка: разбор обязан взять
        // версию deno, а не v8 или typescript.
        let deno = write_script(
            &dir,
            "deno.sh",
            &format!(
                "#!/bin/sh\necho \"{DENO_VERSION_LINE}\"\necho 'v8 15.0.245.2-rusty'\n\
                 echo 'typescript 6.0.3'\nexit 0\n"
            ),
            0o755,
        );

        let report = report_with_deno(&dir, deno_launch(deno.clone(), dir.path())).await;

        assert_eq!(report.deno.name, "deno");
        assert_eq!(report.deno.path, deno.display().to_string());
        assert_eq!(report.deno.status, SidecarStatus::Ok);
        assert_eq!(report.deno.version.as_deref(), Some("2.9.6"));
        assert!(report.deno.reason.is_none());
        assert!(report.deno.stderr_tail.is_none());
        // Соседние строки не пострадали от третьей проверки.
        assert_eq!(report.yt_dlp.version.as_deref(), Some("2026.08.19"));
        assert_eq!(report.ffmpeg.version.as_deref(), Some("9.0.1"));
    }

    #[tokio::test]
    async fn a_missing_deno_is_a_typed_not_found() {
        let dir = tempdir().expect("failed to create temp dir");
        let missing = dir.path().join("deno-does-not-exist");

        let report = report_with_deno(&dir, deno_launch(missing.clone(), dir.path())).await;

        assert_eq!(report.deno.status, SidecarStatus::NotFound);
        assert_eq!(report.deno.os_error_code.as_deref(), Some("ENOENT"));
        assert_eq!(report.deno.path, missing.display().to_string());
        assert!(report.deno.version.is_none());
    }

    #[tokio::test]
    async fn a_non_executable_deno_is_a_typed_permission_denied() {
        let dir = tempdir().expect("failed to create temp dir");
        let deno = write_script(
            &dir,
            "deno.sh",
            &format!("#!/bin/sh\necho \"{DENO_VERSION_LINE}\"\n"),
            0o644,
        );

        let report = report_with_deno(&dir, deno_launch(deno, dir.path())).await;

        assert_eq!(report.deno.status, SidecarStatus::LaunchFailed);
        assert_eq!(
            report.deno.reason,
            Some(LaunchFailedReason::PermissionDenied)
        );
        assert_eq!(report.deno.os_error_code.as_deref(), Some("EACCES"));
        assert!(report.deno.version.is_none());
    }

    #[tokio::test]
    async fn the_ci_stub_under_the_deno_name_is_a_typed_non_zero_exit() {
        // Ровно то, что кладёт `scripts/ci/stub-binaries.mjs`: текст без
        // shebang с правом на исполнение.
        //
        // ИЗМЕРЕНО (TL-110), а не предположено: в тексте заглушки нет
        // NUL-байтов, поэтому exec откатывается на `/bin/sh`, тот читает
        // файл как скрипт и падает на скобке с кодом 2 — это `nonZeroExit`,
        // а не `corrupted` (код 126 даёт только файл с NUL-байтами, см.
        // `converts_a_corrupted_binary_into_launch_failed_status_with_enoexec`).
        let dir = tempdir().expect("failed to create temp dir");
        let deno = write_script(
            &dir,
            "deno",
            "tube-leak CI stub, not a real binary (scripts/ci/stub-binaries.mjs)\n",
            0o755,
        );

        let report = report_with_deno(&dir, deno_launch(deno, dir.path())).await;

        assert_eq!(report.deno.status, SidecarStatus::NonZeroExit);
        assert_eq!(report.deno.exit_code, Some(2));
        assert!(report.deno.version.is_none());
        assert!(
            report
                .deno
                .stderr_tail
                .as_deref()
                .is_some_and(|tail| tail.contains("syntax error")),
            "причина обязана доехать до «Подробнее»: {:?}",
            report.deno.stderr_tail
        );
    }

    #[tokio::test]
    async fn deno_output_without_a_version_is_a_typed_launch_failure_not_ok() {
        let dir = tempdir().expect("failed to create temp dir");
        // Исполняемое, отвечает кодом 0, но это не deno: у yt-dlp и ffmpeg
        // такой вывод ушёл бы на экран как «версия» со статусом ok.
        let deno = write_script(
            &dir,
            "deno.sh",
            "#!/bin/sh\necho 'hello from something else'\nexit 0\n",
            0o755,
        );

        let report = report_with_deno(&dir, deno_launch(deno, dir.path())).await;

        assert_eq!(report.deno.status, SidecarStatus::LaunchFailed);
        assert_eq!(
            report.deno.reason,
            Some(LaunchFailedReason::UnrecognizedOutput),
            "процесс запустился — это не `other` («не запустился»), а нераспознанный вывод"
        );
        assert!(report.deno.os_error_code.is_none());
        assert!(report.deno.version.is_none());
        assert_eq!(
            report.deno.stderr_tail.as_deref(),
            Some("hello from something else"),
            "нераспознанный вывод обязан доехать до «Подробнее»"
        );
    }

    #[test]
    fn the_unrecognized_output_reason_crosses_the_boundary_as_camel_case() {
        assert_eq!(
            serde_json::to_value(LaunchFailedReason::UnrecognizedOutput)
                .expect("причина сериализуется"),
            serde_json::json!("unrecognizedOutput")
        );
    }

    #[tokio::test]
    async fn a_deno_resolve_failure_names_the_binary_and_carries_the_reason() {
        let dir = tempdir().expect("failed to create temp dir");

        let report = report_with_deno(
            &dir,
            Err(SidecarError::LaunchFailed {
                reason: LaunchFailedReason::Other,
                stderr: "каталог данных приложения не определяется: test".to_string(),
            }),
        )
        .await;

        assert_eq!(report.deno.status, SidecarStatus::LaunchFailed);
        assert_eq!(report.deno.path, "deno");
        assert_eq!(
            report.deno.stderr_tail.as_deref(),
            Some("каталог данных приложения не определяется: test")
        );
    }

    #[tokio::test]
    async fn deno_is_checked_without_update_check_and_with_its_cache_in_the_data_dir() {
        // Скрипт-заглушка deno записывает своё окружение в файл и печатает
        // версию. Файл, а не stdout: из вывода в отчёт уходит только
        // версия.
        let dir = tempdir().expect("failed to create temp dir");
        let data_dir = dir.path().join("app-data");
        let seen = dir.path().join("deno-env.txt");
        let deno = write_script(
            &dir,
            "deno.sh",
            &format!(
                "#!/bin/sh\n\
                 printf 'DENO_NO_UPDATE_CHECK=%s\\nDENO_DIR=%s\\n' \
                 \"$DENO_NO_UPDATE_CHECK\" \"$DENO_DIR\" > \"{seen}\"\n\
                 echo \"{DENO_VERSION_LINE}\"\n\
                 exit 0\n",
                seen = seen.display(),
            ),
            0o755,
        );

        let report = report_with_deno(&dir, deno_launch(deno, &data_dir)).await;
        assert_eq!(report.deno.status, SidecarStatus::Ok);

        let recorded = fs::read_to_string(&seen).expect("deno stub must record its environment");
        let lines: Vec<&str> = recorded.lines().collect();
        assert_eq!(lines.len(), 2, "unexpected record: {recorded:?}");
        assert_eq!(lines[0], "DENO_NO_UPDATE_CHECK=1");

        let deno_dir = std::path::Path::new(
            lines[1]
                .strip_prefix("DENO_DIR=")
                .expect("second line is DENO_DIR"),
        );
        assert!(
            deno_dir.starts_with(&data_dir) && deno_dir != data_dir,
            "DENO_DIR обязан быть подкаталогом каталога данных {data_dir:?}, а не {deno_dir:?}"
        );
    }

    #[tokio::test]
    async fn yt_dlp_and_ffmpeg_are_not_given_the_deno_environment() {
        // Окружение deno добавляется только его проверке: `--version`
        // yt-dlp до экстрактора не доходит и deno не запускает (окружение
        // deno yt-dlp получает в запусках разбора и скачивания, TL-109), а
        // ffmpeg оно ни к чему.
        let dir = tempdir().expect("failed to create temp dir");
        let seen = dir.path().join("yt-dlp-env.txt");
        let yt_dlp = write_script(
            &dir,
            "yt-dlp.sh",
            &format!(
                "#!/bin/sh\nprintf '%s' \"$DENO_DIR\" > \"{seen}\"\necho {YT_DLP_VERSION_LINE}\nexit 0\n",
                seen = seen.display(),
            ),
            0o755,
        );
        let ffmpeg = version_script(&dir, "ffmpeg.sh", FFMPEG_VERSION_LINE);
        let deno = version_script(&dir, "deno.sh", DENO_VERSION_LINE);
        let data_dir = dir.path().join("app-data");

        let registry = ChildRegistry::new();
        let report = check_report(
            Ok(yt_dlp),
            Ok(ffmpeg),
            deno_launch(deno, &data_dir),
            &registry,
        )
        .await;
        assert_eq!(report.yt_dlp.status, SidecarStatus::Ok);

        let recorded = fs::read_to_string(&seen).expect("yt-dlp stub must record DENO_DIR");
        assert!(
            !recorded.contains(&data_dir.display().to_string()),
            "yt-dlp получил DENO_DIR проверки deno: {recorded:?}"
        );
    }

    #[tokio::test]
    async fn checks_all_three_sidecars_concurrently_not_sequentially() {
        let dir = tempdir().expect("failed to create temp dir");
        // Рандеву по кругу: yt-dlp ждёт ffmpeg, ffmpeg ждёт deno, deno ждёт
        // yt-dlp. Успех всех трёх возможен, только если все три процесса
        // были живы одновременно: при любом порядке, где хоть одна
        // проверка идёт после завершения другой, первая из них ждёт
        // соседа, которого ещё не запускали.
        let yt_dlp_script = write_script(
            &dir,
            "rendezvous-yt-dlp.sh",
            &rendezvous_script(dir.path(), "yt-dlp", "ffmpeg", YT_DLP_VERSION_LINE),
            0o755,
        );
        let ffmpeg_script = write_script(
            &dir,
            "rendezvous-ffmpeg.sh",
            &rendezvous_script(dir.path(), "ffmpeg", "deno", FFMPEG_VERSION_LINE),
            0o755,
        );
        let deno_script = write_script(
            &dir,
            "rendezvous-deno.sh",
            &rendezvous_script(dir.path(), "deno", "yt-dlp", DENO_VERSION_LINE),
            0o755,
        );

        warm_up(&yt_dlp_script).await;
        warm_up(&ffmpeg_script).await;
        warm_up(&deno_script).await;

        // Вооружает рандеву: до этой строки фикстуры отрабатывали в
        // одиночном режиме (прогрев), после — каждая завершится успехом
        // только увидев маркер соседа.
        fs::write(dir.path().join("armed"), "").expect("failed to arm the rendezvous");

        let registry = ChildRegistry::new();
        let report = check_report(
            Ok(yt_dlp_script),
            Ok(ffmpeg_script),
            deno_launch(deno_script, dir.path()),
            &registry,
        )
        .await;

        for (name, result) in [
            ("yt-dlp", &report.yt_dlp),
            ("ffmpeg", &report.ffmpeg),
            ("deno", &report.deno),
        ] {
            assert_eq!(
                result.status,
                SidecarStatus::Ok,
                "рандеву не состоялось со стороны {name}: проверки идут не параллельно; stderr: {:?}",
                result.stderr_tail
            );
        }
    }

    /// `versionRaw` так, как его видит фронтенд: `None` — ключа в JSON нет.
    fn serialized_version_raw(result: &SidecarCheckResult) -> Option<serde_json::Value> {
        let value = serde_json::to_value(result).expect("результат сериализуется");
        value
            .as_object()
            .expect("результат — объект")
            .get("versionRaw")
            .cloned()
    }

    /// Живой вывод `ffmpeg -version` сборки martin-riedl.de — та же строка,
    /// что в тестах разбора `crate::sidecar::version`.
    const FFMPEG_MARTIN_RIEDL_FIRST_LINE: &str =
        "ffmpeg version 9.0.1-https://www.martin-riedl.de Copyright (c) 2000-2026 the FFmpeg developers";

    #[tokio::test]
    async fn version_raw_of_each_sidecar_is_its_whole_first_version_line() {
        let dir = tempdir().expect("failed to create temp dir");
        let yt_dlp = version_script(&dir, "yt-dlp.sh", YT_DLP_VERSION_LINE);
        let ffmpeg = write_script(
            &dir,
            "ffmpeg.sh",
            &format!(
                "#!/bin/sh\necho '{FFMPEG_MARTIN_RIEDL_FIRST_LINE}'\n\
                 echo 'built with Apple clang version 14.0.0 (clang-1400.0.29.102)'\nexit 0\n"
            ),
            0o755,
        );
        let deno = write_script(
            &dir,
            "deno.sh",
            &format!(
                "#!/bin/sh\necho \"{DENO_VERSION_LINE}\"\necho 'v8 15.0.245.2-rusty'\n\
                 echo 'typescript 6.0.3'\nexit 0\n"
            ),
            0o755,
        );

        let registry = ChildRegistry::new();
        let report = check_report(
            Ok(yt_dlp),
            Ok(ffmpeg),
            deno_launch(deno, dir.path()),
            &registry,
        )
        .await;

        for (result, display, raw) in [
            (&report.yt_dlp, "2026.08.19", YT_DLP_VERSION_LINE),
            (&report.ffmpeg, "9.0.1", FFMPEG_MARTIN_RIEDL_FIRST_LINE),
            (
                &report.deno,
                "2.9.6",
                "deno 2.9.6 (stable, release, aarch64-apple-darwin)",
            ),
        ] {
            assert_eq!(result.status, SidecarStatus::Ok, "{result:?}");
            // Экран не меняется: `version` по-прежнему нормализованная.
            assert_eq!(result.version.as_deref(), Some(display), "{result:?}");
            assert_eq!(result.version_raw.as_deref(), Some(raw), "{result:?}");
            assert_eq!(
                serialized_version_raw(result),
                Some(serde_json::json!(raw)),
                "{}: versionRaw обязан пересечь границу",
                result.name
            );
        }
    }

    #[tokio::test]
    async fn version_raw_is_absent_from_every_failed_check() {
        let dir = tempdir().expect("failed to create temp dir");
        let registry = ChildRegistry::new();
        let failing = write_script(
            &dir,
            "fail.sh",
            &format!("#!/bin/sh\necho \"{YT_DLP_VERSION_LINE}\"\necho boom >&2\nexit 1\n"),
            0o755,
        );
        let slow = write_script(
            &dir,
            "slow.sh",
            &format!("#!/bin/sh\necho \"{YT_DLP_VERSION_LINE}\"\nsleep 5\n"),
            0o755,
        );
        let not_deno = version_script(&dir, "not-deno.sh", "hello from something else");

        let not_found = check_binary(
            "ffmpeg",
            Ok(dir.path().join("missing")),
            &["-version"],
            Duration::from_secs(5),
            sidecar::parse_ffmpeg_version,
            &registry,
        )
        .await;
        let non_zero = check_binary(
            "yt-dlp",
            Ok(failing),
            &["--version"],
            Duration::from_secs(5),
            sidecar::parse_ytdlp_version,
            &registry,
        )
        .await;
        let timeout = check_binary(
            "yt-dlp",
            Ok(slow),
            &["--version"],
            Duration::from_millis(150),
            sidecar::parse_ytdlp_version,
            &registry,
        )
        .await;
        let unrecognized = report_with_deno(&dir, deno_launch(not_deno, dir.path()))
            .await
            .deno;

        for (result, status) in [
            (&not_found, SidecarStatus::NotFound),
            (&non_zero, SidecarStatus::NonZeroExit),
            (&timeout, SidecarStatus::Timeout),
            (&unrecognized, SidecarStatus::LaunchFailed),
        ] {
            assert_eq!(result.status, status, "{result:?}");
            assert!(result.version_raw.is_none(), "{result:?}");
            assert_eq!(
                serialized_version_raw(result),
                None,
                "при {status:?} ключа versionRaw в JSON быть не должно"
            );
        }
    }

    #[test]
    fn clip_version_raw_cuts_by_characters_not_bytes() {
        // Кириллица по два байта: предел в символах попадает на середину
        // символа, если считать байтами, — а ведущий ASCII сдвигает
        // границу на нечётный байт.
        let long = format!("a{}", "ё".repeat(VERSION_RAW_MAX_CHARS * 3));

        let clipped = clip_version_raw(&long);

        assert_eq!(clipped.chars().count(), VERSION_RAW_MAX_CHARS + 1);
        assert!(clipped.ends_with('…'), "{clipped:?}");
        assert!(long.starts_with(clipped.trim_end_matches('…')));

        // Ровно на пределе — не обрезается, хотя байтов вдвое больше.
        let at_limit = "ё".repeat(VERSION_RAW_MAX_CHARS);
        assert_eq!(clip_version_raw(&at_limit), at_limit);
    }

    #[test]
    fn unprintable_symbols_of_a_version_line_are_replaced_not_passed_through() {
        // Воспроизведение ревью TL-15: подменённый deno печатает ESC, RLO,
        // BEL и NUL внутри строки версии.
        let line = "deno 2.9.6 \u{1b}[31m\u{202E}lave\u{7}\u{0}x";

        let clipped = clip_version_raw(line);

        assert_eq!(
            clipped,
            "deno 2.9.6 \u{FFFD}[31m\u{FFFD}lave\u{FFFD}\u{FFFD}x"
        );
        for bidi in [
            '\u{061C}', '\u{200E}', '\u{200F}', '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}',
            '\u{202E}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
        ] {
            assert_eq!(
                clip_version_raw(&format!("a{bidi}b")),
                "a\u{FFFD}b",
                "U+{:04X} обязан быть заменён",
                u32::from(bidi)
            );
        }
        // Обычный текст фильтр не трогает — ни кириллицу, ни символы вне BMP.
        assert_eq!(clip_version_raw("ёж 🦔 2.9.6"), "ёж 🦔 2.9.6");
    }

    #[tokio::test]
    async fn unrecognized_output_of_yt_dlp_and_ffmpeg_shows_its_first_line_only() {
        // Ветка `ShowAsIs`: код 0, версии в выводе нет. На экран уходит
        // первая строка, а не весь поток с внутренними переводами строк.
        let dir = tempdir().expect("failed to create temp dir");
        let registry = ChildRegistry::new();
        let garbage = write_script(
            &dir,
            "garbage.sh",
            "#!/bin/sh\nprintf '\\n  \\ngarbage\\nsecond\\n'\nexit 0\n",
            0o755,
        );

        for (name, args, parse) in [
            (
                "yt-dlp",
                &["--version"][..],
                sidecar::parse_ytdlp_version as fn(&str) -> Option<sidecar::SidecarVersion>,
            ),
            ("ffmpeg", &["-version"][..], sidecar::parse_ffmpeg_version),
        ] {
            let result = check_binary(
                name,
                Ok(garbage.clone()),
                args,
                Duration::from_secs(5),
                parse,
                &registry,
            )
            .await;

            assert_eq!(result.status, SidecarStatus::Ok, "{result:?}");
            assert_eq!(result.version_raw.as_deref(), Some("garbage"), "{result:?}");
            assert_eq!(result.version.as_deref(), Some("garbage"), "{result:?}");
        }
    }

    #[tokio::test]
    async fn a_megabyte_of_unrecognized_output_reaches_the_screen_clipped_and_filtered() {
        let dir = tempdir().expect("failed to create temp dir");
        let registry = ChildRegistry::new();
        let flood = write_script(
            &dir,
            "flood.sh",
            "#!/bin/sh\nprintf 'x\\033'\ni=0\nwhile [ $i -lt 2000 ]; do \
             printf 'ёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёё'; i=$((i + 1)); done\n\
             printf '\\r\\nsecond\\n'\nexit 0\n",
            0o755,
        );

        let result = check_binary(
            "ffmpeg",
            Ok(flood),
            &["-version"],
            Duration::from_secs(5),
            sidecar::parse_ffmpeg_version,
            &registry,
        )
        .await;

        assert_eq!(result.status, SidecarStatus::Ok, "{result:?}");
        let version = result.version.expect("version при ok");
        assert_eq!(version.chars().count(), VERSION_RAW_MAX_CHARS + 1);
        assert!(
            version.starts_with("x\u{FFFD}ё") && version.ends_with('…'),
            "{version:?}"
        );
        assert_eq!(result.version_raw.as_deref(), Some(version.as_str()));
    }

    #[tokio::test]
    async fn recognized_versions_are_shown_unchanged_by_the_filter() {
        let dir = tempdir().expect("failed to create temp dir");
        let registry = ChildRegistry::new();
        let yt_dlp = version_script(&dir, "yt-dlp.sh", YT_DLP_VERSION_LINE);
        let ffmpeg = version_script(&dir, "ffmpeg.sh", FFMPEG_VERSION_LINE);

        let yt_dlp = check_binary(
            "yt-dlp",
            Ok(yt_dlp),
            &["--version"],
            Duration::from_secs(5),
            sidecar::parse_ytdlp_version,
            &registry,
        )
        .await;
        let ffmpeg = check_binary(
            "ffmpeg",
            Ok(ffmpeg),
            &["-version"],
            Duration::from_secs(5),
            sidecar::parse_ffmpeg_version,
            &registry,
        )
        .await;

        assert_eq!(yt_dlp.version.as_deref(), Some("2026.08.19"));
        assert_eq!(yt_dlp.version_raw.as_deref(), Some("2026.08.19"));
        assert_eq!(ffmpeg.version.as_deref(), Some("9.0.1"));
        assert_eq!(ffmpeg.version_raw.as_deref(), Some(FFMPEG_VERSION_LINE));
    }

    #[tokio::test]
    async fn a_megabyte_version_line_reaches_the_contract_clipped() {
        let dir = tempdir().expect("failed to create temp dir");
        // `deno` с настоящим префиксом и мегабайтом многобайтного хвоста в
        // той же строке: разбор проходит, строка — нет.
        let deno = write_script(
            &dir,
            "deno.sh",
            "#!/bin/sh\nprintf 'deno 2.9.6 '\ni=0\nwhile [ $i -lt 2000 ]; do \
             printf 'ёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёёё'; i=$((i + 1)); done\n\
             printf '\\n'\nexit 0\n",
            0o755,
        );

        let report = report_with_deno(&dir, deno_launch(deno, dir.path())).await;

        assert_eq!(
            report.deno.status,
            SidecarStatus::Ok,
            "{:?}",
            report.deno.stderr_tail
        );
        assert_eq!(report.deno.version.as_deref(), Some("2.9.6"));
        let raw = report.deno.version_raw.expect("versionRaw при ok");
        assert_eq!(raw.chars().count(), VERSION_RAW_MAX_CHARS + 1);
        assert!(
            raw.starts_with("deno 2.9.6 ёё") && raw.ends_with('…'),
            "{raw:?}"
        );
    }
}
