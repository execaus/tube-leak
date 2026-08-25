//! Разрешение пути к sidecar-бинарнику (Ф-5 эпика E1).
//!
//! `tauri.conf.json` объявляет sidecar-бинарники через `bundle.externalBin`
//! (`binaries/yt-dlp`, `binaries/ffmpeg`); физические файлы лежат в
//! `src-tauri/binaries/<name>-<target-triple>[.exe]` (см. `binaries.lock.json`,
//! задача TL-6). На этапе `cargo build`/`cargo test` `tauri-build` находит
//! файл, соответствующий текущей target triple, отрезает суффикс триплета
//! от имени и копирует результат рядом со скомпилированным бинарником —
//! в `target/<profile>/` (см. `tauri-build::copy_binaries`); в бандле
//! релиза бандлер кладёт sidecar рядом с исполняемым файлом приложения
//! по тем же соглашениям для каждой ОС.
//!
//! Эта функция воспроизводит тот же алгоритм резолва, что использует сама
//! Tauri для sidecar-механизма (`tauri_plugin_shell::process::Command::new_sidecar`
//! строит путь идентично, через `tauri::utils::platform::current_exe()`),
//! не утаскивая в зависимости весь `tauri-plugin-shell` — он нужен для
//! IPC-команд и JS-биндингов фронтенда, которые здесь не используются:
//! запуск процесса выполняется напрямую из Rust (`crate::sidecar::process`).
//!
//! Резолв не требует `tauri::AppHandle`/окна — `current_exe()` определяется
//! на уровне процесса, поэтому проверяется юнит-тестом без поднятия
//! приложения.

use std::path::PathBuf;

use super::error::SidecarError;
use crate::types::LaunchFailedReason;

/// Возвращает путь к sidecar-бинарнику `name` (`"yt-dlp"` или `"ffmpeg"`,
/// без суффикса target triple — он уже учтён на этапе сборки).
///
/// Существование файла по возвращённому пути не проверяется здесь: это
/// делает попытка запуска (`crate::sidecar::process::run`), которая
/// классифицирует ENOENT в [`SidecarError::NotFound`].
pub fn resolve_sidecar_path(name: &str) -> Result<PathBuf, SidecarError> {
    let exe_path =
        tauri::utils::platform::current_exe().map_err(|_| SidecarError::LaunchFailed {
            reason: LaunchFailedReason::Other,
            stderr: String::new(),
        })?;

    let exe_dir = exe_path.parent().ok_or(SidecarError::LaunchFailed {
        reason: LaunchFailedReason::Other,
        stderr: String::new(),
    })?;

    // `cargo test` кладёт тестовые бинарники в `target/<profile>/deps/`;
    // sidecar-файлы `tauri-build` копирует в `target/<profile>/`, на
    // уровень выше — тот же приём использует `tauri_plugin_shell`.
    let base_dir = if exe_dir.ends_with("deps") {
        exe_dir.parent().unwrap_or(exe_dir)
    } else {
        exe_dir
    };

    let path = base_dir.join(name);

    #[cfg(windows)]
    let path = {
        let mut path = path;
        if path.extension().is_none_or(|ext| ext != "exe") {
            path.as_mut_os_string().push(".exe");
        }
        path
    };

    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embeds_the_requested_sidecar_name_into_the_resolved_path() {
        let path = resolve_sidecar_path("totally-made-up-sidecar-name")
            .expect("resolution must not fail for a well-formed name");

        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("resolved path must have a file name");

        assert!(file_name.starts_with("totally-made-up-sidecar-name"));
    }

    #[test]
    fn resolves_next_to_the_compiled_test_binary() {
        let path = resolve_sidecar_path("yt-dlp").expect("resolution must not fail");

        let expected_dir = std::env::current_exe()
            .expect("current_exe must resolve in a test binary")
            .parent()
            .expect("test binary has a parent dir (target/.../deps)")
            .parent()
            .expect("deps dir has a parent dir (target/...)")
            .to_path_buf();

        assert_eq!(path.parent(), Some(expected_dir.as_path()));
    }

    #[test]
    fn resolves_an_existing_yt_dlp_sidecar_copied_by_tauri_build() {
        let path = resolve_sidecar_path("yt-dlp").expect("resolution must not fail");

        assert!(
            path.exists(),
            "expected tauri-build to have copied the yt-dlp sidecar to {path:?} \
             (see src-tauri/binaries/yt-dlp-<target-triple>)"
        );
    }

    #[test]
    fn resolves_an_existing_ffmpeg_sidecar_copied_by_tauri_build() {
        let path = resolve_sidecar_path("ffmpeg").expect("resolution must not fail");

        assert!(
            path.exists(),
            "expected tauri-build to have copied the ffmpeg sidecar to {path:?} \
             (see src-tauri/binaries/ffmpeg-<target-triple>)"
        );
    }
}
