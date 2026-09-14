//! Транспорт контура обновления yt-dlp — единственная сеть, которую
//! инициирует само приложение, а не процесс yt-dlp (Н-1, решение
//! владельца Р-7).
//!
//! # Что этот модуль есть и чем он не является
//!
//! Он закрывает ровно два шва, оставленных задачами TL-55 и TL-56
//! незакрытыми намеренно: [`MetadataSource`] («сходи по адресу и принеси
//! тело») и замыкание `open` у [`super::fetch::network_source`]
//! («открой поток архива»). Ни разбора, ни потолков, ни классификации
//! отказов здесь нет — они уже написаны и уже покрыты тестами на
//! фикстурах; повторить их здесь значило бы завести второе место, где
//! они могут разойтись.
//!
//! Сети в тестах нет и здесь: этот модуль — тот самый кусок, который
//! проверяется только живым запуском, и потому в нём нет ни одной
//! ветки, кроме перевода отказов `ureq` в два класса, которые
//! [`MetadataError`] умеет различать.
//!
//! # Почему клиент синхронный
//!
//! Приём архива (TL-56) построен на обычном [`Read`]: потоковый счёт
//! байт под потолком и sha256 на лету, один проход. `ureq` отдаёт
//! `Body::into_reader()`, то есть уже принятый код не переписывается.
//! Асинхронный клиент потребовал бы либо моста к `Read`, либо
//! переписывания. Блокирующие вызовы уводит с рабочего потока
//! оркестрация ([`super::orchestrate`]), а не этот модуль.
//!
//! # Три требования к транспорту, снятые измерением (TL-55, 2026-08-29)
//!
//! 1. **`User-Agent` обязателен.** Без него API отвечает `403`
//!    («Request forbidden by administrative rules… make sure your
//!    request has a User-Agent header»). Клиент, который его не пошлёт,
//!    будет получать «источник недоступен» на каждой проверке — отказ,
//!    неотличимый от настоящего. Поэтому заголовок задаётся у агента, а
//!    не у запроса: забыть его на одном из двух вызовов нечем.
//! 2. **Редиректы проходятся.** Адрес файла сумм и адрес архива —
//!    `github.com`, тело отдаёт `objects.githubusercontent.com` через
//!    `302`. Умолчание `ureq` — до десяти редиректов, и оно оставлено
//!    как есть.
//! 3. **Лимит анонимного доступа — 60 запросов в час**
//!    (`x-ratelimit-limit: 60`). Токена у приложения нет и не будет.
//!    Держит этот бюджет расписание ([`super::orchestrate`]), а не
//!    транспорт: здесь нет памяти о прошлых запросах.
//!
//! # Прокси и корни доверия
//!
//! Прокси берётся из окружения (`Proxy::try_from_env` в умолчаниях
//! `ureq`) — системные настройки уважаются, своей инфраструктуры между
//! пользователем и апстримом нет (инвариант CLAUDE.md).
//!
//! Корни доверия — вшитый `webpki-roots`, а не системное хранилище;
//! цена названа в `Cargo.toml` над строкой зависимости: в сети с
//! корпоративным перехватом TLS обновление будет честно отказывать
//! классом «источник недоступен», а приложение — работать (Н-2).

use std::io::{self, Read};
use std::time::Duration;

use super::fetch::{network_source, HttpStatus, StreamArchive};
use super::release::{MetadataError, MetadataRequest, MetadataSource};
use super::update::UpdateAsset;

/// Чем приложение представляется апстриму.
///
/// Ровно имя и версия приложения: идентификаторов пользователя,
/// машины и установки в нём нет (Н-1). Версия берётся из манифеста
/// пакета, а не пишется руками, — иначе она разойдётся с релизом на
/// первом же выпуске.
const USER_AGENT: &str = concat!("tube-leak/", env!("CARGO_PKG_VERSION"));

/// Сколько ждать установления соединения. Общий для обоих видов
/// запросов: соединение либо устанавливается за секунды, либо не
/// устанавливается вовсе.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// Потолок на весь вызов метаданных. Тело здесь — десятки килобайт
/// (замер TL-55: 52 400 байт метаданных релиза, 1 595 байт файла сумм),
/// поэтому тридцать секунд — это «сеть есть, но ответа нет», а не
/// «медленно качается».
const METADATA_TIMEOUT: Duration = Duration::from_secs(30);

/// Потолок на приём тела архива.
///
/// Не сторож зависаний, а именно потолок: `ureq` не умеет мерить паузу
/// между кусками, а умеет — общее время приёма. Десять минут на
/// [`super::fetch::MAX_ARCHIVE_BYTES`] (192 МиБ) — это пол в 320 КиБ/с,
/// а на настоящем ассете (около 54 МиБ) — 90 КиБ/с; медленнее этого
/// связь уже не отличается от отсутствующей.
///
/// Верхняя граница выбрана ценой ошибки в обе стороны. Слишком короткий
/// потолок отменяет обновление на медленном канале — то есть ровно у
/// тех, кому оно тяжелее всего даётся. Слишком длинный оставляет строку
/// «Скачиваем обновление…» висеть после обрыва: работу приложения это
/// не трогает (Н-2), но блок молчит дольше, чем нужно. Десять минут —
/// та точка, где второе ещё терпимо, а первое уже невозможно.
const ARCHIVE_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Клиент контура: один агент на оба вида запросов.
///
/// Один, а не два, потому что общего у них главное — заголовок,
/// редиректы и прокси; различаются только сроки, и они задаются на
/// запросе.
pub struct GithubTransport {
    agent: ureq::Agent,
}

impl GithubTransport {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .user_agent(USER_AGENT)
            .timeout_connect(Some(CONNECT_TIMEOUT))
            // Схема проверяется дважды: белым списком адреса в
            // `super::release::is_github_url` (там же, где Н-1 и
            // записана) и здесь, у самого клиента. Второе — не
            // дублирование первого, а страховка от редиректа на `http://`:
            // белый список видит только тот адрес, который мы составили.
            .https_only(true)
            .build();

        Self {
            agent: ureq::Agent::new_with_config(config),
        }
    }

    /// Тот же клиент, но по петле и открытым текстом — только для тестов
    /// (TL-65): классы отказа проверяются настоящим `ureq` против сервера
    /// на 127.0.0.1, а не собранными руками ошибками. Отличий от боевого
    /// два, и оба про то, чего у петли нет: TLS (`https_only` снят) и
    /// прокси (снят, иначе `HTTP_PROXY` окружения увёл бы запрос мимо
    /// тестового сервера).
    #[cfg(test)]
    fn over_loopback() -> Self {
        let config = ureq::Agent::config_builder()
            .user_agent(USER_AGENT)
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .https_only(false)
            .proxy(None)
            .build();

        Self {
            agent: ureq::Agent::new_with_config(config),
        }
    }

    /// Источник архива поверх релизного ассета (TL-56).
    ///
    /// Запрос делается внутри замыкания, то есть при вызове
    /// [`super::fetch::ArchiveSource::open`], — и это несущее свойство,
    /// а не деталь: потолок по объявленному размеру обязан сработать
    /// **до** соединения, а объявленный размер приходит из метаданных
    /// (`UpdateAsset::size_bytes`), не из заголовков ответа.
    pub fn archive_source(
        &self,
        asset: &UpdateAsset,
    ) -> StreamArchive<impl Fn() -> io::Result<Box<dyn Read>> + use<'_>> {
        let agent = self.agent.clone();
        let url = asset.url.clone();

        network_source(asset, move || {
            let response = agent
                .get(&url)
                .config()
                .timeout_recv_body(Some(ARCHIVE_TIMEOUT))
                .build()
                .call()
                .map_err(ureq_to_io)?;

            Ok(Box::new(response.into_body().into_reader()) as Box<dyn Read>)
        })
    }
}

impl MetadataSource for GithubTransport {
    fn fetch(&self, request: &MetadataRequest) -> Result<Vec<u8>, MetadataError> {
        let mut response = self
            .agent
            .get(&request.url)
            .config()
            .timeout_global(Some(METADATA_TIMEOUT))
            .build()
            .call()
            .map_err(classify)?;

        // Потолок применяется прямо к чтению, а не после него: `max_bytes`
        // едет вместе с адресом именно затем, чтобы расход памяти задавал
        // не источник (doc `MetadataRequest`). Читается на байт больше
        // разрешённого — иначе «ровно потолок» и «потолок с хвостом»
        // выглядели бы одинаково, и разбирающая сторона не смогла бы
        // отличить одно от другого.
        let mut body = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(request.max_bytes.saturating_add(1))
            .read_to_end(&mut body)
            .map_err(|err| MetadataError::Offline {
                reason: format!("тело ответа не дочитано: {err}"),
            })?;

        Ok(body)
    }
}

/// Перевод отказа `ureq` в два класса, которые различает [`MetadataError`].
///
/// Классов ровно два, потому что различать больше транспорт не умеет:
/// «ответ пришёл, но не тот» — это [`ureq::Error::StatusCode`], всё
/// остальное — «соединения не было или оно оборвалось». Отсюда и
/// решение, которое стоит назвать явно: **отказ TLS попадает в
/// `Offline`**, а не в `Http`. Так велит doc самого варианта («DNS,
/// маршрут, TLS, таймаут, обрыв тела»), и так честнее — статуса у
/// такого отказа нет, а выдумывать `0` значило бы показать в логе
/// «ответ 0», то есть ответ, которого не было.
fn classify(error: ureq::Error) -> MetadataError {
    match error {
        ureq::Error::StatusCode(status) => MetadataError::Http {
            status,
            reason: format!("ответ {status} от источника обновлений"),
        },
        other => MetadataError::Offline {
            reason: other.to_string(),
        },
    }
}

/// То же для потока архива, но в форме [`io::Error`]: приём архива
/// (TL-56) говорит с источником через [`Read`] и классифицирует отказ
/// сам — по [`super::fetch::Origin`] и по тому, пришёл ли ответ.
///
/// Разделение то же, что у [`classify`]: статус уходит типом
/// [`HttpStatus`] и становится «источник недоступен», всё остальное —
/// «нет сети» (TL-65). До TL-65 статус уходил строкой, и приём архива
/// отличить его от обрыва не мог.
fn ureq_to_io(error: ureq::Error) -> io::Error {
    match error {
        // `into_io` разворачивает `Error::Io` и теряет остальные
        // варианты в безликом `Other` — статус среди них; его несёт тип.
        ureq::Error::StatusCode(status) => HttpStatus { status }.into_io(),
        other => other.into_io(),
    }
}

#[cfg(test)]
mod tests {
    //! Классы отказа приёма архива на настоящем `ureq` (TL-65).
    //!
    //! Сеть здесь — только петля 127.0.0.1 внутри процесса теста: сервер на
    //! одно соединение поднимается в потоке теста, наружу не уходит ни
    //! одного пакета (CLAUDE.md: «без сетевых вызовов в тестах»). Ни одно
    //! утверждение не зависит от времени.

    use super::*;
    use crate::types::YtDlpUpdateFailure;
    use crate::ytdlp::fetch::{fetch_and_install, FetchError};
    use crate::ytdlp::layout::{ArchiveIdentity, Layout};
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::thread;
    use tempfile::tempdir;

    const VERSION: &str = "2026.09.01";
    const SHA256: &str = "0000000000000000000000000000000000000000000000000000000000000000";

    /// Сервер на одно соединение: дочитывает запрос до пустой строки и
    /// отвечает `response` — или закрывает соединение молча, если `None`.
    fn serve_once(response: Option<&'static [u8]>) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("петля");
        let url = format!(
            "http://{}/yt-dlp_macos.zip",
            listener.local_addr().expect("адрес")
        );
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("соединение");
            let mut reader = BufReader::new(stream.try_clone().expect("второй конец"));
            let mut line = String::new();
            loop {
                line.clear();
                if reader.read_line(&mut line).expect("запрос читается") <= 2 {
                    break;
                }
            }
            if let Some(response) = response {
                stream.write_all(response).expect("ответ пишется");
            }
        });
        (url, server)
    }

    /// Приём ассета по `url` через транспорт до первого отказа.
    fn fetch_from(url: String) -> FetchError {
        let dir = tempdir().expect("tempdir");
        let layout = Layout::new(dir.path());
        layout.create_root().expect("корень");
        let asset = UpdateAsset {
            version: VERSION.to_string(),
            url,
            sha256: SHA256.to_string(),
            size_bytes: 1024,
        };
        let transport = GithubTransport::over_loopback();
        let source = transport.archive_source(&asset);

        fetch_and_install(
            &source,
            ArchiveIdentity::from(&asset),
            &layout,
            &mut |_, _, _| {},
        )
        .expect_err("архива по этому адресу нет")
    }

    #[test]
    fn a_404_on_the_release_asset_is_an_unavailable_source() {
        let (url, server) = serve_once(Some(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\nConnection: close\r\n\r\nNot Found",
        ));
        let error = fetch_from(url);
        server.join().expect("сервер");

        assert!(
            matches!(
                error.to_failure(VERSION),
                YtDlpUpdateFailure::SourceUnavailable { .. }
            ),
            "404 — сеть работает, апстрим отдал не то: {error}"
        );
        assert!(error.to_string().contains("404"), "{error}");
    }

    #[test]
    fn a_connection_that_breaks_or_never_happens_is_no_network() {
        // Соединение принято и закрыто без ответа.
        let (url, server) = serve_once(None);
        let error = fetch_from(url);
        server.join().expect("сервер");
        assert!(
            matches!(
                error.to_failure(VERSION),
                YtDlpUpdateFailure::NetworkUnavailable { .. }
            ),
            "обрыв без ответа: {error}"
        );

        // Порт освобождён до запроса: соединение отклонено.
        let port = TcpListener::bind("127.0.0.1:0")
            .expect("петля")
            .local_addr()
            .expect("адрес")
            .port();
        let error = fetch_from(format!("http://127.0.0.1:{port}/yt-dlp_macos.zip"));
        assert!(
            matches!(
                error.to_failure(VERSION),
                YtDlpUpdateFailure::NetworkUnavailable { .. }
            ),
            "отказ соединения: {error}"
        );
    }
}
