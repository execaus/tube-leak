//! Разбор строки версии sidecar-бинарника в чистую строку версии (Ф-7 эпика E1).
//!
//! Оперирует уже захваченным stdout (см. `crate::sidecar::process::run`) —
//! сам запуск `yt-dlp --version` / `ffmpeg -version` этому модулю не нужен,
//! что позволяет проверять разбор фикстурами без запуска процессов.
//!
//! Разбор возвращает [`SidecarVersion`] — пару «что показываем» и «что
//! бинарник вывел дословно». Это внутренний тип ядра: границу Rust↔TS он не
//! пересекает и в `src/types/` не зеркалится, в контрактный
//! [`crate::types::SidecarCheckResult`] по-прежнему уходит одна строка
//! (`display`), а `raw` используется только для лога.

/// Версия sidecar-бинарника в двух видах.
///
/// Разделение нужно из-за ffmpeg: сторонние билдеры дописывают к номеру
/// версии свой `--extra-version`, из-за чего дословный вывод выглядит как
/// `9.0.1-https://www.martin-riedl.de`. На служебном экране нужен чистый
/// `9.0.1`, но и потерять полную строку нельзя — по ней видно, чья это
/// сборка, когда придётся разбирать баг постобработки.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarVersion {
    /// Нормализованная версия — то, что показывается на служебном экране.
    pub display: String,
    /// Дословный токен версии из вывода бинарника, без нормализации.
    pub raw: String,
}

impl SidecarVersion {
    /// `true`, если нормализация действительно что-то отбросила, — то есть
    /// полная строка несёт сведения, которых нет в `display`, и её стоит
    /// написать в лог.
    pub fn is_normalized(&self) -> bool {
        self.display != self.raw
    }
}

/// Разбирает вывод `yt-dlp --version`.
///
/// В большинстве случаев (pip-релиз) вывод — голая строка версии
/// (`2026.08.19`). Сборки, собранные не из релизного тега (nightly/master),
/// могут дописывать через пробел метаданные сборки (например, короткий
/// хэш коммита) на той же строке — берём только первый токен первой
/// строки, это и есть версия.
///
/// Нормализация [`normalize_version`] здесь **не применяется** намеренно:
/// версия yt-dlp календарная и уже чистая, а её точное значение (включая
/// возможные суффиксы nightly-сборок) нужно сравнивать с апстримом при
/// проверке «yt-dlp устарел» — округлять его нельзя.
pub fn parse_ytdlp_version(raw: &str) -> Option<SidecarVersion> {
    let token = raw.lines().next()?.split_whitespace().next()?;
    if token.is_empty() {
        return None;
    }

    Some(SidecarVersion {
        display: token.to_string(),
        raw: token.to_string(),
    })
}

/// Разбирает вывод `ffmpeg -version`.
///
/// Первая строка имеет вид `ffmpeg version <версия> Copyright ...`. Токен
/// `<версия>` — не обязательно чистый semver: каждый билдер дописывает свой
/// `--extra-version` (а сборки из git — ещё и вывод `git describe`), поэтому
/// в `display` кладётся результат [`normalize_version`], а исходный токен
/// сохраняется в `raw`. Фактические форматы всех четырёх вложенных сборок
/// перечислены в тестах ниже.
pub fn parse_ffmpeg_version(raw: &str) -> Option<SidecarVersion> {
    let mut tokens = raw.lines().next()?.split_whitespace();

    if tokens.next()? != "ffmpeg" {
        return None;
    }
    if tokens.next()? != "version" {
        return None;
    }

    let token = tokens.next()?;
    if token.is_empty() {
        return None;
    }

    Some(SidecarVersion {
        display: normalize_version(token),
        raw: token.to_string(),
    })
}

/// Приводит токен версии к чистому semver-ядру.
///
/// Правило общее, а не заплатка под конкретного билдера: оно опирается на то,
/// как версию формирует сам ffmpeg (`FFMPEG_VERSION` = тег или `git describe`,
/// плюс `--extra-version` через дефис), а не на имена зеркал.
///
/// 1. Отбрасывается ведущий `n` тега ffmpeg (теги релизов называются `n9.0.1`),
///    если сразу за ним идёт цифра.
/// 2. Берётся ведущая последовательность из цифр и точек — всё, что начинается
///    с первого другого символа, это уже `git describe` и/или `--extra-version`.
/// 3. Обрезаются висячие точки.
/// 4. **Если получившееся ядро не содержит точки, токен возвращается целиком.**
///    Этот пункт защищает от порчи версий, которые semver'ом не являются:
///    git-сборка gyan.dev называется `2026-08-23-git-1019f8f036`, и обрезка до
///    `2026` превратила бы её в бессмыслицу, а nightly-сборки ffmpeg вида
///    `N-109874-g0f2f0b1e5f` вообще не имеют числового начала.
///
/// Функция ничего не знает про конкретные зеркала, поэтому смена источника
/// (как в TL-11) её не затрагивает.
fn normalize_version(token: &str) -> String {
    let without_tag_prefix = match token.strip_prefix('n') {
        Some(rest) if rest.starts_with(|c: char| c.is_ascii_digit()) => rest,
        _ => token,
    };

    let core = without_tag_prefix
        .split(|c: char| !c.is_ascii_digit() && c != '.')
        .next()
        .unwrap_or_default()
        .trim_end_matches('.');

    if core.contains('.') {
        core.to_string()
    } else {
        token.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Хелпер: нормализованная версия, которая уходит на служебный экран.
    fn display_of(parsed: Option<SidecarVersion>) -> Option<String> {
        parsed.map(|version| version.display)
    }

    /// Хелпер: дословный токен версии, который уходит в лог.
    fn raw_of(parsed: Option<SidecarVersion>) -> Option<String> {
        parsed.map(|version| version.raw)
    }

    #[test]
    fn parses_bare_ytdlp_version_string() {
        assert_eq!(
            display_of(parse_ytdlp_version("2026.08.19\n")),
            Some("2026.08.19".to_string())
        );
    }

    #[test]
    fn parses_ytdlp_version_with_trailing_build_metadata() {
        assert_eq!(
            display_of(parse_ytdlp_version("2026.08.19 [b64a3e1] (win32_exe)\n")),
            Some("2026.08.19".to_string())
        );
    }

    #[test]
    fn keeps_ytdlp_version_exact_without_normalizing() {
        // Версия yt-dlp сравнивается с апстримом при проверке «устарел»,
        // поэтому суффикс nightly-сборки обязан дожить до UI как есть.
        let parsed = parse_ytdlp_version("2026.08.19.232319\n").expect("version");

        assert_eq!(parsed.display, "2026.08.19.232319");
        assert_eq!(parsed.raw, "2026.08.19.232319");
        assert!(!parsed.is_normalized());
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

        assert_eq!(
            display_of(parse_ffmpeg_version(raw)),
            Some("7.1".to_string())
        );
    }

    #[test]
    fn parses_ffmpeg_version_pinned_by_binaries_lock_json() {
        // Формат evermeet.cx — зеркала, на котором macOS-таргеты стояли до
        // TL-11. Оставлен как регрессия на уже чистый semver: нормализация
        // не должна трогать токен, в котором нечего отбрасывать.
        let raw = "ffmpeg version 9.0.1 Copyright (c) 2000-2025 the FFmpeg developers\n\
                    built with Apple clang version 16.0.0 (clang-1600.0.26.6)\n";

        let parsed = parse_ffmpeg_version(raw).expect("version");

        assert_eq!(parsed.display, "9.0.1");
        assert_eq!(parsed.raw, "9.0.1");
        assert!(!parsed.is_normalized());
    }

    #[test]
    fn normalizes_ffmpeg_version_of_macos_builds() {
        // ПРОВЕРЕНО ЖИВЬЁМ: дословный вывод вложенного бинарника
        // ffmpeg.martin-riedl.de (оба macOS-таргета собраны одним пайплайном,
        // строка версии у них одинаковая — отличается только архитектура).
        let raw = "ffmpeg version 9.0.1-https://www.martin-riedl.de Copyright (c) 2000-2026 \
                    the FFmpeg developers\n\
                    built with Apple clang version 14.0.0 (clang-1400.0.29.102)\n";

        let parsed = parse_ffmpeg_version(raw).expect("version");

        assert_eq!(parsed.display, "9.0.1");
        assert_eq!(parsed.raw, "9.0.1-https://www.martin-riedl.de");
        assert!(parsed.is_normalized());
    }

    #[test]
    fn normalizes_ffmpeg_version_of_windows_build() {
        // ФОРМАТ ПО ДОКУМЕНТАЦИИ, НЕ ПРОВЕРЕН ЖИВЬЁМ: Windows-бинарника на
        // машине разработки нет (Р-6), а gyan.dev отдаёт архив слишком
        // медленно, чтобы тянуть его ради одной строки. Формат взят из
        // описания сборок gyan.dev: к версии дописывается
        // `-<variant>_build-www.gyan.dev`, где variant для нашего пина —
        // `essentials` (см. binaries.lock.json).
        let raw = "ffmpeg version 9.0.1-essentials_build-www.gyan.dev Copyright (c) 2000-2026 \
                    the FFmpeg developers\n";

        let parsed = parse_ffmpeg_version(raw).expect("version");

        assert_eq!(parsed.display, "9.0.1");
        assert_eq!(parsed.raw, "9.0.1-essentials_build-www.gyan.dev");
    }

    #[test]
    fn normalizes_ffmpeg_version_of_linux_build() {
        // ФОРМАТ ПО ДОКУМЕНТАЦИИ, НЕ ПРОВЕРЕН ЖИВЬЁМ: Linux-бинарника на
        // машине разработки нет (Р-6). Формат взят из имени самого
        // релизного архива BtbN/FFmpeg-Builds, вшитого в пин:
        // `ffmpeg-n9.0.1-6-g9d4ca21220-linux64-gpl-9.0.tar.xz` — сборка идёт
        // не с релизного тега, а с коммита после него, поэтому configure
        // подставляет вывод `git describe`: тег `n9.0.1`, счётчик коммитов
        // и хэш. Проверяет сразу оба звена правила — ведущий `n` и суффикс.
        let raw = "ffmpeg version n9.0.1-6-g9d4ca21220 Copyright (c) 2000-2026 \
                    the FFmpeg developers\n";

        let parsed = parse_ffmpeg_version(raw).expect("version");

        assert_eq!(parsed.display, "9.0.1");
        assert_eq!(parsed.raw, "n9.0.1-6-g9d4ca21220");
    }

    #[test]
    fn normalizes_plain_release_tag_without_build_suffix() {
        // Сборка ровно с релизного тега: `git describe` даёт голый `n9.0.1`.
        let raw = "ffmpeg version n9.0.1 Copyright (c) 2000-2026 the FFmpeg developers\n";

        assert_eq!(
            display_of(parse_ffmpeg_version(raw)),
            Some("9.0.1".to_string())
        );
    }

    #[test]
    fn keeps_non_semver_ffmpeg_versions_untouched() {
        // Пункт 4 правила: обрезать тут нечего, а испортить легко.
        // git-сборка gyan.dev — календарная, ведущий `2026` semver'ом не
        // является; nightly-сборка ffmpeg вообще начинается с `N-`.
        let gyan_git = "ffmpeg version 2026-08-23-git-1019f8f036 Copyright (c) 2000-2026 \
                    the FFmpeg developers\n";
        let nightly = "ffmpeg version N-109874-g0f2f0b1e5f Copyright (c) 2000-2026 \
                    the FFmpeg developers\n";

        assert_eq!(
            display_of(parse_ffmpeg_version(gyan_git)),
            Some("2026-08-23-git-1019f8f036".to_string())
        );
        assert_eq!(
            display_of(parse_ffmpeg_version(nightly)),
            Some("N-109874-g0f2f0b1e5f".to_string())
        );
    }

    #[test]
    fn keeps_the_full_build_string_available_for_the_log() {
        // Нормализация не должна терять полную строку: по ней определяется,
        // чья это сборка, когда разбирается баг постобработки.
        let raw = "ffmpeg version 9.0.1-https://www.martin-riedl.de Copyright (c) 2000-2026 \
                    the FFmpeg developers\n";

        assert_eq!(
            raw_of(parse_ffmpeg_version(raw)),
            Some("9.0.1-https://www.martin-riedl.de".to_string())
        );
    }

    #[test]
    fn returns_none_when_output_does_not_look_like_ffmpeg_version() {
        assert_eq!(parse_ffmpeg_version(""), None);
        assert_eq!(parse_ffmpeg_version("not ffmpeg output at all\n"), None);
        assert_eq!(parse_ffmpeg_version("ffmpeg\n"), None);
    }
}
