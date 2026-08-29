//! Проверка обновления yt-dlp по метаданным релиза (TL-55, Ф-1 и Ф-2).
//!
//! Шаг дешёвый и без диска: «есть обновление» — это два ответа GitHub, а
//! не байты архива. Скачивание, проверку суммы и установку делает
//! [`super::fetch`]; общего кода у них нет вовсе — только общий тип
//! [`UpdateAsset`], ради которого он и объявлен в [`super::update`].
//!
//! # Что именно спрашивается у апстрима
//!
//! 1. `GET https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest` —
//!    последний **стабильный** релиз: этот эндпоинт по определению не
//!    возвращает черновики и предрелизы. Из ответа берутся четыре вещи:
//!    тег, наш платформенный onedir-ассет (адрес и объявленный размер),
//!    адрес файла сумм и флаги `draft`/`prerelease`.
//! 2. `GET .../releases/download/<тег>/SHA2-256SUMS` — файл сумм **того
//!    же** релиза (Ф-3). Спрашивается только если тег оказался новее
//!    переданной версии: у актуальной установки второй запрос был бы
//!    тратой лимита ни на что (сторож
//!    `an_already_current_version_costs_one_request`).
//!
//! Флаги `draft`/`prerelease` проверяются повторно, хотя эндпоинт их и
//! так отсеивает. Это не перестраховка, а разделение ответственности:
//! «latest не отдаёт предрелизы» — обещание чужого сервиса, а не наша
//! проверка, и держаться на нём одном значило бы не иметь проверки вовсе.
//!
//! # Белый список вместо догадок
//!
//! Всё, что приезжает из метаданных, — непроверенный ввод, и разбирается
//! он перечислением разрешённого:
//!
//! - **тег** — [`ReleaseVersion::parse`]: три или четыре числа, форма
//!   выведена из всех 136 релизов, какие апстрим публиковал живьём (см.
//!   `tests/fixtures/ytdlp-update/README.md`). Непонятный тег — отказ
//!   класса «источник недоступен», а **не** «считаем, что новее» и не
//!   паника: молчаливый разбор непонятного — ровно тот дефект, который в
//!   этом проекте уже ловили;
//! - **адрес ассета** — сверяется на равенство с каноническим
//!   `https://github.com/yt-dlp/yt-dlp/releases/download/<тег>/<ассет>`.
//!   Адрес всё равно читается из ответа, а не строится молча: расхождение
//!   обязано быть видимым отказом, а не тихой заменой на своё
//!   представление о том, где лежат ассеты;
//! - **строка файла сумм** — ровно 64 шестнадцатеричных символа, два
//!   пробела, имя ассета. Строка, которая не разобралась, — отказ с
//!   номером строки, а не пропуск: пропуская непонятное, легко пропустить
//!   и ту единственную строку, ради которой файл и качали;
//! - **адрес любого запроса** — [`is_github_url`], последняя проверка
//!   перед транспортом (Н-1).
//!
//! # Какой ассет считается нашим
//!
//! Имя апстримного ассета приходит из пина `binaries.lock.json` через
//! `build.rs` ([`UPSTREAM_ASSET`]) — из той же записи, что версия и сумма
//! вложенного архива. Своего `cfg!(target_os)` здесь нет намеренно: он
//! был бы второй правдой о соответствии «тройка → ассет» и разошёлся бы с
//! пином молча (у macOS обе тройки берут один universal2-архив).
//!
//! # Чего здесь нет
//!
//! - **Транспорта.** Ровно как в [`super::fetch`]: [`MetadataSource`] —
//!   абстракция над «сходи по адресу и принеси тело», HTTP-клиент в граф
//!   зависимостей эта задача не вводит. Что транспорт обязан делать,
//!   когда появится, записано ниже.
//! - **Расписания и троттлинга.** Ф-2 требует «не чаще раза в часы»;
//!   когда звать проверку — забота оркестрации (TL-58). Здесь нет ни
//!   таймера, ни памяти о прошлой проверке.
//! - **Резолва активной версии.** Она приходит параметром — типом
//!   [`ReleaseVersion`], а не строкой. Причина не в удобстве: разбор
//!   *локальной* версии может не удаться, и этот отказ не является ни
//!   одним из пяти классов Ф-9 — он не про сеть и не про источник.
//!   Выдумывать ему класс здесь было бы враньём в интерфейсе, поэтому
//!   разбор отдан вызывающему вместе с [`ReleaseVersion::parse`], а
//!   невозможность позвать проверку с непроверенной строкой держит
//!   система типов — тот же приём, что [`super::layout::BuildId`].
//! - **Потолка на размер ассета.** Он есть, но живёт в [`super::fetch`]
//!   (`MAX_ARCHIVE_BYTES`) и стоит там дважды. Повторять его здесь
//!   значило бы завести второе число, которое однажды разойдётся с
//!   первым; этот шаг объявленный размер только переносит.
//!
//! # Что обязан делать транспорт (TL-58)
//!
//! Измерено `curl` при съёмке фикстур, 2026-08-29:
//!
//! - **Заголовок `User-Agent` обязателен.** Без него API отвечает `403`
//!   («Request forbidden by administrative rules… make sure your request
//!   has a User-Agent header»). Клиент, который его не пошлёт, получит не
//!   «обновлений нет», а отказ источника на каждой проверке.
//! - **Редиректы нужно проходить.** Адрес файла сумм и адрес архива —
//!   `github.com`, а тело отдаёт `objects.githubusercontent.com` через
//!   `302`. Хосты редиректа — тоже GitHub, Н-1 это не расширяет.
//! - **Лимит анонимного доступа — 60 запросов в час на адрес**
//!   (`x-ratelimit-limit: 60`). Токена у приложения нет и не будет
//!   (вшитый токен утекает из бинарника), так что расписание Ф-2 обязано
//!   укладываться в этот бюджет с запасом; исчерпание лимита приходит как
//!   `403` и попадает в класс «источник недоступен».
//! - **Системный прокси уважается, идентификаторов пользователя не
//!   передаётся** (Н-1).

use serde::Deserialize;

use super::update::{UpdateAsset, UpdateCheck};
use crate::types::YtDlpUpdateFailure;

/// Метаданные последнего стабильного релиза.
pub const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest";

/// Начало адреса любого релизного ассета yt-dlp.
const DOWNLOAD_URL_PREFIX: &str = "https://github.com/yt-dlp/yt-dlp/releases/download/";

/// Начало адреса любого запроса к API — только репозиторий yt-dlp.
const API_URL_PREFIX: &str = "https://api.github.com/repos/yt-dlp/yt-dlp/";

/// Имя апстримного onedir-ассета, подходящего этой сборке
/// (`yt-dlp_macos.zip`, `yt-dlp_linux.zip`, `yt-dlp_win.zip`).
///
/// Приходит из `binaries.lock.json` через `build.rs` — см. «Какой ассет
/// считается нашим» в doc модуля.
pub const UPSTREAM_ASSET: &str = env!("TUBE_LEAK_YTDLP_UPSTREAM_ASSET");

/// Имя ассета с контрольными суммами релиза.
const CHECKSUMS_ASSET: &str = "SHA2-256SUMS";

/// Потолок на тело ответа с метаданными релиза.
///
/// Замер живого ответа (фикстура `latest-release.json`, снята 2026-08-29):
/// **52 400 байт** на релиз с 24 ассетами и описанием в 11 071 символ.
/// Мегабайт — двадцатикратный запас над этим, то есть апстриму есть куда
/// расти, а «отдадут сколько отдадут» здесь нет: тело метаданных читается
/// в память целиком, и без границы её расход задавал бы источник.
const MAX_RELEASE_JSON_BYTES: u64 = 1024 * 1024;

/// Потолок на тело файла сумм. Замер того же релиза — **1 595 байт** на 19
/// строк; 64 КиB дают сорокакратный запас.
const MAX_CHECKSUMS_BYTES: u64 = 64 * 1024;

/// Сколько шестнадцатеричных символов в записи sha256. Ровно, не «хотя бы»
/// — тот же счёт, что у [`super::layout::BuildId`].
const SHA256_HEX_CHARS: usize = 64;

/// Версия релиза yt-dlp, разобранная по белому списку.
///
/// # Что считается версией
///
/// Форма выведена не из документа, а из всех 136 релизов, какие апстрим
/// опубликовал с 2021-01-07 (фикстура `upstream-releases.json`):
///
/// - `2026.08.19` — три числа через точку, 128 релизов из 136;
/// - `2022.06.22.1` — четвёртое число через точку, 6 релизов;
/// - `2021.01.07-1` — то же четвёртое число, но **через дефис**, 2 релиза.
///
/// Обе формы четвёртой части означают одно: второй выпуск того же дня.
/// Поэтому они и сравниваются одинаково — `2021.01.07-1` **равна**
/// `2021.01.07.1`, хотя написаны по-разному.
///
/// # Почему равенство считается по числам, а не по строке
///
/// Ф-1 говорит «та же версия с другой суммой обновлением не считается»,
/// то есть предмет сравнения — версия, а не её написание. Отсюда
/// [`PartialEq`] и [`Ord`] реализованы вручную по числовому ключу, а
/// исходный текст ([`Self::as_str`]) в сравнении не участвует: выведи их
/// `derive`, и две записи одного и того же выпуска разошлись бы, а более
/// «длинная» из них молча стала бы обновлением для более короткой.
///
/// # Чего разбор не проверяет
///
/// Календарь: `2026.02.30` пройдёт. Проверять его нечем и незачем —
/// упорядочивание тегов от этого не меняется, а придумывать апстриму
/// правила, которых он не обещал, — способ однажды отказать в настоящем
/// релизе.
#[derive(Debug, Clone)]
pub struct ReleaseVersion {
    year: u16,
    month: u8,
    day: u8,
    /// Номер выпуска внутри дня; у обычного релиза — 0.
    revision: u32,
    /// Тег ровно как его написал апстрим — для имени каталога установки и
    /// для показа пользователю.
    text: String,
}

/// Тег, который не разобрался.
///
/// Отдельный тип, а не `Option`: вызывающему нужно знать, что именно не
/// разобралось, — и когда это тег апстрима (класс «источник недоступен»),
/// и когда это локальная запись, которую разбирает оркестрация.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{text:?} — не версия релиза yt-dlp: {reason}")]
pub struct VersionError {
    pub text: String,
    pub reason: String,
}

impl ReleaseVersion {
    /// Разбирает тег релиза по белому списку (см. doc типа).
    pub fn parse(text: &str) -> Result<Self, VersionError> {
        let refuse = |reason: &str| VersionError {
            text: text.to_owned(),
            reason: reason.to_owned(),
        };

        // Ревизия через дефис — историческая форма (`2021.01.07-1`).
        // Дефис допускается ровно один и только после трёх чисел.
        let (head, dashed_revision) = match text.split_once('-') {
            Some((head, revision)) => {
                if revision.contains('-') {
                    return Err(refuse("дефис в теге допускается только один"));
                }
                (head, Some(revision))
            }
            None => (text, None),
        };

        let mut parts = head.split('.');
        let (Some(year), Some(month), Some(day)) = (parts.next(), parts.next(), parts.next())
        else {
            return Err(refuse(
                "ожидались год, месяц и день через точку (`2026.08.19`)",
            ));
        };
        let dotted_revision = parts.next();
        if parts.next().is_some() {
            return Err(refuse("чисел через точку больше четырёх"));
        }

        let revision = match (dotted_revision, dashed_revision) {
            (None, None) => None,
            (Some(revision), None) | (None, Some(revision)) => Some(revision),
            (Some(_), Some(_)) => {
                return Err(refuse(
                    "ревизия названа дважды — и через точку, и через дефис",
                ))
            }
        };

        let year = parse_number(year, 4, 4, &refuse, "год")?;
        let month = parse_number(month, 2, 2, &refuse, "месяц")?;
        let day = parse_number(day, 2, 2, &refuse, "день")?;
        let revision = match revision {
            Some(revision) => parse_number(revision, 1, 6, &refuse, "ревизия")?,
            None => 0,
        };

        if !(2000..=9999).contains(&year) {
            return Err(refuse("год до 2000 — это не тег релиза yt-dlp"));
        }
        if !(1..=12).contains(&month) {
            return Err(refuse("месяц вне 01–12"));
        }
        if !(1..=31).contains(&day) {
            return Err(refuse("день вне 01–31"));
        }

        Ok(Self {
            // Диапазоны проверены выше, сужение потерять ничего не может.
            year: year as u16,
            month: month as u8,
            day: day as u8,
            revision,
            text: text.to_owned(),
        })
    }

    /// Тег ровно как его написал апстрим.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Числовой ключ сравнения — единственное, чем версии отличаются друг
    /// от друга по смыслу.
    fn key(&self) -> (u16, u8, u8, u32) {
        (self.year, self.month, self.day, self.revision)
    }
}

/// Разбирает одно число тега: только ASCII-цифры и только заданной длины.
///
/// `u32` на все четыре поля, сужение — после проверки диапазонов: разбор
/// в `u8` отказал бы на `2026` раньше, чем стало бы понятно, что не так.
fn parse_number(
    text: &str,
    min_digits: usize,
    max_digits: usize,
    refuse: &dyn Fn(&str) -> VersionError,
    what: &str,
) -> Result<u32, VersionError> {
    if !(min_digits..=max_digits).contains(&text.len()) {
        return Err(refuse(&format!(
            "{what}: ожидалось от {min_digits} до {max_digits} цифр, а в {text:?} их {}",
            text.len()
        )));
    }
    // `str::parse` принял бы и `+7`, и цифры не из ASCII, поэтому белый
    // список стоит до него, а не вместо него.
    if !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(refuse(&format!("{what}: {text:?} — не ASCII-цифры")));
    }
    text.parse()
        .map_err(|_| refuse(&format!("{what}: {text:?} не помещается в число")))
}

impl PartialEq for ReleaseVersion {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}

impl Eq for ReleaseVersion {}

impl PartialOrd for ReleaseVersion {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ReleaseVersion {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key().cmp(&other.key())
    }
}

impl std::fmt::Display for ReleaseVersion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.text)
    }
}

/// Запрос к источнику метаданных: куда идти и сколько байт принимать.
///
/// Потолок едет вместе с адресом, а не живёт в транспорте, по той же
/// причине, по какой объявленный размер архива едет в [`UpdateAsset`]:
/// граница должна быть известна **до** запроса и не зависеть от того,
/// насколько аккуратен клиент. Здесь она проверяется ещё раз по факту —
/// заголовки могут её только ужесточить.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataRequest {
    pub url: String,
    pub max_bytes: u64,
}

/// Почему источник не ответил.
///
/// Два варианта, а не пять: транспорт различает ровно то, что умеет
/// различать, — «соединения не было» и «ответ пришёл, но не тот». Всё
/// остальное (форма ответа, лишние байты, чужой адрес) разбирает этот
/// модуль, и транспорту об этом знать нечего.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MetadataError {
    /// Соединение не состоялось или оборвалось: DNS, маршрут, TLS,
    /// таймаут, обрыв тела на середине.
    #[error("нет соединения: {reason}")]
    Offline { reason: String },
    /// Ответ получен, но это не метаданные: 403 (лимит запросов или
    /// отсутствующий `User-Agent`), 404, 5xx.
    #[error("ответ {status}: {reason}")]
    Http { status: u16, reason: String },
}

/// Откуда берутся метаданные. Транспорта здесь нет — см. doc модуля.
pub trait MetadataSource {
    /// Тело ответа целиком. Байты, а не строка: решение о кодировке
    /// принимает разбор, а не транспорт — файл сумм обязан быть ASCII, и
    /// «клиент как-нибудь декодировал» этой проверке помешало бы.
    fn fetch(&self, request: &MetadataRequest) -> Result<Vec<u8>, MetadataError>;
}

/// Почему проверка не состоялась.
///
/// Ровно два класса из пяти (Ф-9): до архива, места и smoke этот шаг не
/// доходит по построению — он ничего не скачивает.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CheckError {
    /// Нет соединения (С-3).
    #[error("проверка обновления не состоялась: {reason}")]
    Network { reason: String },
    /// Сеть есть, а метаданных нет: GitHub не отвечает, ограничение
    /// частоты запросов, ответ не той формы (С-3).
    #[error("источник обновлений ответил не тем: {reason}")]
    Source { reason: String },
}

impl CheckError {
    /// Проекция на контракт Ф-9.
    ///
    /// Версии здесь нет, и это не упущение: на этом шаге ещё не известно,
    /// о какой версии речь, и оба класса её не называют — строки 8 и 9
    /// таблицы состояний дизайна говорят про активную версию, а не про
    /// найденную.
    pub fn to_failure(&self) -> YtDlpUpdateFailure {
        let message = self.to_string();
        match self {
            Self::Network { .. } => YtDlpUpdateFailure::NetworkUnavailable { message },
            Self::Source { .. } => YtDlpUpdateFailure::SourceUnavailable { message },
        }
    }
}

/// Единственные адреса, к которым обращается контур обновления (Н-1).
///
/// Проверка стоит перед вызовом транспорта, а не только в тестах: то,
/// куда приложение ходит по сети, — свойство кода, и держаться оно должно
/// на коде. Ассеты сверяются ещё и на равенство каноническому адресу (см.
/// [`find_asset`]), так что этот список — последняя граница, а не
/// единственная.
fn is_github_url(url: &str) -> bool {
    url.starts_with(API_URL_PREFIX) || url.starts_with(DOWNLOAD_URL_PREFIX)
}

/// Проверяет, есть ли релиз новее `current`.
///
/// Сети здесь нет — есть [`MetadataSource`]; диска нет вовсе. Понижение
/// версии обновлением не считается (С-10: «понижения контур сам по себе
/// не выполняет никогда»), поэтому любой релиз не новее переданной версии
/// даёт [`UpdateCheck::UpToDate`].
pub fn check_for_update(
    source: &dyn MetadataSource,
    current: &ReleaseVersion,
) -> Result<UpdateCheck, CheckError> {
    let body = fetch(
        source,
        &MetadataRequest {
            url: LATEST_RELEASE_URL.to_owned(),
            max_bytes: MAX_RELEASE_JSON_BYTES,
        },
    )?;

    let release = parse_latest_release(&body, UPSTREAM_ASSET)?;

    if release.version <= *current {
        return Ok(UpdateCheck::UpToDate);
    }

    let sums = fetch(
        source,
        &MetadataRequest {
            url: release.checksums_url.clone(),
            max_bytes: MAX_CHECKSUMS_BYTES,
        },
    )?;

    let sha256 = checksum_for(&sums, UPSTREAM_ASSET, &release.checksums_url)?;

    Ok(UpdateCheck::Available(UpdateAsset {
        version: release.version.as_str().to_owned(),
        url: release.asset_url,
        sha256,
        size_bytes: release.asset_bytes,
    }))
}

/// Один запрос: белый список адреса, вызов транспорта, разбор его отказа
/// и потолок по факту.
fn fetch(source: &dyn MetadataSource, request: &MetadataRequest) -> Result<Vec<u8>, CheckError> {
    if !is_github_url(&request.url) {
        return Err(CheckError::Source {
            reason: format!(
                "{} — не адрес релизов yt-dlp/yt-dlp; контур обновления ходит только к ним (Н-1)",
                request.url
            ),
        });
    }

    let body = source.fetch(request).map_err(|error| match error {
        MetadataError::Offline { reason } => CheckError::Network {
            reason: format!("{}: {reason}", request.url),
        },
        MetadataError::Http { status, reason } => CheckError::Source {
            reason: format!("{}: ответ {status} ({reason})", request.url),
        },
    })?;

    if body.len() as u64 > request.max_bytes {
        return Err(CheckError::Source {
            reason: format!(
                "{} отдал {} байт при потолке {} — это не метаданные релиза",
                request.url,
                body.len(),
                request.max_bytes
            ),
        });
    }

    if body.is_empty() {
        return Err(CheckError::Source {
            reason: format!("{} отдал пустое тело", request.url),
        });
    }

    Ok(body)
}

/// То, что нужно контуру от метаданных релиза, и ничего сверх.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LatestRelease {
    version: ReleaseVersion,
    asset_url: String,
    asset_bytes: u64,
    checksums_url: String,
}

/// Ответ API в той части, которая читается. Незнакомые поля serde
/// пропускает: их у ответа двадцать одно, и требовать их все значило бы
/// ломаться от любого расширения API.
#[derive(Debug, Deserialize)]
struct ReleaseJson {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<AssetJson>,
}

#[derive(Debug, Deserialize)]
struct AssetJson {
    name: String,
    /// Объявленный размер. `u64`, поэтому отрицательное значение — отказ
    /// разбора, а не молча взятый модуль.
    size: u64,
    browser_download_url: String,
}

/// Разбирает ответ API: тег, наш ассет, файл сумм.
///
/// `asset_name` параметром, а не константой, ради тестов: так разбор
/// проверяется на всех трёх платформенных ассетах пина сразу, включая те
/// две платформы, машин под которые в проекте нет (Р-6).
fn parse_latest_release(body: &[u8], asset_name: &str) -> Result<LatestRelease, CheckError> {
    let refuse = |reason: String| CheckError::Source { reason };

    let release: ReleaseJson = serde_json::from_slice(body)
        .map_err(|error| refuse(format!("метаданные релиза не разбираются: {error}")))?;

    if release.draft || release.prerelease {
        return Err(refuse(format!(
            "релиз {} помечен как draft={} prerelease={} — контур ставит только стабильные",
            release.tag_name, release.draft, release.prerelease
        )));
    }

    let version = ReleaseVersion::parse(&release.tag_name)
        .map_err(|error| refuse(format!("тег релиза непонятен: {error}")))?;

    let asset = find_asset(&release, asset_name, &version)?;
    let checksums = find_asset(&release, CHECKSUMS_ASSET, &version)?;

    if asset.size == 0 {
        return Err(refuse(format!(
            "ассет {asset_name} релиза {version} объявляет нулевой размер"
        )));
    }

    Ok(LatestRelease {
        version,
        asset_url: asset.browser_download_url.clone(),
        asset_bytes: asset.size,
        checksums_url: checksums.browser_download_url.clone(),
    })
}

/// Находит ассет по имени и сверяет его адрес с каноническим.
///
/// Два ассета с одним именем — отказ, а не «возьмём первый»: GitHub такого
/// не выпускает, а значит ответ, в котором это есть, — не тот ответ, за
/// который он себя выдаёт.
fn find_asset<'a>(
    release: &'a ReleaseJson,
    name: &str,
    version: &ReleaseVersion,
) -> Result<&'a AssetJson, CheckError> {
    let mut found = release.assets.iter().filter(|asset| asset.name == name);

    let Some(asset) = found.next() else {
        return Err(CheckError::Source {
            reason: format!("в релизе {version} нет ассета {name}"),
        });
    };

    if found.next().is_some() {
        return Err(CheckError::Source {
            reason: format!("в релизе {version} ассет {name} назван больше одного раза"),
        });
    }

    let canonical = format!("{DOWNLOAD_URL_PREFIX}{version}/{name}");
    if asset.browser_download_url != canonical {
        return Err(CheckError::Source {
            reason: format!(
                "адрес ассета {name} релиза {version} — {}, а ожидался {canonical}",
                asset.browser_download_url
            ),
        });
    }

    Ok(asset)
}

/// Достаёт из файла сумм строку нашего ассета.
///
/// Формат апстрима — `<64 hex><два пробела><имя>`, перевод строки LF (19
/// строк в снятом файле). Разбор терпимее написания: разделителем
/// считается любой ASCII-пробельный промежуток, а `\r` в конце строки
/// отбрасывается — от переносимости формы файла контур зависеть не
/// должен. А вот строку, которая не разобралась, он **не** пропускает:
/// это отказ с номером строки. Пропуская непонятное, легко пропустить ту
/// единственную строку, ради которой файл и качали.
fn checksum_for(body: &[u8], asset_name: &str, url: &str) -> Result<String, CheckError> {
    let refuse = |reason: String| CheckError::Source { reason };

    let text = std::str::from_utf8(body)
        .map_err(|error| refuse(format!("{url}: файл сумм не текст ({error})")))?;

    let mut found: Option<&str> = None;

    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let number = index + 1;
        let mut fields = line.split_ascii_whitespace();
        let (Some(sum), Some(name)) = (fields.next(), fields.next()) else {
            return Err(refuse(format!(
                "{url}, строка {number}: ожидались сумма и имя ассета, а не {line:?}"
            )));
        };
        if fields.next().is_some() {
            return Err(refuse(format!(
                "{url}, строка {number}: лишние поля после имени ассета ({line:?})"
            )));
        }
        if sum.len() != SHA256_HEX_CHARS || !sum.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(refuse(format!(
                "{url}, строка {number}: {sum:?} — не {SHA256_HEX_CHARS} шестнадцатеричных символов"
            )));
        }

        if name != asset_name {
            continue;
        }
        if found.is_some_and(|previous| previous != sum) {
            return Err(refuse(format!(
                "{url}: у ассета {asset_name} названы две разные суммы"
            )));
        }
        found = Some(sum);
    }

    found
        .map(str::to_owned)
        .ok_or_else(|| refuse(format!("{url}: файл сумм не называет ассет {asset_name}")))
}

/// Проходит ли идентификатор кандидата белый список раскладки.
///
/// Утверждение «разобранная версия всегда годится в имя каталога» держит
/// не этот модуль, а [`super::layout::BuildId`], и проверяется оно тестом
/// на всех 136 живых тегах — приписывать себе чужую гарантию комментарием
/// в этом проекте уже стоило дефекта.
#[cfg(test)]
fn fits_layout(version: &ReleaseVersion, sha256: &str) -> bool {
    super::layout::BuildId::new(version.as_str(), sha256).is_ok()
}

#[cfg(test)]
#[allow(clippy::too_many_lines)]
mod tests {
    use super::*;

    use std::cell::RefCell;
    use std::fs;
    use std::path::{Path, PathBuf};

    use serde_json::{json, Value};

    /// Сумма из пина — она же ожидаемая сумма нашего ассета в снятом
    /// файле сумм.
    const PINNED_SHA256: &str = super::super::layout::BUNDLED_SHA256;
    const PINNED_VERSION: &str = super::super::layout::BUNDLED_VERSION;

    const CHECKSUMS_URL: &str =
        "https://github.com/yt-dlp/yt-dlp/releases/download/2026.08.19/SHA2-256SUMS";

    fn fixtures_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("ytdlp-update")
    }

    fn read_fixture(name: &str) -> Vec<u8> {
        let path = fixtures_dir().join(name);
        fs::read(&path)
            .unwrap_or_else(|err| panic!("фикстура {} не читается: {err}", path.display()))
    }

    fn release_json() -> Value {
        serde_json::from_slice(&read_fixture("latest-release.json"))
            .expect("снятый ответ API — это JSON")
    }

    fn version(text: &str) -> ReleaseVersion {
        ReleaseVersion::parse(text)
            .unwrap_or_else(|err| panic!("{text} обязана разбираться: {err}"))
    }

    /// Источник, отвечающий заранее заданным на каждый адрес и
    /// записывающий, о чём его спросили.
    struct FakeSource {
        answers: Vec<(String, Result<Vec<u8>, MetadataError>)>,
        asked: RefCell<Vec<MetadataRequest>>,
    }

    impl FakeSource {
        fn new() -> Self {
            Self {
                answers: Vec::new(),
                asked: RefCell::new(Vec::new()),
            }
        }

        fn answering(mut self, url: &str, body: impl Into<Vec<u8>>) -> Self {
            self.answers.push((url.to_owned(), Ok(body.into())));
            self
        }

        fn failing(mut self, url: &str, error: MetadataError) -> Self {
            self.answers.push((url.to_owned(), Err(error)));
            self
        }

        /// Штатный источник: снятые ответы на оба адреса.
        fn upstream() -> Self {
            Self::new()
                .answering(LATEST_RELEASE_URL, read_fixture("latest-release.json"))
                .answering(CHECKSUMS_URL, read_fixture("SHA2-256SUMS"))
        }

        /// Тот же источник, но метаданные подменены заданным JSON.
        fn with_release(release: &Value) -> Self {
            Self::new()
                .answering(
                    LATEST_RELEASE_URL,
                    serde_json::to_vec(release).expect("JSON сериализуется"),
                )
                .answering(CHECKSUMS_URL, read_fixture("SHA2-256SUMS"))
        }

        fn asked_urls(&self) -> Vec<String> {
            self.asked
                .borrow()
                .iter()
                .map(|request| request.url.clone())
                .collect()
        }
    }

    impl MetadataSource for FakeSource {
        fn fetch(&self, request: &MetadataRequest) -> Result<Vec<u8>, MetadataError> {
            self.asked.borrow_mut().push(request.clone());

            self.answers
                .iter()
                .find(|(url, _)| *url == request.url)
                .map(|(_, answer)| answer.clone())
                .unwrap_or_else(|| {
                    panic!(
                        "источник не готов отвечать на {} (готов на {:?})",
                        request.url,
                        self.answers.iter().map(|(url, _)| url).collect::<Vec<_>>()
                    )
                })
        }
    }

    /// Активная версия старее снятого релиза — тот самый С-1.
    fn older_than_capture() -> ReleaseVersion {
        version("2026.07.11")
    }

    // --- Белый список версии -------------------------------------------

    #[derive(Debug, serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct UpstreamReleases {
        releases: Vec<UpstreamRelease>,
    }

    #[derive(Debug, serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct UpstreamRelease {
        tag: String,
        published_at: String,
        prerelease: bool,
        draft: bool,
    }

    fn upstream_releases() -> Vec<UpstreamRelease> {
        let parsed: UpstreamReleases =
            serde_json::from_slice(&read_fixture("upstream-releases.json"))
                .expect("опись релизов — это JSON");
        assert_eq!(
            parsed.releases.len(),
            136,
            "фикстура описи релизов подменилась: её объём — часть утверждения о белом списке"
        );
        parsed.releases
    }

    /// Белый список выведен из живых данных, и это проверяется на всех до
    /// одного тегах, какие апстрим публиковал.
    #[test]
    fn every_tag_upstream_ever_published_parses() {
        for release in upstream_releases() {
            let parsed = ReleaseVersion::parse(&release.tag).unwrap_or_else(|err| {
                panic!(
                    "тег {} (опубликован {}) не разобрался: {err}",
                    release.tag, release.published_at
                )
            });
            assert_eq!(
                parsed.as_str(),
                release.tag,
                "текст тега обязан сохраняться"
            );
            assert!(
                fits_layout(&parsed, PINNED_SHA256),
                "версия {} не годится в имя каталога установки",
                release.tag
            );
        }
    }

    /// Три формы тега, встречавшиеся живьём, — именно три, и посчитаны они
    /// по фикстуре, а не по памяти.
    #[test]
    fn the_three_forms_of_a_tag_are_the_ones_measured() {
        let releases = upstream_releases();
        let plain = releases
            .iter()
            .filter(|release| release.tag.matches('.').count() == 2 && !release.tag.contains('-'))
            .count();
        let dotted = releases
            .iter()
            .filter(|release| release.tag.matches('.').count() == 3)
            .count();
        let dashed = releases
            .iter()
            .filter(|release| release.tag.contains('-'))
            .count();

        assert_eq!((plain, dotted, dashed), (128, 6, 2));
        assert_eq!(plain + dotted + dashed, releases.len());
    }

    /// Порядок по версии совпадает с порядком публикации — на всех
    /// стабильных релизах. Единственное исключение на 136 записей —
    /// предрелиз, и оно названо поимённо, а не обойдено молчанием.
    #[test]
    fn versions_order_the_way_upstream_published_them() {
        let releases = upstream_releases();

        let stable: Vec<&UpstreamRelease> = releases
            .iter()
            .filter(|release| !release.prerelease && !release.draft)
            .collect();
        assert_eq!(stable.len(), 134, "предрелизов в описи ровно два");

        for pair in stable.windows(2) {
            let (previous, next) = (pair[0], pair[1]);
            assert!(
                previous.published_at < next.published_at,
                "опись обязана быть отсортирована по дате публикации"
            );
            assert!(
                version(&previous.tag) < version(&next.tag),
                "{} опубликован раньше {}, а версия у него не меньше",
                previous.tag,
                next.tag
            );
        }

        // То самое исключение: предрелиз 2022.08.18.36 вышел ПОСЛЕ
        // стабильного 2022.08.19. `releases/latest` предрелизы не отдаёт,
        // поэтому контуру оно не встретится, — но проверка обязана знать,
        // что оно есть, а не считать порядок безусловным.
        let out_of_order = releases
            .iter()
            .find(|release| release.tag == "2022.08.18.36")
            .expect("предрелиз 2022.08.18.36 есть в описи");
        let after = releases
            .iter()
            .find(|release| release.tag == "2022.08.19")
            .expect("стабильный 2022.08.19 есть в описи");
        assert!(out_of_order.prerelease);
        assert!(out_of_order.published_at > after.published_at);
        assert!(version(&out_of_order.tag) < version(&after.tag));
    }

    /// Две формы записи одного и того же выпуска равны, а не «одна новее».
    #[test]
    fn the_two_forms_of_a_same_day_revision_are_the_same_version() {
        assert_eq!(version("2021.01.07-1"), version("2021.01.07.1"));
        assert!(version("2021.01.07-1") > version("2021.01.07"));
        assert!(version("2021.01.07") < version("2021.01.07.1"));
    }

    #[test]
    fn versions_compare_by_number_not_by_text() {
        assert!(version("2026.08.19") < version("2026.09.01"));
        assert!(version("2025.12.31") < version("2026.01.01"));
        assert!(version("2026.08.09") < version("2026.08.19"));
        assert_eq!(version("2026.08.19"), version("2026.08.19"));
        assert!(version("2022.08.18.36") > version("2022.08.18.2"));
    }

    /// Всё, что не разобралось однозначно, — отказ, а не догадка.
    #[test]
    fn tags_outside_the_white_list_are_refused() {
        let refused = [
            "",
            "latest",
            "v2026.08.19",
            "2026.8.19",
            "26.08.19",
            "2026.08.19rc1",
            "2026.08.19-rc1",
            "2026.13.01",
            "2026.00.19",
            "2026.08.32",
            "2026.08.00",
            "1999.12.31",
            "2026.08.19.",
            "2026.08.19-",
            "2026.08.19-1-2",
            "2026.08.19.1.2",
            "2026.08.19.1-2",
            "2026.08.19.1234567",
            " 2026.08.19",
            "2026.08.19 ",
            "2026.08.19\n",
            "+026.08.19",
            "٢٠٢٦.٠٨.١٩",
        ];

        for tag in refused {
            let outcome = ReleaseVersion::parse(tag);
            assert!(
                outcome.is_err(),
                "{tag:?} не является версией релиза, а разобрался как {outcome:?}"
            );
        }
    }

    // --- Исходы проверки -----------------------------------------------

    /// С-1: апстрим новее — на выходе описатель ассета целиком, поле в
    /// поле, и сумма в нём — та самая, что вложена в бандл.
    #[test]
    fn a_newer_release_turns_into_an_update_asset() {
        let source = FakeSource::upstream();

        let outcome = check_for_update(&source, &older_than_capture()).expect("проверка удалась");

        let UpdateCheck::Available(asset) = outcome else {
            panic!(
                "релиз {PINNED_VERSION} новее 2026.07.11 — ожидалось обновление, а не {outcome:?}"
            );
        };

        assert_eq!(asset.version, "2026.08.19");
        assert_eq!(
            asset.url,
            format!("{DOWNLOAD_URL_PREFIX}2026.08.19/{UPSTREAM_ASSET}")
        );
        assert_eq!(
            asset.sha256, PINNED_SHA256,
            "сумма нашего ассета в снятом файле сумм обязана совпадать с пином"
        );
        assert_eq!(asset.size_bytes, measured_asset_bytes(UPSTREAM_ASSET));
        assert_eq!(
            source.asked_urls(),
            vec![LATEST_RELEASE_URL.to_owned(), CHECKSUMS_URL.to_owned()]
        );
    }

    /// Размер ассета, измеренный **другим** набором фикстур (TL-18/TL-56,
    /// диапазонные запросы к тем же ассетам). Сверять размер с тем же
    /// файлом, из которого его и достали, значило бы проверять равенство
    /// строки самой себе.
    fn measured_asset_bytes(asset_name: &str) -> u64 {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("ytdlp-onedir")
            .join("upstream-trees.json");
        let raw = fs::read(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
        let measured: Value = serde_json::from_slice(&raw).expect("замер деревьев — это JSON");

        measured["trees"]
            .as_array()
            .expect("в замере есть массив trees")
            .iter()
            .find(|tree| {
                tree["asset"] == json!(asset_name) && tree["release"] == json!(PINNED_VERSION)
            })
            .and_then(|tree| tree["assetBytes"].as_u64())
            .unwrap_or_else(|| {
                panic!("в замере деревьев нет ассета {asset_name} релиза {PINNED_VERSION}")
            })
    }

    /// Снятый релиз — тот же, что вложен в бандл. Сторож не косметический:
    /// на этом равенстве держится проверка «сумма из файла сумм совпала с
    /// пином», то есть единственное место, где новый код сверяется с уже
    /// доказанным фактом (пин сверен побайтно в `build.rs`).
    #[test]
    fn the_captured_release_is_the_one_pinned_into_the_bundle() {
        let release = release_json();
        assert_eq!(
            release["tag_name"],
            json!(PINNED_VERSION),
            "пин уехал на другую версию — перекапти tests/fixtures/ytdlp-update/ \
             (как — в README рядом с фикстурой)"
        );
    }

    /// С-2: активная версия и есть последняя — второго запроса не будет.
    #[test]
    fn an_already_current_version_costs_one_request() {
        let source = FakeSource::upstream();

        let outcome =
            check_for_update(&source, &version(PINNED_VERSION)).expect("проверка удалась");

        assert_eq!(outcome, UpdateCheck::UpToDate);
        assert_eq!(
            source.asked_urls(),
            vec![LATEST_RELEASE_URL.to_owned()],
            "файл сумм актуальной версии качать незачем — Ф-2 про дешёвую проверку"
        );
    }

    /// Активная новее апстрима (откатились на свежую сборку, апстрим отозвал
    /// релиз) — это не обновление: понижений контур не делает (С-10).
    #[test]
    fn a_release_older_than_the_active_one_is_not_an_update() {
        let source = FakeSource::upstream();

        let outcome = check_for_update(&source, &version("2026.09.01")).expect("проверка удалась");

        assert_eq!(outcome, UpdateCheck::UpToDate);
        assert_eq!(source.asked_urls(), vec![LATEST_RELEASE_URL.to_owned()]);
    }

    /// Несравнимый тег — типизированный отказ, а не паника и не «новее».
    #[test]
    fn an_unparsable_tag_is_a_typed_refusal() {
        for tag in ["nightly", "v2026.08.19", "2026.08.19-rc1", ""] {
            let mut release = release_json();
            release["tag_name"] = json!(tag);
            let source = FakeSource::with_release(&release);

            let error = check_for_update(&source, &older_than_capture())
                .expect_err("непонятный тег не может дать исход проверки");

            assert!(
                matches!(error, CheckError::Source { .. }),
                "тег {tag:?} — отказ источника, а не {error:?}"
            );
            assert!(
                error.to_string().contains("тег релиза непонятен"),
                "в диагностике должно быть видно, что дело в теге: {error}"
            );
            assert_eq!(
                source.asked_urls(),
                vec![LATEST_RELEASE_URL.to_owned()],
                "после непонятного тега файл сумм не качается"
            );
            assert!(matches!(
                error.to_failure(),
                YtDlpUpdateFailure::SourceUnavailable { .. }
            ));
        }
    }

    /// Нет сети — класс «нет соединения», и он не превращается в
    /// «источник недоступен» по дороге (строки 8 и 9 таблицы состояний
    /// говорят пользователю разное).
    #[test]
    fn no_connection_is_reported_as_network_unavailable() {
        let source = FakeSource::new().failing(
            LATEST_RELEASE_URL,
            MetadataError::Offline {
                reason: "dns error: failed to lookup address information".to_owned(),
            },
        );

        let error = check_for_update(&source, &older_than_capture()).expect_err("сети нет");

        assert!(matches!(error, CheckError::Network { .. }), "{error:?}");
        assert!(matches!(
            error.to_failure(),
            YtDlpUpdateFailure::NetworkUnavailable { .. }
        ));
    }

    /// Ответ пришёл, но это не метаданные: лимит запросов, 5xx, 404.
    #[test]
    fn an_http_error_is_reported_as_source_unavailable() {
        for (status, reason) in [
            (403u16, "API rate limit exceeded"),
            (429, "too many requests"),
            (500, "internal server error"),
            (502, "bad gateway"),
            (404, "not found"),
        ] {
            let source = FakeSource::new().failing(
                LATEST_RELEASE_URL,
                MetadataError::Http {
                    status,
                    reason: reason.to_owned(),
                },
            );

            let error = check_for_update(&source, &older_than_capture())
                .expect_err("метаданных нет — исхода проверки быть не может");

            assert!(
                matches!(error, CheckError::Source { .. }),
                "ответ {status} — отказ источника, а не {error:?}"
            );
            assert!(
                error.to_string().contains(&status.to_string()),
                "код ответа обязан попасть в диагностику: {error}"
            );
            assert!(matches!(
                error.to_failure(),
                YtDlpUpdateFailure::SourceUnavailable { .. }
            ));
        }
    }

    /// Отказ на **втором** запросе — тоже отказ проверки, а не обновление
    /// без суммы.
    #[test]
    fn a_checksums_file_that_did_not_arrive_fails_the_whole_check() {
        let source = FakeSource::new()
            .answering(LATEST_RELEASE_URL, read_fixture("latest-release.json"))
            .failing(
                CHECKSUMS_URL,
                MetadataError::Offline {
                    reason: "connection reset by peer".to_owned(),
                },
            );

        let error =
            check_for_update(&source, &older_than_capture()).expect_err("файла сумм не будет");

        assert!(matches!(error, CheckError::Network { .. }), "{error:?}");
    }

    /// Черновик и предрелиз отсеиваются нами, а не только эндпоинтом.
    #[test]
    fn a_release_that_is_not_stable_is_refused() {
        for flag in ["draft", "prerelease"] {
            let mut release = release_json();
            release[flag] = json!(true);
            let source = FakeSource::with_release(&release);

            let error = check_for_update(&source, &older_than_capture())
                .expect_err("нестабильный релиз не ставится");

            assert!(matches!(error, CheckError::Source { .. }), "{error:?}");
            assert!(error.to_string().contains(flag), "{error}");
        }
    }

    /// Ассета нашей платформы в релизе нет — отказ, а не первый попавшийся.
    #[test]
    fn a_release_without_our_platform_asset_is_refused() {
        let mut release = release_json();
        strip_asset(&mut release, UPSTREAM_ASSET);
        let source = FakeSource::with_release(&release);

        let error = check_for_update(&source, &older_than_capture()).expect_err("ставить нечего");

        assert!(matches!(error, CheckError::Source { .. }), "{error:?}");
        assert!(error.to_string().contains(UPSTREAM_ASSET), "{error}");
    }

    /// Файла сумм в релизе нет — отказ до того, как что-то скачано.
    #[test]
    fn a_release_without_a_checksums_asset_is_refused() {
        let mut release = release_json();
        strip_asset(&mut release, CHECKSUMS_ASSET);
        let source = FakeSource::with_release(&release);

        let error = check_for_update(&source, &older_than_capture()).expect_err("сверять нечем");

        assert!(matches!(error, CheckError::Source { .. }), "{error:?}");
        assert!(error.to_string().contains(CHECKSUMS_ASSET), "{error}");
    }

    fn strip_asset(release: &mut Value, name: &str) {
        let assets = release["assets"].as_array_mut().expect("массив ассетов");
        assets.retain(|asset| asset["name"] != json!(name));
    }

    fn set_asset_field(release: &mut Value, name: &str, field: &str, value: Value) {
        let assets = release["assets"].as_array_mut().expect("массив ассетов");
        let asset = assets
            .iter_mut()
            .find(|asset| asset["name"] == json!(name))
            .unwrap_or_else(|| panic!("в снятом релизе есть ассет {name}"));
        asset[field] = value;
    }

    /// Н-1: адрес, приехавший из метаданных, не берётся на веру. Ни чужой
    /// хост, ни похожий, ни путь наружу.
    #[test]
    fn an_asset_url_that_is_not_the_canonical_one_is_refused() {
        let elsewhere = [
            "https://evil.test/yt-dlp/yt-dlp/releases/download/2026.08.19/yt-dlp_macos.zip",
            "https://github.com.evil.test/yt-dlp/yt-dlp/releases/download/2026.08.19/yt-dlp_macos.zip",
            "http://github.com/yt-dlp/yt-dlp/releases/download/2026.08.19/yt-dlp_macos.zip",
            "https://github.com/yt-dlp/yt-dlp/releases/download/2026.08.19/../../../evil.zip",
            "https://github.com/evil/yt-dlp/releases/download/2026.08.19/yt-dlp_macos.zip",
            "https://github.com/yt-dlp/yt-dlp/releases/download/2020.01.01/yt-dlp_macos.zip",
        ];

        for url in elsewhere {
            for asset in [UPSTREAM_ASSET, CHECKSUMS_ASSET] {
                let mut release = release_json();
                set_asset_field(&mut release, asset, "browser_download_url", json!(url));
                let source = FakeSource::with_release(&release);

                let error = check_for_update(&source, &older_than_capture())
                    .expect_err("адрес ассета не канонический");

                assert!(
                    matches!(error, CheckError::Source { .. }),
                    "{url}: {error:?}"
                );
                assert!(
                    error.to_string().contains("ожидался"),
                    "{url}: в диагностике должен быть канонический адрес — {error}"
                );
            }
        }
    }

    /// Ассет с нулевым размером скачивать нечем: `UpdateAsset::size_bytes`
    /// — знаменатель полосы и первый потолок TL-56.
    #[test]
    fn an_asset_without_a_declared_size_is_refused() {
        let mut release = release_json();
        set_asset_field(&mut release, UPSTREAM_ASSET, "size", json!(0));
        let source = FakeSource::with_release(&release);

        let error = check_for_update(&source, &older_than_capture()).expect_err("размера нет");

        assert!(matches!(error, CheckError::Source { .. }), "{error:?}");
        assert!(error.to_string().contains("нулевой размер"), "{error}");
    }

    /// Отрицательный размер — это не «модуль от числа», а отказ разбора.
    #[test]
    fn a_negative_size_does_not_parse() {
        let mut release = release_json();
        set_asset_field(&mut release, UPSTREAM_ASSET, "size", json!(-1));
        let source = FakeSource::with_release(&release);

        let error = check_for_update(&source, &older_than_capture()).expect_err("размер не число");

        assert!(matches!(error, CheckError::Source { .. }), "{error:?}");
    }

    /// Два ассета с одним именем — противоречивый ответ, а не «возьмём
    /// первый».
    #[test]
    fn a_duplicated_asset_name_is_refused() {
        let mut release = release_json();
        let twin = release["assets"]
            .as_array()
            .expect("массив ассетов")
            .iter()
            .find(|asset| asset["name"] == json!(UPSTREAM_ASSET))
            .expect("наш ассет в снятом релизе есть")
            .clone();
        release["assets"]
            .as_array_mut()
            .expect("массив ассетов")
            .push(twin);
        let source = FakeSource::with_release(&release);

        let error =
            check_for_update(&source, &older_than_capture()).expect_err("ответ противоречив");

        assert!(matches!(error, CheckError::Source { .. }), "{error:?}");
        assert!(error.to_string().contains("больше одного раза"), "{error}");
    }

    /// Тело не той формы вовсе (HTML капчи, обрезанный JSON, пустота).
    #[test]
    fn a_body_that_is_not_release_metadata_is_refused() {
        for body in [
            &b"<!DOCTYPE html><html><body>rate limited</body></html>"[..],
            b"{\"tag_name\": \"2026.08.19\"",
            b"[]",
            b"null",
        ] {
            let source = FakeSource::new().answering(LATEST_RELEASE_URL, body);

            let error = check_for_update(&source, &older_than_capture())
                .expect_err("это не метаданные релиза");

            assert!(matches!(error, CheckError::Source { .. }), "{error:?}");
        }
    }

    #[test]
    fn an_empty_body_is_refused() {
        let source = FakeSource::new().answering(LATEST_RELEASE_URL, Vec::new());

        let error = check_for_update(&source, &older_than_capture()).expect_err("пустое тело");

        assert!(matches!(error, CheckError::Source { .. }), "{error:?}");
        assert!(error.to_string().contains("пустое тело"), "{error}");
    }

    /// Потолок на тело стоит **до** разбора: источник не назначает расход
    /// памяти проверки, даже если отдаёт синтаксически верный JSON.
    #[test]
    fn an_oversized_body_is_refused_before_it_is_parsed() {
        let mut release = release_json();
        release["body"] = json!("x".repeat(2 * 1024 * 1024));
        let source = FakeSource::with_release(&release);

        let error = check_for_update(&source, &older_than_capture()).expect_err("тело за потолком");

        assert!(matches!(error, CheckError::Source { .. }), "{error:?}");
        assert!(error.to_string().contains("при потолке"), "{error}");
    }

    /// Тот же потолок — у файла сумм, и он свой, гораздо теснее.
    #[test]
    fn an_oversized_checksums_file_is_refused() {
        let source = FakeSource::new()
            .answering(LATEST_RELEASE_URL, read_fixture("latest-release.json"))
            .answering(
                CHECKSUMS_URL,
                vec![b'a'; (MAX_CHECKSUMS_BYTES + 1) as usize],
            );

        let error = check_for_update(&source, &older_than_capture()).expect_err("файл сумм раздут");

        assert!(matches!(error, CheckError::Source { .. }), "{error:?}");
        assert!(error.to_string().contains("при потолке"), "{error}");
    }

    // --- Файл сумм ------------------------------------------------------

    /// Строка ищется по имени ассета, а не по позиции: в снятом файле
    /// наша строка не первая и не последняя.
    #[test]
    fn the_checksum_is_taken_by_asset_name_not_by_position() {
        let body = read_fixture("SHA2-256SUMS");
        let text = std::str::from_utf8(&body).expect("файл сумм — текст");
        let ours = text
            .lines()
            .position(|line| line.ends_with(UPSTREAM_ASSET))
            .expect("наша строка в файле сумм есть");
        assert!(
            ours > 0 && ours + 1 < text.lines().count(),
            "наша строка обязана быть внутри файла, иначе тест ничего не доказывает"
        );

        let sum = checksum_for(&body, UPSTREAM_ASSET, CHECKSUMS_URL).expect("сумма нашлась");
        assert_eq!(sum, PINNED_SHA256);
    }

    /// Разбор одинаково работает для всех трёх платформенных ассетов пина
    /// — включая те две платформы, машин под которые в проекте нет (Р-6).
    #[test]
    fn every_platform_asset_of_the_pin_resolves_to_its_pinned_sum() {
        let pin_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("binaries.lock.json");
        let raw = fs::read(&pin_path).unwrap_or_else(|err| panic!("{}: {err}", pin_path.display()));
        let pin: Value = serde_json::from_slice(&raw).expect("пин — это JSON");

        let targets = pin["ytDlp"]["targets"]
            .as_object()
            .expect("в пине есть таргеты yt-dlp");
        assert_eq!(targets.len(), 4, "в пине четыре тройки");

        let release = read_fixture("latest-release.json");
        let sums = read_fixture("SHA2-256SUMS");

        for (target, entry) in targets {
            let url = entry["url"].as_str().expect("у записи пина есть url");
            let asset = url
                .rsplit('/')
                .next()
                .expect("у url есть последний сегмент");
            let expected = entry["sha256"].as_str().expect("у записи пина есть sha256");

            let parsed = parse_latest_release(&release, asset)
                .unwrap_or_else(|err| panic!("{target}: метаданные не разобрались: {err}"));
            assert_eq!(parsed.asset_url, url, "{target}: адрес ассета");
            assert_eq!(parsed.version.as_str(), PINNED_VERSION, "{target}: версия");

            let sum = checksum_for(&sums, asset, CHECKSUMS_URL)
                .unwrap_or_else(|err| panic!("{target}: сумма не нашлась: {err}"));
            assert_eq!(
                sum, expected,
                "{target}: сумма из файла сумм разошлась с пином"
            );
        }
    }

    /// Непонятная строка — отказ с номером строки, а не тихий пропуск.
    #[test]
    fn a_malformed_checksums_line_is_a_refusal_not_a_silent_skip() {
        let good = format!("{PINNED_SHA256}  {UPSTREAM_ASSET}\n");
        let cases = [
            ("hello world\n", "строка 1"),
            ("deadbeef  yt-dlp_macos.zip\n", "строка 1"),
            (
                "0000000000000000000000000000000000000000000000000000000000000zzz  yt-dlp\n",
                "строка 1",
            ),
            ("одинокое-слово\n", "строка 1"),
            (
                "1fa6733c37ea6fb51c99ad8fe785e7b7e5f3246c9b980230329d4fb72ed8d4d6  yt-dlp  лишнее\n",
                "строка 1",
            ),
        ];

        for (bad, where_) in cases {
            let body = format!("{bad}{good}");
            let error = checksum_for(body.as_bytes(), UPSTREAM_ASSET, CHECKSUMS_URL)
                .expect_err("непонятная строка обязана останавливать разбор");

            assert!(
                matches!(error, CheckError::Source { .. }),
                "{bad:?}: {error:?}"
            );
            assert!(
                error.to_string().contains(where_),
                "{bad:?}: в диагностике обязан быть номер строки — {error}"
            );
        }
    }

    /// Пустые строки — не «непонятное»: их пропуск ничего не скрывает.
    #[test]
    fn blank_lines_do_not_break_the_checksums_file() {
        let body = format!("\n\n{PINNED_SHA256}  {UPSTREAM_ASSET}\n\n");

        let sum =
            checksum_for(body.as_bytes(), UPSTREAM_ASSET, CHECKSUMS_URL).expect("сумма нашлась");

        assert_eq!(sum, PINNED_SHA256);
    }

    /// CRLF и хвостовые пробелы не должны делать файл нечитаемым: форма
    /// файла — не то, ради чего стоит отказывать в обновлении.
    #[test]
    fn crlf_line_endings_are_read_the_same_way() {
        let body = format!("{PINNED_SHA256}  {UPSTREAM_ASSET}\r\n");

        let sum =
            checksum_for(body.as_bytes(), UPSTREAM_ASSET, CHECKSUMS_URL).expect("сумма нашлась");

        assert_eq!(sum, PINNED_SHA256);
    }

    /// Файл сумм без нашего ассета — отказ: качать архив, сверять который
    /// нечем, контур не станет (Ф-3).
    #[test]
    fn a_checksums_file_without_our_asset_is_refused() {
        let body = format!("{PINNED_SHA256}  yt-dlp.tar.gz\n");

        let error = checksum_for(body.as_bytes(), UPSTREAM_ASSET, CHECKSUMS_URL)
            .expect_err("нашего ассета в файле нет");

        assert!(matches!(error, CheckError::Source { .. }), "{error:?}");
        assert!(error.to_string().contains(UPSTREAM_ASSET), "{error}");
    }

    /// Две разные суммы у одного ассета — противоречие, а не «первая
    /// побеждает».
    #[test]
    fn a_contradictory_checksums_file_is_refused() {
        let other = "0".repeat(SHA256_HEX_CHARS);
        let body = format!("{PINNED_SHA256}  {UPSTREAM_ASSET}\n{other}  {UPSTREAM_ASSET}\n");

        let error = checksum_for(body.as_bytes(), UPSTREAM_ASSET, CHECKSUMS_URL)
            .expect_err("файл сумм противоречив");

        assert!(matches!(error, CheckError::Source { .. }), "{error:?}");
        assert!(error.to_string().contains("две разные суммы"), "{error}");
    }

    /// Одна и та же сумма, названная дважды, противоречием не является.
    #[test]
    fn a_repeated_identical_line_is_not_a_contradiction() {
        let body =
            format!("{PINNED_SHA256}  {UPSTREAM_ASSET}\n{PINNED_SHA256}  {UPSTREAM_ASSET}\n");

        let sum =
            checksum_for(body.as_bytes(), UPSTREAM_ASSET, CHECKSUMS_URL).expect("сумма нашлась");

        assert_eq!(sum, PINNED_SHA256);
    }

    #[test]
    fn a_checksums_file_that_is_not_text_is_refused() {
        let error = checksum_for(&[0xff, 0xfe, 0x00], UPSTREAM_ASSET, CHECKSUMS_URL)
            .expect_err("это не текст");

        assert!(matches!(error, CheckError::Source { .. }), "{error:?}");
        assert!(error.to_string().contains("не текст"), "{error}");
    }

    // --- Н-1: куда контур ходит ----------------------------------------

    /// Ни один запрос проверки не уходит мимо GitHub — проверено на всех
    /// сценариях набора разом, а не на счастливом пути.
    #[test]
    fn every_request_the_check_makes_goes_to_github() {
        let mut releases = vec![release_json()];
        let mut broken = release_json();
        broken["tag_name"] = json!("nightly");
        releases.push(broken);
        let mut newer = release_json();
        newer["tag_name"] = json!("2027.01.01");
        set_asset_field(
            &mut newer,
            UPSTREAM_ASSET,
            "browser_download_url",
            json!(format!("{DOWNLOAD_URL_PREFIX}2027.01.01/{UPSTREAM_ASSET}")),
        );
        set_asset_field(
            &mut newer,
            CHECKSUMS_ASSET,
            "browser_download_url",
            json!(format!("{DOWNLOAD_URL_PREFIX}2027.01.01/{CHECKSUMS_ASSET}")),
        );
        releases.push(newer);

        let mut asked = Vec::new();
        for release in &releases {
            for current in ["2020.01.01", PINNED_VERSION, "2030.01.01"] {
                let source = FakeSource::new()
                    .answering(
                        LATEST_RELEASE_URL,
                        serde_json::to_vec(release).expect("JSON сериализуется"),
                    )
                    .answering(CHECKSUMS_URL, read_fixture("SHA2-256SUMS"))
                    .answering(
                        &format!("{DOWNLOAD_URL_PREFIX}2027.01.01/{CHECKSUMS_ASSET}"),
                        read_fixture("SHA2-256SUMS"),
                    );
                let _ = check_for_update(&source, &version(current));
                asked.extend(source.asked_urls());
            }
        }

        assert!(
            asked.len() >= releases.len(),
            "сценарии обязаны были хоть куда-то сходить, а запросов {}",
            asked.len()
        );
        for url in asked {
            assert!(
                url.starts_with(API_URL_PREFIX) || url.starts_with(DOWNLOAD_URL_PREFIX),
                "запрос ушёл мимо GitHub: {url}"
            );
        }
    }

    /// Последняя граница перед транспортом: адрес не из белого списка не
    /// доходит до источника вовсе — соединения не возникает.
    #[test]
    fn a_url_outside_the_white_list_never_reaches_the_transport() {
        let source = FakeSource::new();

        for url in [
            "https://evil.test/releases/latest",
            "http://api.github.com/repos/yt-dlp/yt-dlp/releases/latest",
            "https://api.github.com/repos/evil/yt-dlp/releases/latest",
            "https://api.github.com.evil.test/repos/yt-dlp/yt-dlp/releases/latest",
            "file:///etc/passwd",
        ] {
            let error = fetch(
                &source,
                &MetadataRequest {
                    url: url.to_owned(),
                    max_bytes: MAX_RELEASE_JSON_BYTES,
                },
            )
            .expect_err("адрес вне белого списка");

            assert!(
                matches!(error, CheckError::Source { .. }),
                "{url}: {error:?}"
            );
        }

        assert!(
            source.asked_urls().is_empty(),
            "источник не должен был получить ни одного запроса, а получил {:?}",
            source.asked_urls()
        );
    }

    /// Оба адреса, которыми пользуется проверка, — GitHub, и это
    /// утверждение проверяется на самих константах, а не на их описании.
    #[test]
    fn the_constants_point_at_the_official_upstream() {
        assert!(is_github_url(LATEST_RELEASE_URL));
        assert!(LATEST_RELEASE_URL.ends_with("/releases/latest"));
        assert!(is_github_url(&format!(
            "{DOWNLOAD_URL_PREFIX}{PINNED_VERSION}/{CHECKSUMS_ASSET}"
        )));
        assert!(UPSTREAM_ASSET.ends_with(".zip"), "{UPSTREAM_ASSET}");
        assert!(
            !UPSTREAM_ASSET.contains('/'),
            "имя ассета — не путь: {UPSTREAM_ASSET}"
        );
    }

    // --- Фикстуры -------------------------------------------------------

    /// Сырые ответы — те же, что снимали: размер каждого записан в описи.
    #[test]
    fn raw_captures_match_their_manifest() {
        let manifest: Value = serde_json::from_slice(&read_fixture("captures.json"))
            .expect("опись съёмок — это JSON");
        let files = manifest["files"].as_array().expect("в описи есть files");
        assert_eq!(files.len(), 2, "описаны оба сырых ответа");

        let mut described: Vec<String> = Vec::new();
        for file in files {
            let name = file["name"].as_str().expect("у записи описи есть имя");
            let bytes = file["bytes"].as_u64().expect("у записи описи есть размер");
            let url = file["url"].as_str().expect("у записи описи есть адрес");

            assert_eq!(
                read_fixture(name).len() as u64,
                bytes,
                "{name}: размер разошёлся с описью — сырой ответ правили руками?"
            );
            assert!(
                url.starts_with(API_URL_PREFIX) || url.starts_with(DOWNLOAD_URL_PREFIX),
                "{name}: снято не с GitHub ({url})"
            );
            described.push(name.to_owned());
        }
        described.sort();

        let mut on_disk: Vec<String> = fs::read_dir(fixtures_dir())
            .expect("каталог фикстур читается")
            .map(|entry| {
                entry
                    .expect("запись каталога")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .filter(|name| {
                !matches!(
                    name.as_str(),
                    "README.md" | "captures.json" | "upstream-releases.json"
                )
            })
            .collect();
        on_disk.sort();

        assert_eq!(
            described, on_disk,
            "опись и каталог разошлись: фикстура без описи выглядит снятой, не будучи ею"
        );
    }
}
