//! Реестр PID процессов, ещё выполняющихся под управлением
//! [`super::process::run`] — единственный надёжный способ прибить их на
//! выходе из приложения (TL-10, дефект Ф-2 эпика E1, обнаруженный на
//! собранном `.dmg`: `yt-dlp`/`ffmpeg` оставались в дереве процессов ОС
//! после выхода из приложения, реродительшись на PID 1).
//!
//! # Почему `Command::kill_on_drop` не гарантия
//!
//! Tauri/tao на `RunEvent::Exit` завершают процесс приложения через
//! `std::process::exit` (см. `tauri::App::run`, «the process is exited
//! directly using `std::process::exit`») — эта функция не выполняет Rust
//! `Drop`-глу вообще ни для чего в процессе. Если в этот момент где-то в
//! памяти жив `tokio::process::Child` с `kill_on_drop(true)` (например,
//! фоновая `check_sidecar`-задача ещё не успела завершить `child.wait()`),
//! его `Drop` просто никогда не выполнится — сигнал убийства не будет
//! отправлен вовсе.
//!
//! # Почему убийства одного PID недостаточно
//!
//! `yt-dlp`, поставляемый как onefile-сборка PyInstaller, на macOS — это
//! **два** процесса: bootloader-обёртка (тот PID, что видит наш код после
//! `spawn`) форкает распакованный python-процесс и дожидается его
//! завершения, пересылая ему сигналы. Если bootloader убит `SIGKILL`
//! напрямую (как это делает `kill_on_drop`/`Child::start_kill`), сам он не
//! успевает переслать сигнал форкнутому потомку — тот остаётся жить,
//! реродительшись на PID 1. Единственный надёжный способ — убить всю
//! группу процессов (см. [`group_kill_command`]), а не один PID.

use std::collections::HashSet;
use std::sync::Mutex;

/// Потокобезопасный реестр PID процессов, зарегистрированных как «сейчас
/// выполняются». Живёт как Tauri-состояние (`app.manage`) на весь срок
/// жизни приложения — единственный экземпляр на процесс, но также
/// свободно конструируется отдельно в тестах ([`ChildRegistry::new`]) без
/// какой-либо связи с живым Tauri-приложением.
#[derive(Debug, Default)]
pub struct ChildRegistry {
    pids: Mutex<HashSet<u32>>,
}

impl ChildRegistry {
    /// Создаёт пустой реестр.
    pub fn new() -> Self {
        Self::default()
    }

    /// Регистрирует `pid` как «выполняется». Вызывается сразу после
    /// успешного `spawn`, пока процесс гарантированно жив (см.
    /// `super::process::run`).
    pub fn register(&self, pid: u32) {
        let mut guard = self
            .pids
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.insert(pid);
    }

    /// Снимает регистрацию `pid` — вызывается на каждом штатном пути
    /// завершения `run` (успех, ошибка запуска, таймаут после явного
    /// убийства группы): к этому моменту процесс либо завершился сам, либо
    /// уже был явно убит вызывающей стороной, и больше не должен считаться
    /// «висящим» на случай последующего выхода из приложения.
    pub fn unregister(&self, pid: u32) {
        let mut guard = self
            .pids
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.remove(&pid);
    }

    /// `true`, если реестр не отслеживает ни одного PID — используется в
    /// тестах, чтобы убедиться, что штатное завершение `run` не оставляет
    /// «забытых» записей. Вне `#[cfg(test)]` не вызывается нигде в
    /// продакшен-пути — `#[allow(dead_code)]` по той же причине, что и в
    /// `crate::types` (часть публичного API модуля, не мёртвый код по
    /// смыслу).
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        let guard = self
            .pids
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.is_empty()
    }

    /// Синхронно убивает все ещё зарегистрированные группы процессов (см.
    /// doc модуля) и очищает реестр.
    ///
    /// Вызывается из колбэка `RunEvent::Exit` в `main.rs` — синхронный
    /// контекст на главном потоке приложения, без гарантированно живого
    /// async-рантайма, поэтому используется блокирующий `std::process::Command`,
    /// а не `tokio::process`.
    pub fn kill_all(&self) {
        let pids: Vec<u32> = {
            let mut guard = self
                .pids
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.drain().collect()
        };

        for pid in pids {
            let (program, args) = group_kill_command(pid);
            // Best-effort: если процесс уже завершился сам между
            // регистрацией и вызовом `kill_all`, команда просто ничего не
            // найдёт — не повод падать на выходе из приложения. stdout/stderr
            // подавлены: диагностика самой `kill`/`taskkill` («No such
            // process» и т.п.) не нужна ни в проде, ни в логах теста — это
            // ожидаемый штатный случай, не сигнал об ошибке.
            let _ = std::process::Command::new(program)
                .args(&args)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
    }
}

/// Платформенно-специфичная команда, убивающая всю группу/дерево
/// процессов с лидером `pid`.
///
/// На Unix `pid` — одновременно и `pgid`: `super::process::run` спавнит
/// процесс через `process_group(0)`, что делает его лидером новой группы
/// (см. `Command::process_group` в `tokio`), поэтому `kill -9 -<pid>`
/// убивает и его, и всех его потомков (форкнутых или унаследовавших
/// группу через `exec`), не дожидаясь, пока они сами переспросят сигнал у
/// родителя (см. doc модуля, почему это критично для PyInstaller-сборок
/// `yt-dlp`).
///
/// На Windows `process_group` не выставляется (у `tokio::process::Command`
/// это API вообще недоступно вне `cfg(unix)`) — вместо этого используется
/// `taskkill /T`, который убивает процесс и всё дерево процессов,
/// запущенных им, опираясь на собственный учёт родитель/потомок в ОС.
///
/// Возвращает `(программа, аргументы)`, а не исполняет команду сама —
/// вызывающая сторона решает, каким `Command` это исполнить:
/// синхронным (`kill_all`, контекст без async-рантайма) или асинхронным
/// (`super::process::run`, путь таймаута внутри уже запущенного
/// tokio-рантайма).
pub(crate) fn group_kill_command(pid: u32) -> (&'static str, Vec<String>) {
    #[cfg(unix)]
    {
        ("kill", vec!["-9".to_string(), format!("-{pid}")])
    }
    #[cfg(windows)]
    {
        (
            "taskkill",
            vec![
                "/PID".to_string(),
                pid.to_string(),
                "/T".to_string(),
                "/F".to_string(),
            ],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::time::Duration;

    use tempfile::tempdir;
    use tokio::process::Command;
    use tokio::time;

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

    /// `kill -0 <pid>` — POSIX-идиома «жив ли процесс», без побочных
    /// эффектов: возвращает успех, если процесс с этим PID существует
    /// (независимо от прав на его убийство).
    fn process_is_alive(pid: u32) -> bool {
        std::process::Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    #[test]
    fn starts_empty() {
        let registry = ChildRegistry::new();
        assert!(registry.is_empty());
    }

    #[test]
    fn tracks_a_registered_pid_until_it_is_unregistered() {
        let registry = ChildRegistry::new();

        registry.register(4242);
        assert!(!registry.is_empty());

        registry.unregister(4242);
        assert!(registry.is_empty());
    }

    #[test]
    fn unregistering_an_unknown_pid_is_a_harmless_no_op() {
        let registry = ChildRegistry::new();

        registry.unregister(999_999);

        assert!(registry.is_empty());
    }

    #[tokio::test]
    async fn kill_all_terminates_every_registered_process_and_empties_the_registry() {
        let dir = tempdir().expect("failed to create temp dir");
        let registry = ChildRegistry::new();

        let mut children = Vec::new();
        for i in 0..3 {
            let script = write_script(
                &dir,
                &format!("sleep-{i}.sh"),
                "#!/bin/sh\nexec sleep 30\n",
                0o755,
            );
            #[cfg_attr(not(unix), allow(unused_mut))]
            let mut command = Command::new(&script);
            #[cfg(unix)]
            command.process_group(0);
            let child = command.spawn().expect("fixture process must spawn");
            let pid = child.id().expect("freshly spawned child must have a pid");
            registry.register(pid);
            children.push((child, pid));
        }

        registry.kill_all();

        assert!(
            registry.is_empty(),
            "kill_all must drain every tracked pid from the registry"
        );

        for (child, _pid) in &mut children {
            let status = time::timeout(Duration::from_secs(5), child.wait())
                .await
                .expect("kill_all must cause the process to exit promptly, not linger")
                .expect("wait must succeed once the process has been killed");
            assert!(
                !status.success(),
                "process killed by kill_all must not report a successful exit"
            );
        }
    }

    /// Воспроизводит форму реального дефекта TL-10: bootloader-подобный
    /// процесс форкает фонового потомка (не дожидаясь его через `wait`, в
    /// отличие от `captures_stderr_written_before_the_process_is_killed_by_a_timeout`
    /// в `process.rs`) — оба процесса живут в одной группе, унаследованной
    /// от `process_group(0)`. `kill_all` должен убить обоих одним вызовом,
    /// а не только прямого потомка (см. doc `group_kill_command`, почему
    /// одиночный `kill_on_drop`/`start_kill` этого не гарантирует).
    #[cfg(unix)]
    #[tokio::test]
    async fn kill_all_also_terminates_a_forked_grandchild_sharing_the_process_group() {
        let dir = tempdir().expect("failed to create temp dir");
        let marker = dir.path().join("grandchild-pid");
        let registry = ChildRegistry::new();

        let script = write_script(
            &dir,
            "bootloader-like.sh",
            &format!(
                "#!/bin/sh\nsleep 30 &\nchild=$!\necho \"$child\" > '{}'\nwait \"$child\"\n",
                marker.display()
            ),
            0o755,
        );

        let mut command = Command::new(&script);
        command.process_group(0);
        let mut child = command.spawn().expect("fixture process must spawn");
        let pid = child.id().expect("freshly spawned child must have a pid");
        registry.register(pid);

        // Ждём, пока скрипт успеет форкнуть потомка и записать его PID в
        // маркер — без этого мы бы проверяли до того, как грандчайлд вообще
        // появился. Бюджет — 5 секунд (250 * 20мс), а не сотни миллисекунд:
        // первый запуск нового исполняемого файла на занятой машине (весь
        // набор тестов выполняется параллельно) несёт тот же одноразовый
        // оверхед планировщика, что описан в `warm_up`-фикстурах
        // `process.rs`/`commands::sidecar`.
        let grandchild_pid: u32 = 'wait_for_marker: {
            for _ in 0..250 {
                if let Ok(contents) = fs::read_to_string(&marker) {
                    if let Ok(pid) = contents.trim().parse() {
                        break 'wait_for_marker pid;
                    }
                }
                time::sleep(Duration::from_millis(20)).await;
            }
            panic!("grandchild pid marker never appeared");
        };
        assert!(
            process_is_alive(grandchild_pid),
            "grandchild must be alive before kill_all runs, otherwise the test proves nothing"
        );

        registry.kill_all();

        let status = time::timeout(Duration::from_secs(5), child.wait())
            .await
            .expect("bootloader-like process must exit promptly after kill_all")
            .expect("wait must succeed once the process has been killed");
        assert!(!status.success());

        // Даём ОС короткое окно на реальное освобождение PID после SIGKILL.
        let mut grandchild_dead = false;
        for _ in 0..50 {
            if !process_is_alive(grandchild_pid) {
                grandchild_dead = true;
                break;
            }
            time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            grandchild_dead,
            "kill_all must kill the whole process group, not just the direct child — \
             the forked grandchild (pid {grandchild_pid}) survived"
        );
    }
}
