//! Запуск sidecar-бинарника с аргументами, захват вывода и таймаут
//! (Ф-6, Ф-8 эпика E1).
//!
//! Работает напрямую с путём к исполняемому файлу (`&Path`), а не с
//! `crate::sidecar::resolve::resolve_sidecar_path` — это разделение
//! позволяет тестировать запуск/таймаут/классификацию ошибок на временных
//! фикстурных скриптах (`tempfile`) без резолва настоящих sidecar-путей.

use std::io;
use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::task::JoinHandle;
use tokio::time;

use super::error::SidecarError;
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
pub async fn run(
    program: &Path,
    args: &[&str],
    timeout: Duration,
) -> Result<RunOutput, SidecarError> {
    let mut command = Command::new(program);
    command
        .args(args)
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn().map_err(classify_spawn_error)?;

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
            // Процесс пережил `timeout` — убиваем явно (в отличие от
            // прежней реализации, `child` живёт в этой функции, а не внутри
            // упавшего таймаутом future, так что `kill_on_drop` здесь не
            // сработает сам по себе).
            let _ = child.start_kill();

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

    #[tokio::test]
    async fn returns_stdout_when_the_process_exits_successfully() {
        let dir = tempdir().expect("failed to create temp dir");
        let script = write_script(
            &dir,
            "ok.sh",
            "#!/bin/sh\necho hello-sidecar\nexit 0\n",
            0o755,
        );

        let output = run(&script, &[], Duration::from_secs(20))
            .await
            .expect("script must succeed");

        assert_eq!(output.stdout.trim(), "hello-sidecar");
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

        let output = run(&script, &["--version"], Duration::from_secs(20))
            .await
            .expect("script must succeed");

        assert_eq!(output.stdout.trim(), "--version");
    }

    #[tokio::test]
    async fn returns_not_found_when_the_binary_does_not_exist() {
        let dir = tempdir().expect("failed to create temp dir");
        let missing = dir.path().join("does-not-exist");

        let result = run(&missing, &[], Duration::from_secs(20)).await;

        assert_eq!(result, Err(SidecarError::NotFound));
    }

    #[tokio::test]
    async fn returns_permission_denied_when_the_binary_is_not_executable() {
        let dir = tempdir().expect("failed to create temp dir");
        let script = write_script(&dir, "not-executable.sh", "#!/bin/sh\nexit 0\n", 0o644);

        let result = run(&script, &[], Duration::from_secs(20)).await;

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

        let result = run(&script, &[], Duration::from_secs(20)).await;

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

        let result = run(&script, &[], Duration::from_secs(20)).await;

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

        let result = run(&script, &[], Duration::from_secs(20)).await;

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

        let result = run(&script, &[], Duration::from_secs(20)).await;

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

        let started = Instant::now();
        let result = run(&script, &[], Duration::from_millis(100)).await;
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
    }

    /// См. doc [`captures_stderr_written_before_the_process_is_killed_by_a_timeout`]
    /// и аналогичный прогрев в `crate::commands::sidecar` тестах — платит
    /// одноразовый оверхед первого запуска именно этого файла вне
    /// измеряемого окна теста. `script` рассчитан на то, что процесс не
    /// завершается сам — прогрев принудительно убивает его коротким
    /// собственным таймаутом, не дожидаясь EOF.
    async fn warm_up(script: &Path) {
        let _ = run(script, &[], Duration::from_secs(20)).await;
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

        let timeout = Duration::from_millis(300);
        let result = run(&script, &[], timeout).await;

        assert_eq!(
            result,
            Err(SidecarError::Timeout {
                ms: timeout.as_millis() as u64,
                stderr: "partial diagnostic output\n".to_string(),
            })
        );
    }
}
