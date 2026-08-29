//! Smoke-проверка подготовленной установки yt-dlp — последний рубеж перед
//! переключением активной записи (Ф-6 эпика E6, задача TL-57).
//!
//! # Место в конвейере
//!
//! Порядок Ф-6 буквальный: кандидат скачан и распакован
//! ([`super::fetch`]) → **запущен здесь** → и только после успеха
//! оркестрация (TL-58) делает его активным через
//! [`super::state::InstallState::activate`]. Внутри `activate` этой
//! проверки нет намеренно (см. её doc), и здесь нет переключения: модуль
//! отвечает на один вопрос — «эта установка вообще запускается и
//! называет ту версию, которую обещали метаданные?».
//!
//! # Чего проверка не гарантирует
//!
//! Того, что держит не она:
//!
//! - **дерево на месте и сходится с манифестом** — это
//!   [`super::layout::validate`], и она уже отработала внутри
//!   установки; smoke запускает то, что ей дали, и о содержимом дерева
//!   ничего не знает;
//! - **убийство зависшего процесса и снятие его с учёта** — это
//!   [`crate::sidecar::run`] (группа процессов, реестр PID, TL-10). Здесь
//!   только назначается срок;
//! - **отсутствие мусора после убийства по сроку.** Это свойство
//!   onedir-поставки, а не наша уборка. Замер (Apple Silicon, macOS 26.6,
//!   настоящее дерево `yt-dlp_macos` из пина): холодный `--version`,
//!   убитый `kill -9` по группе на пятой секунде, не оставил в `$TMPDIR`
//!   ни одной новой записи — ни `_MEIxxxxxx`, ни чего-либо ещё, до и
//!   после убийства список каталога совпадал побайтово. Так и должно
//!   быть: `_MEI*` разворачивал **однофайловый** бутлоадер PyInstaller,
//!   от которого проект ушёл в TL-12 (дефект #13); onedir держит своё
//!   `_internal` рядом с исполняемым файлом и во временный каталог не
//!   пишет. Появись такой мусор снова (смена формата поставки апстримом)
//!   — убирать его придётся здесь, и этот абзац станет неверным раньше,
//!   чем код.
//!
//! # Откуда взят таймаут
//!
//! Ф-6 просит «короткий таймаут по образцу существующей пробы», и взять
//! буквально [`super::prepare`]`::PROBE_TIMEOUT` (5 с) было бы прямым
//! повторением дефекта #13: пять секунд — это цена запуска **тёплого**
//! дерева, а кандидат приходит сюда холодным. Холодным по построению —
//! [`super::fetch::fetch_and_install`] распаковывает дерево и на этом
//! заканчивает, прогрева в нём нет ни одного, — и, что важнее,
//! «тёплость» вообще не наблюдаема снаружи: она принадлежит кэшу
//! `syspolicyd`, а не нам (тот же довод, по которому подготовка не
//! записывает флаг «уже прогрето»).
//!
//! Замеры на настоящем дереве из пина (Apple Silicon, macOS 26.6,
//! `/usr/bin/time -p`, `real`), TL-57:
//!
//! | состояние дерева                              | `--version` |
//! |-----------------------------------------------|-------------|
//! | только что распакованное (холодное)           | 24,07 с     |
//! | оно же, три следующих запуска                 | 0,24–0,25 с |
//!
//! Сходится с TL-12 (24,58 / 25,33 / 36,37 с холодных) и с диагностикой
//! #13 (27,8–39,1 с). Поэтому срок здесь — тот же, что у прогрева
//! ([`super::prepare::WARMUP_TIMEOUT`]), и равенство сторожится на этапе
//! компиляции: это одна и та же цена одного и того же явления, и
//! разъехаться этим двум значениям нечем.
//!
//! Асимметрия цены ошибки решает вопрос окончательно. Срок — это
//! **потолок**, а не ожидание: тёплое дерево отвечает за четверть
//! секунды и возвращает управление сразу, так что длинный потолок в
//! норме не стоит ничего. Слишком короткий стоит несравнимо больше:
//! исправное обновление объявляется непригодным, а вместе с этим (см.
//! ниже) навсегда попадает в запрет — то есть контур обновления
//! перестаёт работать вовсе, и никто этого не заметит, потому что
//! пользователю такой отказ не показывается как поломка (С-5).
//!
//! Прогревом эта проверка при этом не становится: она не эмитит
//! прогресса, не отправляет событий и не считает файлы. Она просто
//! оплачивает первый запуск, если за него ещё не платили. Отсюда
//! обязанность вызывающего: Н-3 запрещает греть дерево во время активной
//! загрузки ролика, и держать это — оркестрации (TL-58), потому что про
//! идущие задачи известно ей, а не этому модулю.
//!
//! # Почему провал закрывает дорогу навсегда, а не «на сутки»
//!
//! С-5 требует буквально: тот же build id не устанавливается повторно до
//! появления **причины** — нового релиза или действия пользователя.
//! Времени в этом списке нет, и это отличие от журнала починок
//! ([`super::prepare`]) осознанное. Там остывание по [`REPAIR_COOLDOWN`]
//! необходимо: чинится **единственная** установка, без которой
//! приложение не работает, и вечный отказ поймал бы в ловушку любого,
//! кто устранил причину. Здесь всё наоборот — активная установка цела и
//! работает, а повтор стоит полного круга: шестьдесят мегабайт из сети,
//! распаковка ста тридцати файлов и тридцать секунд дисковой работы
//! ради заведомо того же исхода. Ровно этот узор — операция, стабильно
//! упирающаяся в таймаут и повторяющаяся по расписанию, — уже записан в
//! долг #22, и повторять его здесь нечем оправдать.
//!
//! Выходов из запрета два, оба из С-5: новый релиз апстрима — это другой
//! build id, то есть другой файл журнала и чистая история; действие
//! пользователя — [`forget`].
//!
//! [`REPAIR_COOLDOWN`]: super::prepare

use std::path::Path;
use std::time::Duration;

use crate::sidecar::{self, ChildRegistry, SidecarError};
use crate::types::YtDlpUpdateFailure;

use super::fetch::PreparedCandidate;
use super::layout::{BuildId, Layout, RepairLog};

/// Аргументы проверки. Те же, что у прогрева и пробы: вопрос «ты
/// запускаешься и какая ты версия?» задаётся в приложении одним
/// способом.
const SMOKE_ARGS: &[&str] = &["--version"];

/// Сколько ждать ответа. Обоснование — в doc модуля; коротко: кандидат
/// приходит холодным, а холодный запуск стоит 24–36 с.
const SMOKE_TIMEOUT: Duration = super::prepare::WARMUP_TIMEOUT;

// Сторож равенства: если цена холодного запуска когда-нибудь изменится,
// её пересматривают в одном месте, а не в двух. Разъехавшись, эти
// значения означали бы, что прогрев считает дерево живым, а проверка —
// уже нет (или наоборот), причём на одной и той же машине.
const _: () = assert!(SMOKE_TIMEOUT.as_secs() == super::prepare::WARMUP_TIMEOUT.as_secs());

/// Сколько проваленных запусков нужно, чтобы больше не пробовать.
///
/// Один, в отличие от [`super::prepare`]`::MAX_REPAIR_ATTEMPTS` (два), и
/// разница ровно в том, есть ли альтернатива. Там второй заход — шанс
/// для единственной установки, без которой приложение не работает.
/// Здесь альтернатива есть и она работает прямо сейчас, а второй заход
/// стоит полного круга «скачать, распаковать, прогреть».
const MAX_SMOKE_ATTEMPTS: u32 = 1;

/// Сколько символов чужого ответа попадает в диагностику.
///
/// Потолок нужен потому, что ответ — это вывод чужого процесса, а не
/// строка версии: разбор берёт первый токен первой строки, но токен
/// длиной в мегабайт остаётся токеном. Шестьдесят четыре — вдвое больше
/// потолка длины самой версии (`MAX_VERSION_CHARS`), то есть любую
/// настоящую версию видно целиком, а всё, что длиннее, интересно только
/// фактом «это не версия».
const MAX_ANSWER_CHARS: usize = 64;

/// Почему подготовленная установка не годится к переключению.
///
/// Четыре варианта против одного класса контракта — и это не
/// расхождение. Наружу (Ф-9) все четыре уходят одним
/// [`YtDlpUpdateFailure::SmokeCheckFailed`]: пользователю они предлагают
/// одно и то же (ничего, работа продолжается на активной версии), и
/// плодить ради них варианты в TS-зеркале значило бы обещать различие,
/// которого на экране нет. Внутри же разница есть, и она проверяемая:
/// тест, доказывающий «отвечает не тем», не должен ловить эту ветку
/// подстрокой в сообщении.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SmokeError {
    /// Не ответил за отведённое время; процесс убит по группе.
    #[error("yt-dlp {version} не ответил на `--version` за {}{}", secs(*.timeout_ms), tail(.stderr))]
    NoAnswer {
        version: String,
        timeout_ms: u64,
        stderr: String,
    },

    /// Ответил, но назвал не ту версию — или не назвал версии вовсе.
    ///
    /// Пустой и неразбираемый вывод сюда же, а не в отдельный вариант:
    /// «ответил не тем» — это про то, что ожидаемой версии в ответе нет,
    /// а по какой причине её там нет, лекарства не меняет.
    #[error("ожидалась версия yt-dlp {expected}, а установка называет себя «{answered}»")]
    VersionMismatch { expected: String, answered: String },

    /// Не запустился, убит сигналом или завершился с ненулевым кодом.
    #[error("yt-dlp {version} не запускается: {reason}{}", tail(.stderr))]
    Crashed {
        version: String,
        reason: String,
        stderr: String,
    },

    /// Этот build id уже проваливал проверку — повторять нечего (С-5).
    ///
    /// Не «ошибка ошибки», а полноценный исход: снаружи он выглядит тем
    /// же классом Ф-9, потому что и означает то же самое — эта версия не
    /// установлена и не будет.
    #[error(
        "yt-dlp {version} уже не прошёл проверку запуска {at} ({last_reason}) — \
         повторять установку нечем: до нового релиза апстрима или действия \
         пользователя этот выпуск пропускается"
    )]
    AlreadyFailed {
        version: String,
        at: String,
        last_reason: String,
    },
}

impl SmokeError {
    /// Проекция на контракт Ф-9 — один класс на все варианты.
    ///
    /// Версия берётся из самой ошибки, а не приходит параметром (в
    /// отличие от [`super::fetch::FetchError::to_failure`], где её знает
    /// только вызывающий): здесь её знает **каждый** вариант, и второй
    /// источник той же строки — это способ однажды показать не ту.
    pub fn to_failure(&self) -> YtDlpUpdateFailure {
        YtDlpUpdateFailure::SmokeCheckFailed {
            version: self.version().to_string(),
            message: self.to_string(),
        }
    }

    /// О какой версии речь.
    fn version(&self) -> &str {
        match self {
            Self::NoAnswer { version, .. }
            | Self::Crashed { version, .. }
            | Self::AlreadyFailed { version, .. } => version,
            // У этого варианта версия названа тем полем, которое и есть
            // предмет спора: ожидали `expected`, а получили что угодно.
            Self::VersionMismatch { expected, .. } => expected,
        }
    }

    /// Пробовали ли на самом деле запускать. Нужно вызывающему для лога:
    /// «проверили и не вышло» и «даже не пробовали» — разные строки, и
    /// различать их по тексту сообщения было бы гаданием.
    #[allow(dead_code)] // Читает оркестрация (TL-58).
    pub fn was_attempted(&self) -> bool {
        !matches!(self, Self::AlreadyFailed { .. })
    }
}

/// Хвост stderr в скобках — или ничего, если хвоста нет.
///
/// Отдельная функция, потому что её зовёт `#[error(...)]` двух вариантов;
/// stderr наружу идёт только так, свёрнутым в «Подробнее» (Н-4 E2, Ф-9).
fn tail(stderr: &str) -> String {
    sidecar::stderr_tail(stderr).map_or_else(String::new, |tail| format!(" (stderr: {tail})"))
}

/// Срок в секундах с десятой долей.
///
/// Не целочисленное деление на тысячу: боевое значение делится нацело, а
/// тестовое (двести миллисекунд) превратилось бы в «за 0 с» — сообщение,
/// которое сообщает неправду о том, сколько ждали.
fn secs(ms: u64) -> String {
    format!("{:.1} с", ms as f64 / 1000.0)
}

/// Запускает подготовленную установку и сверяет её ответ с ожидаемой
/// версией.
///
/// `Ok(())` означает ровно одно: процесс завершился нулевым кодом и
/// назвал ту версию, которую обещали метаданные релиза. Всё остальное —
/// [`SmokeError`], и любой отказ, кроме [`SmokeError::AlreadyFailed`],
/// попадает в журнал запрета рядом с установкой.
pub async fn smoke_check(
    candidate: &PreparedCandidate,
    layout: &Layout,
    registry: &ChildRegistry,
) -> Result<(), SmokeError> {
    smoke_check_with(candidate, layout, registry, SMOKE_TIMEOUT).await
}

/// То же с явным сроком — существует ради тестов: ветку «не отвечает»
/// иначе пришлось бы воспроизводить двухминутным ожиданием в каждом
/// прогоне `cargo test`. Боевой путь всегда берёт [`SMOKE_TIMEOUT`].
async fn smoke_check_with(
    candidate: &PreparedCandidate,
    layout: &Layout,
    registry: &ChildRegistry,
    timeout: Duration,
) -> Result<(), SmokeError> {
    // Запрет — до запуска, а не после: смысл журнала в том, чтобы не
    // платить за заведомо тот же исход, а платит здесь не только этот
    // запуск (см. `previous_failure`).
    if let Some(previous) = previous_failure(layout, &candidate.build_id) {
        return Err(SmokeError::AlreadyFailed {
            version: candidate.version.clone(),
            at: describe_when(&previous),
            last_reason: previous.last_reason,
        });
    }

    let outcome = run_and_judge(&candidate.executable, &candidate.version, registry, timeout).await;

    if let Err(error) = &outcome {
        record_failure(layout, &candidate.build_id, error);
    }

    outcome
}

/// Запуск и разбор его исхода — три класса Ф-6 («не отвечает», «отвечает
/// не тем», «падает») и ни одного обращения к диску.
///
/// Отдельно от [`smoke_check_with`] затем, чтобы запись в журнал не могла
/// случиться на пути, где запуска не было: [`SmokeError::AlreadyFailed`]
/// эта функция не возвращает по построению.
async fn run_and_judge(
    executable: &Path,
    expected: &str,
    registry: &ChildRegistry,
    timeout: Duration,
) -> Result<(), SmokeError> {
    match sidecar::run(executable, SMOKE_ARGS, timeout, registry).await {
        Ok(output) => {
            // `raw`, а не `display`: у yt-dlp они совпадают (разбор
            // ничего не нормализует — см. doc `parse_ytdlp_version`), и
            // сверять надо ровно то, что бинарник вывел, а не то, что мы
            // из этого поняли.
            match sidecar::parse_ytdlp_version(&output.stdout) {
                Some(version) if version.raw == expected => Ok(()),
                Some(version) => Err(SmokeError::VersionMismatch {
                    expected: expected.to_string(),
                    answered: clip(&version.raw),
                }),
                // Вывода нет вовсе или в нём нет ни одного токена. Это та
                // же ветка «отвечает не тем»: ожидаемой версии в ответе
                // нет, а чем именно она заменена — пустотой или чужой
                // строкой, — лекарства не меняет.
                None => Err(SmokeError::VersionMismatch {
                    expected: expected.to_string(),
                    answered: "(ничего не вывел)".to_string(),
                }),
            }
        }
        Err(SidecarError::Timeout { ms, stderr }) => Err(SmokeError::NoAnswer {
            version: expected.to_string(),
            timeout_ms: ms,
            stderr,
        }),
        Err(error) => {
            let stderr = match &error {
                SidecarError::LaunchFailed { stderr, .. }
                | SidecarError::NonZeroExit { stderr, .. } => stderr.clone(),
                // `NotFound` и `Timeout` сюда не приходят: первый stderr
                // не имеет вовсе (процесс не стартовал), второй разобран
                // веткой выше.
                _ => String::new(),
            };
            Err(SmokeError::Crashed {
                version: expected.to_string(),
                reason: error.to_string(),
                stderr,
            })
        }
    }
}

/// Запись о том, что этот build id уже проваливал запуск, — или `None`,
/// если пробовать можно.
///
/// Публична не «на будущее»: запрет обязан работать раньше самой
/// проверки. К моменту, когда smoke могла бы отказать сама, шестьдесят
/// мегабайт уже приняты из сети и распакованы на диск, то есть С-5 («тот
/// же build id не устанавливается повторно») исполнен не был. Спросить
/// **до** скачивания может только тот, кто скачивание и затевает, — то
/// есть оркестрация.
#[allow(dead_code)] // Спрашивает оркестрация (TL-58) до скачивания.
pub fn previous_failure(layout: &Layout, build_id: &BuildId) -> Option<RepairLog> {
    let log = RepairLog::read(&layout.smoke_path(build_id));
    (log.attempts >= MAX_SMOKE_ATTEMPTS).then_some(log)
}

/// Снимает запрет с build id — «действие пользователя» из С-5.
///
/// Единственный выход из запрета, кроме нового релиза апстрима (у того
/// другой build id и, значит, чистая история). Кто именно его зовёт —
/// ручная проверка обновлений или ручной откат, — решает TL-58; здесь
/// объявлено, что такой выход есть, потому что без него запрет вечен, а
/// С-5 обещает обратное.
#[allow(dead_code)] // Зовёт ручное действие пользователя (TL-58).
pub fn forget(layout: &Layout, build_id: &BuildId) {
    RepairLog::clear(&layout.smoke_path(build_id));
}

/// Пишет факт провала рядом с установкой.
///
/// Неудача записи не меняет исхода проверки — она и так провалена, — но
/// обязана быть заметна в логе, и не из аккуратности: не записавшийся
/// запрет означает, что следующая проверка обновлений скачает и
/// распакует тот же кандидат заново.
fn record_failure(layout: &Layout, build_id: &BuildId, error: &SmokeError) {
    let path = layout.smoke_path(build_id);
    let history = RepairLog::read(&path);
    let attempted = history.with_attempt(&error.to_string(), crate::clock::now_unix_secs());

    if let Err(err) = attempted.write_atomic(&path) {
        eprintln!(
            "yt-dlp: не удалось записать провал проверки запуска {build_id}: {err} — \
             запрет не сохранён, тот же кандидат будет скачан снова"
        );
    }
}

/// Когда была прошлая попытка — человекочитаемо и без вранья, если
/// штампа нет.
fn describe_when(log: &RepairLog) -> String {
    if log.last_attempt_at.is_empty() {
        "ранее".to_string()
    } else {
        log.last_attempt_at.clone()
    }
}

/// Обрезает чужой ответ до [`MAX_ANSWER_CHARS`] символов, по символам, а
/// не по байтам: обрезка UTF-8 посередине символа — паника.
fn clip(answer: &str) -> String {
    if answer.chars().count() <= MAX_ANSWER_CHARS {
        return answer.to_string();
    }

    let head: String = answer.chars().take(MAX_ANSWER_CHARS).collect();
    format!("{head}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    use tempfile::{tempdir, TempDir};

    use crate::ytdlp::layout::ArchiveIdentity;
    use crate::ytdlp::state::{self, InUse, InstallEntry, InstallState};

    /// Версия, которую обещали метаданные релиза, и сумма, которой этот
    /// кандидат адресуется. Заведомо новее пина — не потому, что код их
    /// где-то сравнивает (он не сравнивает нигде), а чтобы читатель не
    /// гадал, кто кому предшественник.
    const CANDIDATE_VERSION: &str = "2030.01.01";
    const CANDIDATE_SHA: &str = "beef0123456789abcdef0123456789abcdef0123456789abcdef0123456789ab";

    const EXECUTABLE_NAME: &str = "yt-dlp_fake";

    /// Каждый скрипт первым делом оставляет отметку «меня запускали».
    /// Именно она, а не текст сообщения, доказывает, что запрет
    /// действительно **не пускает к запуску**, а не просто переписывает
    /// исход после него.
    const MARK_RUN: &str = "touch \"$0.ran\"\n";

    fn script(body: &str) -> String {
        format!("#!/bin/sh\n{MARK_RUN}{body}")
    }

    fn prints(version: &str) -> String {
        script(&format!("echo {version}\n"))
    }

    struct Fixture {
        _dir: TempDir,
        data_dir: PathBuf,
        registry: ChildRegistry,
    }

    fn fixture() -> Fixture {
        let dir = tempdir().expect("tempdir");
        let data_dir = dir.path().join("app-data");
        let fixture = Fixture {
            _dir: dir,
            data_dir,
            registry: ChildRegistry::new(),
        };
        fixture.layout().create_root().expect("создать корень");
        fixture
    }

    impl Fixture {
        fn layout(&self) -> Layout {
            Layout::new(&self.data_dir)
        }

        fn identity(&self) -> ArchiveIdentity<'static> {
            ArchiveIdentity {
                version: CANDIDATE_VERSION,
                sha256: CANDIDATE_SHA,
            }
        }

        fn build_id(&self) -> BuildId {
            self.identity()
                .build_id()
                .expect("образец обязан проходить проверку идентификатора")
        }

        /// Раскладывает дерево кандидата ровно в том виде, в каком его
        /// оставляет установка (TL-56): каталог `<версия>-<sha12>`,
        /// исполняемый файл в корне, `_internal` рядом.
        fn candidate(&self, body: &str) -> PreparedCandidate {
            let build_id = self.build_id();
            let dir = self.layout().install_dir(&build_id);
            let executable = dir.join(EXECUTABLE_NAME);

            fs::create_dir_all(dir.join("_internal")).expect("создать дерево");
            fs::write(dir.join("_internal/lib.so"), b"pretend-shared-library").expect("библиотека");
            self.write_executable(&executable, body);

            PreparedCandidate {
                build_id,
                version: CANDIDATE_VERSION.to_string(),
                dir,
                executable,
            }
        }

        fn write_executable(&self, path: &Path, body: &str) {
            fs::write(path, body).expect("записать исполняемый файл");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o755))
                    .expect("бит выполнения");
            }
        }

        /// Отметка, которую скрипт оставляет первой же строкой: `$0.ran`
        /// рядом с собой.
        fn run_mark(candidate: &PreparedCandidate) -> PathBuf {
            let mut mark = candidate.executable.clone().into_os_string();
            mark.push(".ran");
            PathBuf::from(mark)
        }

        /// Запускался ли бинарник кандидата с момента последнего
        /// [`Self::forget_that_it_ran`].
        fn was_run(&self, candidate: &PreparedCandidate) -> bool {
            Self::run_mark(candidate).exists()
        }

        fn forget_that_it_ran(&self, candidate: &PreparedCandidate) {
            fs::remove_file(Self::run_mark(candidate)).expect("отметка обязана существовать");
        }

        async fn smoke(&self, candidate: &PreparedCandidate) -> Result<(), SmokeError> {
            self.smoke_within(candidate, Duration::from_secs(30)).await
        }

        async fn smoke_within(
            &self,
            candidate: &PreparedCandidate,
            timeout: Duration,
        ) -> Result<(), SmokeError> {
            smoke_check_with(candidate, &self.layout(), &self.registry, timeout).await
        }

        fn ban(&self, candidate: &PreparedCandidate) -> RepairLog {
            RepairLog::read(&self.layout().smoke_path(&candidate.build_id))
        }

        fn ban_exists(&self, candidate: &PreparedCandidate) -> bool {
            self.layout().smoke_path(&candidate.build_id).exists()
        }
    }

    #[tokio::test]
    async fn an_installation_that_names_the_expected_version_passes() {
        let fixture = fixture();
        let candidate = fixture.candidate(&prints(CANDIDATE_VERSION));

        fixture
            .smoke(&candidate)
            .await
            .expect("установка, назвавшая ожидаемую версию, обязана пройти");

        assert!(fixture.was_run(&candidate), "проверка обязана запускать");
        assert!(
            !fixture.ban_exists(&candidate),
            "успешная проверка не оставляет запрета: иначе первое же обновление \
             закрыло бы дорогу самому себе"
        );
    }

    #[tokio::test]
    async fn an_installation_that_names_someone_elses_version_fails() {
        // Критерий приёмки: подменённый бинарник печатает чужую версию —
        // это обязан быть провал, а не успех. Форма ответа при этом
        // безупречна: нулевой код, одна строка, разбирается как версия.
        let fixture = fixture();
        let candidate = fixture.candidate(&prints("2019.12.31"));

        let error = fixture
            .smoke(&candidate)
            .await
            .expect_err("чужая версия — это провал");

        assert_eq!(
            error,
            SmokeError::VersionMismatch {
                expected: CANDIDATE_VERSION.to_string(),
                answered: "2019.12.31".to_string(),
            }
        );
        assert!(fixture.was_run(&candidate));
        assert_eq!(
            fixture.ban(&candidate).attempts,
            1,
            "провал обязан быть записан, иначе тот же кандидат приедет снова"
        );
    }

    #[tokio::test]
    async fn an_installation_that_never_answers_fails() {
        let fixture = fixture();
        let candidate = fixture.candidate(&script("sleep 30\n"));

        let error = fixture
            .smoke_within(&candidate, Duration::from_millis(200))
            .await
            .expect_err("молчание — это провал");

        assert!(
            matches!(error, SmokeError::NoAnswer { ref version, timeout_ms, .. }
                if version == CANDIDATE_VERSION && timeout_ms == 200),
            "не тот класс: {error:?}"
        );
        assert_eq!(fixture.ban(&candidate).attempts, 1);
    }

    #[tokio::test]
    async fn an_installation_that_exits_nonzero_fails() {
        let fixture = fixture();
        let candidate = fixture.candidate(&script("echo 'GLIBC_2.38 not found' >&2\nexit 3\n"));

        let error = fixture
            .smoke(&candidate)
            .await
            .expect_err("ненулевой код — это провал");

        assert!(
            matches!(error, SmokeError::Crashed { ref version, .. } if version == CANDIDATE_VERSION),
            "не тот класс: {error:?}"
        );
        assert!(
            error.to_string().contains("GLIBC_2.38 not found"),
            "stderr обязан доехать до «Подробнее» — по нему владелец и \
             узнаёт, чем именно сборка не подошла: {error}"
        );
        assert_eq!(fixture.ban(&candidate).attempts, 1);
    }

    #[tokio::test]
    async fn an_installation_that_says_nothing_at_all_fails() {
        let fixture = fixture();
        let candidate = fixture.candidate(&script("exit 0\n"));

        let error = fixture
            .smoke(&candidate)
            .await
            .expect_err("пустой ответ — не ответ");

        assert_eq!(
            error,
            SmokeError::VersionMismatch {
                expected: CANDIDATE_VERSION.to_string(),
                answered: "(ничего не вывел)".to_string(),
            }
        );
    }

    #[tokio::test]
    async fn a_flood_of_output_does_not_become_the_message() {
        let fixture = fixture();
        let candidate = fixture.candidate(&script("printf 'x%.0s' $(seq 1 5000)\necho\n"));

        let error = fixture
            .smoke(&candidate)
            .await
            .expect_err("пять тысяч «x» — не версия");

        let SmokeError::VersionMismatch { answered, .. } = &error else {
            panic!("не тот класс: {error:?}");
        };
        assert_eq!(
            answered.chars().count(),
            MAX_ANSWER_CHARS + 1,
            "ответ обязан быть обрезан вместе с многоточием: {answered}"
        );
        assert!(
            error.to_string().chars().count() < 200,
            "сообщение уходит в лог и в «Подробнее», а не в файл: {error}"
        );
    }

    #[tokio::test]
    async fn a_build_id_that_already_failed_is_not_run_again() {
        // Критерий приёмки: троттлинг. Кандидат провалился, потом «стал
        // исправным» — но повторять его установку контур не обязан и не
        // должен: причины (нового релиза или действия пользователя) не
        // появилось.
        let fixture = fixture();
        let candidate = fixture.candidate(&prints("2019.12.31"));
        fixture.smoke(&candidate).await.expect_err("первый провал");

        fixture.write_executable(&candidate.executable, &prints(CANDIDATE_VERSION));
        fixture.forget_that_it_ran(&candidate);

        let error = fixture
            .smoke(&candidate)
            .await
            .expect_err("тот же build id повторно не проверяется");

        assert!(
            matches!(error, SmokeError::AlreadyFailed { ref version, .. }
                if version == CANDIDATE_VERSION),
            "повтор обязан быть назван вслух, а не пройти молча: {error:?}"
        );
        assert!(
            !fixture.was_run(&candidate),
            "запрет обязан стоять ДО запуска: иначе цена «не пробовать снова» — \
             тот же круг «скачать, распаковать, прогреть»"
        );
        assert!(
            !error.was_attempted(),
            "исход обязан отличаться от честной попытки: их лечат по-разному"
        );
        assert_eq!(
            fixture.ban(&candidate).attempts,
            1,
            "отказ без попытки не считается попыткой"
        );
    }

    #[tokio::test]
    async fn a_manual_action_reopens_the_door() {
        // Второй выход из запрета по С-5 (первый — новый релиз, у него
        // другой build id и, значит, чистая история).
        let fixture = fixture();
        let candidate = fixture.candidate(&prints("2019.12.31"));
        fixture.smoke(&candidate).await.expect_err("первый провал");

        forget(&fixture.layout(), &candidate.build_id);
        fixture.write_executable(&candidate.executable, &prints(CANDIDATE_VERSION));
        fixture.forget_that_it_ran(&candidate);

        fixture
            .smoke(&candidate)
            .await
            .expect("после ручного действия проверка выполняется заново");
        assert!(fixture.was_run(&candidate));
    }

    #[tokio::test]
    async fn the_ban_outlives_the_cleanup_that_removes_the_candidate() {
        // Самое неприятное место всей задачи: провалившийся кандидат не
        // активен и не известно-хорош, то есть уборка (Ф-8) сносит его
        // дерево в ближайший проход. Уедь запрет вместе с деревом —
        // следующая проверка скачает те же шестьдесят мегабайт, и так по
        // расписанию до нового релиза.
        let fixture = fixture();
        let candidate = fixture.candidate(&prints("2019.12.31"));
        fixture.smoke(&candidate).await.expect_err("провал");

        let layout = fixture.layout();
        let mut state = InstallState::default();
        state
            .activate(
                &layout,
                InstallEntry::new("2026.08.19", &"a1".repeat(32)).expect("проверенный образец"),
            )
            .expect("запись обязана сохраняться");

        let report = state::cleanup(&layout, &state, &InUse::new(), &[]);

        assert!(
            report.removed.contains(&candidate.dir),
            "тест бессмыслен, если уборка не тронула кандидата: {report:?}"
        );
        assert!(
            fixture.ban_exists(&candidate),
            "запрет обязан пережить установку, о которой он говорит"
        );
        assert!(
            previous_failure(&layout, &candidate.build_id).is_some(),
            "переживший файл обязан по-прежнему читаться как запрет"
        );
    }

    #[test]
    fn every_reason_becomes_the_single_contract_class() {
        let reasons = [
            SmokeError::NoAnswer {
                version: CANDIDATE_VERSION.to_string(),
                timeout_ms: 120_000,
                stderr: String::new(),
            },
            SmokeError::VersionMismatch {
                expected: CANDIDATE_VERSION.to_string(),
                answered: "2019.12.31".to_string(),
            },
            SmokeError::Crashed {
                version: CANDIDATE_VERSION.to_string(),
                reason: "sidecar exited with non-zero code 3".to_string(),
                stderr: "boom".to_string(),
            },
            SmokeError::AlreadyFailed {
                version: CANDIDATE_VERSION.to_string(),
                at: "2026-08-29T10:00:00Z".to_string(),
                last_reason: "не запускается".to_string(),
            },
        ];

        for reason in reasons {
            let failure = reason.to_failure();
            let YtDlpUpdateFailure::SmokeCheckFailed { version, message } = &failure else {
                panic!("Ф-6 знает ровно один класс отказа, а получен {failure:?}");
            };
            assert_eq!(
                version, CANDIDATE_VERSION,
                "версия обязана называться: строка 12 таблицы состояний её показывает"
            );
            assert!(
                !message.is_empty(),
                "за «Подробнее» обязан стоять текст: {reason:?}"
            );
        }
    }

    #[test]
    fn the_timeout_covers_a_cold_tree_with_room_to_spare() {
        // Сторож против «сократим до пробы»: пять секунд здесь означали
        // бы, что исправное обновление на любой машине объявляется
        // непригодным и попадает в вечный запрет.
        //
        // 36,4 с — худший измеренный холодный запуск (TL-12); 24,07 с —
        // измеренный в TL-57 на настоящем дереве пина. Тройной запас —
        // тот же, которым выбран таймаут прогрева.
        const WORST_COLD_MEASURED_MS: u64 = 36_400;
        assert!(
            SMOKE_TIMEOUT.as_millis() as u64 >= WORST_COLD_MEASURED_MS * 3,
            "срок {} мс не покрывает троекратный худший холодный запуск",
            SMOKE_TIMEOUT.as_millis()
        );
    }
}
