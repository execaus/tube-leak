//! Канонический идентификатор ролика из ссылки — правило сравнения
//! дублей (Ф-8, Р-5) — TL-72.
//!
//! Р-5 формулирует правило прямо: дубль ищется **по id ролика, а не по
//! строке URL**. Одна и та же ссылка приходит в разных формах записи —
//! из адресной строки, из «Поделиться» на телефоне, с лишними
//! параметрами трекинга, — и сравнение строк дало бы сторожа, который
//! пропускает ровно то, ради чего заведён.
//!
//! Функция чистая: сети нет, процессов нет, аллокация одна — на сам id.
//! Наружу, в контракт и TS-зеркало, id **не выходит**: сравнение целиком
//! на стороне ядра (решение декомпозиции E4, зафиксировано в
//! `types.rs`). Второй половиной пары «ролик + качество» служит уже
//! существующее поле `streams` запроса постановки — нового поля контракта
//! эта задача не заводит.
//!
//! # Чем это не является
//!
//! Оркестрация разбора ([`crate::probe`], TL-32) записала инвариант:
//! собственный парсер ссылок YouTube заводить нельзя, адреса — дело
//! yt-dlp. Инвариант не нарушен, и вот граница.
//!
//! Запрещено то, от чего зависит **запуск**: набор аргументов и сама
//! ссылка, уходящая в argv. Здесь не меняется ни то, ни другое — в
//! yt-dlp по-прежнему уезжает исходная строка пользователя, как её
//! ввели, и этот разбор её не касается. Сюда же второй пояс: ни один
//! отказ пользователю на этой функции не стоит. Она отвечает на
//! единственный вопрос — «это точно тот же ролик, что уже стоит в
//! очереди?», — и её единственный неверный ответ, который стоит бояться,
//! это ложное «да»: две **разные** ссылки, слипшиеся в один id, стоят
//! пользователю не скачанного ролика. Ложное «нет» (форма записи не
//! разобрана) стоит ему двух одинаковых загрузок — неприятно, но
//! обратимо и заметно.
//!
//! Отсюда весь строй функции: **белый список форм**, а не чёрный список
//! запретов. Всё, что не разобралось однозначно, — [`None`], а не
//! догадка. Планировщик (TL-73) решает, что делать с [`None`]; правильный
//! для него ответ — «дублем не считать», потому что доказательства
//! тождества у ядра нет.
//!
//! # Откуда взят список форм
//!
//! Не из головы. Две опоры:
//!
//! 1. **Требование Ф-8 и постановка TL-72** перечисляют формы записи
//!    поимённо: `watch?v=ID`, `watch?v=ID&list=…`, `youtu.be/ID`,
//!    `m.youtube.com/watch?v=ID`, `/shorts/ID`, с `www.` и без,
//!    `http`/`https`, произвольный регистр домена, завершающий `/`,
//!    лишние query-параметры. Ровно этот список здесь и реализован.
//! 2. **Замер настоящим yt-dlp** — тем самым пином, который запускает
//!    приложение (2026.08.19, установка в каталоге данных), macOS 26.6,
//!    2026-08-31. Сети в замере нет: запуск шёл через заведомо мёртвый
//!    прокси (`--proxy http://127.0.0.1:1`), а извлечённый id yt-dlp
//!    печатает в строке `ERROR: [youtube] <id>: …` **до** первого
//!    сетевого обмена. Формы E2-фикстур (`tests/fixtures/ytdlp-probe/`)
//!    входят в замер целиком.
//!
//! Что показал замер (в скобках — extractor, которому yt-dlp отдал
//! адрес):
//!
//! | Форма | yt-dlp | Здесь |
//! |---|---|---|
//! | `https://www.youtube.com/watch?v=ID` | `aqz-KE-bpKQ` (youtube) | id |
//! | `http://youtube.com/watch?v=ID` | тот же id (youtube) | id |
//! | `https://m.youtube.com/watch?v=ID` | тот же id (youtube) | id |
//! | `…/watch?v=ID&list=PL1&index=2` | тот же id (youtube) | id |
//! | `…/watch?app=desktop&v=ID`, `…?list=PL1&v=ID` | тот же id (youtube) | id |
//! | `…/watch?v=ID#t=10` | тот же id (youtube) | id |
//! | `…/watch/?v=ID` | тот же id (youtube) | id |
//! | `https://youtu.be/ID`, `…/ID/`, `…/ID?t=30`, `…/ID?si=…` | тот же id (youtube) | id |
//! | `https://www.youtube.com/shorts/ID`, `…/ID/`, `…/ID?feature=share` | тот же id (youtube) | id |
//! | `https://m.youtube.com/shorts/ID`, `https://youtube.com/shorts/ID` | тот же id (youtube) | id |
//! | `https://www.youtube.com/watch/ID` | `generic` — **не** ролик | `None` |
//! | `https://youtu.be/watch?v=ID` | `generic` | `None` |
//! | `https://www.youtube.com//watch?v=ID`, `https://youtu.be//ID` | `generic` | `None` |
//! | `https://www.youtube.com/watch?V=ID` (заглавный ключ) | `generic` | `None` |
//! | `https://www.youtube.com/SHORTS/ID` | `youtube:tab` — вкладка, не ролик | `None` |
//! | `https://m.youtu.be/ID`, `https://www.youtu.be/ID` | `generic` | `None` |
//! | `…/watch?v=aqz-KE-bpK` (10 знаков) | `youtube:truncated_id` — отказ | `None` |
//! | `https://www.youtube.com/embed/ID`, `/v/ID`, `/live/ID` | тот же id (youtube) | `None`, см. ниже |
//! | `https://music.youtube.com/watch?v=ID` | тот же id (youtube) | `None`, см. ниже |
//! | `https://www.youtube-nocookie.com/embed/ID` | тот же id (youtube) | `None`, см. ниже |
//!
//! Последние три строки — сознательный недобор, а не пропуск. Замер
//! показывает, что yt-dlp считает их тем же роликом, но требование Ф-8
//! их не называет, а расширять белый список сверх требования — решение
//! не исполнителя задачи. Цена недобора известна и мала: пользователь,
//! вставивший ссылку из `music.youtube.com` дважды, получит две
//! одинаковые загрузки вместо отказа. Пункт вынесен в отчёт TL-72.
//!
//! # Почему длина id ровно 11
//!
//! Не «известно из интернета», а измерено с двух сторон. Все id во всех
//! фикстурах проекта — 11 знаков из `[A-Za-z0-9_-]`; ссылка с 10 знаками
//! уходит у yt-dlp в extractor `youtube:truncated_id` («Incomplete
//! YouTube ID»), то есть отвергается им самим.
//!
//! Проверка длины здесь не украшение — она несёт весь класс ложных «да».
//! Без неё `https://youtu.be/watch?v=ID` дал бы «id» `watch`, и **любые
//! два** таких адреса слиплись бы в один ролик. Тест на это стоит
//! отдельно.
//!
//! Ссылку длиннее 11 знаков (`…?v=IDxx`) yt-dlp молча обрезает до первых
//! 11 и скачивает ролик `ID`. Здесь такая форма отвергается: обрезание
//! — догадка о внутренностях чужого инструмента, а цена отказа —
//! непойманный дубль у формы записи, которой в живых ссылках не бывает.

use std::fmt;

/// Длина идентификатора ролика — см. «Почему длина id ровно 11» в doc
/// модуля.
const ID_LEN: usize = 11;

/// Хосты, у которых разбираются формы `watch?v=ID` и `/shorts/ID`.
///
/// Список закрытый и сравнивается целиком: хост с портом, с userinfo или
/// с лишним поддоменом в него не попадает и разобран не будет.
const WATCH_HOSTS: [&str; 3] = ["youtube.com", "www.youtube.com", "m.youtube.com"];

/// Хост короткой формы — так копируют с телефона и из «Поделиться».
///
/// Без `www.` и без `m.`: замер показал, что обе приставки уводят адрес
/// в extractor `generic`, то есть роликом для yt-dlp он не является.
const SHORT_HOST: &str = "youtu.be";

/// Канонический идентификатор ролика — то, по чему сравниваются дубли
/// (Ф-8).
///
/// Отдельный тип, а не `String`, ровно за одним: перепутать его с
/// названием, с id задачи или с сырой ссылкой не даст компилятор. За
/// границу окна не уходит и `Serialize` не имеет намеренно — сравнение
/// живёт целиком в ядре.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
// Единственный потребитель — планировщик очереди (TL-73), и до него
// продакшен-вызовов у типа нет. Глушитель снимается вместе с ним;
// прецедент — `super::CHANGED_EVENT`.
#[allow(dead_code)]
pub struct VideoId(String);

#[allow(dead_code)]
impl VideoId {
    /// Идентификатор строкой — для лога и для ключа в структурах
    /// планировщика.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for VideoId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Канонический id ролика из ссылки — или [`None`], если форма записи не
/// разобрана однозначно.
///
/// Разбираемые формы, отказы и обоснование каждого — в doc модуля.
/// Функция тотальна: любая строка, включая пустую, обрезанную посреди
/// UTF-8-последовательности пользователем или собранную из мусора,
/// получает ответ, а не панику.
#[allow(dead_code)]
pub fn canonical_video_id(url: &str) -> Option<VideoId> {
    let url = url.trim();

    // Тот же отказ, что у `probe::validate_url` (Ф-2 E2): валидная
    // ссылка не содержит ни пробелов внутри, ни управляющих символов.
    if url
        .chars()
        .any(|symbol| symbol.is_whitespace() || symbol.is_control())
    {
        return None;
    }

    let rest = strip_scheme(url)?;

    // Фрагмент отбрасывается целиком и до всего остального: `#t=10`
    // адресует место внутри ролика, а не другой ролик (замер: yt-dlp
    // отдаёт тот же id).
    let rest = rest.split('#').next().unwrap_or_default();

    let (host, tail) = split_host(rest)?;
    let (path, query) = tail.split_once('?').unwrap_or((tail, ""));

    // Хост сравнивается без учёта регистра — так требует Ф-8 («регистр
    // домена») и так устроены хосты в RFC 3986. Расхождение с yt-dlp
    // здесь известно и названо в отчёте TL-72: сам yt-dlp на
    // `WWW.YOUTUBE.COM` уходит в extractor `generic`.
    if WATCH_HOSTS
        .iter()
        .any(|known| host.eq_ignore_ascii_case(known))
    {
        // Ключ параметра — строго строчный `v`: замер показал, что
        // `?V=ID` yt-dlp роликом не считает.
        if path == "/watch" || path == "/watch/" {
            return query
                .split('&')
                .find_map(|pair| pair.strip_prefix("v="))
                .and_then(video_id);
        }

        // Путь — строго строчный `shorts`: `/SHORTS/ID` уходит у yt-dlp
        // в `youtube:tab`, то есть означает вкладку канала, а не ролик.
        if let Some(segment) = path.strip_prefix("/shorts/") {
            return path_video_id(segment);
        }

        return None;
    }

    if host.eq_ignore_ascii_case(SHORT_HOST) {
        return path_video_id(path.strip_prefix('/')?);
    }

    None
}

/// Отрезает схему `http://` или `https://` без учёта её регистра.
///
/// Правило то же, что у `probe::validate_url`, и оно там первично: то,
/// что эта функция разобрала, обязано быть ссылкой и с точки зрения
/// разбора (тест `every_accepted_form_is_a_link_probe_would_also_accept`
/// стоит именно на этом). Схема без `//`, чужая схема и ссылка без схемы
/// — не наш случай.
fn strip_scheme(url: &str) -> Option<&str> {
    ["https://", "http://"].into_iter().find_map(|scheme| {
        url.get(..scheme.len())
            .filter(|prefix| prefix.eq_ignore_ascii_case(scheme))
            .map(|_| &url[scheme.len()..])
    })
}

/// Делит остаток адреса на хост и то, что за ним, — либо отказывает.
///
/// Хост кончается первым `/` или `?`. Адрес без того и другого
/// (`https://youtu.be`) пути не содержит, значит и ролика в нём нет.
fn split_host(rest: &str) -> Option<(&str, &str)> {
    let end = rest.find(['/', '?'])?;
    Some(rest.split_at(end))
}

/// Идентификатор из сегмента пути: допускается ровно один завершающий
/// `/` (Ф-8), всё остальное за id — отказ.
///
/// Глубокий путь (`youtu.be/ID/extra`) yt-dlp разбирает, а здесь он
/// отвергается: цена — непойманный дубль у формы, которой в живых
/// ссылках не бывает, а разрешать «что угодно после id» — это ровно
/// чёрный список наизнанку.
fn path_video_id(segment: &str) -> Option<VideoId> {
    video_id(segment.strip_suffix('/').unwrap_or(segment))
}

/// Проверяет, что перед нами идентификатор ролика, и заворачивает его.
///
/// Ровно [`ID_LEN`] знаков из `[A-Za-z0-9_-]`. Проверка по байтам
/// безопасна: любой байт вне ASCII отвергается тем же условием, значит
/// длина в байтах равна длине в символах.
fn video_id(token: &str) -> Option<VideoId> {
    if token.len() != ID_LEN {
        return None;
    }

    token
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        .then(|| VideoId(token.to_owned()))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::probe::validate_url;

    /// Ролик замера — Big Buck Bunny 4K, тот же, на котором сняты
    /// фикстуры разбора (`tests/fixtures/ytdlp-probe/`).
    const ID: &str = "aqz-KE-bpKQ";

    /// Формы записи **одной и той же** ссылки. Каждая проверена живым
    /// yt-dlp (таблица в doc модуля): он на всех отдаёт `aqz-KE-bpKQ`.
    const SAME_LINK: [&str; 16] = [
        "https://www.youtube.com/watch?v=aqz-KE-bpKQ",
        "http://www.youtube.com/watch?v=aqz-KE-bpKQ",
        "https://youtube.com/watch?v=aqz-KE-bpKQ",
        "https://m.youtube.com/watch?v=aqz-KE-bpKQ",
        "HTTPS://WWW.YOUTUBE.COM/watch?v=aqz-KE-bpKQ",
        "https://www.youtube.com/watch/?v=aqz-KE-bpKQ",
        "https://www.youtube.com/watch?v=aqz-KE-bpKQ&list=PLbpi6ZahtOH6&index=2",
        "https://www.youtube.com/watch?app=desktop&v=aqz-KE-bpKQ",
        "https://www.youtube.com/watch?list=PLbpi6ZahtOH6&v=aqz-KE-bpKQ",
        "https://www.youtube.com/watch?v=aqz-KE-bpKQ#t=10",
        "https://youtu.be/aqz-KE-bpKQ",
        "http://youtu.be/aqz-KE-bpKQ",
        "https://youtu.be/aqz-KE-bpKQ/",
        "https://youtu.be/aqz-KE-bpKQ?si=Kx1yQ7wSomething",
        "https://www.youtube.com/shorts/aqz-KE-bpKQ",
        "  https://m.youtube.com/shorts/aqz-KE-bpKQ?feature=share  ",
    ];

    fn id_of(url: &str) -> Option<String> {
        canonical_video_id(url).map(|id| id.as_str().to_owned())
    }

    // ───────────────────── Ф-8: одна ссылка — один id ──────────────────

    #[test]
    fn every_written_form_of_one_link_gives_one_id() {
        // Критерий приёмки TL-72: не меньше восьми форм записи одной
        // ссылки дают идентичный id. Их шестнадцать, и каждая снята
        // замером, а не придумана.
        for url in SAME_LINK {
            assert_eq!(
                id_of(url).as_deref(),
                Some(ID),
                "форма записи «{url}» — тот же ролик"
            );
        }

        let distinct: HashSet<Option<String>> = SAME_LINK.iter().map(|url| id_of(url)).collect();
        assert_eq!(
            distinct.len(),
            1,
            "все формы обязаны схлопнуться в один ответ, а получилось {distinct:?}"
        );
    }

    #[test]
    fn different_videos_never_share_an_id() {
        // Вторая половина сторожа. Ролики — из фикстур E2, формы записи
        // намеренно разные: слипнуться они не должны ни в одной паре,
        // включая пары «разная форма, разный ролик».
        let links = [
            "https://www.youtube.com/watch?v=aqz-KE-bpKQ",
            "https://youtu.be/jNQXAC9IVRw",
            "https://m.youtube.com/watch?v=jfKfPfyJRdk",
            "https://www.youtube.com/shorts/kJQP7kiw5Fk",
            "https://www.youtube.com/watch?v=9bZkp7q19f0&list=PL1",
            "https://youtu.be/UbAsuvO-164/",
            "https://www.youtube.com/watch?v=00000000000",
            "https://www.youtube.com/watch?v=_-_-_-_-_-_",
        ];

        let ids: Vec<String> = links
            .iter()
            .map(|url| id_of(url).unwrap_or_else(|| panic!("«{url}» — разбираемая форма")))
            .collect();

        let distinct: HashSet<&String> = ids.iter().collect();
        assert_eq!(
            distinct.len(),
            links.len(),
            "разные ролики обязаны остаться разными, а получилось {ids:?}"
        );
    }

    #[test]
    fn two_different_links_never_collapse_into_one_id() {
        // Сторож ложного «да» — того единственного неверного ответа,
        // который стоит пользователю не скачанного ролика (doc модуля).
        //
        // Пары подобраны под конкретные ослабления разбора, а не «на
        // всякий случай»: снятая проверка длины склеивает первую пару в
        // id `watch`, снятый белый список путей — вторую в `playlist`,
        // обрезание длинного токена до одиннадцати знаков — третью,
        // приведение id к одному регистру — четвёртую, «нормализация»
        // `-`/`_` — пятую, снятый белый список хостов — шестую.
        //
        // Утверждение — «не слиплись», а не «обе разобрались»: часть пар
        // сейчас честно отдаёт [`None`] с обеих сторон, и это правильный
        // ответ. Красным тест становится ровно тогда, когда две разные
        // ссылки начинают отдавать один и тот же id.
        let pairs = [
            (
                "https://youtu.be/watch?v=aqz-KE-bpKQ",
                "https://youtu.be/watch?v=jNQXAC9IVRw",
            ),
            (
                "https://www.youtube.com/playlist?list=PLbpi6ZahtOH6",
                "https://www.youtube.com/playlist?list=PLE-hcHh2ZvPs",
            ),
            (
                "https://www.youtube.com/watch?v=aqz-KE-bpKQZZ",
                "https://www.youtube.com/watch?v=aqz-KE-bpKQXX",
            ),
            (
                "https://www.youtube.com/watch?v=aqz-KE-bpKQ",
                "https://www.youtube.com/watch?v=aqz-KE-bpKq",
            ),
            (
                "https://www.youtube.com/watch?v=aqz-KE-bpKQ",
                "https://www.youtube.com/watch?v=aqz_KE_bpKQ",
            ),
            (
                "https://youtu.be/aqz-KE-bpKQ",
                "https://evil.example/aqz-KE-bpKQ",
            ),
        ];

        for (left, right) in pairs {
            assert_ne!(left, right, "пара обязана быть из разных ссылок");

            let (left_id, right_id) = (id_of(left), id_of(right));
            assert!(
                left_id.is_none() || left_id != right_id,
                "разные ссылки «{left}» и «{right}» слиплись в один ролик {left_id:?}"
            );
        }
    }

    #[test]
    fn the_id_keeps_every_character_class_youtube_uses() {
        // Дефис и подчёркивание — обычные знаки id; потеря любого из них
        // склеила бы разные ролики.
        for id in ["aqz-KE-bpKQ", "_-_-_-_-_-_", "00000000000", "aaaaaaaaaaa"] {
            assert_eq!(
                id_of(&format!("https://youtu.be/{id}")).as_deref(),
                Some(id)
            );
        }
    }

    // ─────────────── Отказы: то, что yt-dlp роликом не считает ─────────

    #[test]
    fn forms_yt_dlp_does_not_treat_as_a_video_are_refused() {
        // Каждая строка — замер (таблица в doc модуля), а не догадка:
        // на этих формах пинованный yt-dlp уходит в `generic`,
        // `youtube:tab` или `youtube:truncated_id`.
        for url in [
            "https://www.youtube.com/watch/aqz-KE-bpKQ",
            "https://youtu.be/watch?v=aqz-KE-bpKQ",
            "https://www.youtube.com//watch?v=aqz-KE-bpKQ",
            "https://youtu.be//aqz-KE-bpKQ",
            "https://www.youtube.com/watch?V=aqz-KE-bpKQ",
            "https://www.youtube.com/SHORTS/aqz-KE-bpKQ",
            "https://m.youtu.be/aqz-KE-bpKQ",
            "https://www.youtu.be/aqz-KE-bpKQ",
            "https://www.youtube.com/watch?v=aqz-KE-bpK",
            "https://www.youtube.com/watch?v=",
        ] {
            assert_eq!(id_of(url), None, "«{url}» роликом не является");
        }
    }

    #[test]
    fn a_truncated_or_overlong_id_is_refused() {
        // Ровно [`ID_LEN`] знаков. Короче — `youtube:truncated_id` у
        // самого yt-dlp; длиннее — он молча берёт первые одиннадцать, но
        // догадываться об этом здесь не за что.
        for id in ["aqz-KE-bpK", "aqz-KE-bpKQZ", "aqz-KE-bpKQZZ", ""] {
            assert_eq!(id_of(&format!("https://youtu.be/{id}")), None, "id «{id}»");
            assert_eq!(
                id_of(&format!("https://www.youtube.com/watch?v={id}")),
                None,
                "id «{id}»"
            );
        }
    }

    #[test]
    fn a_deep_path_after_the_id_is_refused() {
        // yt-dlp такой адрес разбирает, здесь он отвергается сознательно
        // (doc `path_video_id`): «что угодно после id» — чёрный список
        // наизнанку.
        assert_eq!(id_of("https://youtu.be/aqz-KE-bpKQ/extra"), None);
        assert_eq!(
            id_of("https://www.youtube.com/shorts/aqz-KE-bpKQ/extra"),
            None
        );
    }

    #[test]
    fn addresses_that_are_not_a_single_video_are_refused() {
        // Ровно те адреса, на которых E2 живьём получила классы
        // «плейлист» и «канал» (`tests/fixtures/ytdlp-probe/`): ролика в
        // них нет, и придумывать его нельзя.
        for url in [
            "https://www.youtube.com/playlist?list=PLbpi6ZahtOH6Blw3RGYpWkSByi_T7Rygb",
            "https://www.youtube.com/channel/UCBR8-60-B28hp2BmDPdntcQ",
            "https://www.youtube.com/@NASA",
            "https://www.youtube.com/@NASA/videos",
            "https://www.youtube.com/",
            "https://youtu.be/",
            "https://youtu.be",
        ] {
            assert_eq!(id_of(url), None, "«{url}» — не ролик");
        }
    }

    #[test]
    fn a_host_that_only_looks_like_youtube_is_refused() {
        // Белый список сравнивается целиком, а не «содержит»: иначе
        // чужой домен с youtube.com в имени получил бы наш id.
        for url in [
            "https://youtube.com.evil.example/watch?v=aqz-KE-bpKQ",
            "https://notyoutube.com/watch?v=aqz-KE-bpKQ",
            "https://youtu.be.evil.example/aqz-KE-bpKQ",
            "https://evil.example/youtu.be/aqz-KE-bpKQ",
            "https://user@www.youtube.com/watch?v=aqz-KE-bpKQ",
            "https://www.youtube.com:443/watch?v=aqz-KE-bpKQ",
            "ftp://www.youtube.com/watch?v=aqz-KE-bpKQ",
            "www.youtube.com/watch?v=aqz-KE-bpKQ",
        ] {
            assert_eq!(id_of(url), None, "«{url}» — чужой адрес");
        }
    }

    // ────────────────────── Тотальность и согласие ─────────────────────

    #[test]
    fn junk_gets_an_answer_and_never_a_panic() {
        for input in [
            "",
            "   ",
            "просто текст",
            "-о--",
            "--flat-playlist",
            "/Users/me/video.mp4",
            "javascript:alert(1)",
            "https://",
            "https:///watch?v=aqz-KE-bpKQ",
            "https://www.youtube.com/watch?v=aqz KE bpKQ",
            "https://www.youtube.com/watch?v=aqz-KE-bpKQ%26list=PL1",
            "https://пример.рф/watch?v=aqz-KE-bpKQ",
            "https://www.youtube.com/watch?v=аqz-KE-bpKQ",
            "https://www.youtube.com/watch?v=🎬🎬🎬🎬🎬🎬🎬🎬🎬🎬🎬",
        ] {
            assert_eq!(id_of(input), None, "ввод «{input}»");
        }
    }

    #[test]
    fn no_generated_string_panics_and_no_answer_breaks_the_id_shape() {
        // Перебор, а не пример: строки собираются из кусков, которыми
        // формы записи и отличаются. Проверяется двоякое — что ответ
        // вообще есть (паники нет) и что всякое «да» — настоящий id, а
        // не что-то, что просто оказалось на его месте.
        const PIECES: [&str; 12] = [
            "https://",
            "www.youtube.com",
            "youtu.be",
            "/",
            "?",
            "#",
            "&",
            "v=",
            "watch",
            "aqz-KE-bpKQ",
            "%2F",
            "ы",
        ];

        let mut checked = 0_u32;
        let mut accepted = 0_u32;
        let mut input = String::new();

        for length in 1..=5_u32 {
            for mut code in 0..PIECES.len().pow(length) {
                input.clear();
                for _ in 0..length {
                    input.push_str(PIECES[code % PIECES.len()]);
                    code /= PIECES.len();
                }

                checked += 1;
                if let Some(id) = canonical_video_id(&input) {
                    accepted += 1;
                    assert_eq!(id.as_str().len(), ID_LEN, "ввод «{input}»");
                    assert!(
                        id.as_str().bytes().all(|byte| byte.is_ascii_alphanumeric()
                            || byte == b'-'
                            || byte == b'_'),
                        "ввод «{input}» дал id «{id}»"
                    );
                }
            }
        }

        assert_eq!(checked, 271_452, "перебор обязан быть полным");
        assert!(
            accepted > 0,
            "перебор, ничего не принявший, ничего и не проверил"
        );
    }

    #[test]
    fn every_accepted_form_is_a_link_probe_would_also_accept() {
        // Шов с E2: белый список этой функции обязан быть подмножеством
        // того, что разбор вообще пропускает к yt-dlp (Ф-2). Иначе ядро
        // считало бы дублем ввод, который до загрузки не доходит вовсе.
        for url in SAME_LINK {
            assert!(
                canonical_video_id(url).is_some(),
                "форма «{url}» разбирается здесь"
            );
            assert!(
                validate_url(url).is_ok(),
                "форму «{url}» обязан пропускать и разбор ссылки (E2)"
            );
        }
    }
}
