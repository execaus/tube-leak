//! Разбор строки версии sidecar-бинарника в чистую строку версии (Ф-7 эпика E1).
//!
//! Оперирует уже захваченным stdout (см. `crate::sidecar::process::run`) —
//! сам запуск `yt-dlp --version` / `ffmpeg -version` этому модулю не нужен,
//! что позволяет проверять разбор фикстурами без запуска процессов.

/// Разбирает вывод `yt-dlp --version`.
///
/// В большинстве случаев (pip-релиз) вывод — голая строка версии
/// (`2026.08.19`). Сборки, собранные не из релизного тега (nightly/master),
/// могут дописывать через пробел метаданные сборки (например, короткий
/// хэш коммита) на той же строке — берём только первый токен первой
/// строки, это и есть версия.
pub fn parse_ytdlp_version(raw: &str) -> Option<String> {
    let token = raw.lines().next()?.split_whitespace().next()?;
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

/// Разбирает вывод `ffmpeg -version`.
///
/// Первая строка имеет вид `ffmpeg version <версия> Copyright ...`, где
/// `<версия>` — не обязательно чистый semver: сборки со сторонних
/// зеркал (gyan.dev для Windows, evermeet.cx для macOS — см.
/// `binaries.lock.json`) дописывают через дефис имя сборки
/// (`6.0-full_build-www.gyan.dev`), которое возвращается как есть.
pub fn parse_ffmpeg_version(raw: &str) -> Option<String> {
    let mut tokens = raw.lines().next()?.split_whitespace();

    if tokens.next()? != "ffmpeg" {
        return None;
    }
    if tokens.next()? != "version" {
        return None;
    }

    let version = tokens.next()?;
    if version.is_empty() {
        None
    } else {
        Some(version.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bare_ytdlp_version_string() {
        assert_eq!(
            parse_ytdlp_version("2026.08.19\n"),
            Some("2026.08.19".to_string())
        );
    }

    #[test]
    fn parses_ytdlp_version_with_trailing_build_metadata() {
        assert_eq!(
            parse_ytdlp_version("2026.08.19 [b64a3e1] (win32_exe)\n"),
            Some("2026.08.19".to_string())
        );
    }

    #[test]
    fn returns_none_for_empty_ytdlp_output() {
        assert_eq!(parse_ytdlp_version(""), None);
        assert_eq!(parse_ytdlp_version("\n"), None);
    }

    #[test]
    fn parses_ffmpeg_version_from_full_real_world_output() {
        let raw = "ffmpeg version 7.1 Copyright (c) 2000-2024 the FFmpeg developers\n\
                    built with Apple clang version 15.0.0 (clang-1500.3.9.4)\n\
                    configuration: --prefix=/usr/local --enable-gpl\n\
                    libavutil      59.  8.100 / 59.  8.100\n";

        assert_eq!(parse_ffmpeg_version(raw), Some("7.1".to_string()));
    }

    #[test]
    fn parses_ffmpeg_version_pinned_by_binaries_lock_json() {
        // Версия из src-tauri/binaries.lock.json (TL-6): 9.0.1, собрано
        // evermeet.cx — реальный формат первой строки этого зеркала.
        let raw = "ffmpeg version 9.0.1 Copyright (c) 2000-2025 the FFmpeg developers\n\
                    built with Apple clang version 16.0.0 (clang-1600.0.26.6)\n";

        assert_eq!(parse_ffmpeg_version(raw), Some("9.0.1".to_string()));
    }

    #[test]
    fn parses_ffmpeg_version_with_hyphenated_build_suffix() {
        // Формат зеркала gyan.dev для Windows (см. binaries.lock.json).
        let raw = "ffmpeg version 6.0-full_build-www.gyan.dev Copyright (c) 2000-2023 \
                    the FFmpeg developers\n";

        assert_eq!(
            parse_ffmpeg_version(raw),
            Some("6.0-full_build-www.gyan.dev".to_string())
        );
    }

    #[test]
    fn returns_none_when_output_does_not_look_like_ffmpeg_version() {
        assert_eq!(parse_ffmpeg_version(""), None);
        assert_eq!(parse_ffmpeg_version("not ffmpeg output at all\n"), None);
        assert_eq!(parse_ffmpeg_version("ffmpeg\n"), None);
    }
}
