//! Окружение процесса deno — JavaScript-рантайма, который yt-dlp запускает
//! для YouTube-извлечения (TL-110, решение владельца #114).
//!
//! deno поставляется третьим sidecar (`externalBin` `binaries/deno`, TL-108)
//! и резолвится тем же [`super::resolve_sidecar_path`], что и ffmpeg. Этот
//! модуль отвечает только за то, **с каким окружением** deno запускается,
//! — в одном месте, потому что потребителей два:
//!
//! - проверка версии на служебном экране (`crate::commands::sidecar`,
//!   TL-110) запускает `deno --version` сама;
//! - разбор и скачивание (TL-109) запускают yt-dlp, а deno — его потомок и
//!   наследует окружение yt-dlp; туда уходят те же переменные.
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
//!   (замер TL-110: каталог остался пустым), а если он понадобится
//!   рантайму, deno создаст его сам.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

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
    /// процесса (`crate::sidecar::run_with_env`). Добавляются к
    /// унаследованному окружению, а не заменяют его.
    pub fn vars(&self) -> [(&'static str, &OsStr); 2] {
        [
            (NO_UPDATE_CHECK_VAR, OsStr::new("1")),
            (DENO_DIR_VAR, self.deno_dir.as_os_str()),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value_of<'a>(vars: &'a [(&'static str, &'a OsStr)], name: &str) -> Option<&'a OsStr> {
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
}
