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
use std::time::Duration;

use tokio::process::Command;
use tokio::time;

use super::error::SidecarError;
use crate::types::LaunchFailedReason;

/// Запускает `program` с аргументами `args`, ждёт завершения не дольше
/// `timeout` и возвращает захваченный stdout как UTF-8 строку (лоссово —
/// вывод yt-dlp/ffmpeg не гарантированно валиден в UTF-8 побайтово, но для
/// строки версии этого достаточно).
///
/// По истечении `timeout` процесс принудительно убивается и возвращается
/// [`SidecarError::Timeout`]. Используется идиома `kill_on_drop` + гонка с
/// [`tokio::time::timeout`] вокруг [`tokio::process::Child::wait_with_output`]
/// (рекомендованный tokio способ прервать дочерний процесс по таймауту):
/// `wait_with_output` вычитывает stdout/stderr конкурентно с ожиданием
/// завершения, что также исключает дедлок на заполненном пайпе при
/// большом выводе (в отличие от чтения stdout только после `wait`).
///
/// `timeout` — параметр вызывающего кода (TL-5 подставит реальные лимиты
/// для yt-dlp/ffmpeg по «Ресурсы машины»/UX-требованиям), здесь не
/// захардкожен.
pub async fn run(program: &Path, args: &[&str], timeout: Duration) -> Result<String, SidecarError> {
    let mut command = Command::new(program);
    command
        .args(args)
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let child = command.spawn().map_err(classify_spawn_error)?;

    let output = match time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(_wait_error)) => {
            // Практически недостижимо (означало бы, что процессом уже кто-то
            // управлял конкурентно) — не классифицируемая по ENOENT/EACCES
            // ошибка запуска.
            return Err(SidecarError::LaunchFailed {
                reason: LaunchFailedReason::Other,
            });
        }
        Err(_elapsed) => {
            // `Timeout::poll` внутри `tokio::time::timeout` роняет
            // обёрнутый future по истечении срока — вместе с ним роняется
            // и захваченный им `Child`, что с `kill_on_drop(true)` убивает
            // процесс (см. doc `Command::kill_on_drop`).
            return Err(SidecarError::Timeout {
                ms: timeout.as_millis() as u64,
            });
        }
    };

    if let Some(error) = classify_exit_status(output.status) {
        return Err(error);
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Классифицирует ошибку `Command::spawn` (ENOENT/EACCES/повреждённый
/// формат исполняемого файла).
fn classify_spawn_error(err: io::Error) -> SidecarError {
    match err.kind() {
        io::ErrorKind::NotFound => SidecarError::NotFound,
        io::ErrorKind::PermissionDenied => SidecarError::LaunchFailed {
            reason: LaunchFailedReason::PermissionDenied,
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
                    };
                }
            }

            SidecarError::LaunchFailed {
                reason: LaunchFailedReason::Other,
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
fn classify_exit_status(status: ExitStatus) -> Option<SidecarError> {
    match status.code() {
        Some(0) => None,
        Some(SHELL_NOT_EXECUTABLE_EXIT_CODE) => Some(SidecarError::LaunchFailed {
            reason: LaunchFailedReason::Corrupted,
        }),
        Some(code) => Some(SidecarError::NonZeroExit { code }),
        None => Some(SidecarError::LaunchFailed {
            reason: LaunchFailedReason::Corrupted,
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

        assert_eq!(output.trim(), "hello-sidecar");
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

        assert_eq!(output.trim(), "--version");
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
                reason: LaunchFailedReason::PermissionDenied
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

        assert_eq!(
            result,
            Err(SidecarError::LaunchFailed {
                reason: LaunchFailedReason::Corrupted
            })
        );
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
                reason: LaunchFailedReason::Corrupted
            })
        );
    }

    #[tokio::test]
    async fn returns_non_zero_exit_when_the_process_fails() {
        let dir = tempdir().expect("failed to create temp dir");
        let script = write_script(&dir, "fail.sh", "#!/bin/sh\nexit 3\n", 0o755);

        let result = run(&script, &[], Duration::from_secs(20)).await;

        assert_eq!(result, Err(SidecarError::NonZeroExit { code: 3 }));
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

        assert_eq!(result, Err(SidecarError::Timeout { ms: 100 }));
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
}
