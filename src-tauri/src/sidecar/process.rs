//! Запуск sidecar-бинарника с аргументами, захват вывода и таймаут
//! (Ф-6, Ф-8 эпика E1).
//!
//! Работает напрямую с путём к исполняемому файлу (`&Path`), а не с
//! `crate::sidecar::resolve::resolve_sidecar_path` — это разделение
//! позволяет тестировать запуск/таймаут/классификацию ошибок на временных
//! фикстурных скриптах (`tempfile`) без резолва настоящих sidecar-путей.

use std::ffi::OsStr;
use std::io;
use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::task::JoinHandle;
use tokio::time;

use super::error::SidecarError;
use super::registry::{group_kill_command, ChildRegistry};
use crate::types::LaunchFailedReason;

/// Захваченный вывод успешно завершившегося (`exit code == 0`) процесса.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutput {
    /// stdout, декодированный как UTF-8 лоссово (вывод yt-dlp/ffmpeg не
    /// гарантированно валиден в UTF-8 побайтово, но для строки версии
    /// этого достаточно).
    pub stdout: String,
    /// stderr на тот же лад — обычно пуст при успешном завершении, но
    /// заполняется, если бинарник пишет предупреждения в stderr даже
    /// при коде выхода `0`.
    pub stderr: String,
}

/// Максимальная длина хвоста stderr в Unicode-символах (не байтах).
///
/// По контракту TL-1 для «Подробнее» на служебном экране достаточно
/// «~1000 символов или ~20 строк»; тот же предел действует и для
/// «Подробнее» у разбора ссылки (Н-4 эпика E2) — это одна конвенция
/// приложения, а не два независимых решения, поэтому и константа одна.
pub const STDERR_TAIL_MAX_CHARS: usize = 1000;

/// Обрезает stderr до последних [`STDERR_TAIL_MAX_CHARS`] символов.
///
/// `None`, если после `trim()` не осталось ничего: «Подробнее», за
/// которым пусто, — это состояние без содержания, а не диагностика.
/// Полный поток пишется в лог приложения и на экран не попадает ни в
/// каком состоянии.
pub fn stderr_tail(stderr: &str) -> Option<String> {
    let trimmed = stderr.trim();
    if trimmed.is_empty() {
        return None;
    }

    let char_count = trimmed.chars().count();
    if char_count <= STDERR_TAIL_MAX_CHARS {
        Some(trimmed.to_string())
    } else {
        Some(
            trimmed
                .chars()
                .skip(char_count - STDERR_TAIL_MAX_CHARS)
                .collect(),
        )
    }
}

/// Запускает `program` с аргументами `args`, ждёт завершения не дольше
/// `timeout` и возвращает захваченные stdout/stderr.
///
/// По истечении `timeout` процесс принудительно убивается и возвращается
/// [`SidecarError::Timeout`]. stdout/stderr читаются в отдельных задачах,
/// конкурентно с ожиданием завершения процесса (`Child::wait`) — это
/// исключает дедлок на заполненном пайпе при большом выводе (в отличие от
/// чтения после `wait`).
///
/// Читающие задачи пишут вычитанные байты в общий буфер по мере
/// поступления (а не только по достижении EOF): скрипт с shebang
/// (`#!/bin/sh …`), запущенный как sidecar в тестовых фикстурах, — это
/// интерпретатор, который сам форкает внешние команды (например, `sleep`),
/// и те **наследуют** пишущий конец пайпа. `child.start_kill()` убивает
/// только прямого потомка (интерпретатор); если у него остался живой
/// потомок с открытой копией пишущего конца, пайп не закрывается и чтение
/// до EOF никогда не завершится. Общий буфер снимает эту зависимость: на
/// пути таймаута достаточно взять то, что уже накоплено к моменту
/// убийства, не дожидаясь EOF.
///
/// `timeout` — параметр вызывающего кода (TL-5 подставляет реальные лимиты
/// для yt-dlp/ffmpeg — см. `crate::commands::sidecar`), здесь не
/// захардкожен.
///
/// `registry` — реестр PID выполняющихся процессов (TL-10, см. doc
/// [`super::registry`]): `run` регистрирует спавненный процесс сразу после
/// `spawn` и снимает регистрацию на любом собственном пути завершения
/// (успех, ошибка запуска, таймаут — после явного убийства всей группы).
/// Если приложение выходит, пока `run` ещё не вернул управление, PID
/// остаётся в реестре, и `RunEvent::Exit` в `main.rs` синхронно убивает его
/// группу — единственная надёжная точка после того, как Tauri/tao
/// завершают процесс через `std::process::exit`, обходя Rust `Drop`.
///
/// На Unix процесс спавнится как лидер новой группы (`process_group(0)`) —
/// это то, что делает возможным убийство по группе, а не только по одному
/// PID (см. doc [`super::registry::group_kill_command`], почему одного PID
/// недостаточно для PyInstaller-сборок `yt-dlp`).
pub async fn run(
    program: &Path,
    args: &[&str],
    timeout: Duration,
    registry: &ChildRegistry,
) -> Result<RunOutput, SidecarError> {
    // Собственный одноразовый дескриптор: отменять этот запуск снаружи
    // некому, и `handle` остаётся пустышкой, которая ничего не стоит.
    run_cancellable(program, args, &[], timeout, registry, &RunHandle::new()).await
}

/// То же, что [`run`], плюс переменные окружения `env`, которые
/// **добавляются** к унаследованному окружению приложения (TL-110).
///
/// Добавляются, а не заменяют: окружение приложения несёт системные
/// настройки прокси (`HTTP_PROXY` и родня), а CLAUDE.md требует их
/// уважать; очищенное окружение молча отрезало бы пользователя за
/// корпоративным прокси.
///
/// Потребители — проверка версии deno на служебном экране
/// (`crate::sidecar::DenoEnv`) и оба запуска yt-dlp, чей потомок deno
/// наследует окружение (TL-109): разбор через [`run_cancellable`],
/// скачивание через [`run_streaming`]. Сами переменные ставятся в одном
/// месте — [`sidecar_command`].
pub async fn run_with_env(
    program: &Path,
    args: &[&str],
    env: &[(&str, &OsStr)],
    timeout: Duration,
    registry: &ChildRegistry,
) -> Result<RunOutput, SidecarError> {
    run_cancellable(program, args, env, timeout, registry, &RunHandle::new()).await
}

/// То же, что [`run`], плюс отмена снаружи через [`RunHandle`] (Ф-8 эпика
/// E2): пока процесс жив, владелец `handle` может в любой момент убить его
/// группу, не дожидаясь ни завершения, ни таймаута.
///
/// Отмена — это именно убийство группы процессов (см. doc [`RunHandle`]),
/// а не сброс возвращаемого future: `kill_on_drop` шлёт `SIGKILL` одному
/// прямому потомку, и для двухпроцессной сборки (PyInstaller onefile) этого
/// не хватает — ровно тот дефект, ради которого в E1 появился
/// [`ChildRegistry`]. Отменённый запуск возвращает обычный результат
/// убитого процесса ([`SidecarError::LaunchFailed`] с
/// [`LaunchFailedReason::Corrupted`] — кода завершения у убитого сигналом
/// процесса нет), а отличить отмену от честного отказа вызывающий может по
/// [`RunHandle::was_cancelled`].
///
/// `env` — добавочное окружение, см. doc [`run_with_env`]. Разбор ссылки
/// передаёт сюда окружение deno ([`crate::sidecar::YtDlpJsRuntime`],
/// TL-109): deno — потомок yt-dlp и наследует его окружение.
pub async fn run_cancellable(
    program: &Path,
    args: &[&str],
    env: &[(&str, &OsStr)],
    timeout: Duration,
    registry: &ChildRegistry,
    handle: &RunHandle,
) -> Result<RunOutput, SidecarError> {
    let mut child = sidecar_command(program, args, env)
        .spawn()
        .map_err(classify_spawn_error)?;
    let pid = child.id();
    if let Some(pid) = pid {
        registry.register(pid);
    }
    // Снимает регистрацию на любом пути возврата из `run` ниже (успех,
    // ошибка, таймаут) — см. doc параметра `registry` выше. Не имеет
    // отношения к абрупт-завершению самого процесса приложения: в этом
    // случае `run` вообще не успевает вернуться, `Drop` этого guard'а не
    // выполняется, и PID остаётся в реестре ровно для того, чтобы его
    // подхватил `RunEvent::Exit`.
    let _unregister_on_return = pid.map(|pid| UnregisterGuard { registry, pid });

    // Отмена могла прийти между входом в функцию и `spawn` — тогда убивать
    // было ещё нечего, и убить нужно прямо сейчас. Порядок гарантирован
    // тем, что между `spawn` и этой строкой нет ни одной точки `await`:
    // либо `attach` увидит отмену, либо отмена увидит PID.
    if !handle.attach(pid) {
        if let Some(pid) = pid {
            kill_process_group(pid).await;
        }
    }
    // Снимает с дескриптора право убивать: после возврата из функции
    // процесс уже завершён (сам, по таймауту или по отмене), и его PID
    // операционная система вправе выдать кому-то другому.
    let _finish_on_return = FinishGuard(handle);

    let stdout_pipe = child.stdout.take().expect("stdout must be piped");
    let stderr_pipe = child.stderr.take().expect("stderr must be piped");
    let stdout_task = spawn_reader(stdout_pipe);
    let stderr_task = spawn_reader(stderr_pipe);

    match time::timeout(timeout, child.wait()).await {
        Ok(Ok(status)) => {
            let stdout = collect_reader(stdout_task).await;
            let stderr = collect_reader(stderr_task).await;

            if let Some(error) = classify_exit_status(status, stderr.clone()) {
                return Err(error);
            }

            Ok(RunOutput { stdout, stderr })
        }
        Ok(Err(_wait_error)) => {
            // Практически недостижимо (означало бы, что процессом уже кто-то
            // управлял конкурентно) — не классифицируемая по ENOENT/EACCES
            // ошибка запуска.
            Err(SidecarError::LaunchFailed {
                reason: LaunchFailedReason::Other,
                stderr: String::new(),
            })
        }
        Err(_elapsed) => {
            // Процесс пережил `timeout` — убиваем явно всю его группу, а не
            // только прямой потомок: одиночный `child.start_kill()`
            // (SIGKILL напрямую в PyInstaller-bootloader) не даёт ему
            // шанса переслать сигнал уже форкнутому потомку (см. doc
            // `super::registry::group_kill_command`). На Windows группы нет
            // (`process_group` недоступен вне `cfg(unix)`), но
            // `group_kill_command` там же откатывается на `taskkill /T`,
            // убивающий дерево процессов через собственный учёт ОС — прямой
            // `start_kill()` избыточен в обоих случаях и убран, чтобы не
            // дублировать источники истины.
            if let Some(pid) = pid {
                kill_process_group(pid).await;
            } else {
                // Практически недостижимо: `pid` берётся сразу после
                // успешного `spawn`, до этой точки не пройти без него.
                let _ = child.start_kill();
            }

            // Снимаем то, что читающая задача уже накопила в общем буфере —
            // без ожидания EOF (см. doc `run`): к моменту истечения
            // `timeout` она успела вычитать всё, что реально было в пайпе,
            // а само чтение продолжает жить в фоне (возможно, бесконечно
            // из-за живого потомка) и больше не нужно вызывающей стороне.
            let stderr = stderr_task.snapshot();
            stdout_task.abort();

            Err(SidecarError::Timeout {
                ms: timeout.as_millis() as u64,
                stderr,
            })
        }
    }
}

/// Собирает команду запуска sidecar — единственное место, где задаются
/// общие для всех запусков свойства процесса: пайпы, `kill_on_drop`,
/// собственная группа процессов на Unix (см. doc [`run`]) и добавочное
/// окружение (см. doc [`run_with_env`]).
fn sidecar_command(program: &Path, args: &[&str], env: &[(&str, &OsStr)]) -> Command {
    let mut command = Command::new(program);
    command
        .args(args)
        .envs(env.iter().copied())
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);
    command
}

/// Исход запуска, чей stdout читался построчно ([`run_streaming`]).
///
/// Не `RunOutput`, и разница не косметическая: у долгого процесса stdout
/// уже разобран и выброшен по ходу дела, а ненулевой код завершения —
/// не ошибка запуска, а предмет классификации
/// ([`crate::download::classify`]). Поэтому здесь и код, и stderr едут
/// значением, а `Err` остаётся только за тем, чего не случилось вовсе, —
/// за неудавшимся `spawn`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamedRun {
    /// Код завершения; `None` — процесс убит сигналом (в том числе нами:
    /// отменой или сроком бездействия).
    pub exit_code: Option<i32>,
    /// stderr целиком. Наружу уходит только хвост ([`stderr_tail`]).
    pub stderr: String,
    /// Процесс убит потому, что истёк срок, назначенный вызывающим.
    ///
    /// Отдельный признак, а не догадка по отсутствию кода завершения:
    /// без кода возвращается и процесс, убитый отменой пользователя, и
    /// процесс, снятый сроком, — а решения по ним противоположные.
    pub deadline_expired: bool,
}

/// Запускает `program` и **отдаёт stdout построчно по мере поступления**,
/// вместо того чтобы копить его до конца процесса.
///
/// Нужно ровно там, где вывод процесса — не результат, а ход работы:
/// строки прогресса yt-dlp разбираются и превращаются в события, пока
/// загрузка идёт (Ф-2 эпика E3). [`run`] и [`run_cancellable`] для этого
/// не годятся по устройству: они возвращают вывод одним куском, когда
/// смотреть на него уже поздно.
///
/// # Срок бездействия вместо таймаута
///
/// Общего таймаута у этой функции нет и быть не может: у скачивания нет
/// правдоподобной верхней границы — часовой ролик на медленной сети
/// законно качается часами. Вместо него — **срок бездействия**: момент,
/// до которого обязана прийти следующая строка. Первый срок задаёт
/// `first_deadline`, каждый следующий возвращает сам `on_line`, то есть
/// таймер **перевооружается на каждой строке**, а чем именно считается
/// продвижение, решает вызывающий (для скачивания это принятые байты, а
/// не факт вывода строки, — см. [`crate::download::retry`]).
/// `on_line`, вернувший `None`, снимает срок вовсе.
///
/// Истёкший срок убивает **группу** процессов (см. doc
/// [`super::registry::group_kill_command`]) и поднимает
/// [`StreamedRun::deadline_expired`]. Это не отмена: флаг
/// [`RunHandle::was_cancelled`] при этом не поднимается, и вызывающий
/// различает «сняли сроком» и «отменил пользователь» без гадания.
///
/// # Отмена
///
/// Работает так же, как в [`run_cancellable`], и тем же дескриптором:
/// убийство группы закрывает stdout, чтение упирается в EOF, функция
/// возвращает обычный исход убитого процесса. Отдельной ветки на отмену
/// внутри нет намеренно — она была бы вторым источником правды рядом с
/// уже проверенным механизмом.
///
/// Убивается именно группа, поэтому вместе с yt-dlp уходит и его потомок
/// deno (TL-109): yt-dlp порождает его без новой сессии и группы.
///
/// # Окружение
///
/// `env` добавляется к унаследованному, как у [`run_with_env`]; скачивание
/// передаёт сюда окружение deno ([`crate::sidecar::YtDlpJsRuntime`]).
pub async fn run_streaming(
    program: &Path,
    args: &[&str],
    env: &[(&str, &OsStr)],
    registry: &ChildRegistry,
    handle: &RunHandle,
    first_deadline: Instant,
    on_line: &mut (dyn FnMut(&str) -> Option<Instant> + Send),
) -> Result<StreamedRun, SidecarError> {
    let mut child = sidecar_command(program, args, env)
        .spawn()
        .map_err(classify_spawn_error)?;
    let pid = child.id();
    if let Some(pid) = pid {
        registry.register(pid);
    }
    let _unregister_on_return = pid.map(|pid| UnregisterGuard { registry, pid });

    // Отмена могла прийти между входом в функцию и `spawn` — та же
    // гонка и то же её закрытие, что в `run_cancellable`: между `spawn`
    // и этой строкой нет ни одной точки `await`.
    if !handle.attach(pid) {
        if let Some(pid) = pid {
            kill_process_group(pid).await;
        }
    }
    let _finish_on_return = FinishGuard(handle);

    let stdout_pipe = child.stdout.take().expect("stdout must be piped");
    let stderr_pipe = child.stderr.take().expect("stderr must be piped");
    // stderr копится целиком: он не ход работы, а материал классификации,
    // и нужен только в конце.
    let stderr_task = spawn_reader(stderr_pipe);

    let mut lines = BufReader::new(stdout_pipe).lines();
    let mut deadline = Some(first_deadline);
    let mut deadline_expired = false;

    loop {
        let next = lines.next_line();
        let line = match deadline {
            Some(moment) => match time::timeout_at(time::Instant::from_std(moment), next).await {
                Ok(read) => read,
                Err(_elapsed) => {
                    deadline_expired = true;
                    break;
                }
            },
            None => next.await,
        };

        match line {
            Ok(Some(line)) => deadline = on_line(&line),
            // EOF: процесс закрыл stdout — он завершается сам, отменён
            // или убит.
            Ok(None) => break,
            // Ошибка чтения (разрушенный пайп) — читать больше нечего;
            // решение принимается по коду завершения, как и при EOF.
            Err(_) => break,
        }
    }

    if deadline_expired {
        if let Some(pid) = pid {
            kill_process_group(pid).await;
        } else {
            // Практически недостижимо: `pid` берётся сразу после
            // успешного `spawn`.
            let _ = child.start_kill();
        }
    }

    let status = match time::timeout(EXIT_GRACE, child.wait()).await {
        Ok(Ok(status)) => Some(status),
        // Процесс закрыл stdout, но сам не ушёл. Так ведёт себя только
        // зависший потомок: дальше ждать нечего, убиваем группу.
        Ok(Err(_)) | Err(_) => {
            if let Some(pid) = pid {
                kill_process_group(pid).await;
            }
            child.wait().await.ok()
        }
    };

    // Убитый процесс до EOF на stderr может и не дойти (живой потомок с
    // унаследованной копией пишущего конца — та же ловушка, что описана
    // в doc `run`), поэтому на пути убийства берётся снимок накопленного.
    let stderr = if deadline_expired {
        stderr_task.snapshot()
    } else {
        collect_reader(stderr_task).await
    };

    Ok(StreamedRun {
        exit_code: status.and_then(|status| status.code()),
        stderr,
        deadline_expired,
    })
}

/// Сколько ждать ухода процесса после того, как его stdout закрылся.
///
/// Не таймаут работы (её граница — срок бездействия), а страховка от
/// потомка, который закрыл вывод и завис: у здорового процесса между
/// EOF и `exit` проходят миллисекунды.
const EXIT_GRACE: Duration = Duration::from_secs(60);

/// Убивает всю группу процессов `pid` (см. doc
/// [`super::registry::group_kill_command`], почему группу, а не один PID).
///
/// Best-effort: процесс мог завершиться сам между решением убить и самим
/// убийством — тогда команда просто ничего не найдёт. stdout/stderr
/// подавлены по той же причине, что в [`ChildRegistry::kill_all`]:
/// «No such process» — штатный случай, а не сигнал об ошибке.
async fn kill_process_group(pid: u32) {
    let (program, args) = group_kill_command(pid);
    let _ = Command::new(program)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
}

/// Дескриптор одного запуска [`run_cancellable`], через который его можно
/// отменить снаружи (Ф-8 эпика E2: новая ссылка или очистка поля обязаны
/// завершить идущий разбор).
///
/// Отмена убивает **группу** процессов, а не сбрасывает future запуска:
/// причины — в doc [`super::registry`] (убийства одного PID недостаточно
/// для двухпроцессных сборок) и в doc [`run_cancellable`].
///
/// Дескриптор одноразовый и живёт ровно столько, сколько один запуск.
/// Три состояния сменяются в одну сторону:
///
/// 1. до `spawn` — PID ещё неизвестен, отмена запоминается флагом;
/// 2. процесс жив — отмена немедленно убивает его группу;
/// 3. запуск вернул управление ([`FinishGuard`]) — убивать нечего и
///    **нельзя**: тот же номер PID ОС вправе выдать другому процессу.
#[derive(Debug, Default)]
pub struct RunHandle {
    state: StdMutex<HandleState>,
    /// Будит тех, кто ждёт отмены ([`RunHandle::cancelled`]).
    notify: tokio::sync::Notify,
}

#[derive(Debug, Default)]
struct HandleState {
    /// PID запущенного процесса; `None` — ещё не запущен или запуск не
    /// сообщил PID.
    pid: Option<u32>,
    cancelled: bool,
    /// Запуск вернул управление: PID больше не наш, убивать по нему нельзя.
    finished: bool,
}

impl RunHandle {
    /// Создаёт дескриптор незапущенного процесса.
    pub fn new() -> Self {
        Self::default()
    }

    /// Отменяет запуск: помечает дескриптор отменённым, будит ждущих и,
    /// если процесс уже запущен и ещё не завершён, убивает его группу.
    ///
    /// Возврат из `cancel` означает, что убийство уже **отправлено** (а на
    /// Unix — что `kill(2)` уже отработал): вызывающий может стартовать
    /// следующий процесс, не рискуя оставить два живых сразу.
    ///
    /// # Известное микроокно (осознанно оставлено в E2)
    ///
    /// PID читается под мьютексом, а убивается после его отпускания.
    /// Между этими двумя моментами процесс теоретически может завершиться
    /// сам, а ОС — выдать тот же номер кому-то ещё; тогда убийство уйдёт
    /// не туда. Закрывать окно в E2 нечем без удержания мьютекса через
    /// `await`, а цена сейчас нулевая: процесс один, живёт секунды, и
    /// номера PID на такой дистанции не переиспользуются.
    ///
    /// **В E3 это перестанет быть теоретическим:** отмена скачивания и
    /// склейки переиспользует этот же код при куда большей текучке
    /// процессов (докачки, ретраи, ffmpeg на каждый файл). Там окно
    /// придётся закрыть — например, убийством под собственным
    /// async-мьютексом дескриптора или проверкой, что PID всё ещё наш.
    pub async fn cancel(&self) {
        let pid = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.cancelled = true;
            // После возврата из запуска PID уже не наш — см. doc типа.
            if state.finished {
                None
            } else {
                state.pid
            }
        };

        self.notify.notify_waiters();

        if let Some(pid) = pid {
            kill_process_group(pid).await;
        }
    }

    /// Была ли запрошена отмена. Так вызывающий отличает «процесс убит
    /// нами» от «процесс отказал сам»: убитый сигналом процесс возвращает
    /// обычный [`SidecarError::LaunchFailed`], неотличимый по значению.
    pub fn was_cancelled(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .cancelled
    }

    /// Ждёт отмены этого запуска.
    ///
    /// Продакшен-путь ждать отмену не умеет и не должен: настоящий запуск
    /// узнаёт об отмене тем, что его процесс убит. Ожидание нужно
    /// подменяемому запускателю в тестах оркестрации (TL-32) — он
    /// изображает «процесс, который висит, пока его не убьют», не запуская
    /// ничего. Отсюда `cfg(test)`: за пределами тестов у метода
    /// вызывающего нет и быть не должно.
    ///
    /// Регистрация в [`tokio::sync::Notify`] делается **до** проверки
    /// флага: иначе отмена, случившаяся между проверкой и ожиданием, была
    /// бы потеряна, и ожидание не проснулось бы никогда.
    #[cfg(test)]
    pub async fn cancelled(&self) {
        loop {
            let notified = self.notify.notified();
            if self.was_cancelled() {
                return;
            }
            notified.await;
            if self.was_cancelled() {
                return;
            }
        }
    }

    /// Связывает дескриптор с запущенным процессом.
    ///
    /// `false` — отмена пришла раньше запуска, и вызывающий обязан убить
    /// процесс сам: к моменту прихода отмены убивать было нечего.
    fn attach(&self, pid: Option<u32>) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.pid = pid;
        !state.cancelled
    }

    /// Закрывает дескриптор: запуск вернул управление.
    fn finish(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.finished = true;
    }
}

/// RAII-хэлпер: закрывает [`RunHandle`] при любом выходе из
/// [`run_cancellable`] — дальше по этому PID убивать нельзя (см. doc
/// [`RunHandle`], состояние 3).
struct FinishGuard<'a>(&'a RunHandle);

impl Drop for FinishGuard<'_> {
    fn drop(&mut self) {
        self.0.finish();
    }
}

/// RAII-хэлпер: снимает регистрацию `pid` из [`ChildRegistry`] при любом
/// штатном выходе из области видимости `run` (успех, ошибка, ранний
/// `return`) — то есть во всех случаях, где `run` продолжает быть частью
/// обычного потока управления приложения. Не защищает от абрупт-завершения
/// самого процесса приложения (`std::process::exit`) — для этого и
/// существует сам реестр, см. doc [`super::registry`].
struct UnregisterGuard<'a> {
    registry: &'a ChildRegistry,
    pid: u32,
}

impl Drop for UnregisterGuard<'_> {
    fn drop(&mut self) {
        self.registry.unregister(self.pid);
    }
}

/// Фоновая задача, непрерывно вычитывающая поток в общий буфер (лоссовый
/// UTF-8, см. [`RunOutput`]), плюс доступ к этому буферу, не зависящий от
/// завершения самой задачи.
struct ReaderTask {
    join: JoinHandle<()>,
    buf: Arc<StdMutex<Vec<u8>>>,
}

impl ReaderTask {
    /// Прерывает чтение и возвращает то, что уже накоплено в буфере на
    /// данный момент — не дожидаясь EOF (см. doc `run`, зачем это нужно).
    fn snapshot(self) -> String {
        self.join.abort();
        let bytes = self
            .buf
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// Прерывает чтение, не забирая накопленное — используется, когда
    /// поток больше не нужен вызывающей стороне (stdout на пути таймаута).
    fn abort(self) {
        self.join.abort();
    }
}

/// Запускает фоновую задачу, непрерывно вычитывающую поток чанками в общий
/// буфер — в отличие от однократного `read_to_end`, это позволяет забрать
/// уже накопленные байты, даже если задача сама никогда не увидит EOF
/// (живой потомок с унаследованной копией пишущего конца пайпа, см. doc
/// `run`).
fn spawn_reader<R>(mut pipe: R) -> ReaderTask
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    let buf: Arc<StdMutex<Vec<u8>>> = Arc::new(StdMutex::new(Vec::new()));
    let writer_buf = Arc::clone(&buf);

    let join = tokio::spawn(async move {
        let mut chunk = [0u8; 8192];
        loop {
            match pipe.read(&mut chunk).await {
                Ok(0) => break,
                Ok(n) => {
                    let mut guard = writer_buf
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    guard.extend_from_slice(&chunk[..n]);
                }
                // Ошибка чтения (редкая: разрушенный пайп) трактуется как
                // «больше нечего читать» — не блокирует основной поток
                // классификации ошибки.
                Err(_) => break,
            }
        }
    });

    ReaderTask { join, buf }
}

/// Дожидается EOF читающей задачи (процесс уже завершился штатно —
/// `child.wait()` вернул статус, пайп по определению будет закрыт) и
/// возвращает всё, что она накопила. Падение задачи (паника) не мешает
/// забрать уже накопленные в общем буфере байты.
async fn collect_reader(task: ReaderTask) -> String {
    let ReaderTask { join, buf } = task;
    let _ = join.await;
    let bytes = buf.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Классифицирует ошибку `Command::spawn` (ENOENT/EACCES/повреждённый
/// формат исполняемого файла).
fn classify_spawn_error(err: io::Error) -> SidecarError {
    // `spawn` не создал процесс — stderr в принципе не существует.
    match err.kind() {
        io::ErrorKind::NotFound => SidecarError::NotFound,
        io::ErrorKind::PermissionDenied => SidecarError::LaunchFailed {
            reason: LaunchFailedReason::PermissionDenied,
            stderr: String::new(),
        },
        _ => {
            #[cfg(unix)]
            {
                // ENOEXEC: ядро отказалось выполнить файл — не распознан
                // формат исполняемого файла (битый/неполный бинарник,
                // текстовый файл без корректного `#!`-shebang и т.п.).
                const ENOEXEC: i32 = 8;
                if err.raw_os_error() == Some(ENOEXEC) {
                    return SidecarError::LaunchFailed {
                        reason: LaunchFailedReason::Corrupted,
                        stderr: String::new(),
                    };
                }
            }

            SidecarError::LaunchFailed {
                reason: LaunchFailedReason::Other,
                stderr: String::new(),
            }
        }
    }
}

/// POSIX-конвенция большинства shell/exec-реализаций: `126` — «файл найден
/// и помечен исполняемым, но не удалось выполнить его как программу».
///
/// На практике это ровно то, что получает Rust `Command::spawn` на Unix
/// при попытке запустить файл без валидного формата исполняемого файла и
/// без `#!`-shebang: ядро возвращает `ENOEXEC`, а `exec`-семейство внутри
/// libc откатывается на попытку интерпретировать файл как shell-скрипт,
/// которая и проваливается с этим кодом — `ENOEXEC` не долетает до
/// вызывающего кода как `io::Error` (проверено фикстурой ниже), поэтому
/// классифицировать «битый бинарник» приходится по этому коду выхода, а
/// не по `io::ErrorKind`.
const SHELL_NOT_EXECUTABLE_EXIT_CODE: i32 = 126;

/// Классифицирует итог `Child::wait` после успешного `spawn`.
///
/// Возвращает `None`, если процесс завершился успешно (`exit code == 0`).
/// Два случая трактуются как «повреждённый/не сумевший корректно
/// стартовать бинарник» ([`LaunchFailedReason::Corrupted`]), а не как
/// управляемый ненулевой выход:
/// - процесс убит сигналом до контролируемого завершения (на Unix —
///   `status.code().is_none()`);
/// - процесс завершился с [`SHELL_NOT_EXECUTABLE_EXIT_CODE`] — см. её doc.
///
/// В обоих случаях, а также при обычном ненулевом коде выхода, процесс
/// успел стартовать — `stderr`, накопленный к моменту завершения,
/// передаётся в возвращаемую ошибку (см. doc [`run`]).
fn classify_exit_status(status: ExitStatus, stderr: String) -> Option<SidecarError> {
    match status.code() {
        Some(0) => None,
        Some(SHELL_NOT_EXECUTABLE_EXIT_CODE) => Some(SidecarError::LaunchFailed {
            reason: LaunchFailedReason::Corrupted,
            stderr,
        }),
        Some(code) => Some(SidecarError::NonZeroExit { code, stderr }),
        None => Some(SidecarError::LaunchFailed {
            reason: LaunchFailedReason::Corrupted,
            stderr,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::time::Instant;

    use tempfile::tempdir;

    /// Создаёт временный shell-скрипт с заданным содержимым и правами
    /// доступа `mode` (см. `std::os::unix::fs::PermissionsExt`).
    fn write_script(
        dir: &tempfile::TempDir,
        name: &str,
        contents: &str,
        mode: u32,
    ) -> std::path::PathBuf {
        let path = dir.path().join(name);
        fs::write(&path, contents).expect("failed to write fixture script");
        fs::set_permissions(&path, fs::Permissions::from_mode(mode))
            .expect("failed to chmod fixture script");
        path
    }

    /// Почему в скриптах ниже `/bin/echo`, а не встроенный `echo`.
    ///
    /// Встроенный пишет через stdio самой оболочки, а он на пайпе
    /// полностью буферизован: строки выходят одним куском в конце
    /// процесса, и «построчно, пока процесс жив» проверить на таком
    /// скрипте нельзя — проверялось бы обратное. Внешний `/bin/echo` —
    /// отдельный процесс на строку, и его буфер сбрасывается его же
    /// выходом. Настоящего yt-dlp это не касается: `--newline` он несёт
    /// именно затем, чтобы писать построчно.
    const ECHO: &str = "/bin/echo";

    /// Запас на порождение процесса под нагрузкой набора.
    ///
    /// Первый срок и срок между строками — разные величины, и смешивать их
    /// нельзя: первый обязан пережить `sh` плюс `/bin/echo` на загруженной
    /// машине (замерено до 0,9 с, но потолка у этого нет), а второй должен
    /// быть коротким, иначе бездействие нечем поймать. Слей их в одно
    /// число — и тест либо ловит бездействие, либо не флакует, но не то и
    /// другое сразу.
    const SPAWN_ALLOWANCE: Duration = Duration::from_secs(30);

    /// Собирает `on_line`, который складывает строки и держит срок
    /// `silence` от каждой из них.
    fn collector(
        seen: &mut Vec<String>,
        silence: Duration,
    ) -> impl FnMut(&str) -> Option<Instant> + Send + '_ {
        move |line: &str| {
            seen.push(line.to_string());
            Some(Instant::now() + silence)
        }
    }

    #[tokio::test]
    async fn streams_stdout_line_by_line_while_the_process_is_still_alive() {
        // Предмет проверки — именно «пока жив»: `run` отдал бы те же
        // строки, но одним куском в конце, и прогресс скачивания рисовать
        // было бы уже поздно.
        let dir = tempdir().expect("failed to create temp dir");
        let script = write_script(
            &dir,
            "lines.sh",
            &format!("#!/bin/sh\n{ECHO} first\nsleep 0.4\n{ECHO} second\nexit 0\n"),
            0o755,
        );
        let registry = ChildRegistry::new();
        let handle = RunHandle::new();

        let mut seen: Vec<(String, Duration)> = Vec::new();
        let started = Instant::now();
        let mut on_line = |line: &str| -> Option<Instant> {
            seen.push((line.to_string(), started.elapsed()));
            Some(Instant::now() + Duration::from_secs(20))
        };

        let run = run_streaming(
            &script,
            &[],
            &[],
            &registry,
            &handle,
            Instant::now() + SPAWN_ALLOWANCE,
            &mut on_line,
        )
        .await
        .expect("script must spawn");

        assert_eq!(run.exit_code, Some(0));
        assert!(!run.deadline_expired);
        let names: Vec<&str> = seen.iter().map(|(line, _)| line.as_str()).collect();
        assert_eq!(names, ["first", "second"]);
        // Проверяется разрыв, а не абсолютный момент: абсолютный зависит
        // от того, как быстро машина под нагрузкой тестов породила
        // процесс, а разрыв в 0,4 с между строками может появиться только
        // если первая приехала до того, как процесс напечатал вторую.
        // Отдай он их одним куском в конце — разрыва не было бы вовсе.
        let gap = seen[1].1 - seen[0].1;
        assert!(
            gap >= Duration::from_millis(300),
            "строки приехали вместе (разрыв {gap:?}) — значит не построчно"
        );
        assert!(registry.is_empty(), "pid обязан уйти из реестра");
    }

    #[tokio::test]
    async fn kills_the_process_when_it_goes_silent_past_the_deadline() {
        // Сторож С-8 живьём: процесс печатает строку и замолкает, не
        // закрывая stdout, — то же, что делает замерший поток.
        let dir = tempdir().expect("failed to create temp dir");
        let script = write_script(
            &dir,
            "stall.sh",
            &format!("#!/bin/sh\n{ECHO} alive\nsleep 30\n"),
            0o755,
        );
        // Срок между строками — много меньше `sleep 30` внутри скрипта:
        // сработать он может только по бездействию, а не по концу
        // процесса. Порождение процесса он не покрывает — для этого есть
        // отдельный запас.
        const STALL_DEADLINE: Duration = Duration::from_millis(1_500);

        let registry = ChildRegistry::new();
        let handle = RunHandle::new();

        let mut seen = Vec::new();
        let started = Instant::now();
        let run = run_streaming(
            &script,
            &[],
            &[],
            &registry,
            &handle,
            Instant::now() + SPAWN_ALLOWANCE,
            &mut collector(&mut seen, STALL_DEADLINE),
        )
        .await
        .expect("script must spawn");

        assert!(run.deadline_expired, "срок обязан сработать");
        assert!(
            !handle.was_cancelled(),
            "срок — не отмена: их различает вызывающий, и путать их нельзя"
        );
        assert_eq!(seen, ["alive"]);
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "убийство по сроку не должно ждать конца процесса (sleep 30)"
        );
        assert!(registry.is_empty());
    }

    #[tokio::test]
    async fn a_talking_process_never_reaches_its_deadline() {
        // Обратная сторона того же: срок перевооружается на каждой
        // строке, поэтому процесс, который говорит чаще срока, живёт до
        // собственного конца — даже если строк много, а срок короток.
        let dir = tempdir().expect("failed to create temp dir");
        let script = write_script(
            &dir,
            "chatty.sh",
            &format!(
                "#!/bin/sh\ni=0\nwhile [ $i -lt 8 ]; do {ECHO} tick $i; sleep 0.4; i=$((i+1)); done\nexit 0\n"
            ),
            0o755,
        );
        // Срок на строку — меньше, чем весь прогон (8 × 0,4 с ≈ 3,2 с), и
        // больше, чем разрыв между строками даже под нагрузкой набора.
        // Без перевооружения процесс не дожил бы до конца.
        const CHATTY_BUDGET: Duration = Duration::from_millis(2_500);

        let registry = ChildRegistry::new();
        let handle = RunHandle::new();

        let mut seen = Vec::new();
        let started = Instant::now();
        let run = run_streaming(
            &script,
            &[],
            &[],
            &registry,
            &handle,
            Instant::now() + SPAWN_ALLOWANCE,
            &mut collector(&mut seen, CHATTY_BUDGET),
        )
        .await
        .expect("script must spawn");

        assert!(!run.deadline_expired);
        assert_eq!(run.exit_code, Some(0));
        assert_eq!(seen.len(), 8);
        assert!(
            started.elapsed() > CHATTY_BUDGET,
            "прогон обязан быть длиннее одного срока, иначе перевооружение \
             нечем проверить: он прожил {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn cancellation_stops_the_stream_and_is_told_apart_from_the_deadline() {
        let dir = tempdir().expect("failed to create temp dir");
        let script = write_script(
            &dir,
            "long.sh",
            &format!("#!/bin/sh\n{ECHO} started\nsleep 30\n"),
            0o755,
        );
        let registry = ChildRegistry::new();
        let handle = Arc::new(RunHandle::new());

        let canceller = Arc::clone(&handle);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            canceller.cancel().await;
        });

        let mut seen = Vec::new();
        let run = run_streaming(
            &script,
            &[],
            &[],
            &registry,
            &handle,
            Instant::now() + SPAWN_ALLOWANCE,
            &mut collector(&mut seen, SPAWN_ALLOWANCE),
        )
        .await
        .expect("script must spawn");

        assert!(handle.was_cancelled());
        assert!(
            !run.deadline_expired,
            "отмена не должна выглядеть как истёкший срок"
        );
        assert!(registry.is_empty());
    }

    #[tokio::test]
    async fn keeps_stderr_of_a_failed_run_and_reports_its_exit_code() {
        // Ненулевой код здесь не ошибка запуска, а предмет классификации
        // (`crate::download::classify`), поэтому едет значением.
        let dir = tempdir().expect("failed to create temp dir");
        let script = write_script(
            &dir,
            "fail.sh",
            &format!("#!/bin/sh\n{ECHO} out\n{ECHO} 'ERROR: nope' >&2\nexit 3\n"),
            0o755,
        );
        let registry = ChildRegistry::new();
        let handle = RunHandle::new();

        let mut seen = Vec::new();
        let run = run_streaming(
            &script,
            &[],
            &[],
            &registry,
            &handle,
            Instant::now() + SPAWN_ALLOWANCE,
            &mut collector(&mut seen, SPAWN_ALLOWANCE),
        )
        .await
        .expect("script must spawn");

        assert_eq!(run.exit_code, Some(3));
        assert!(run.stderr.contains("ERROR: nope"));
        assert_eq!(seen, ["out"]);
    }

    #[tokio::test]
    async fn a_missing_binary_is_the_only_thing_that_fails_the_call_itself() {
        let dir = tempdir().expect("failed to create temp dir");
        let registry = ChildRegistry::new();
        let handle = RunHandle::new();
        let mut seen = Vec::new();

        let error = run_streaming(
            &dir.path().join("does-not-exist"),
            &[],
            &[],
            &registry,
            &handle,
            Instant::now() + SPAWN_ALLOWANCE,
            &mut collector(&mut seen, SPAWN_ALLOWANCE),
        )
        .await
        .expect_err("missing binary must fail the call");

        assert!(matches!(error, SidecarError::NotFound));
    }

    #[tokio::test]
    async fn returns_stdout_when_the_process_exits_successfully() {
        let dir = tempdir().expect("failed to create temp dir");
        let script = write_script(
            &dir,
            "ok.sh",
            "#!/bin/sh\necho hello-sidecar\nexit 0\n",
            0o755,
        );
        let registry = ChildRegistry::new();

        let output = run(&script, &[], Duration::from_secs(20), &registry)
            .await
            .expect("script must succeed");

        assert_eq!(output.stdout.trim(), "hello-sidecar");
        assert!(
            registry.is_empty(),
            "run must unregister the pid once it returns"
        );
    }

    #[tokio::test]
    async fn forwards_arguments_to_the_spawned_process() {
        let dir = tempdir().expect("failed to create temp dir");
        let script = write_script(
            &dir,
            "echo-args.sh",
            "#!/bin/sh\necho \"$1\"\nexit 0\n",
            0o755,
        );

        let registry = ChildRegistry::new();
        let output = run(&script, &["--version"], Duration::from_secs(20), &registry)
            .await
            .expect("script must succeed");

        assert_eq!(output.stdout.trim(), "--version");
    }

    #[tokio::test]
    async fn passes_extra_environment_on_top_of_the_inherited_one() {
        // Скрипт печатает добавленную переменную и унаследованную `HOME`:
        // первая проверяет, что окружение доходит до процесса, вторая —
        // что оно добавляется, а не заменяет окружение приложения (иначе
        // пропали бы и системные настройки прокси).
        let dir = tempdir().expect("failed to create temp dir");
        let script = write_script(
            &dir,
            "print-env.sh",
            "#!/bin/sh\nprintf '%s\\n%s\\n' \"$TUBE_LEAK_TEST_EXTRA\" \"$HOME\"\nexit 0\n",
            0o755,
        );
        let registry = ChildRegistry::new();

        let output = run_with_env(
            &script,
            &[],
            &[("TUBE_LEAK_TEST_EXTRA", OsStr::new("passed"))],
            Duration::from_secs(20),
            &registry,
        )
        .await
        .expect("script must succeed");

        let inherited_home = std::env::var("HOME").unwrap_or_default();
        let lines: Vec<&str> = output.stdout.lines().collect();
        assert_eq!(lines, ["passed", inherited_home.as_str()]);
    }

    #[tokio::test]
    async fn returns_not_found_when_the_binary_does_not_exist() {
        let dir = tempdir().expect("failed to create temp dir");
        let missing = dir.path().join("does-not-exist");
        let registry = ChildRegistry::new();

        let result = run(&missing, &[], Duration::from_secs(20), &registry).await;

        assert_eq!(result, Err(SidecarError::NotFound));
    }

    #[tokio::test]
    async fn returns_permission_denied_when_the_binary_is_not_executable() {
        let dir = tempdir().expect("failed to create temp dir");
        let script = write_script(&dir, "not-executable.sh", "#!/bin/sh\nexit 0\n", 0o644);
        let registry = ChildRegistry::new();

        let result = run(&script, &[], Duration::from_secs(20), &registry).await;

        assert_eq!(
            result,
            Err(SidecarError::LaunchFailed {
                reason: LaunchFailedReason::PermissionDenied,
                // Отказ на уровне `spawn` — процесс не стартовал, stderr
                // недостижим по определению (см. doc `SidecarError`).
                stderr: String::new(),
            })
        );
    }

    #[tokio::test]
    async fn returns_corrupted_when_the_file_is_not_a_valid_executable_format() {
        let dir = tempdir().expect("failed to create temp dir");
        // Помечен исполняемым, но не является ни валидным бинарником, ни
        // скриптом с `#!`-shebang. На практике это не всплывает как
        // `io::Error` из `spawn` (см. doc `SHELL_NOT_EXECUTABLE_EXIT_CODE`):
        // ядро откатывается на shell-фолбэк, который проваливается с
        // кодом 126.
        let script = write_script(&dir, "garbage", "not a real executable\x00\x01\x02", 0o755);
        let registry = ChildRegistry::new();

        let result = run(&script, &[], Duration::from_secs(20), &registry).await;

        // Здесь процесс успевает стартовать (shell-фолбэк), поэтому в
        // отличие от EACCES/ENOENT-случаев stderr в принципе достижим —
        // но его точное содержимое (сообщение конкретной реализации
        // shell) не является частью контракта, поэтому здесь проверяется
        // только классификация.
        match result {
            Err(SidecarError::LaunchFailed {
                reason: LaunchFailedReason::Corrupted,
                ..
            }) => {}
            other => panic!("expected LaunchFailed{{Corrupted}}, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn returns_corrupted_when_the_process_is_killed_by_a_signal_before_exiting() {
        let dir = tempdir().expect("failed to create temp dir");
        // Стартовал (получил PID), но не завершился контролируемо — упал
        // по сигналу до вызова `exit`; на Unix `ExitStatus::code()` в этом
        // случае возвращает `None`.
        let script = write_script(&dir, "self-signal.sh", "#!/bin/sh\nkill -SEGV $$\n", 0o755);
        let registry = ChildRegistry::new();

        let result = run(&script, &[], Duration::from_secs(20), &registry).await;

        assert_eq!(
            result,
            Err(SidecarError::LaunchFailed {
                reason: LaunchFailedReason::Corrupted,
                // Скрипт не пишет в stderr перед сигналом.
                stderr: String::new(),
            })
        );
    }

    #[tokio::test]
    async fn returns_non_zero_exit_when_the_process_fails() {
        let dir = tempdir().expect("failed to create temp dir");
        let script = write_script(&dir, "fail.sh", "#!/bin/sh\nexit 3\n", 0o755);
        let registry = ChildRegistry::new();

        let result = run(&script, &[], Duration::from_secs(20), &registry).await;

        assert_eq!(
            result,
            Err(SidecarError::NonZeroExit {
                code: 3,
                stderr: String::new(),
            })
        );
    }

    #[tokio::test]
    async fn captures_stderr_when_the_process_fails_with_a_non_zero_exit_code() {
        let dir = tempdir().expect("failed to create temp dir");
        let script = write_script(
            &dir,
            "fail-with-stderr.sh",
            "#!/bin/sh\necho 'error: unsupported URL' >&2\nexit 1\n",
            0o755,
        );
        let registry = ChildRegistry::new();

        let result = run(&script, &[], Duration::from_secs(20), &registry).await;

        assert_eq!(
            result,
            Err(SidecarError::NonZeroExit {
                code: 1,
                stderr: "error: unsupported URL\n".to_string(),
            })
        );
    }

    #[tokio::test]
    async fn returns_timeout_and_kills_the_process_when_it_runs_too_long() {
        let dir = tempdir().expect("failed to create temp dir");
        // Отмечает своё нормальное завершение файлом-маркером *после* сна —
        // так тест ниже отличает «процесс правда убит» от «просто не
        // дождались, а он тем временем осиротело доработал и коснулся маркера».
        let marker = dir.path().join("finished-normally");
        let script = write_script(
            &dir,
            "slow.sh",
            &format!("#!/bin/sh\nsleep 1\ntouch '{}'\n", marker.display()),
            0o755,
        );
        let registry = ChildRegistry::new();

        let started = Instant::now();
        let result = run(&script, &[], Duration::from_millis(100), &registry).await;
        let elapsed = started.elapsed();

        assert_eq!(
            result,
            Err(SidecarError::Timeout {
                ms: 100,
                // Скрипт не успевает ничего вывести перед сном.
                stderr: String::new(),
            })
        );
        assert!(
            elapsed < Duration::from_secs(5),
            "timeout must cut the run short instead of waiting out the full sleep, took {elapsed:?}"
        );

        // Ждём дольше, чем длился бы `sleep 1` в скрипте, если бы процесс
        // не был убит — маркер должен так и не появиться.
        time::sleep(Duration::from_millis(1500)).await;
        assert!(
            !marker.exists(),
            "process must have been killed by the timeout instead of running to completion, \
             found marker at {marker:?}"
        );
        assert!(
            registry.is_empty(),
            "run must unregister the pid even on the timeout path, after killing it"
        );
    }

    /// См. doc [`captures_stderr_written_before_the_process_is_killed_by_a_timeout`]
    /// и аналогичный прогрев в `crate::commands::sidecar` тестах — платит
    /// одноразовый оверхед первого запуска именно этого файла вне
    /// измеряемого окна теста. `script` рассчитан на то, что процесс не
    /// завершается сам — прогрев принудительно убивает его коротким
    /// собственным таймаутом, не дожидаясь EOF.
    async fn warm_up(script: &Path) {
        let registry = ChildRegistry::new();
        let _ = run(script, &[], Duration::from_secs(20), &registry).await;
    }

    #[tokio::test]
    async fn captures_stderr_written_before_the_process_is_killed_by_a_timeout() {
        let dir = tempdir().expect("failed to create temp dir");
        // Пишет в stderr, потом надолго засыпает — таймаут должен убить
        // процесс, но то, что уже попало в пайп до убийства, должно
        // остаться доступным вызывающей стороне.
        //
        // `exec sleep 30`, а не просто `sleep 30` отдельной строкой — это
        // принципиально: обычный `sleep 30` отдельной командой в POSIX-шелле
        // форкает `sleep` отдельным дочерним процессом, который наследует
        // открытый write-конец пайпа stderr; `child.start_kill()` убивает
        // только сам `sh` (прямого ребёнка), а форкнутый `sleep` остаётся
        // жить и продолжает держать пайп открытым — EOF на нашей стороне
        // не наступает, пока не завершится он сам (то есть все 30 секунд).
        // `exec` заменяет образ процесса `sh` на `sleep` (тот же PID, без
        // форка) — тогда `start_kill()` убивает именно тот процесс, что
        // держит пайп, и EOF наступает сразу же после убийства. На практике
        // это уже избыточная подстраховка поверх общего буфера в `run`
        // (см. его doc), который не зависит от EOF вовсе — оставлено, чтобы
        // фикстура была корректна и для читателя, незнакомого с этой
        // деталью реализации.
        let script = write_script(
            &dir,
            "slow-with-stderr.sh",
            "#!/bin/sh\necho 'partial diagnostic output' >&2\nexec sleep 30\n",
            0o755,
        );

        // Прогрев вне измеряемого окна — см. doc `warm_up`: самый первый
        // запуск нового исполняемого файла на некоторых машинах/песочницах
        // несёт одноразовый фиксированный оверхед (несколько сотен
        // миллисекунд — секунды), способный поглотить весь `timeout` ниже
        // до того, как процесс успеет выполнить `echo`.
        warm_up(&script).await;

        let registry = ChildRegistry::new();
        let timeout = Duration::from_millis(300);
        let result = run(&script, &[], timeout, &registry).await;

        assert_eq!(
            result,
            Err(SidecarError::Timeout {
                ms: timeout.as_millis() as u64,
                stderr: "partial diagnostic output\n".to_string(),
            })
        );
    }
    /// Жив ли процесс с таким PID.
    ///
    /// Через `ps`, а не через `kill(2)`: крейта `libc` в графе нет, а
    /// заводить его ради одной проверки в тесте — плохая сделка.
    fn process_is_alive(pid: u32) -> bool {
        std::process::Command::new("ps")
            .arg("-p")
            .arg(pid.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    /// Ждёт, пока процесс исчезнет из таблицы (осиротевшего потомка ещё
    /// должен подобрать и похоронить init) — не дольше пары секунд.
    async fn wait_until_dead(pid: u32) -> bool {
        for _ in 0..200 {
            if !process_is_alive(pid) {
                return true;
            }
            time::sleep(Duration::from_millis(10)).await;
        }
        false
    }

    /// Ждёт, пока скрипт-фикстура запишет PID своего потомка.
    async fn pid_written_by_the_script(path: &Path) -> u32 {
        for _ in 0..600 {
            if let Ok(text) = fs::read_to_string(path) {
                if let Ok(pid) = text.trim().parse::<u32>() {
                    return pid;
                }
            }
            time::sleep(Duration::from_millis(10)).await;
        }
        panic!("скрипт-фикстура так и не сообщил PID своего потомка");
    }

    #[tokio::test]
    async fn cancelling_a_run_kills_the_forked_grandchild_too() {
        // Тот самый дефект, ради которого в E1 появился TL-10: убийство
        // одного прямого потомка (`kill_on_drop`) оставляет форкнутого им
        // внука жить и реродиться на PID 1. Здесь это проверяется на
        // настоящем дереве процессов, а не рассуждением: скрипт форкает
        // долгий `sleep` (это и есть «внук»), сообщает его PID и засыпает
        // сам.
        let dir = tempdir().expect("failed to create temp dir");
        let pid_file = dir.path().join("grandchild.pid");
        let marker = dir.path().join("finished-normally");
        let script = write_script(
            &dir,
            "forks-a-child.sh",
            &format!(
                "#!/bin/sh\nsleep 30 &\necho $! > '{}'\nsleep 30\ntouch '{}'\n",
                pid_file.display(),
                marker.display()
            ),
            0o755,
        );

        let registry = ChildRegistry::new();
        let handle = RunHandle::new();

        let started = Instant::now();
        let (result, grandchild) = tokio::join!(
            // Таймаут заведомо больше, чем живёт скрипт: если бы отмена не
            // работала, тест ждал бы полминуты и упал по времени, а не
            // прошёл бы «за компанию» с таймаутом.
            run_cancellable(
                &script,
                &[],
                &[],
                Duration::from_secs(30),
                &registry,
                &handle
            ),
            async {
                let grandchild = pid_written_by_the_script(&pid_file).await;
                assert!(
                    process_is_alive(grandchild),
                    "внук должен быть жив до отмены — иначе тест ничего не проверяет"
                );
                handle.cancel().await;
                grandchild
            }
        );
        let elapsed = started.elapsed();

        assert!(
            matches!(
                result,
                Err(SidecarError::LaunchFailed {
                    reason: LaunchFailedReason::Corrupted,
                    ..
                })
            ),
            "убитый сигналом процесс не оставляет кода завершения: {result:?}"
        );
        assert!(
            elapsed < Duration::from_secs(20),
            "отмена обязана прервать запуск, а не дождаться его конца: {elapsed:?}"
        );
        assert!(
            handle.was_cancelled(),
            "вызывающий отличает отмену от честного отказа только по этому флагу"
        );
        assert!(
            wait_until_dead(grandchild).await,
            "внук {grandchild} пережил отмену — значит убит был только прямой \
             потомок, и это ровно дефект Ф-2 эпика E1 (осиротевший yt-dlp)"
        );
        assert!(
            !marker.exists(),
            "скрипт не должен был досидеть до конца — он убит, а не дождался"
        );
        assert!(
            registry.is_empty(),
            "отменённый запуск снимается с реестра так же, как убитый по таймауту"
        );
    }

    /// Скрипт, печатающий унаследованные `HOME` и `PATH` построчно.
    ///
    /// Переменные родителя, а не выставленные тестом: `std::env::set_var`
    /// в параллельном наборе меняет окружение всех потоков разом (а
    /// `HTTPS_PROXY` — ещё и соседним тестам контура обновления), тогда
    /// как `HOME` и `PATH` у `cargo test` есть всегда. Системные настройки
    /// прокси доходят до процесса тем же наследованием.
    fn inherited_env_script(dir: &tempfile::TempDir) -> std::path::PathBuf {
        write_script(
            dir,
            "print-inherited.sh",
            "#!/bin/sh\nprintf '%s\\n%s\\n' \"$HOME\" \"$PATH\"\nexit 0\n",
            0o755,
        )
    }

    /// `HOME` и `PATH` этого процесса — то, что обязан унаследовать потомок.
    fn parent_home_and_path() -> Vec<String> {
        let home = std::env::var("HOME").expect("у cargo test задан HOME");
        let path = std::env::var("PATH").expect("у cargo test задан PATH");
        assert!(!path.is_empty(), "пустой PATH ничего не доказывает");
        vec![home, path]
    }

    #[tokio::test]
    async fn a_cancellable_run_without_extra_env_inherits_the_parent_environment() {
        // Остаток ревью TL-110: окружение добавляется к унаследованному, а
        // не заменяет его, и на пути разбора тоже. Мутация `env_clear()` в
        // `sidecar_command` обязана это ронять.
        let dir = tempdir().expect("failed to create temp dir");
        let script = inherited_env_script(&dir);
        let registry = ChildRegistry::new();

        let output = run_cancellable(
            &script,
            &[],
            &[],
            Duration::from_secs(20),
            &registry,
            &RunHandle::new(),
        )
        .await
        .expect("script must succeed");

        let lines: Vec<String> = output.stdout.lines().map(str::to_string).collect();
        assert_eq!(lines, parent_home_and_path());
    }

    #[tokio::test]
    async fn a_streaming_run_without_extra_env_inherits_the_parent_environment() {
        // То же для пути скачивания.
        let dir = tempdir().expect("failed to create temp dir");
        let script = inherited_env_script(&dir);
        let registry = ChildRegistry::new();
        let handle = RunHandle::new();

        let mut seen = Vec::new();
        let run = run_streaming(
            &script,
            &[],
            &[],
            &registry,
            &handle,
            Instant::now() + SPAWN_ALLOWANCE,
            &mut collector(&mut seen, SPAWN_ALLOWANCE),
        )
        .await
        .expect("script must spawn");

        assert_eq!(run.exit_code, Some(0));
        assert_eq!(seen, parent_home_and_path());
    }

    #[tokio::test]
    async fn cancelling_a_streaming_run_kills_the_forked_grandchild_too() {
        // TL-109: так yt-dlp держит deno — потомок в той же группе
        // процессов (yt-dlp порождает его без новой сессии) и со своими
        // пайпами, а не с нашим stdout. Отмена скачивания обязана убить и
        // его, а не только прямого потомка.
        //
        // `exec sleep`: прямой потомок сам не держит ничего, кроме
        // собственного stdout, поэтому убийство одного PID вместо группы
        // даёт EOF и тест доходит до проверки внука, а не висит. Если
        // отмена не убьёт ничего, процесс снимет срок бездействия — и тест
        // покраснеет на `deadline_expired`, а не будет ждать `sleep 300`.
        let dir = tempdir().expect("failed to create temp dir");
        let pid_file = dir.path().join("grandchild.pid");
        let script = write_script(
            &dir,
            "streams-and-forks.sh",
            &format!(
                "#!/bin/sh\nsleep 300 >/dev/null 2>&1 &\necho $! > '{}'\n{ECHO} started\nexec sleep 300\n",
                pid_file.display()
            ),
            0o755,
        );
        let registry = ChildRegistry::new();
        let handle = RunHandle::new();

        let mut seen = Vec::new();
        let mut on_line = collector(&mut seen, SPAWN_ALLOWANCE);
        let (run, grandchild) = tokio::join!(
            run_streaming(
                &script,
                &[],
                &[],
                &registry,
                &handle,
                Instant::now() + SPAWN_ALLOWANCE,
                &mut on_line,
            ),
            async {
                let grandchild = pid_written_by_the_script(&pid_file).await;
                assert!(
                    process_is_alive(grandchild),
                    "внук должен быть жив до отмены — иначе тест ничего не проверяет"
                );
                handle.cancel().await;
                grandchild
            }
        );
        let run = run.expect("script must spawn");

        assert!(handle.was_cancelled());
        assert!(
            !run.deadline_expired,
            "процесс снят сроком, а не отменой — отмена не убила ничего"
        );
        assert_eq!(
            run.exit_code, None,
            "убитый сигналом процесс кода не оставляет"
        );
        assert!(
            wait_until_dead(grandchild).await,
            "внук {grandchild} пережил отмену скачивания — так пережил бы её и deno"
        );
        assert!(registry.is_empty());
    }

    #[tokio::test]
    async fn a_cancellation_that_arrives_before_the_spawn_kills_the_process_at_once() {
        // Единственная ветка отмены, порядок в которой держится ручным
        // рассуждением («между `spawn` и `attach` нет ни одной точки
        // `await`»): отмена пришла, когда убивать было ещё нечего. Процесс
        // всё равно обязан не пережить свой запуск.
        let dir = tempdir().expect("failed to create temp dir");
        let marker = dir.path().join("finished-normally");
        let script = write_script(
            &dir,
            "slow-after-cancel.sh",
            &format!("#!/bin/sh\nsleep 30\ntouch '{}'\n", marker.display()),
            0o755,
        );

        let registry = ChildRegistry::new();
        let handle = RunHandle::new();

        // Отмена до запуска: PID ещё не существует, остаётся только флаг.
        handle.cancel().await;

        let started = Instant::now();
        let result = run_cancellable(
            &script,
            &[],
            &[],
            Duration::from_secs(30),
            &registry,
            &handle,
        )
        .await;
        let elapsed = started.elapsed();

        assert!(
            matches!(
                result,
                Err(SidecarError::LaunchFailed {
                    reason: LaunchFailedReason::Corrupted,
                    ..
                })
            ),
            "процесс убит сразу после старта: {result:?}"
        );
        assert!(
            elapsed < Duration::from_secs(10),
            "запуск не должен доживать ни до конца скрипта, ни до таймаута: {elapsed:?}"
        );
        time::sleep(Duration::from_millis(200)).await;
        assert!(
            !marker.exists(),
            "процесс обязан быть убит, а не отработать осиротело"
        );
        assert!(registry.is_empty(), "PID снят с реестра и на этой ветке");
    }

    #[tokio::test]
    async fn cancelling_a_run_that_already_returned_kills_nothing() {
        // Состояние 3 дескриптора: запуск вернул управление, PID больше не
        // наш. Отмена после этого обязана быть пустой операцией — иначе
        // убийство ушло бы по номеру, который ОС уже могла выдать другому
        // процессу.
        let dir = tempdir().expect("failed to create temp dir");
        let script = write_script(&dir, "quick.sh", "#!/bin/sh\nexit 0\n", 0o755);

        let registry = ChildRegistry::new();
        let handle = RunHandle::new();

        run_cancellable(
            &script,
            &[],
            &[],
            Duration::from_secs(20),
            &registry,
            &handle,
        )
        .await
        .expect("скрипт завершается сам и успешно");

        // Не виснет, не паникует и никого не убивает: убивать уже нечего.
        handle.cancel().await;

        assert!(handle.was_cancelled(), "запрос отмены зафиксирован честно");
        assert!(registry.is_empty());
    }
}
