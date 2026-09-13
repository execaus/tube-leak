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
//!    лишние query-параметры.
//! 2. **Замер настоящим yt-dlp** — тем самым пином, который запускает
//!    приложение (2026.08.19, установка в каталоге данных), macOS 26.6,
//!    2026-08-31. Сети в замере нет: запуск шёл через заведомо мёртвый
//!    прокси (`--proxy http://127.0.0.1:1`), а извлечённый id yt-dlp
//!    печатает в строке `ERROR: [youtube] <id>: …` **до** первого
//!    сетевого обмена. Формы E2-фикстур (`tests/fixtures/ytdlp-probe/`)
//!    входят в замер целиком.
//!
//! Белый список шире буквы Ф-8, и это решение ведущего, а не самодеятельность
//! разбора: принимаются формы, про которые **измерено**, что yt-dlp
//! извлекает из них тот же самый id, — то есть скачается ровно тот же
//! файл. Тождество здесь не угадывается, а берётся у инструмента,
//! который его и определяет. Обоснование — несимметричная цена ошибки:
//! пропущенный дубль стоит второй часовой загрузки того же ролика и
//! удвоенных обращений к YouTube, а
//! ложного слипания разных роликов эти формы дать не могут по самому
//! замеру.
//!
//! Расширять список дальше «по аналогии» нельзя: следующая форма
//! добавляется тем же способом — замером, а не рассуждением.
//!
//! # Матрица «хост × форма пути» (замер 2026-08-31)
//!
//! Все тридцать ячеек отдали `aqz-KE-bpKQ` через extractor `youtube` —
//! шесть хостов ([`VIDEO_HOSTS`]) на пять форм (`watch?v=ID` плюс
//! четыре [`ID_PATH_PREFIXES`]):
//!
//! ```text
//!                            watch?v=  shorts/  embed/  v/  live/
//! youtube.com                   id       id      id     id   id
//! www.youtube.com               id       id      id     id   id
//! m.youtube.com                 id       id      id     id   id
//! music.youtube.com             id       id      id     id   id
//! youtube-nocookie.com          id       id      id     id   id
//! www.youtube-nocookie.com      id       id      id     id   id
//! ```
//!
//! Отдельно измерено, что матрицу не ломают ни завершающий `/`
//! (`/embed/ID/`, `/v/ID/`, `/live/ID/`), ни измеренный query
//! (`/embed/ID?start=30`, `music…/watch?v=ID&list=PL1`), ни форма
//! `music…/watch/?v=ID`. Матрица закрыта тестом
//! `every_measured_host_and_path_form_gives_the_same_id`: он перебирает
//! те же тридцать ячеек.
//!
//! **Query у форм с id в пути разбирается по своему белому списку** —
//! см. [`PATH_FORM_QUERY_PARAMS`]. Это не педантизм: там же нашёлся
//! настоящий класс ложных «да» (`/shorts/videoseries?list=…` и
//! `/embed/live_stream?channel=…` — оба плейсхолдера ровно в
//! одиннадцать знаков), и он существовал ещё до расширения матрицы,
//! потому что форма `/shorts/` названа самим требованием Ф-8.
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
//! | `https://www.youtube.com/embed/ID`, `/v/ID`, `/live/ID` | тот же id (youtube) | id |
//! | `https://music.youtube.com/watch?v=ID` | тот же id (youtube) | id |
//! | `https://www.youtube-nocookie.com/embed/ID` | тот же id (youtube) | id |
//! | `https://www.youtube.com/EMBED/ID` | `youtube:tab` — вкладка, не ролик | `None` |
//! | `https://music.youtube.com/watch?V=ID` | `generic` | `None` |
//! | `https://youtu.be/ID/extra` | тот же id (youtube) | `None`, см. `path_video_id` |
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

/// Хосты, у которых разбираются формы `watch?v=ID`, `/shorts/ID`,
/// `/embed/ID`, `/v/ID` и `/live/ID`.
///
/// Список закрытый и сравнивается целиком: хост с портом, с userinfo или
/// с лишним поддоменом в него не попадает и разобран не будет.
///
/// Шесть хостов, а не три: `music.` и `youtube-nocookie.com` добавлены
/// по замеру (см. матрицу в doc модуля), а не по догадке.
const VIDEO_HOSTS: [&str; 6] = [
    "youtube.com",
    "www.youtube.com",
    "m.youtube.com",
    "music.youtube.com",
    "youtube-nocookie.com",
    "www.youtube-nocookie.com",
];

/// Формы пути, у которых id стоит сразу за префиксом: `/shorts/ID`,
/// `/embed/ID`, `/v/ID`, `/live/ID`.
///
/// Каждая измерена на каждом хосте [`VIDEO_HOSTS`] (матрица в doc
/// модуля) — тот же id, тот же extractor `youtube`. Префиксы строчные:
/// `/SHORTS/ID` и `/EMBED/ID` тем же замером уходят в `youtube:tab`, то
/// есть означают вкладку канала, а не ролик.
const ID_PATH_PREFIXES: [&str; 4] = ["/shorts/", "/embed/", "/v/", "/live/"];

/// Query-параметры, разрешённые формам, у которых id стоит в пути
/// ([`ID_PATH_PREFIXES`] и `youtu.be/ID`).
///
/// Здесь белый список нужнее всего, и вот почему. В позицию id YouTube
/// ставит **плейсхолдеры**, а настоящий адрес прячет в query, — и оба
/// известных плейсхолдера длиной ровно в одиннадцать знаков, то есть
/// проверка формы id их не отсеивает:
///
/// ```text
/// /embed/videoseries?list=PLbpi…      → youtube:tab PLbpi…   (плейлист)
/// /shorts/videoseries?list=PLbpi…     → youtube:tab PLbpi…   (плейлист)
/// /v/videoseries?list=…, /live/…      → youtube:tab          (плейлист)
/// /embed/live_stream?channel=UCBR8…   → youtube:tab UCBR8…/live (канал)
/// ```
///
/// Без этого списка два **разных** плейлиста давали бы один и тот же
/// «ролик» `videoseries`, и планировщик отклонил бы второй как дубль.
/// Тест `two_different_links_never_collapse_into_one_id` ловит ровно это
/// — он это и поймал.
///
/// Поэтому у форм с id в пути разрешён только тот query, про который
/// **измерено**, что он адрес не меняет: `?feature=share` (кнопка
/// «Поделиться» у shorts), `?si=…` (она же у youtu.be), `?t=30` (метка
/// времени), `?start=30` (она же у embed). Незнакомый параметр — отказ,
/// а не догадка; цена — непойманный дубль.
///
/// Формы `watch?v=ID` это не касается: там id приходит из самого
/// параметра `v`, а `list`, `index` и `app` рядом с ним измерены — тот
/// же ролик (`--no-playlist`, С-9 E2).
const PATH_FORM_QUERY_PARAMS: [&str; 4] = ["feature", "si", "start", "t"];

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
pub struct VideoId(String);

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
    if VIDEO_HOSTS
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

        return ID_PATH_PREFIXES
            .iter()
            .find_map(|prefix| path.strip_prefix(prefix))
            .and_then(|segment| path_video_id(segment, query));
    }

    if host.eq_ignore_ascii_case(SHORT_HOST) {
        return path_video_id(path.strip_prefix('/')?, query);
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
///
/// Query проверяется здесь же и по белому списку
/// ([`PATH_FORM_QUERY_PARAMS`]): у форм с id в пути незнакомый параметр
/// может означать, что сегмент — плейсхолдер, а настоящий адрес в
/// query.
fn path_video_id(segment: &str, query: &str) -> Option<VideoId> {
    let query_is_harmless = query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .all(|pair| {
            let name = pair.split_once('=').map_or(pair, |(name, _)| name);
            PATH_FORM_QUERY_PARAMS.contains(&name)
        });

    if !query_is_harmless {
        return None;
    }

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
    const SAME_LINK: &[&str] = &[
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
        // Формы, добавленные по замеру решением ведущего (doc модуля):
        // yt-dlp извлекает из них тот же самый id, то есть скачается
        // ровно тот же файл.
        "https://www.youtube.com/embed/aqz-KE-bpKQ",
        "https://www.youtube.com/embed/aqz-KE-bpKQ/",
        "https://www.youtube.com/embed/aqz-KE-bpKQ?start=30",
        "https://www.youtube.com/v/aqz-KE-bpKQ",
        "https://www.youtube.com/live/aqz-KE-bpKQ",
        "https://www.youtube.com/live/aqz-KE-bpKQ/",
        "https://music.youtube.com/watch?v=aqz-KE-bpKQ",
        "https://music.youtube.com/watch/?v=aqz-KE-bpKQ",
        "https://music.youtube.com/watch?v=aqz-KE-bpKQ&list=PL1",
        "https://www.youtube-nocookie.com/embed/aqz-KE-bpKQ",
        "https://youtube-nocookie.com/embed/aqz-KE-bpKQ",
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
    fn every_measured_host_and_path_form_gives_the_same_id() {
        // Замер 2026-08-31 (пин yt-dlp 2026.08.19, мёртвый прокси, без
        // сети): все тридцать ячеек матрицы «хост × форма пути» отдали
        // один и тот же id через extractor `youtube`. Тест перебирает те
        // же тридцать — таблица в doc модуля и код обязаны совпадать.
        let mut cells = 0_u32;

        for host in VIDEO_HOSTS {
            let forms = [
                format!("https://{host}/watch?v={ID}"),
                format!("https://{host}/shorts/{ID}"),
                format!("https://{host}/embed/{ID}"),
                format!("https://{host}/v/{ID}"),
                format!("https://{host}/live/{ID}"),
            ];

            for url in forms {
                cells += 1;
                assert_eq!(id_of(&url).as_deref(), Some(ID), "ячейка «{url}»");
            }
        }

        assert_eq!(
            cells,
            VIDEO_HOSTS.len() as u32 * (ID_PATH_PREFIXES.len() as u32 + 1),
            "перебрана обязана быть вся матрица, а не её часть"
        );
        assert_eq!(cells, 30, "тридцать ячеек — ровно столько и измерено");
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
            // Плейсхолдер в позиции id — тот класс ложных «да», который
            // проверкой формы id не ловится вовсе: `videoseries` и
            // `live_stream` длиной ровно одиннадцать знаков. Ловит его
            // белый список query ([`PATH_FORM_QUERY_PARAMS`]), и без
            // него оба адреса пары становятся одним «роликом».
            (
                "https://www.youtube.com/embed/videoseries?list=PLbpi6ZahtOH6",
                "https://www.youtube.com/embed/videoseries?list=PLE-hcHh2ZvPs",
            ),
            (
                "https://www.youtube.com/shorts/videoseries?list=PLbpi6ZahtOH6",
                "https://www.youtube.com/shorts/videoseries?list=PLE-hcHh2ZvPs",
            ),
            (
                "https://youtu.be/videoseries?list=PLbpi6ZahtOH6",
                "https://youtu.be/videoseries?list=PLE-hcHh2ZvPs",
            ),
            (
                "https://www.youtube.com/embed/live_stream?channel=UCBR8-60-B28hp2BmDPdntcQ",
                "https://www.youtube.com/embed/live_stream?channel=UCLA_DiR1FfKNvjuUpBHmylQ",
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
            "https://www.youtube.com/EMBED/aqz-KE-bpKQ",
            "https://music.youtube.com/watch?V=aqz-KE-bpKQ",
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
    fn a_placeholder_in_the_id_position_is_refused() {
        // Замер 2026-08-31 (пин 2026.08.19, мёртвый прокси): у всех форм
        // с id в пути YouTube ставит в его позицию плейсхолдер, а
        // настоящий адрес кладёт в query, — и yt-dlp уходит в
        // `youtube:tab`, то есть адресует плейлист или канал.
        for url in [
            "https://www.youtube.com/embed/videoseries?list=PLbpi6ZahtOH6",
            "https://www.youtube.com/shorts/videoseries?list=PLbpi6ZahtOH6",
            "https://www.youtube.com/v/videoseries?list=PLbpi6ZahtOH6",
            "https://www.youtube.com/live/videoseries?list=PLbpi6ZahtOH6",
            "https://www.youtube.com/embed/live_stream?channel=UCBR8-60-B28hp2BmDPdntcQ",
            // Единственная форма, где сам yt-dlp считает плейсхолдер
            // роликом (`[youtube] videoseries`), — и всё равно отказ:
            // белый список query не знает параметра `list`, а гадать,
            // что имел в виду инструмент, здесь не за что.
            "https://youtu.be/videoseries?list=PLbpi6ZahtOH6",
        ] {
            assert_eq!(id_of(url), None, "«{url}» адресует не ролик");
        }
    }

    #[test]
    fn only_measured_query_parameters_are_ignored_on_path_forms() {
        // Разрешены четыре измеренных параметра — и они действительно
        // ничего не меняют.
        for url in [
            "https://youtu.be/aqz-KE-bpKQ?si=Kx1yQ7wSomething",
            "https://youtu.be/aqz-KE-bpKQ?t=30",
            "https://youtu.be/aqz-KE-bpKQ?si=Kx1yQ7wSomething&t=30",
            "https://www.youtube.com/shorts/aqz-KE-bpKQ?feature=share",
            "https://www.youtube.com/embed/aqz-KE-bpKQ?start=30",
        ] {
            assert_eq!(id_of(url).as_deref(), Some(ID), "«{url}»");
        }

        // Незнакомый параметр — отказ, а не «наверное, это трекинг».
        // Цена известна и названа в doc: непойманный дубль.
        for url in [
            "https://youtu.be/aqz-KE-bpKQ?utm_source=telegram",
            "https://www.youtube.com/shorts/aqz-KE-bpKQ?list=PLbpi6ZahtOH6",
            "https://www.youtube.com/embed/aqz-KE-bpKQ?channel=UCBR8-60-B28hp2BmDPdntcQ",
            "https://youtu.be/aqz-KE-bpKQ?si=x&unknown=1",
        ] {
            assert_eq!(id_of(url), None, "«{url}» — незнакомый параметр");
        }
    }

    #[test]
    fn a_deep_path_after_the_id_is_refused() {
        // yt-dlp такой адрес разбирает, здесь он отвергается сознательно
        // (doc `path_video_id`): «что угодно после id» — чёрный список
        // наизнанку.
        for url in [
            "https://youtu.be/aqz-KE-bpKQ/extra",
            "https://www.youtube.com/shorts/aqz-KE-bpKQ/extra",
            "https://www.youtube.com/embed/aqz-KE-bpKQ/extra",
            "https://music.youtube.com/live/aqz-KE-bpKQ/extra",
        ] {
            assert_eq!(id_of(url), None, "«{url}»");
        }
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
