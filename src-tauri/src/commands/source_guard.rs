//! Сторож по исходникам: где в продакшен-коде открывается хранилище.
//!
//! Общий для истории (TL-90) и настроек (TL-91): у обоих хранилищ процессный
//! флаг, и лишний вызов `…Store::open` в продакшене забрал бы флаг первым —
//! хранилище на сеанс тихо стало бы недоступно у настоящего владельца. Что
//! сканер ловит и чего не ловит, записано в doc
//! `commands::history::tests::the_store_is_opened_in_exactly_one_production_place`;
//! правила здесь те же для любого типа.

use std::fs;
use std::path::Path;

/// Что нашёл сканер в продакшен-коде.
pub(super) struct Places {
    /// Места токена `<Тип>::open` (с вызовом или ссылкой на функцию), по
    /// одному на вхождение: `путь:строка` относительно `src/`.
    pub calls: Vec<String>,
    /// Однострочные переименованный импорт (`<Тип> as …`) и псевдоним самого
    /// типа (`type … = …<Тип>;`).
    pub aliases: Vec<String>,
    /// Сколько файлов просмотрено — чтобы сторож не ослеп молча.
    pub scanned: usize,
}

/// Сколько раз в строке кода встречается токен `token` — но не как начало
/// более длинного имени (`open_isolated`, `opener`).
pub(super) fn tokens(code: &str, token: &str) -> usize {
    code.match_indices(token)
        .filter(|(at, _)| {
            !code[at + token.len()..].starts_with(|c: char| c == '_' || c.is_alphanumeric())
        })
        .count()
}

/// Обходит все `.rs` под `src/`, кроме `*_tests.rs`, пропуская строки,
/// начинающиеся с `//`.
pub(super) fn production_places(type_name: &str) -> Places {
    let token = format!("{type_name}::open");
    let renamed_import = format!("{type_name} as ");
    let qualified = format!("::{type_name}");
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut places = Places {
        calls: Vec::new(),
        aliases: Vec::new(),
        scanned: 0,
    };
    let mut stack = vec![src.clone()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("каталог обходится") {
            let path = entry.expect("элемент").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            if !name.ends_with(".rs") || name.ends_with("_tests.rs") {
                continue;
            }
            places.scanned += 1;
            let text = fs::read_to_string(&path).expect("исходник читается");
            for (number, line) in text.lines().enumerate() {
                let code = line.trim_start();
                if code.starts_with("//") {
                    continue;
                }
                let place = format!(
                    "{}:{}",
                    path.strip_prefix(&src).expect("под src").display(),
                    number + 1
                );
                places
                    .calls
                    .extend((0..tokens(code, &token)).map(|_| place.clone()));
                let type_alias = code.split_whitespace().any(|word| word == "type")
                    && code.split_once('=').is_some_and(|(_, rhs)| {
                        let rhs = rhs.trim().trim_end_matches(';').trim_end();
                        rhs == type_name || rhs.ends_with(&qualified)
                    });
                if code.contains(&renamed_import) || type_alias {
                    places.aliases.push(place);
                }
            }
        }
    }
    places
}

/// Номер строки первого токена `token` в коде после строки, начинающейся с
/// `impl_marker` (комментарии пропускаются так же, как в сканере). Так
/// сторож сверяет место, а не только количество.
pub(super) fn first_line_in_impl(text: &str, impl_marker: &str, token: &str) -> usize {
    let mut in_impl = false;
    for (number, line) in text.lines().enumerate() {
        let code = line.trim_start();
        in_impl |= code.starts_with(impl_marker);
        if in_impl && !code.starts_with("//") && tokens(code, token) > 0 {
            return number + 1;
        }
    }
    panic!("токена {token} внутри {impl_marker} нет");
}
