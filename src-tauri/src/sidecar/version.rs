//! Разбор строки версии sidecar-бинарника в чистую строку версии (Ф-7 эпика E1).
//!
//! Оперирует уже захваченным stdout (см. `crate::sidecar::process::run`) —
//! сам запуск `yt-dlp --version` / `ffmpeg -version` / `deno --version`
//! этому модулю не нужен,
//! что позволяет проверять разбор фикстурами без запуска процессов.
//!
//! Разбор возвращает [`SidecarVersion`] — «что показываем», «что бинарник
//! вывел дословно» и строку, из которой это разобрано. Это внутренний тип
//! ядра: границу Rust↔TS он не пересекает и в `src/types/` не зеркалится. В
//! контрактный [`crate::types::SidecarCheckResult`] уходят `display` (поле
//! `version`) и `line` (поле `versionRaw`, TL-15) — последнее обрезается
//! на границе команды; `raw` используется для лога и сверки версии yt-dlp.

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
    /// Первая строка вывода, из которой разобран токен, — целиком, без
    /// краевых пробелов и без обрезки по длине (TL-15). Шире `raw`: у
    /// ffmpeg в ней видно сборщика, у deno — канал и целевую тройку.
    pub line: String,
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
    let line = raw.lines().next()?;
    let token = line.split_whitespace().next()?;
    if token.is_empty() {
        return None;
    }

    Some(SidecarVersion {
        display: token.to_string(),
        raw: token.to_string(),
        line: line.trim().to_string(),
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
    let line = raw.lines().next()?;
    let mut tokens = line.split_whitespace();

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
        line: line.trim().to_string(),
    })
}

/// Разбирает вывод `deno --version` (TL-110).
///
/// Вывод трёхстрочный, версия deno — в первой строке:
///
/// ```text
/// deno 2.9.6 (stable, release, aarch64-apple-darwin)
/// v8 15.0.245.2-rusty
/// typescript 6.0.3
/// ```
///
/// Разбор тот же, каким yt-dlp сам опознаёт рантайм (`^deno (\S+)`), плюс
/// требование, чтобы токен начинался с цифры: иначе строка вида
/// `deno (stable, …)` без номера отдала бы на экран `(stable,`.
///
/// Нормализация — общая с ffmpeg ([`normalize_version`]): у релизной
/// сборки токен уже чистый (`2.9.6`), а canary-сборки дописывают хэш
/// коммита через `+` (`2.9.6+0a1b2c3`) — на экран уходит `2.9.6`, полный
/// токен остаётся в `raw` для лога.
///
/// `None` — вывод не похож на deno вовсе. В отличие от yt-dlp и ffmpeg,
/// у deno это не «показать как есть», а отказ — см.
/// `crate::commands::sidecar`.
pub fn parse_deno_version(raw: &str) -> Option<SidecarVersion> {
    let line = raw.lines().next()?;
    let mut tokens = line.split_whitespace();

    if tokens.next()? != "deno" {
        return None;
    }

    let token = tokens.next()?;
    if !token.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }

    Some(SidecarVersion {
        display: normalize_version(token),
        raw: token.to_string(),
        line: line.trim().to_string(),
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
    fn keeps_the_whole_first_line_for_diagnostics_of_each_sidecar() {
        // TL-15: `line` — то, что уходит в `versionRaw`. Строки — живые:
        // ffmpeg martin-riedl.de (см. `normalizes_ffmpeg_version_of_macos_builds`),
        // трёхстрочный вывод пина deno 2.9.6, голый вывод pip-релиза yt-dlp.
        let ffmpeg = "ffmpeg version 9.0.1-https://www.martin-riedl.de Copyright (c) 2000-2026 \
                      the FFmpeg developers\n\
                      built with Apple clang version 14.0.0 (clang-1400.0.29.102)\n";

        assert_eq!(
            parse_ffmpeg_version(ffmpeg).expect("version").line,
            "ffmpeg version 9.0.1-https://www.martin-riedl.de Copyright (c) 2000-2026 \
             the FFmpeg developers"
        );
        assert_eq!(
            parse_deno_version(REAL_DENO_VERSION_OUTPUT)
                .expect("version")
                .line,
            "deno 2.9.6 (stable, release, aarch64-apple-darwin)"
        );
        assert_eq!(
            parse_ytdlp_version("2026.08.19\n").expect("version").line,
            "2026.08.19"
        );
    }

    #[test]
    fn the_first_line_is_neither_the_token_nor_a_later_line_and_has_no_crlf() {
        // Метаданные сборки yt-dlp после токена в `line` остаются, `\r`
        // Windows-вывода — нет; строки v8/typescript у deno не попадают.
        let parsed =
            parse_ytdlp_version("2026.08.19 [b64a3e1] (win32_exe)\r\nsecond\r\n").expect("version");

        assert_eq!(parsed.display, "2026.08.19");
        assert_eq!(parsed.line, "2026.08.19 [b64a3e1] (win32_exe)");

        let deno = parse_deno_version(REAL_DENO_VERSION_OUTPUT).expect("version");
        assert!(!deno.line.contains("v8"), "{:?}", deno.line);
        assert!(!deno.line.contains('\n'), "{:?}", deno.line);
    }

    #[test]
    fn returns_none_when_output_does_not_look_like_ffmpeg_version() {
        assert_eq!(parse_ffmpeg_version(""), None);
        assert_eq!(parse_ffmpeg_version("not ffmpeg output at all\n"), None);
        assert_eq!(parse_ffmpeg_version("ffmpeg\n"), None);
    }

    /// ПРОВЕРЕНО ЖИВЬЁМ: дословный вывод
    /// `src-tauri/binaries/deno-aarch64-apple-darwin --version` (пин 2.9.6,
    /// TL-108), запущенного с `DENO_NO_UPDATE_CHECK=1` и `DENO_DIR` во
    /// временном каталоге — ровно то окружение, с которым его зовёт
    /// служебный экран.
    const REAL_DENO_VERSION_OUTPUT: &str = "deno 2.9.6 (stable, release, aarch64-apple-darwin)\n\
                                            v8 15.0.245.2-rusty\n\
                                            typescript 6.0.3\n";

    #[test]
    fn parses_deno_version_from_real_output_of_the_pinned_binary() {
        let parsed = parse_deno_version(REAL_DENO_VERSION_OUTPUT).expect("version");

        assert_eq!(parsed.display, "2.9.6");
        assert_eq!(parsed.raw, "2.9.6");
        assert!(!parsed.is_normalized());
    }

    #[test]
    fn takes_the_deno_version_not_the_v8_or_typescript_one() {
        // Строки v8 и typescript тоже «имя плюс номер»; разбор обязан
        // брать первую строку, а не первую попавшуюся версию.
        let parsed = parse_deno_version(REAL_DENO_VERSION_OUTPUT).expect("version");

        assert_ne!(parsed.display, "15.0.245.2");
        assert_ne!(parsed.display, "6.0.3");
    }

    #[test]
    fn normalizes_deno_canary_build_metadata() {
        // ФОРМАТ ПО ДОКУМЕНТАЦИИ, НЕ ПРОВЕРЕН ЖИВЬЁМ: canary-сборки deno
        // дописывают хэш коммита через `+`. Мы canary не поставляем, но
        // подменённый бинарник не должен ломать строку на экране.
        let raw = "deno 2.9.6+0a1b2c3 (canary, release, aarch64-apple-darwin)\n";

        let parsed = parse_deno_version(raw).expect("version");

        assert_eq!(parsed.display, "2.9.6");
        assert_eq!(parsed.raw, "2.9.6+0a1b2c3");
        assert!(parsed.is_normalized());
    }

    #[test]
    fn returns_none_when_output_does_not_look_like_deno_version() {
        assert_eq!(parse_deno_version(""), None);
        assert_eq!(parse_deno_version("\n"), None);
        assert_eq!(parse_deno_version("deno\n"), None);
        assert_eq!(
            parse_deno_version("deno (stable, release, aarch64-apple-darwin)\n"),
            None
        );
        assert_eq!(
            parse_deno_version("tube-leak CI stub, not a real binary\n"),
            None
        );
        // Чужой sidecar под именем deno: версия есть, но не его.
        assert_eq!(
            parse_deno_version(
                "ffmpeg version 9.0.1 Copyright (c) 2000-2026 the FFmpeg developers\n"
            ),
            None
        );
        assert_eq!(parse_deno_version("2026.08.19\n"), None);
        // «Имя плюс номер», но имя не deno. Без проверки первого слова
        // эти строки разобрались бы как версия: мутация, отменившая
        // проверку префикса, пережила остальные случаи этого теста.
        assert_eq!(parse_deno_version("v8 15.0.245.2-rusty\n"), None);
        assert_eq!(parse_deno_version("node 22.1.0\n"), None);
    }
}
