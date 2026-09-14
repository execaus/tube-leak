//! Склейка скачанных потоков в один файл (Ф-9, С-11) — TL-42.
//!
//! Перепаковка без перекодирования (remux): дорожки переносятся в общий
//! контейнер как есть, `-c copy`. Секунды работы вместо часов, и ни один
//! бит дорожек не меняется — то же, что делает yt-dlp своим
//! постпроцессором, но **нашим** процессом.
//!
//! # Почему ffmpeg зовёт наше ядро, а не yt-dlp своим постпроцессором
//!
//! Решение дизайна по Ф-9, здесь только следствия, которые видно в коде:
//!
//! - **Отмена.** Процесс склейки — отдельная запись в том же реестре E1
//!   ([`crate::sidecar::ChildRegistry`]), что и yt-dlp, и убивается той же
//!   отменой ([`RunHandle`]) группой процессов. Будь склейка
//!   постпроцессором, в фазе Merging пришлось бы либо убивать группу
//!   yt-dlp целиком (неотличимо от отмены в Downloading), либо выяснять
//!   PID чужого вложенного процесса.
//! - **Класс ошибки.** stderr этого процесса — только его собственный,
//!   поэтому классификация С-11 тривиальна: **любая** его неудача и есть
//!   [`DownloadFailure::MergeFailed`], без текстовых эвристик. Здесь нет
//!   и не будет разбора формулировок ffmpeg — ни для «нет места», ни для
//!   «нет прав»: наверху таблицы Ф-10 эти классы принадлежат фазе
//!   скачивания, а в фазе склейки ошибка называется «не удалось склеить»
//!   и точка. Фикстуры `disk-full-mid-merge` и `output-not-writable`
//!   сняты живьём именно затем, чтобы это правило стояло на снятом
//!   выводе, а не на убеждении.
//! - **ffmpeg из дистрибутива.** Путь резолвит [`crate::sidecar`] тем же
//!   кодом, что и служебный экран E1, а не флаг `--ffmpeg-location`,
//!   переданный чужому процессу в надежде, что тот его уважит.
//!
//! # Чего этот модуль не знает
//!
//! Он не знает ни про прогрессивные форматы, ни про «только аудио», ни
//! про фазы задачи: звать склейку или не звать — решает оркестрация
//! (TL-44). Здесь склейка — операция, а не этап автомата.
//!
//! Он также **не финализирует имя**: результат кладётся под рабочим
//! именем ([`working_file_name`]), а переименование в финальное — переход
//! Merging → Done у TL-44. Отсюда же и Ф-8 («под финальным именем
//! недосклеенного не бывает никогда») выполняется по построению: под
//! финальным именем этот модуль не пишет вовсе.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::{Duration, Instant};

use super::error::DownloadFailure;
use crate::sidecar::{
    run_cancellable, stderr_tail, ChildRegistry, RunHandle, RunOutput, SidecarError,
};
use crate::types::DownloadErrorDetails;

/// Предел ожидания одной склейки, в секундах.
///
/// Это **страховка от зависшего процесса, а не ожидаемое время**:
/// пользователю ждать столько не придётся никогда, а если придётся — у
/// него есть работающая кнопка «Отменить», которая не ждёт таймаута.
///
/// # Замер (живьём, вложенный ffmpeg 9.0.1, Apple Silicon, macOS 15)
///
/// Перепаковка файла 283 МБ (h264 1080p30 + aac) заняла **0,42 / 0,57 /
/// 0,77 с** на трёх прогонах подряд — то есть 370–670 МБ/с на встроенном
/// SSD с горячим кешем. Отсюда порядок величины: гигабайт ≈ 2–3 с.
///
/// # Откуда 30 минут
///
/// Худший правдоподобный случай MVP — 4K-ролик на несколько часов,
/// десятки гигабайт, на внешнем механическом диске, где чтение двух
/// потоков и запись результата делят одну головку: 20 МБ/с — уже
/// пессимистичная, но не выдуманная оценка. 30 ГБ при 20 МБ/с — это
/// 25 минут. Значение выбрано как ближайшая круглая граница за этой
/// оценкой.
///
/// Меньше брать нельзя: таймаут, срабатывающий на честно идущей склейке,
/// превращает готовую загрузку в ошибку С-11 на ровном месте — а второй
/// попытки, которая пройдёт быстрее, у такого случая нет. Больше — тоже
/// незачем: за полчаса без единого признака жизни процесс уже точно завис,
/// а не работает.
///
/// Число подлежит калибровке на bundle вместе с остальными числами эпика
/// (урок TL-12): здесь оно снято замером скорости перепаковки, но не
/// проверено на многочасовом 4K-ролике — такого материала в проекте нет.
pub const MERGE_TIMEOUT_SECS: u64 = 1800;

/// [`MERGE_TIMEOUT_SECS`] как [`Duration`] — то, что уходит в запуск.
const MERGE_TIMEOUT: Duration = Duration::from_secs(MERGE_TIMEOUT_SECS);

/// Метка рабочего файла склейки в имени: `<основа>.tl-merging.<ext>`.
///
/// Расширение остаётся **последним** намеренно: ffmpeg выбирает мультиплексор
/// по расширению выходного файла, и `…​.tl-merging.mp4` для него такой же
/// mp4, как и финальное имя.
///
/// Файл с таким именем в папке назначения может появиться только от нашей
/// же прерванной склейки, поэтому перезапись (`-y`) чужих данных не
/// уничтожает, а повтору после сбоя не мешает.
const WORKING_MARK: &str = "tl-merging";

/// Контейнер результата.
///
/// Выбирается по совместимости расширений скачанных потоков, а не задаётся
/// снаружи: какие дорожки скачаны, знает файл на диске, а не вызывающий.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeContainer {
    /// Обе дорожки из семейства MP4 (`.mp4` + `.m4a` — обычный случай
    /// YouTube для h264/AAC).
    Mp4,
    /// Обе дорожки из семейства WebM (`.webm` + `.webm`/`.weba` — VP9/Opus).
    Webm,
    /// Всё остальное: Matroska принимает любую пару кодеков.
    Mkv,
}

impl MergeContainer {
    /// Расширение файла результата — оно же способ сказать ffmpeg, каким
    /// мультиплексором писать.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Webm => "webm",
            Self::Mkv => "mkv",
        }
    }
}

/// Расширения, которые mp4-мультиплексор принимает как родные.
const MP4_FAMILY: [&str; 5] = ["mp4", "m4a", "m4v", "m4b", "mov"];

/// То же для webm.
const WEBM_FAMILY: [&str; 2] = ["webm", "weba"];

/// Контейнер для пары скачанных потоков.
///
/// Правило то же, по которому контейнер выбирает yt-dlp, когда сливает
/// потоки сам: расширения из одного семейства — этот контейнер, иначе
/// Matroska. Своего изобретено не было: у нас те же входные файлы и тот же
/// ffmpeg, а «угадать лучше» здесь означало бы разойтись с тем, что
/// пользователь получил бы от yt-dlp напрямую.
///
/// Состав семейств сознательно узкий — под то, что YouTube реально отдаёт
/// (`mp4`/`m4a` и `webm`/`weba`); всё незнакомое честно уезжает в mkv,
/// который примет любую пару. Это не «на всякий случай»: цена ошибки
/// несимметрична. Лишний mkv — рабочий файл с непривычным расширением,
/// а неверно выбранный mp4 — отказ склейки целиком, и выглядит он ровно
/// как фикстура `container-refuses-codec` (h264 + aac в webm: «Only VP8 or
/// VP9 or AV1 video and Vorbis or Opus audio … are supported for WebM»,
/// код 234).
///
/// Расширения сравниваются без учёта регистра: `.MP4` — тот же mp4.
pub fn container_for(video: &Path, audio: &Path) -> MergeContainer {
    let video_ext = extension_of(video);
    let audio_ext = extension_of(audio);

    if in_family(&video_ext, &MP4_FAMILY) && in_family(&audio_ext, &MP4_FAMILY) {
        MergeContainer::Mp4
    } else if in_family(&video_ext, &WEBM_FAMILY) && in_family(&audio_ext, &WEBM_FAMILY) {
        MergeContainer::Webm
    } else {
        MergeContainer::Mkv
    }
}

fn extension_of(path: &Path) -> String {
    path.extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

fn in_family(ext: &str, family: &[&str]) -> bool {
    family.iter().any(|known| ext.eq_ignore_ascii_case(known))
}

/// Имя рабочего файла склейки для основы имени `stem`.
///
/// Отдельная функция, а не строка внутри запуска, потому что её проверяет
/// тест Ф-8: рабочее имя обязано отличаться от финального (`stem.ext`) и
/// обязано кончаться расширением контейнера.
pub fn working_file_name(stem: &str, container: MergeContainer) -> String {
    format!("{stem}.{WORKING_MARK}.{}", container.extension())
}

/// Что склеить и куда положить результат.
#[derive(Debug, Clone, Copy)]
pub struct MergeRequest<'a> {
    /// Файл видеопотока, скачанный yt-dlp.
    pub video: &'a Path,
    /// Файл аудиопотока.
    pub audio: &'a Path,
    /// Папка назначения — та же, где лежат оба потока и где окажется
    /// готовый файл.
    pub destination_dir: &'a Path,
    /// Основа имени **без** расширения. Модуль пишет не под ней, а под
    /// рабочим именем ([`working_file_name`]); финальное имя — дело
    /// оркестрации (TL-44), и с TL-89 оно строится из шаблона отдельно:
    /// сюда оркестрация отдаёт рабочую основу с id ролика (TL-104).
    pub stem: &'a str,
}

/// Готовый склеенный файл — под рабочим именем.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergedFile {
    /// Полный путь к результату. Переименовать его в финальное имя —
    /// задача вызывающего (TL-44).
    pub path: PathBuf,
    /// Контейнер, в который лёг результат: из него берётся расширение
    /// финального имени («расширение — по фактическому контейнеру», С-1).
    pub container: MergeContainer,
}

/// Чем кончилась склейка.
///
/// Трёхзначно по той же причине, что и вердикт попытки скачивания
/// ([`crate::download::classify::AttemptVerdict`]): отмена — не ошибка и
/// класса в девятке Ф-10 не имеет, поэтому в `Result` она не выражается.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeVerdict {
    /// Готово: файл лежит под рабочим именем.
    Merged(MergedFile),
    /// Склейку отменил пользователь ([`RunHandle::cancel`]). Рабочий файл
    /// удалён — на диске от склейки не осталось ничего (Ф-4, таблица
    /// «Отмена по фазам»). Скачанные потоки при этом целы: их удаляет
    /// оркестрация, которая одна и знает, что отменяется вся задача.
    Cancelled,
    /// Склейка не удалась. Класс всегда один —
    /// [`DownloadFailure::MergeFailed`]; рабочий файл удалён.
    Failed(DownloadFailure),
}

/// Запускатель ffmpeg — единственный шов между склейкой и настоящим
/// процессом.
///
/// Ровно тот же приём, что у [`crate::probe::YtDlpLauncher`], и по той же
/// причине: без шва не проверить ни «процесс попал в реестр и ушёл из
/// него», ни «отмена посреди склейки убирает недоделанный файл» — для
/// этого пришлось бы гонять в тестах настоящую перепаковку настоящего
/// медиафайла. Продакшен-реализация одна — [`SidecarFfmpeg`].
pub trait FfmpegLauncher: Send + Sync {
    /// Запускает ffmpeg с `args`, ждёт не дольше `timeout` и отдаёт то,
    /// что осталось от процесса. `handle` — дескриптор отмены.
    fn launch<'a>(
        &'a self,
        args: &'a [&'a str],
        timeout: Duration,
        handle: &'a RunHandle,
    ) -> Pin<Box<dyn Future<Output = Result<RunOutput, SidecarError>> + Send + 'a>>;
}

/// Продакшен-реализация [`FfmpegLauncher`]: запуск через домен `sidecar`
/// (E1). Реестр процессов, группа процессов и таймаут живут в
/// [`crate::sidecar::run_cancellable`] — своего трекинга PID здесь нет и
/// быть не должно.
pub struct SidecarFfmpeg<'a> {
    executable: PathBuf,
    registry: &'a ChildRegistry,
}

impl<'a> SidecarFfmpeg<'a> {
    /// `executable` — путь к вложенному ffmpeg
    /// ([`crate::sidecar::resolve_sidecar_path`]); `registry` — реестр PID
    /// из состояния приложения, тот же, в котором лежит yt-dlp.
    pub fn new(executable: PathBuf, registry: &'a ChildRegistry) -> Self {
        Self {
            executable,
            registry,
        }
    }
}

impl FfmpegLauncher for SidecarFfmpeg<'_> {
    fn launch<'a>(
        &'a self,
        args: &'a [&'a str],
        timeout: Duration,
        handle: &'a RunHandle,
    ) -> Pin<Box<dyn Future<Output = Result<RunOutput, SidecarError>> + Send + 'a>> {
        Box::pin(run_cancellable(
            &self.executable,
            args,
            timeout,
            self.registry,
            handle,
        ))
    }
}

/// Аргументы запуска, кроме путей. Каждый — с обоснованием:
///
/// - `-hide_banner` — баннер сборки (десяток строк `configuration:`) в
///   «Подробнее» не нужен: версия ffmpeg и так на служебном экране E1, а
///   хвост stderr ограничен (Н-4) и его лучше отдать самой ошибке.
/// - `-loglevel warning` — успешная перепаковка при этом молчит вовсе
///   (снято живьём: у всех четырёх успешных фикстур оба потока пусты), а
///   отказ печатает ровно то, что произошло. Уровень ниже (`error`)
///   съел бы предупреждения, которые объясняют неочевидный сбой; уровень
///   по умолчанию (`info`) добавляет к каждому успеху ~1,8 КБ описания
///   дорожек — в логе это шум, а в «Подробнее» оно вытеснило бы саму
///   ошибку.
/// - `-y` — перезаписать рабочий файл, оставшийся от прошлой прерванной
///   склейки, вместо вопроса «Overwrite?». Вопрос здесь опаснее, чем
///   кажется: stdin процесса — `/dev/null` (так его открывает
///   [`run_cancellable`]), и без `-y` ffmpeg на существующем файле просто
///   отказался бы работать.
/// - `-map 0:v:0 -map 1:a:0` — берём первую видеодорожку первого входа и
///   первую аудиодорожку второго. Без явного отображения ffmpeg выбирает
///   дорожки сам «по лучшей», и на потоках, где выбирать не из чего, это
///   дало бы тот же результат — но правило «видео из первого, звук из
///   второго» перестало бы быть записанным.
/// - `-c copy` — то самое отсутствие перекодирования (Ф-9).
///
/// Чего здесь нет намеренно: `-movflags +faststart` (второй проход по
/// готовому файлу ради ускорения перемотки при **потоковой** отдаче — у
/// локального файла её нет), `-nostdin` (stdin уже `/dev/null`, дублировать
/// источник истины не за чем) и `-threads` (перепаковка упирается в диск,
/// а не в процессор).
const MERGE_ARGS: [&str; 4] = ["-hide_banner", "-loglevel", "warning", "-y"];

/// Полный argv склейки.
///
/// Все три пути идут с префиксом `file:` — приём, взятый у yt-dlp
/// (`_ffmpeg_filename_argument`), и не из вежливости:
///
/// - имя, начинающееся с `-`, без префикса читается как опция;
/// - **относительный** путь с двоеточием читается как «протокол»:
///   проверено живьём, `-i "-weird: name.f137.mp4"` даёт «Protocol not
///   found. Did you mean file:-weird: name.f137.mp4?», а с префиксом тот
///   же файл склеивается (фикстура `success-awkward-filename`).
///
/// Названия роликов дают и то и другое: заголовок вида «Rust: часть 2»
/// после санитизации остаётся с двоеточием на macOS и Linux.
///
/// Возвращает владеющие строки: префикс `file:` приклеивается к пути, а
/// заимствовать у временного значения нечего.
fn merge_args(video: &str, audio: &str, output: &str) -> Vec<String> {
    let mut args: Vec<String> = MERGE_ARGS.iter().map(|arg| (*arg).to_string()).collect();
    args.push("-i".to_string());
    args.push(format!("file:{video}"));
    args.push("-i".to_string());
    args.push(format!("file:{audio}"));
    args.push("-map".to_string());
    args.push("0:v:0".to_string());
    args.push("-map".to_string());
    args.push("1:a:0".to_string());
    args.push("-c".to_string());
    args.push("copy".to_string());
    args.push(format!("file:{output}"));
    args
}

/// Склеивает два скачанных потока в один файл (Ф-9).
///
/// Возврат — [`MergeVerdict`]; исключений из правила «неудача этого
/// процесса = [`DownloadFailure::MergeFailed`]» нет ни одного, включая
/// таймаут и отсутствие самого бинарника.
///
/// # Что остаётся на диске
///
/// Скачанных потоков функция не трогает никогда — ни при успехе, ни при
/// отказе, ни при отмене: они собственность оркестрации, и именно на них
/// стоит обещание «повтор пересобирает файл заново без повторного
/// скачивания» (таблица Ф-10 для С-11).
///
/// Свой рабочий файл, наоборот, убирает за собой всегда, кроме успеха, —
/// и это не гигиена, а требование: ffmpeg, убитый посреди перепаковки,
/// оставляет недописанный файл (снято живьём: 75 МБ на убийстве через
/// 0,15 с, фикстура `killed-mid-merge`), а на отказе — обрубок в
/// несколько сотен байт (`container-refuses-codec`) или всё свободное
/// место тома (`disk-full-mid-merge`, 6 078 464 байта).
pub async fn merge_streams<L>(
    launcher: &L,
    request: &MergeRequest<'_>,
    handle: &RunHandle,
) -> MergeVerdict
where
    L: FfmpegLauncher + ?Sized,
{
    let container = container_for(request.video, request.audio);
    let output = request
        .destination_dir
        .join(working_file_name(request.stem, container));

    let (Some(video), Some(audio), Some(output_arg)) = (
        request.video.to_str(),
        request.audio.to_str(),
        output.to_str(),
    ) else {
        // Известное ограничение, а не забытый случай: аргументы запуска
        // домен `sidecar` принимает строками (`&[&str]`), и путь, не
        // представимый в UTF-8, в них не выражается. Такой путь может
        // возникнуть только из папки назначения — имя файла строит TL-40
        // из уже валидной строки, — и на macOS/Windows не возникает вовсе
        // (там имена и так Unicode). Ломать ради этого сигнатуру,
        // которой пользуются ещё два вызывающих, дороже, чем честно
        // отказать.
        eprintln!("merge: путь не представим в UTF-8, склейка не запускалась");
        return MergeVerdict::Failed(merge_failed(DownloadErrorDetails {
            stderr_tail: None,
            exit_code: None,
        }));
    };

    let args = merge_args(video, audio, output_arg);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    eprintln!("merge: запуск ffmpeg {}", args.join(" "));

    let started = Instant::now();
    let outcome = launcher.launch(&args, MERGE_TIMEOUT, handle).await;
    let elapsed = started.elapsed();

    // Отмена проверяется раньше вида ошибки: убитый нами процесс возвращает
    // ту же `LaunchFailed`, что и процесс, отказавший сам (см. doc
    // `run_cancellable`), и отличить их можно только по дескриптору.
    //
    // Отмена, пришедшая в последний миг — когда ffmpeg уже дописал файл, —
    // тоже отмена: результат удаляется. Иначе нажатие «Отменить» иногда
    // оставляло бы в папке готовый файл, а обещание «на диске ничего не
    // осталось» (Ф-4) стало бы зависеть от планировщика.
    if handle.was_cancelled() {
        remove_working_file(&output);
        eprintln!(
            "merge: склейка отменена и процесс убит через {} мс",
            elapsed.as_millis()
        );
        return MergeVerdict::Cancelled;
    }

    match outcome {
        Ok(RunOutput { stderr, .. }) => {
            // Код завершения нулевой, но файла нет — верить коду больше,
            // чем файловой системе, здесь не за что: дальше по цепочке
            // TL-44 будет переименовывать то, чего не существует.
            //
            // Кода завершения в деталях нет, хотя он известен и равен нулю
            // (тот же приём, что в `probe::orchestrate::card`): «Подробнее»
            // под заголовком «Не удалось склеить», где написано, что
            // процесс отработал успешно, — противоречие, а не диагностика.
            // Чего именно не хватило, сказано строкой лога рядом.
            if !output.exists() {
                eprintln!("merge: ffmpeg отчитался успехом, но файла результата нет");
                return MergeVerdict::Failed(merge_failed(details(&stderr, None)));
            }

            eprintln!(
                "merge: готово за {} мс, контейнер {}",
                elapsed.as_millis(),
                container.extension()
            );
            MergeVerdict::Merged(MergedFile {
                path: output,
                container,
            })
        }
        Err(error) => {
            remove_working_file(&output);
            let details = match &error {
                SidecarError::NonZeroExit { code, stderr } => details(stderr, Some(*code)),
                // Убитый по таймауту процесс своего кода не оставил.
                SidecarError::Timeout { stderr, .. } => details(stderr, None),
                SidecarError::LaunchFailed { stderr, .. } => details(stderr, None),
                // Вложенного ffmpeg нет на месте — рассказывать в
                // «Подробнее» нечего, процесса не было.
                SidecarError::NotFound => details("", None),
            };
            eprintln!("merge: отказ за {} мс: {error}", elapsed.as_millis());
            MergeVerdict::Failed(merge_failed(details))
        }
    }
}

/// Единственный класс отказа этого модуля (С-11).
fn merge_failed(details: DownloadErrorDetails) -> DownloadFailure {
    DownloadFailure::MergeFailed { details }
}

/// Технические детали для «Подробнее» (Н-4) — тот же хвост, что у разбора
/// и у скачивания.
fn details(stderr: &str, exit_code: Option<i32>) -> DownloadErrorDetails {
    DownloadErrorDetails {
        stderr_tail: stderr_tail(stderr),
        exit_code,
    }
}

/// Убирает недоделанный результат склейки.
///
/// Отсутствие файла — штатный исход, а не ошибка: ffmpeg мог не дойти до
/// его создания вовсе (`input-missing`, `input-truncated`,
/// `output-not-writable` — все три сняты живьём и файла не оставили).
/// Неудача самого удаления — тоже не повод менять класс отказа: он уже
/// «не удалось склеить», и от того, что рядом остался обрубок, это не
/// перестаёт быть правдой. Но в лог она попадает: это единственный путь,
/// на котором в папке назначения может остаться наш мусор.
fn remove_working_file(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => eprintln!("merge: недоделанный результат удалён"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => eprintln!("merge: не удалось удалить недоделанный результат: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Mutex as StdMutex;

    use tempfile::{tempdir, TempDir};
    use tokio::time;

    use super::*;
    use crate::download::fixtures;
    use crate::sidecar::parse_ffmpeg_version;
    use crate::types::{DownloadErrorKind, LaunchFailedReason};

    /// Успешные фикстуры и та пара расширений, на которой каждая снята.
    ///
    /// Таблица связывает снятый живьём материал с [`container_for`]: у
    /// каждой строки настоящий ffmpeg **принял** эту пару в этот
    /// контейнер, а не отказал как в `container-refuses-codec`.
    const SUCCESSES: [(&str, &str, &str, MergeContainer); 4] = [
        (
            "success-mp4-h264-aac.json",
            "mp4",
            "m4a",
            MergeContainer::Mp4,
        ),
        (
            "success-webm-vp9-opus.json",
            "webm",
            "webm",
            MergeContainer::Webm,
        ),
        ("success-mkv-mixed.json", "mp4", "webm", MergeContainer::Mkv),
        (
            "success-awkward-filename.json",
            "mp4",
            "m4a",
            MergeContainer::Mp4,
        ),
    ];

    /// Отказавшие фикстуры и код завершения, с которым они сняты.
    /// `None` — процесс убит сигналом.
    const FAILURES: [(&str, Option<i32>); 6] = [
        ("container-refuses-codec.json", Some(234)),
        ("input-missing.json", Some(254)),
        ("input-truncated.json", Some(183)),
        ("output-not-writable.json", Some(243)),
        ("disk-full-mid-merge.json", Some(228)),
        ("killed-mid-merge.json", None),
    ];

    /// Запускатель, проигрывающий снятую фикстуру вместо запуска ffmpeg:
    /// отдаёт её код завершения и потоки и — главное — воспроизводит то,
    /// что осталось на диске после настоящего процесса.
    struct Replay {
        outcome: fixtures::MergeOutcome,
        seen: StdMutex<Option<(Vec<String>, Duration)>>,
    }

    impl Replay {
        fn of(name: &str) -> Self {
            Self {
                outcome: fixtures::merge_outcome(name),
                seen: StdMutex::new(None),
            }
        }

        fn seen_args(&self) -> Vec<String> {
            self.seen
                .lock()
                .expect("запуск состоялся")
                .clone()
                .expect("запуск состоялся")
                .0
        }

        fn seen_timeout(&self) -> Duration {
            self.seen
                .lock()
                .expect("запуск состоялся")
                .clone()
                .expect("запуск состоялся")
                .1
        }
    }

    impl FfmpegLauncher for Replay {
        fn launch<'a>(
            &'a self,
            args: &'a [&'a str],
            timeout: Duration,
            _handle: &'a RunHandle,
        ) -> Pin<Box<dyn Future<Output = Result<RunOutput, SidecarError>> + Send + 'a>> {
            *self.seen.lock().expect("реестр аргументов") =
                Some((args.iter().map(|arg| (*arg).to_string()).collect(), timeout));

            Box::pin(async move {
                // Файл результата ровно там и тогда, где его оставил
                // настоящий процесс. Размер не воспроизводится: 75 МБ
                // недосклеенного (`killed-mid-merge`) значат ровно то же,
                // что и один байт, — что файл есть и его надо убрать.
                if self.outcome.output_left_bytes.is_some() {
                    let path = args
                        .last()
                        .expect("последний аргумент — выходной файл")
                        .strip_prefix("file:")
                        .expect("выходной файл идёт с префиксом file:");
                    fs::write(path, b"partial").expect("запись недоделанного результата");
                }

                let stderr = self.outcome.stderr.clone();
                match self.outcome.exit_code {
                    Some(0) => Ok(RunOutput {
                        stdout: self.outcome.stdout.clone(),
                        stderr,
                    }),
                    Some(code) => Err(SidecarError::NonZeroExit { code, stderr }),
                    // Убитый сигналом процесс своего кода не оставил — так
                    // его отдаёт `run_cancellable`.
                    None => Err(SidecarError::LaunchFailed {
                        reason: LaunchFailedReason::Corrupted,
                        stderr,
                    }),
                }
            })
        }
    }

    /// Запускатель, который обязан не пригодиться: процесса на этом пути
    /// быть не должно вовсе.
    struct NeverLaunched;

    impl FfmpegLauncher for NeverLaunched {
        fn launch<'a>(
            &'a self,
            _args: &'a [&'a str],
            _timeout: Duration,
            _handle: &'a RunHandle,
        ) -> Pin<Box<dyn Future<Output = Result<RunOutput, SidecarError>> + Send + 'a>> {
            panic!("процесс склейки не должен запускаться на этом пути");
        }
    }

    /// Папка с двумя «скачанными» потоками — пустыми файлами с теми
    /// расширениями, которые важны для выбора контейнера.
    fn workspace(video_ext: &str, audio_ext: &str) -> (TempDir, PathBuf, PathBuf) {
        let dir = tempdir().expect("временный каталог");
        let video = dir.path().join(format!("Ролик.f137.{video_ext}"));
        let audio = dir.path().join(format!("Ролик.f140.{audio_ext}"));
        fs::write(&video, b"video").expect("файл видеопотока");
        fs::write(&audio, b"audio").expect("файл аудиопотока");
        (dir, video, audio)
    }

    fn request<'a>(dir: &'a Path, video: &'a Path, audio: &'a Path) -> MergeRequest<'a> {
        MergeRequest {
            video,
            audio,
            destination_dir: dir,
            stem: "Ролик",
        }
    }

    fn failure_of(verdict: MergeVerdict) -> DownloadFailure {
        match verdict {
            MergeVerdict::Failed(failure) => failure,
            other => panic!("ожидался отказ склейки, получено {other:?}"),
        }
    }

    #[test]
    fn the_list_of_fixtures_matches_the_files_on_disk() {
        // Фикстура, о которой таблицы не знают, выглядела бы покрытым
        // случаем, не будучи им; удалённая — молча уменьшила бы набор.
        let mut declared: Vec<String> = SUCCESSES
            .iter()
            .map(|(name, ..)| (*name).to_string())
            .chain(FAILURES.iter().map(|(name, _)| (*name).to_string()))
            .collect();
        declared.sort();

        assert_eq!(declared, fixtures::merge_files_on_disk());
        assert_eq!(declared, {
            let mut listed: Vec<String> = fixtures::MERGE_FIXTURES
                .iter()
                .map(|name| (*name).to_string())
                .collect();
            listed.sort();
            listed
        });
    }

    #[test]
    fn fixtures_are_real_output_of_the_pinned_ffmpeg() {
        // Та же связь, что у фикстур yt-dlp (TL-30/31/41/43): фикстуры
        // заморожены, а ffmpeg — нет. Коды завершения и формулировки
        // задаёт апстрим, и смена пина обязана громко ломать этот тест.
        let pinned = fixtures::pinned_ffmpeg_version();

        for name in fixtures::MERGE_FIXTURES {
            let outcome = fixtures::merge_outcome(name);

            assert_eq!(
                outcome.capture.ffmpeg_version, pinned,
                "{name}: исход снят не тем ffmpeg, который вложен в приложение \
                 ({pinned} по binaries.lock.json). Пин сменили — переснимите \
                 фикстуры по README, а не правьте эту строку"
            );

            // Сырая строка сводится к пину тем же разбором, что и на
            // служебном экране: вложенная сборка называет себя
            // «9.0.1-https://www.martin-riedl.de», и сверять её с пином
            // буквально было бы сверкой с именем зеркала, а не с версией.
            let parsed = parse_ffmpeg_version(&outcome.capture.ffmpeg_version_raw)
                .unwrap_or_else(|| panic!("{name}: строка версии ffmpeg не разбирается"));
            assert_eq!(
                parsed.display, pinned,
                "{name}: фикстура снята бинарником версии {}, а вложен {pinned}",
                parsed.display
            );
        }
    }

    #[test]
    fn every_fixture_says_out_loud_whether_it_was_captured_live() {
        // В этом наборе смоделированных нет ни одной: каждый исход снят
        // настоящим ffmpeg на настоящих файлах. Появится смоделированный —
        // его придётся внести сюда руками, и в README тоже.
        for name in fixtures::MERGE_FIXTURES {
            let outcome = fixtures::merge_outcome(name);
            assert!(
                outcome.capture.is_live(),
                "{name}: набор склейки состоит только из снятого живьём; \
                 смоделированную фикстуру нужно назвать здесь и в README"
            );
        }
    }

    #[test]
    fn fixtures_were_shot_with_the_arguments_the_module_builds() {
        // Фикстура, снятая другой командой, доказывает поведение другой
        // команды. Плейсхолдеры подставляются вместо путей — argv
        // собирается тем же кодом, что и в продакшене.
        let expected = merge_args("<video>", "<audio>", "<output>");

        for name in fixtures::MERGE_FIXTURES {
            let outcome = fixtures::merge_outcome(name);
            assert_eq!(
                outcome.capture.argv, expected,
                "{name}: аргументы запуска изменились — переснимите фикстуры \
                 по README, а не правьте эту строку"
            );
        }
    }

    #[test]
    fn picks_the_container_by_the_family_of_both_extensions() {
        let cases: [(&str, &str, MergeContainer); 8] = [
            ("v.mp4", "a.m4a", MergeContainer::Mp4),
            ("v.MP4", "a.M4A", MergeContainer::Mp4),
            ("v.webm", "a.webm", MergeContainer::Webm),
            ("v.webm", "a.weba", MergeContainer::Webm),
            // Пары из разных семейств — только Matroska.
            ("v.mp4", "a.webm", MergeContainer::Mkv),
            ("v.webm", "a.m4a", MergeContainer::Mkv),
            // Незнакомое расширение и его отсутствие — тоже Matroska:
            // «угадать получше» здесь означало бы рискнуть отказом склейки.
            ("v.avi", "a.m4a", MergeContainer::Mkv),
            ("v", "a", MergeContainer::Mkv),
        ];

        for (video, audio, expected) in cases {
            assert_eq!(
                container_for(Path::new(video), Path::new(audio)),
                expected,
                "{video} + {audio}"
            );
        }
    }

    #[test]
    fn the_container_of_every_success_is_the_one_ffmpeg_accepted_live() {
        // Таблица совместимости не выдумана: под каждой её строкой лежит
        // фикстура, где настоящий ffmpeg принял эту пару в этот контейнер
        // с кодом 0.
        for (name, video_ext, audio_ext, container) in SUCCESSES {
            let outcome = fixtures::merge_outcome(name);
            assert_eq!(outcome.exit_code, Some(0), "{name}: фикстура не про успех");
            assert_eq!(
                container_for(
                    Path::new(&format!("v.{video_ext}")),
                    Path::new(&format!("a.{audio_ext}"))
                ),
                container,
                "{name}: выбранный контейнер разошёлся со снятой фикстурой"
            );
        }
    }

    #[test]
    fn the_working_name_is_never_the_final_name() {
        // Ф-8: под финальным именем недосклеенного не бывает никогда —
        // проще всего это гарантировать тем, что модуль под ним не пишет.
        for container in [
            MergeContainer::Mp4,
            MergeContainer::Webm,
            MergeContainer::Mkv,
        ] {
            let working = working_file_name("Ролик", container);
            let final_name = format!("Ролик.{}", container.extension());

            assert_ne!(working, final_name);
            assert!(
                working.ends_with(&format!(".{}", container.extension())),
                "{working}: расширение обязано остаться последним — по нему \
                 ffmpeg выбирает мультиплексор"
            );
            assert!(working.starts_with("Ролик."));
        }
    }

    #[tokio::test]
    async fn a_captured_success_becomes_a_merged_file_under_the_working_name() {
        for (name, video_ext, audio_ext, container) in SUCCESSES {
            let (dir, video, audio) = workspace(video_ext, audio_ext);
            let launcher = Replay::of(name);

            let verdict = merge_streams(
                &launcher,
                &request(dir.path(), &video, &audio),
                &RunHandle::new(),
            )
            .await;

            let expected = dir.path().join(working_file_name("Ролик", container));
            assert_eq!(
                verdict,
                MergeVerdict::Merged(MergedFile {
                    path: expected.clone(),
                    container
                }),
                "{name}"
            );
            assert!(
                expected.exists(),
                "{name}: результат обязан остаться на диске"
            );
        }
    }

    #[tokio::test]
    async fn every_captured_failure_is_the_single_merge_class() {
        // Решение дизайна по Ф-9 буквально: любая неудача ЭТОГО процесса —
        // «не удалось склеить», без разбора формулировок.
        for (name, exit_code) in FAILURES {
            let (dir, video, audio) = workspace("mp4", "m4a");
            let launcher = Replay::of(name);

            let verdict = merge_streams(
                &launcher,
                &request(dir.path(), &video, &audio),
                &RunHandle::new(),
            )
            .await;

            let failure = failure_of(verdict);
            assert_eq!(failure.kind(), DownloadErrorKind::MergeFailed, "{name}");

            let outcome = fixtures::merge_outcome(name);
            let contract = failure.to_contract(crate::types::PartialData::Kept);

            match contract.details {
                Some(details) => {
                    assert_eq!(details.exit_code, exit_code, "{name}");
                    assert_eq!(
                        details.stderr_tail.is_some(),
                        !outcome.stderr.trim().is_empty(),
                        "{name}: хвост stderr есть ровно тогда, когда процессу \
                         было что сказать"
                    );
                }
                // Единственная фикстура без кода завершения и без единой
                // строки в stderr — убитая сигналом. «Подробнее», за
                // которым пусто, границу команды не пересекает (Н-4), и это
                // не пробел: сказать про такой процесс действительно нечего.
                None => assert_eq!(
                    (exit_code, outcome.stderr.trim()),
                    (None, ""),
                    "{name}: пустое «Подробнее» допустимо только у процесса, \
                     который не оставил ни кода, ни вывода"
                ),
            }
        }
    }

    #[tokio::test]
    async fn a_disk_full_during_the_merge_is_still_a_merge_failure() {
        // Отдельным тестом, потому что соблазн очевиден: в фазе скачивания
        // тот же текст ОС даёт класс `diskFull` (TL-43), и склейке ничего
        // не стоило бы «улучшить» классификацию, посмотрев в stderr. Ф-10
        // относит класс к фазе, а не к тексту: в фазе Merging это «не
        // удалось склеить», и пользователю предлагается повтор, который
        // не будет качать заново.
        let (dir, video, audio) = workspace("mp4", "m4a");
        let launcher = Replay::of("disk-full-mid-merge.json");

        let verdict = merge_streams(
            &launcher,
            &request(dir.path(), &video, &audio),
            &RunHandle::new(),
        )
        .await;

        let failure = failure_of(verdict);
        assert!(fixtures::merge_outcome("disk-full-mid-merge.json")
            .stderr
            .contains("No space left on device"));
        assert_eq!(failure.kind(), DownloadErrorKind::MergeFailed);
    }

    #[tokio::test]
    async fn a_failure_removes_the_unfinished_result_and_keeps_both_streams() {
        // Из шести отказов двое оставили на диске файл: обрубок в 262 байта
        // (`container-refuses-codec`) и 6 078 464 байта, забившие том
        // (`disk-full-mid-merge`). Оба обязаны исчезнуть.
        for (name, _) in FAILURES {
            let outcome = fixtures::merge_outcome(name);
            let (dir, video, audio) = workspace("mp4", "m4a");
            let launcher = Replay::of(name);

            let verdict = merge_streams(
                &launcher,
                &request(dir.path(), &video, &audio),
                &RunHandle::new(),
            )
            .await;
            assert!(matches!(verdict, MergeVerdict::Failed(_)), "{name}");

            let working = dir
                .path()
                .join(working_file_name("Ролик", MergeContainer::Mp4));
            assert!(
                !working.exists(),
                "{name}: недоделанный результат ({:?} байт после настоящего \
                 процесса) остался на диске",
                outcome.output_left_bytes
            );
            assert!(
                video.exists() && audio.exists(),
                "{name}: скачанные потоки трогать нельзя — на них стоит \
                 обещание «повтор пересобирает файл без повторного скачивания»"
            );
        }
    }

    #[tokio::test]
    async fn a_success_without_a_file_on_disk_is_not_a_success() {
        // Код 0 без файла — противоречие, и разбирать его должен тот, кто
        // его увидел, а не TL-44, переименовывающий несуществующее.
        let (dir, video, audio) = workspace("mp4", "m4a");
        let mut launcher = Replay::of("success-mp4-h264-aac.json");
        launcher.outcome.output_left_bytes = None;

        let verdict = merge_streams(
            &launcher,
            &request(dir.path(), &video, &audio),
            &RunHandle::new(),
        )
        .await;

        let failure = failure_of(verdict);
        assert_eq!(failure.kind(), DownloadErrorKind::MergeFailed);
        // Нулевой код завершения в «Подробнее» под заголовком «Не удалось
        // склеить» — противоречие, а не диагностика (прецедент
        // `probe::orchestrate::card`); stderr успешной перепаковки пуст, и
        // показывать пользователю нечего вовсе.
        assert!(failure
            .to_contract(crate::types::PartialData::Kept)
            .details
            .is_none());
    }

    #[tokio::test]
    async fn the_launch_gets_the_merge_timeout_and_the_paths_of_both_streams() {
        let (dir, video, audio) = workspace("mp4", "m4a");
        let launcher = Replay::of("success-mp4-h264-aac.json");

        let verdict = merge_streams(
            &launcher,
            &request(dir.path(), &video, &audio),
            &RunHandle::new(),
        )
        .await;
        assert!(matches!(verdict, MergeVerdict::Merged(_)));

        assert_eq!(launcher.seen_timeout(), MERGE_TIMEOUT);
        let args = launcher.seen_args();
        assert_eq!(
            args.iter().filter(|arg| arg.starts_with("file:")).count(),
            3,
            "все три пути идут с префиксом file: — см. doc merge_args"
        );
        assert!(args.contains(&format!("file:{}", video.display())));
        assert!(args.contains(&format!("file:{}", audio.display())));
        assert_eq!(
            args.last().map(String::as_str),
            Some(
                format!(
                    "file:{}",
                    dir.path()
                        .join(working_file_name("Ролик", MergeContainer::Mp4))
                        .display()
                )
                .as_str()
            )
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_path_that_is_not_valid_utf8_fails_without_starting_a_process() {
        use std::os::unix::ffi::OsStrExt;

        let dir = PathBuf::from(OsStr::from_bytes(b"/tmp/tube-leak-\xff"));
        let video = dir.join("v.mp4");
        let audio = dir.join("a.m4a");

        let verdict = merge_streams(
            &NeverLaunched,
            &request(&dir, &video, &audio),
            &RunHandle::new(),
        )
        .await;

        let failure = failure_of(verdict);
        assert_eq!(failure.kind(), DownloadErrorKind::MergeFailed);
        // Процесса не было — «Подробнее» пустое и границу не пересечёт.
        assert!(failure
            .to_contract(crate::types::PartialData::Kept)
            .details
            .is_none());
    }

    /// Скрипт вместо ffmpeg: создаёт файл результата по последнему
    /// аргументу (сняв префикс `file:`) и дальше ведёт себя как сказано.
    ///
    /// Запускается настоящим [`SidecarFfmpeg`], то есть через
    /// `run_cancellable` с настоящим реестром и настоящей группой
    /// процессов, — иначе тесты реестра и отмены проверяли бы подделку.
    fn write_fake_ffmpeg(dir: &TempDir, name: &str, tail: &str) -> PathBuf {
        let path = dir.path().join(name);
        fs::write(
            &path,
            format!("#!/bin/sh\nfor arg in \"$@\"; do last=\"$arg\"; done\n: > \"${{last#file:}}\"\n{tail}\n"),
        )
        .expect("скрипт-заглушка");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
        path
    }

    /// Ждёт, пока в реестре появится хоть один PID.
    async fn wait_until_registered(registry: &ChildRegistry) -> bool {
        for _ in 0..500 {
            if !registry.is_empty() {
                return true;
            }
            time::sleep(Duration::from_millis(10)).await;
        }
        false
    }

    #[tokio::test]
    async fn the_process_lives_in_the_sidecar_registry_and_leaves_it_on_return() {
        // Критерий TL-42: реестр E1 переиспользуется, собственного учёта
        // PID у склейки нет. Проверяется на настоящем запуске: скрипт
        // держится 30 с, за это время реестр обязан его видеть.
        let dir = tempdir().expect("временный каталог");
        let script = write_fake_ffmpeg(&dir, "slow-ffmpeg.sh", "sleep 30");
        let registry = ChildRegistry::new();
        let launcher = SidecarFfmpeg::new(script, &registry);
        let handle = RunHandle::new();

        let video = dir.path().join("Ролик.f137.mp4");
        let audio = dir.path().join("Ролик.f140.m4a");
        fs::write(&video, b"v").expect("видеопоток");
        fs::write(&audio, b"a").expect("аудиопоток");
        let request = request(dir.path(), &video, &audio);

        let (verdict, registered) =
            tokio::join!(merge_streams(&launcher, &request, &handle), async {
                let seen = wait_until_registered(&registry).await;
                // Дальше держать процесс незачем: убийство — тот же путь, что
                // и у отмены, а предмет этого теста — сама регистрация.
                handle.cancel().await;
                seen
            });

        assert!(
            registered,
            "процесс склейки обязан попасть в реестр sidecar, пока он жив"
        );
        assert_eq!(verdict, MergeVerdict::Cancelled);
        assert!(
            registry.is_empty(),
            "вернувшийся запуск обязан сняться с реестра — иначе выход из \
             приложения будет добивать давно мёртвый PID"
        );
    }

    #[tokio::test]
    async fn cancelling_mid_merge_removes_the_unfinished_result() {
        // С-5 и таблица «Отмена по фазам»: в фазе Merging убивается
        // процесс ffmpeg, а недосклеенный файл удаляется. Что настоящий
        // ffmpeg действительно оставляет его после убийства, снято живьём
        // (`killed-mid-merge`: 75 МБ на убийстве через 0,15 с).
        let dir = tempdir().expect("временный каталог");
        let script = write_fake_ffmpeg(&dir, "slow-ffmpeg.sh", "sleep 30");
        let registry = ChildRegistry::new();
        let launcher = SidecarFfmpeg::new(script, &registry);
        let handle = RunHandle::new();

        let video = dir.path().join("Ролик.f137.mp4");
        let audio = dir.path().join("Ролик.f140.m4a");
        fs::write(&video, b"v").expect("видеопоток");
        fs::write(&audio, b"a").expect("аудиопоток");
        let request = request(dir.path(), &video, &audio);
        let working = dir
            .path()
            .join(working_file_name("Ролик", MergeContainer::Mp4));

        let (verdict, ()) = tokio::join!(merge_streams(&launcher, &request, &handle), async {
            assert!(wait_until_registered(&registry).await);
            // Файл результата к этому моменту уже создан скриптом — как его
            // создаёт настоящий ffmpeg, начав писать.
            handle.cancel().await;
        });

        assert_eq!(verdict, MergeVerdict::Cancelled);
        assert!(
            !working.exists(),
            "после отмены в фазе склейки на диске не должно остаться ничего \
             от неё самой"
        );
        assert!(
            video.exists() && audio.exists(),
            "потоки удаляет оркестрация — она одна знает, что отменена вся \
             задача, а не только склейка"
        );
    }

    #[tokio::test]
    async fn a_process_that_exits_non_zero_is_a_merge_failure_end_to_end() {
        // Тот же путь целиком, но без подмены исхода: настоящий запуск,
        // настоящий ненулевой код.
        let dir = tempdir().expect("временный каталог");
        let script = write_fake_ffmpeg(
            &dir,
            "failing-ffmpeg.sh",
            "echo 'Error muxing a packet' >&2\nexit 234",
        );
        let registry = ChildRegistry::new();
        let launcher = SidecarFfmpeg::new(script, &registry);

        let video = dir.path().join("Ролик.f137.mp4");
        let audio = dir.path().join("Ролик.f140.m4a");
        fs::write(&video, b"v").expect("видеопоток");
        fs::write(&audio, b"a").expect("аудиопоток");
        let working = dir
            .path()
            .join(working_file_name("Ролик", MergeContainer::Mp4));

        let verdict = merge_streams(
            &launcher,
            &request(dir.path(), &video, &audio),
            &RunHandle::new(),
        )
        .await;

        let failure = failure_of(verdict);
        assert_eq!(failure.kind(), DownloadErrorKind::MergeFailed);
        let details = failure
            .to_contract(crate::types::PartialData::Kept)
            .details
            .expect("код завершения и хвост stderr");
        assert_eq!(details.exit_code, Some(234));
        assert_eq!(
            details.stderr_tail.as_deref(),
            Some("Error muxing a packet")
        );
        assert!(!working.exists(), "обрубок обязан быть убран");
        assert!(registry.is_empty());
    }
}
