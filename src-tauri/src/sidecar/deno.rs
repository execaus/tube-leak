//! deno — JavaScript-рантайм, который yt-dlp запускает для
//! YouTube-извлечения (решение владельца #114): с каким окружением он
//! запускается (TL-110) и как yt-dlp узнаёт, какой deno брать (TL-109).
//!
//! deno поставляется третьим sidecar (`externalBin` `binaries/deno`, TL-108)
//! и резолвится тем же [`super::resolve_sidecar_path`], что и ffmpeg.
//! Потребителей два, и оба берут путь и окружение **отсюда**, одним
//! значением ([`DenoLaunch`]):
//!
//! - проверка версии на служебном экране (`crate::commands::sidecar`)
//!   запускает `deno --version` сама;
//! - разбор и скачивание запускают yt-dlp, а deno — его потомок: путь
//!   уходит в argv yt-dlp, окружение — в окружение процесса yt-dlp, которое
//!   deno наследует ([`YtDlpJsRuntime`]).
//!
//! # Окружение
//!
//! Две переменные, и обе — про инварианты CLAUDE.md, а не про удобство:
//!
//! - `DENO_NO_UPDATE_CHECK=1`. Без неё deno раз в сутки сам ходит на
//!   `dl.deno.land`/GitHub проверять свою версию — лишнее сетевое
//!   обращение с машины пользователя, которого приложение не заказывало
//!   («соединение прямое, без промежуточных серверов»), да ещё и
//!   предложение обновить бинарник, который обновляется только релизом.
//! - `DENO_DIR` в каталоге данных приложения. Без неё deno кладёт кэш в
//!   домашний каталог пользователя (`~/Library/Caches/deno` и аналоги) —
//!   след приложения вне его каталога данных, который не убирается вместе
//!   с ним. Каталог не создаётся заранее: `deno --version` его не трогает
//!   (замер TL-110 и TL-109: каталог остался пустым), а если он понадобится
//!   рантайму, deno создаст его сам.
//!
//! # Аргументы yt-dlp (TL-109)
//!
//! `--js-runtimes deno:<абсолютный путь>` — или `--no-js-runtimes`, если
//! запускать deno нельзя. Флаг не опускается никогда: без него yt-dlp ищет
//! `deno` в `PATH` пользователя и запускает чужой бинарник. Замер без сети
//! (yt-dlp 2026.08.19, `-v`, мёртвый прокси `--proxy http://127.0.0.1:1`,
//! подставной `deno 2.2.0` первым в `PATH`):
//!
//! | argv                          | `[debug] JS runtimes:`        |
//! |-------------------------------|-------------------------------|
//! | `--js-runtimes deno:<sidecar>` | `deno-2.9.6`                  |
//! | без флага                     | `deno-2.2.0 (unsupported)`    |
//! | `--no-js-runtimes`            | `none (disabled)`             |
//! | `--js-runtimes deno:<нет файла>` | `none`                     |
//!
//! Там же обёртка по нашему пути записала окружение, с которым yt-dlp
//! запустил deno: `DENO_NO_UPDATE_CHECK=1` и `DENO_DIR` из окружения
//! запуска yt-dlp — то есть наследование, на которое опирается этот модуль,
//! измерено, а не предположено.
//!
//! Путь без окружения и окружение без пути не передать по построению:
//! [`YtDlpJsRuntime`] делается только из [`DenoLaunch`], а тот — только от
//! каталога данных.

use std::ffi::OsStr;
use std::fmt::Display;
use std::path::{Path, PathBuf};

use super::error::SidecarError;
use crate::types::LaunchFailedReason;

/// Имя sidecar deno — то, что уходит в [`super::resolve_sidecar_path`].
const DENO_SIDECAR: &str = "deno";

/// Имя рантайма в значении `--js-runtimes <имя>:<путь>` — из словаря
/// yt-dlp, а не имя файла; совпадение с [`DENO_SIDECAR`] не обязательное.
const DENO_RUNTIME: &str = "deno";

/// Имя подкаталога кэша deno внутри каталога данных приложения.
const DENO_DIR_NAME: &str = "deno";

/// Переменная, отключающая фоновую проверку обновлений deno.
const NO_UPDATE_CHECK_VAR: &str = "DENO_NO_UPDATE_CHECK";

/// Переменная, задающая каталог кэша deno.
const DENO_DIR_VAR: &str = "DENO_DIR";

/// Окружение, с которым запускается deno (см. шапку модуля).
///
/// Конструируется только от каталога данных приложения: каталог кэша
/// нельзя собрать из чего-то другого, поэтому «`DENO_DIR` внутри каталога
/// данных» — свойство типа, а не договорённость вызывающих.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DenoEnv {
    deno_dir: PathBuf,
}

impl DenoEnv {
    /// Окружение deno для приложения с каталогом данных `data_dir`.
    pub fn in_data_dir(data_dir: &Path) -> Self {
        Self {
            deno_dir: data_dir.join(DENO_DIR_NAME),
        }
    }

    /// Пары «переменная — значение» в форме, которую принимает запуск
    /// процесса (`crate::sidecar::run_with_env` и родня). Добавляются к
    /// унаследованному окружению, а не заменяют его.
    pub fn vars(&self) -> [(&'static str, &OsStr); 2] {
        [
            (NO_UPDATE_CHECK_VAR, OsStr::new("1")),
            (DENO_DIR_VAR, self.deno_dir.as_os_str()),
        ]
    }
}

/// Всё, что нужно для запуска deno: путь к sidecar и окружение.
///
/// Путь и окружение едут вместе, чтобы запустить deno без окружения было
/// нельзя по построению: нет каталога данных — нет и запуска.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DenoLaunch {
    path: PathBuf,
    env: DenoEnv,
}

impl DenoLaunch {
    /// Запуск deno по пути `path` с окружением от каталога данных `data_dir`.
    pub fn new(path: PathBuf, data_dir: &Path) -> Self {
        Self {
            path,
            env: DenoEnv::in_data_dir(data_dir),
        }
    }

    /// Резолвит deno: sidecar по имени через `resolve_sidecar` плюс
    /// окружение от каталога данных `data_dir`.
    ///
    /// Чистая функция: и каталог данных (у приложения —
    /// `app.path().app_data_dir()`), и резолвер sidecar (у приложения —
    /// [`super::resolve_sidecar_path`]) приходят снаружи, поэтому тест
    /// видит, **какое** имя резолвится и куда ложится `DENO_DIR`.
    ///
    /// Каталог данных, который не определяется, — отказ
    /// ([`LaunchFailedReason::Other`]) с причиной в `stderr`, а не запуск с
    /// кэшем «где-нибудь»: иначе deno без `DENO_DIR` писал бы в домашний
    /// каталог пользователя. Резолвер при этом не зовётся вовсе.
    ///
    /// В лог функция не пишет: причину пишет вызывающий, одной строкой, и
    /// у двух потребителей она разная (см. [`DenoLaunch::failure_reason`]).
    pub fn resolve<E: Display>(
        data_dir: Result<PathBuf, E>,
        resolve_sidecar: impl FnOnce(&str) -> Result<PathBuf, SidecarError>,
    ) -> Result<Self, SidecarError> {
        let data_dir = data_dir.map_err(|err| SidecarError::LaunchFailed {
            reason: LaunchFailedReason::Other,
            stderr: format!("каталог данных приложения не определяется: {err}"),
        })?;

        Ok(Self::new(resolve_sidecar(DENO_SIDECAR)?, &data_dir))
    }

    /// Путь к бинарнику deno.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Окружение, с которым deno запускается.
    pub fn env(&self) -> &DenoEnv {
        &self.env
    }

    /// Причина отказа [`DenoLaunch::resolve`] одной строкой для лога:
    /// текст из `stderr`, если он есть (каталог данных), иначе `Display`
    /// ошибки (резолвер sidecar).
    pub fn failure_reason(error: &SidecarError) -> String {
        match error {
            SidecarError::LaunchFailed { stderr, .. }
            | SidecarError::NonZeroExit { stderr, .. }
            | SidecarError::Timeout { stderr, .. }
                if !stderr.trim().is_empty() =>
            {
                stderr.trim().to_string()
            }
            other => other.to_string(),
        }
    }
}

/// Флаг, которым yt-dlp получает путь к рантайму.
const JS_RUNTIMES_FLAG: &str = "--js-runtimes";

/// Флаг, отключающий JS-рантаймы yt-dlp целиком.
const NO_JS_RUNTIMES_FLAG: &str = "--no-js-runtimes";

/// JS-рантайм запуска yt-dlp — аргументы и окружение одним значением
/// (см. шапку модуля).
///
/// Единственный построитель для обоих запусков yt-dlp: разбора
/// (`crate::probe::SidecarLauncher`) и скачивания
/// (`crate::download::SidecarDownloader`). Запускатель берёт у него и
/// [`YtDlpJsRuntime::argv`], и [`YtDlpJsRuntime::env`] — поэтому путь к
/// deno без `DENO_*` в окружении (или наоборот) в процесс не попадает.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YtDlpJsRuntime {
    runtime: Runtime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Runtime {
    /// `--js-runtimes deno:<путь>` и окружение [`DenoEnv`].
    Deno { spec: String, env: DenoEnv },
    /// `--no-js-runtimes`, окружение не добавляется.
    Disabled,
}

impl YtDlpJsRuntime {
    /// Рантайм по исходу резолва deno.
    ///
    /// Отказ резолва — `--no-js-runtimes`, а не пропуск флага (иначе yt-dlp
    /// возьмёт deno из `PATH`, см. шапку модуля); причина пишется в лог
    /// одной строкой. Путь, который не записывается строкой UTF-8, — тот же
    /// отказ: argv запуска у проекта строковый, а подменять путь его
    /// лоссовой копией значило бы указать yt-dlp несуществующий файл.
    pub fn from_deno(deno: Result<DenoLaunch, SidecarError>) -> Self {
        let launch = match deno {
            Ok(launch) => launch,
            Err(error) => {
                return Self::disabled(&DenoLaunch::failure_reason(&error));
            }
        };

        let Some(path) = launch.path.to_str() else {
            return Self::disabled(&format!(
                "путь к deno не записывается строкой UTF-8: {}",
                launch.path.display()
            ));
        };

        Self {
            runtime: Runtime::Deno {
                spec: format!("{DENO_RUNTIME}:{path}"),
                env: launch.env,
            },
        }
    }

    /// Рантайм отключён по причине `reason` (пишется в лог).
    fn disabled(reason: &str) -> Self {
        eprintln!("yt-dlp: JS-рантайм отключён ({NO_JS_RUNTIMES_FLAG}): {reason}");
        Self {
            runtime: Runtime::Disabled,
        }
    }

    /// Полный argv запуска: аргументы рантайма, затем `args`.
    ///
    /// Аргументы рантайма идут **первыми** — заведомо до разделителя `--`,
    /// за которым у обоих запусков стоит только ссылка (Ф-2 E2, Ф-1 E3).
    pub fn argv<'a>(&'a self, args: &[&'a str]) -> Vec<&'a str> {
        let mut argv = match &self.runtime {
            Runtime::Deno { spec, .. } => vec![JS_RUNTIMES_FLAG, spec.as_str()],
            Runtime::Disabled => vec![NO_JS_RUNTIMES_FLAG],
        };
        argv.extend_from_slice(args);
        argv
    }

    /// Добавочное окружение процесса yt-dlp: [`DenoEnv`] или ничего.
    pub fn env(&self) -> Vec<(&'static str, &OsStr)> {
        match &self.runtime {
            Runtime::Deno { env, .. } => env.vars().to_vec(),
            Runtime::Disabled => Vec::new(),
        }
    }
}

/// Оснастка тестов запуска yt-dlp с рантаймом: скрипт-заглушка, который
/// записывает свой argv и окружение, — общая для тестов разбора и
/// скачивания, чтобы оба пути проверялись одним и тем же способом.
#[cfg(test)]
pub mod testing {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    /// Что увидел запущенный процесс.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct LaunchRecord {
        pub args: Vec<String>,
        /// `None` — переменная не задана вовсе.
        pub deno_no_update_check: Option<String>,
        pub deno_dir: Option<String>,
        /// Унаследованный `PATH` — свидетель того, что окружение
        /// добавляется, а не заменяет родительское.
        pub path: Option<String>,
    }

    impl LaunchRecord {
        pub fn read(record: &Path) -> Self {
            let text = fs::read_to_string(record)
                .unwrap_or_else(|err| panic!("заглушка не записала запуск {record:?}: {err}"));
            let mut parsed = Self {
                args: Vec::new(),
                deno_no_update_check: None,
                deno_dir: None,
                path: None,
            };
            for line in text.lines() {
                if let Some(arg) = line.strip_prefix("arg=") {
                    parsed.args.push(arg.to_string());
                } else if let Some(value) = line.strip_prefix("env DENO_NO_UPDATE_CHECK=") {
                    parsed.deno_no_update_check = Some(value.to_string());
                } else if let Some(value) = line.strip_prefix("env DENO_DIR=") {
                    parsed.deno_dir = Some(value.to_string());
                } else if let Some(value) = line.strip_prefix("env PATH=") {
                    parsed.path = Some(value.to_string());
                }
            }
            parsed
        }

        /// Аргументы до разделителя `--`; паникует, если его нет.
        pub fn args_before_separator(&self) -> &[String] {
            let at = self
                .args
                .iter()
                .position(|arg| arg == "--")
                .expect("в argv yt-dlp обязан быть разделитель `--`");
            &self.args[..at]
        }
    }

    /// Пишет в `dir` исполняемую заглушку yt-dlp: она записывает argv и
    /// окружение в файл и выходит с кодом 0. Возвращает путь к заглушке и
    /// к файлу записи. Переменная, которой нет, не пишется вовсе — так
    /// «не задана» отличается от «задана пустой».
    pub fn recording_ytdlp(dir: &Path) -> (PathBuf, PathBuf) {
        let record = dir.join("launch-record.txt");
        let script = dir.join("yt-dlp-recording.sh");
        let text = format!(
            "#!/bin/sh\n\
             : > '{record}'\n\
             for arg in \"$@\"; do printf 'arg=%s\\n' \"$arg\" >> '{record}'; done\n\
             for name in DENO_NO_UPDATE_CHECK DENO_DIR PATH; do\n\
             \x20 eval \"set_=\\${{$name+x}} value_=\\${{$name}}\"\n\
             \x20 if [ -n \"$set_\" ]; then printf 'env %s=%s\\n' \"$name\" \"$value_\" >> '{record}'; fi\n\
             done\n\
             exit 0\n",
            record = record.display(),
        );
        fs::write(&script, text).expect("заглушка yt-dlp пишется");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755))
            .expect("заглушка yt-dlp делается исполняемой");
        (script, record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::cell::RefCell;

    fn value_of<'a>(vars: &[(&'static str, &'a OsStr)], name: &str) -> Option<&'a OsStr> {
        vars.iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| *value)
    }

    #[test]
    fn disables_the_update_check() {
        let env = DenoEnv::in_data_dir(Path::new("/data/tube-leak"));
        let vars = env.vars();

        assert_eq!(
            value_of(&vars, "DENO_NO_UPDATE_CHECK"),
            Some(OsStr::new("1"))
        );
    }

    #[test]
    fn puts_the_deno_cache_inside_the_app_data_dir() {
        let data_dir = Path::new("/data/tube-leak");
        let env = DenoEnv::in_data_dir(data_dir);
        let vars = env.vars();

        let deno_dir = Path::new(value_of(&vars, "DENO_DIR").expect("DENO_DIR must be set"));
        assert_eq!(deno_dir, data_dir.join("deno"));
        assert!(deno_dir.starts_with(data_dir));
    }

    // ───────────────────────── резолв (DenoLaunch) ──────────────────────

    #[test]
    fn resolve_asks_for_the_deno_sidecar_and_puts_deno_dir_in_the_data_dir() {
        // Остаток ревью TL-110: мутация «резолвить `yt-dlp` вместо `deno`»
        // выживала — связку имени и каталога не видел ни один тест.
        let asked = RefCell::new(Vec::new());
        let data_dir = PathBuf::from("/data/tube-leak");

        let launch = DenoLaunch::resolve(Ok::<_, String>(data_dir.clone()), |name| {
            asked.borrow_mut().push(name.to_string());
            Ok(PathBuf::from(format!("/app/bin/{name}")))
        })
        .expect("каталог данных и sidecar есть — запуск есть");

        assert_eq!(asked.into_inner(), ["deno"], "резолвится sidecar deno");
        assert_eq!(launch.path(), Path::new("/app/bin/deno"));
        let vars = launch.env().vars();
        assert_eq!(
            value_of(&vars, "DENO_DIR").map(Path::new),
            Some(data_dir.join("deno").as_path())
        );
    }

    #[test]
    fn resolve_without_a_data_dir_refuses_and_never_resolves_the_sidecar() {
        let asked = RefCell::new(0);

        let error = DenoLaunch::resolve(Err("нет каталога"), |_| {
            *asked.borrow_mut() += 1;
            Ok(PathBuf::from("/app/bin/deno"))
        })
        .expect_err("без каталога данных deno не запускается");

        assert_eq!(*asked.borrow(), 0);
        assert_eq!(
            error,
            SidecarError::LaunchFailed {
                reason: LaunchFailedReason::Other,
                stderr: "каталог данных приложения не определяется: нет каталога".to_string(),
            }
        );
        assert_eq!(
            DenoLaunch::failure_reason(&error),
            "каталог данных приложения не определяется: нет каталога"
        );
    }

    #[test]
    fn a_resolver_failure_travels_as_is() {
        let error = DenoLaunch::resolve(Ok::<_, String>(PathBuf::from("/data")), |_| {
            Err(SidecarError::NotFound)
        })
        .expect_err("резолвер отказал");

        assert_eq!(error, SidecarError::NotFound);
        assert_eq!(
            DenoLaunch::failure_reason(&error),
            "sidecar binary not found"
        );
    }

    // ─────────────────────── аргументы yt-dlp (TL-109) ──────────────────

    const PROBE_LIKE: [&str; 4] = ["-J", "--no-playlist", "--", "https://youtu.be/x"];

    #[test]
    fn an_available_deno_is_passed_by_absolute_path_before_the_separator() {
        let runtime = YtDlpJsRuntime::from_deno(Ok(DenoLaunch::new(
            PathBuf::from("/app/bin/deno"),
            Path::new("/data/tube-leak"),
        )));

        let argv = runtime.argv(&PROBE_LIKE);
        let separator = argv.iter().position(|arg| *arg == "--").expect("`--`");
        let flag = argv
            .iter()
            .position(|arg| *arg == "--js-runtimes")
            .expect("--js-runtimes");

        assert!(flag + 1 < separator, "флаг и значение — до `--`: {argv:?}");
        assert_eq!(argv[flag + 1], "deno:/app/bin/deno");
        assert!(!argv.contains(&"--no-js-runtimes"));
        assert_eq!(&argv[argv.len() - 4..], PROBE_LIKE, "свои аргументы целы");

        let env = runtime.env();
        assert_eq!(
            value_of(&env, "DENO_NO_UPDATE_CHECK"),
            Some(OsStr::new("1"))
        );
        assert_eq!(
            value_of(&env, "DENO_DIR").map(Path::new),
            Some(Path::new("/data/tube-leak/deno"))
        );
    }

    #[test]
    fn without_deno_or_without_a_data_dir_runtimes_are_disabled_and_no_deno_env_is_added() {
        for failure in [
            SidecarError::NotFound,
            SidecarError::LaunchFailed {
                reason: LaunchFailedReason::Other,
                stderr: "каталог данных приложения не определяется: test".to_string(),
            },
        ] {
            let case = format!("{failure:?}");
            let runtime = YtDlpJsRuntime::from_deno(Err(failure));
            let argv = runtime.argv(&PROBE_LIKE);

            assert_eq!(argv[0], "--no-js-runtimes", "{case}");
            assert!(
                !argv.iter().any(|arg| arg.starts_with("--js-runtimes")),
                "{case}: {argv:?}"
            );
            assert_eq!(&argv[1..], PROBE_LIKE);
            assert!(runtime.env().is_empty(), "{case}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_path_that_is_not_utf8_disables_runtimes_instead_of_pointing_elsewhere() {
        use std::os::unix::ffi::OsStrExt;

        let path = PathBuf::from(OsStr::from_bytes(b"/app/bin/\xFFdeno"));
        let runtime = YtDlpJsRuntime::from_deno(Ok(DenoLaunch::new(path, Path::new("/data"))));

        assert_eq!(runtime.argv(&[]), ["--no-js-runtimes"]);
        assert!(runtime.env().is_empty());
    }
}
