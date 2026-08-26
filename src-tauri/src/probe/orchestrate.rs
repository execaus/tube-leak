//! Оркестрация разбора ссылки (Ф-1, Ф-2, Ф-8) — TL-32.
//!
//! Здесь сходятся три готовых куска: валидация ввода (своя, ниже), запуск
//! yt-dlp через домен [`crate::sidecar`] (E1), классификация исхода
//! ([`classify`], TL-31) и схлопывание форматов в лестницу
//! ([`build_quality_ladder`], TL-30). Собственной логики разбора здесь нет
//! — только порядок вызовов, аргументы запуска, таймаут и правило «не
//! более одного разбора одновременно».
//!
//! # Аргументы запуска ([`METADATA_ARGS`])
//!
//! Набор один на все формы ссылки, и каждый его элемент проверен живым
//! запуском (yt-dlp 2026.08.19, macOS 26.6, 2026-08-26; фикстуры и замеры
//! — в `tests/fixtures/ytdlp-probe/orchestration/`):
//!
//! - `-J` — метаданные одним JSON в stdout; на нём стоит вся классификация
//!   и вся лестница.
//! - `--no-playlist` — `watch?v=…&list=…` разбирается как **ролик**
//!   (решение анализа E2, С-9). Без флага тот же адрес уходит в extractor
//!   `youtube:tab` и становится классом «плейлист».
//! - `--flat-playlist` — без него ссылка на плейлист не классифицируется
//!   вовсе: yt-dlp обходит каждый ролик, и замер TL-31 показал, что
//!   плейлист из 19 роликов не завершился за 60 с. Пользователь получил бы
//!   «разбор не уложился в отведённое время» вместо «плейлисты не
//!   поддерживаются» — то есть С-9 не выполнялся бы никогда.
//! - `--playlist-end 1` — того же рода условие выполнимости, но для
//!   **канала**, и найдено уже здесь, замером TL-32: `/@NASA` с
//!   `--flat-playlist`, но без ограничения, не завершился и за 45 с —
//!   yt-dlp постранично забирает всю выдачу канала. С ограничением тот же
//!   адрес отвечает за 3,4 с. Классификация смотрит только на `_type` и
//!   `extractor` (TL-31), поэтому первой записи ей хватает с запасом.
//! - `--` — разделитель аргументов (Ф-2): всё после него yt-dlp
//!   трактует как позиционный аргумент, а не как флаг. Проверено живьём:
//!   `-o--` **до** разделителя съедается опцией `-o` («You must provide at
//!   least one URL»), после разделителя становится ссылкой («'-o--' is not
//!   a valid URL»). Второй пояс поверх [`validate_url`], который такой
//!   ввод и так не пропускает.
//!
//! Форма ссылки на набор аргументов не влияет — это осознанный выбор
//! против ветвления «по виду ссылки»: ветвление означало бы, что реальный
//! набор аргументов зависит от того, распознали ли мы форму адреса
//! **до** запуска, то есть от собственного парсера ссылок YouTube. Такой
//! парсер — ровно то, чего эпик и CLAUDE.md запрещают заводить: адреса
//! меняет YouTube, а отвечать за них должен yt-dlp.
//!
//! Ценой единого набора стало то, что фикстуры успеха, снятые в TL-30 и
//! TL-31 базовой формой (`-J --no-playlist`), финальный набор не
//! покрывают. Поэтому он снят отдельно и целиком: полная лестница
//! (`final-argv-success-4k`), ролик внутри плейлиста
//! (`final-argv-watch-with-list`), короткая форма `youtu.be`
//! (`final-argv-short-form`), плейлист и канал — и на них стоят тесты
//! ниже.
//!
//! # Что не задаётся аргументами
//!
//! Прокси, число соединений и повторы остаются дефолтными — «естественное
//! поведение» (Н-2, CLAUDE.md): системный прокси yt-dlp уважает сам.
//! Пользовательский конфиг yt-dlp (`~/.config/yt-dlp/config`) не
//! отключается: `--ignore-config` не добавлен сознательно — фикстуры
//! сняты без него, а поведение «как у обычного yt-dlp на машине
//! пользователя» ближе к инварианту эпика, чем герметичность запуска.
//!
//! # Одновременно — не более одного разбора (Ф-8)
//!
//! Вытеснение, а не отказ: новый вызов сам добивает предыдущий. Фронтенд
//! на это рассчитывает — при замене одной ссылки другой он **не** зовёт
//! `cancel_probe` (тест на стороне ui), и отказ нового вызова сломал бы
//! С-3.
//!
//! Механизм — счётчик поколений в [`ProbeSession`] плюс очередь
//! ([`ProbeSession::gate`]). Вызов, пришедший позже, увеличивает счётчик и
//! убивает процесс предыдущего **до** того, как встанет в очередь;
//! вызов, чьё поколение устарело, пока он ждал очереди, не запускает
//! ничего вовсе. Поэтому два подряд вставленных адреса дают ровно один
//! запуск yt-dlp, а не два последовательных (Н-2, «один разбор — один
//! запуск»).
//!
//! # Чем становится вытесненный вызов
//!
//! Классов Ф-6 ровно девять, «вытеснен» среди них нет, а контракт
//! (TL-27) смержен и не переписывается ради этого: десятый класс — это
//! правка в трёх областях (Rust, TS-зеркало, таблица текстов экрана) ради
//! состояния, которого пользователь не увидит. `Option` в позиции успеха
//! отвергнут по той же причине и ещё по одной: ревью TL-33 проверило
//! запуском, что текущая карточка на пустом результате падает.
//!
//! Поэтому вытесненный вызов — [`ProbeFailure::YtDlpFailure`] с
//! под-причиной [`YtDlpFailureReason::Generic`], и это сознательно
//! «безопасный неправильный ответ»: до экрана он не доходит (сторож по
//! поколениям на фронтенде отбрасывает и resolve, и reject устаревшего
//! вызова), а если бы дошёл — показал бы «Не удалось получить данные о
//! ролике. Попробуйте ещё раз» с работающей кнопкой «Повторить», а не
//! ложную «нет сети» или бесконечное ожидание. В лог при этом пишется,
//! что разбор именно вытеснен, — там места хватает.
//!
//! # Два исхода без выразимого успеха
//!
//! Контракт требует и название, и длительность, а лестница без единого
//! пункта — карточка без единого действия. Оба случая решены явно, чтобы
//! решение не принял `unwrap_or(0)` в чьём-нибудь коде:
//!
//! - **метаданные без длительности** — отказ. Показать карточку не из
//!   чего: `durationSecs` не опционален (Ф-5, К-1);
//! - **успех с пустой лестницей** (завершённая трансляция без форматов) —
//!   отказ. «Успешная» карточка, где нечего выбрать и нечего будет
//!   скачать в E3, — это состояние без честного выхода.
//!
//! Оба — тот же [`ProbeFailure::YtDlpFailure`]: единственный класс Ф-6,
//! который честно означает «данные получить не удалось, причина не
//! опознана». Различаются они только строкой лога — `message` контракта
//! берётся из `Display` варианта и один на класс.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::classify::{classify, YtDlpOutcome};
use super::error::ProbeFailure;
use super::quality::build_quality_ladder;
use crate::sidecar::{
    run_cancellable, stderr_tail, ChildRegistry, RunHandle, RunOutput, SidecarError,
};
use crate::types::{ProbeErrorDetails, ProbeResult, YtDlpFailureReason};

/// Таймаут одного разбора, в секундах.
///
/// Значение задано дизайном E2 и этой задачей не пересматривается;
/// калибровка (Н-1, урок E1: некалиброванные таймауты дважды провалили
/// приёмку) подтвердила запас, а не изменила число.
///
/// # Замеры, снятые финальным набором аргументов (пин 2026.08.19)
///
/// Apple Silicon, macOS 26.6, прогретое дерево yt-dlp в каталоге данных —
/// то есть состояние, в котором разбор вообще доступен пользователю
/// (С-11: до готовности yt-dlp поле ссылки неактивно).
///
/// Прямой запуск того же бинарника, по 4 запуска подряд на ролик:
///
/// | ролик                                   | разброс                 |
/// |-----------------------------------------|-------------------------|
/// | Big Buck Bunny (полная лестница до 4K)  | 2,71–2,99 с             |
/// | Gangnam Style (максимум 1080p)          | 2,92–3,06 с             |
/// | MrBeast (110 аудиодорожек, 22 языка)    | 3,28–3,40 с             |
/// | «Me at the zoo» (максимум 240p)         | 2,64–2,90 с             |
///
/// Полный путь ядра (`probe` целиком: валидация, запуск, классификация,
/// лестница), релизный профиль, та же машина: **3,88 с** на первом разборе
/// после старта приложения и 3,32 / 3,20 с на следующих. Разница с прямым
/// запуском — цена первого обращения, а не самой оркестрации.
///
/// # Чего этот замер не покрывает
///
/// Замер снят **не через смонтированный `.app`**. Бандл собран
/// (`npm run tauri build`), образ смонтирован, приложение из образа
/// запущено и живо — но вставить ссылку в поле снаружи нечем: разбор
/// начинается вводом пользователя, а у автоматизации нет разрешения
/// macOS «Упрощённый доступ» (`osascript` получает −1719). Поэтому числа
/// выше сняты релизной сборкой того же кода, дошедшей до `probe` тем же
/// путём, что и команда, — от бандла её отделяет только IPC-хоп
/// (единицы миллисекунд на локальном IPC) и рендер карточки.
///
/// То есть таймаут откалиброван на **ядре релизной сборки**, а не на
/// пути «вставка ссылки → карточка» целиком. Оставшийся кусок закрывает
/// визуальный проход владельца при приёмке эпика (К-1), и это
/// расхождение с формулировкой критерия записано здесь намеренно, а не
/// умолчано.
///
/// # Вывод
///
/// Худшее измеренное время — 3,88 с при цели Н-1 «карточка ≤ 10 с»:
/// расхождения с целью нет, запас до цели 2,6×, до таймаута — 7,7×.
/// Уменьшать значение вслед за замером нельзя: 30 с — это не ожидаемое
/// время, а граница, за которой ожидание перестаёт быть осмысленным, и
/// она обязана пережить медленную сеть, ролик с сотнями форматов и машину
/// слабее эталонной. Увеличивать — тем более: Н-1 требует ≤ 10 с в норме,
/// а 30 с уже втрое больше.
pub const PROBE_TIMEOUT_SECS: u64 = 30;

/// [`PROBE_TIMEOUT_SECS`] как [`Duration`] — то, что уходит в запуск.
const PROBE_TIMEOUT: Duration = Duration::from_secs(PROBE_TIMEOUT_SECS);

/// Аргументы запуска yt-dlp за метаданными — всё, кроме самой ссылки.
///
/// Почему именно эти пять и почему один набор на все формы ссылки —
/// в doc модуля. Порядок значим только в одном: `--` идёт последним, и
/// ссылка приписывается сразу за ним ([`probe_args`]).
const METADATA_ARGS: [&str; 6] = [
    "-J",
    "--no-playlist",
    "--flat-playlist",
    "--playlist-end",
    "1",
    "--",
];

/// Полный argv разбора: [`METADATA_ARGS`] и ссылка после разделителя.
///
/// Отдельная функция, а не строка внутри [`probe`], потому что её
/// проверяет тест Ф-2: ссылка обязана стоять после `--` и быть последним
/// элементом, что бы в ней ни было.
fn probe_args(url: &str) -> Vec<&str> {
    let mut args = METADATA_ARGS.to_vec();
    args.push(url);
    args
}

/// Проверяет ввод до формирования аргументов и до всякого запуска (Ф-2,
/// С-4) и возвращает ссылку в том виде, в каком она пойдёт в argv.
///
/// Принимается только `http://…`/`https://…` с непустым хостом. Схема
/// сверяется без учёта регистра (`HTTPS://…` — та же ссылка), обрамляющие
/// пробелы срезаются: они появляются при вставке из буфера и в ссылке
/// значить ничего не могут.
///
/// Отвергается всё остальное, включая строку с ведущим `-`: до аргументов
/// запуска такой ввод не доходит вовсе — процесса не существует, и это
/// первый из двух поясов Ф-2 (второй — разделитель `--`).
///
/// Домен не проверяется: «ссылка не на YouTube» — не класс Ф-6, и решать
/// это должен yt-dlp своим extractor'ом, а не наш список доменов.
/// Внутренние пробелы и управляющие символы отвергаются: валидная ссылка
/// их не содержит, а ввод с ними — это точно не то, что пользователь
/// скопировал из адресной строки.
pub fn validate_url(input: &str) -> Result<&str, ProbeFailure> {
    let url = input.trim();

    let rest = strip_scheme(url).ok_or(ProbeFailure::NotAUrl)?;
    let host = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .trim();
    if host.is_empty() {
        return Err(ProbeFailure::NotAUrl);
    }

    if url
        .chars()
        .any(|symbol| symbol.is_whitespace() || symbol.is_control())
    {
        return Err(ProbeFailure::NotAUrl);
    }

    Ok(url)
}

/// Остаток ссылки после `http://`/`https://`, если схема именно такая.
fn strip_scheme(url: &str) -> Option<&str> {
    ["https://", "http://"].into_iter().find_map(|scheme| {
        url.get(..scheme.len())
            .filter(|prefix| prefix.eq_ignore_ascii_case(scheme))
            .map(|_| &url[scheme.len()..])
    })
}

/// Запускатель yt-dlp — единственный шов между оркестрацией и настоящим
/// процессом.
///
/// Существует ради проверяемости требований, которые иначе не проверить
/// без сети и без реального yt-dlp: «на не-ссылку процесс не порождается
/// вовсе» (Ф-2) и «второй разбор вытесняет первый» (Ф-8). Продакшен-путь
/// — ровно одна реализация, [`SidecarLauncher`].
pub trait YtDlpLauncher: Send + Sync {
    /// Запускает yt-dlp с `args`, ждёт не дольше `timeout` и отдаёт то,
    /// что осталось от процесса. `handle` — дескриптор отмены: пока
    /// запуск идёт, оркестрация вправе убить его снаружи.
    fn launch<'a>(
        &'a self,
        args: &'a [&'a str],
        timeout: Duration,
        handle: &'a RunHandle,
    ) -> Pin<Box<dyn Future<Output = Result<RunOutput, SidecarError>> + Send + 'a>>;
}

/// Продакшен-реализация [`YtDlpLauncher`]: запуск через домен `sidecar`
/// (E1) — резолв пути делает вызывающая команда, реестр процессов и
/// таймаут живут в [`crate::sidecar::run_cancellable`].
pub struct SidecarLauncher<'a> {
    executable: PathBuf,
    registry: &'a ChildRegistry,
}

impl<'a> SidecarLauncher<'a> {
    /// `executable` — путь к готовому yt-dlp (каталог данных, см.
    /// [`crate::ytdlp`]); `registry` — реестр PID из состояния приложения.
    pub fn new(executable: PathBuf, registry: &'a ChildRegistry) -> Self {
        Self {
            executable,
            registry,
        }
    }
}

impl YtDlpLauncher for SidecarLauncher<'_> {
    fn launch<'a>(
        &'a self,
        args: &'a [&'a str],
        timeout: Duration,
        handle: &'a RunHandle,
    ) -> Pin<Box<dyn Future<Output = Result<RunOutput, SidecarError>> + Send + 'a>> {
        Box::pin(run_cancellable(
            &self.executable,
            args,
            timeout,
            self.registry,
            handle,
        ))
    }
}

/// Состояние «идёт разбор» на всё приложение: живёт как Tauri-состояние
/// (`app.manage`), ровно один экземпляр на процесс.
///
/// Держит две вещи: счётчик поколений (кто последний спросил) и очередь
/// (кто сейчас работает). Зачем обе — в doc модуля.
#[derive(Debug)]
pub struct ProbeSession {
    inner: StdMutex<SessionState>,
    /// Очередь на запуск: держится всё время работы разбора, поэтому
    /// следующий вызов физически не может стартовать, пока предыдущий не
    /// вернул управление. `tokio::sync::Mutex` — потому что удерживается
    /// через `await`, и он честно-очередной (FIFO).
    gate: tokio::sync::Mutex<()>,
    /// Таймаут одного запуска. Поле, а не константа в коде запуска, —
    /// чтобы тест таймаута не ждал реальные 30 с; продакшен-путь
    /// ([`ProbeSession::new`]) подставляет [`PROBE_TIMEOUT`] и другого
    /// значения взять неоткуда.
    timeout: Duration,
}

#[derive(Debug, Default)]
struct SessionState {
    /// Монотонный счётчик: увеличивается на каждый вызов `probe_url` и на
    /// каждую отмену. Вызов, чьё поколение перестало быть текущим, свой
    /// запуск не начинает.
    generation: u64,
    /// Дескриптор процесса идущего разбора, если он идёт.
    current: Option<Arc<RunHandle>>,
}

impl Default for ProbeSession {
    fn default() -> Self {
        Self::new()
    }
}

impl ProbeSession {
    /// Пустая сессия с продакшен-таймаутом ([`PROBE_TIMEOUT_SECS`]).
    pub fn new() -> Self {
        Self::with_timeout(PROBE_TIMEOUT)
    }

    fn with_timeout(timeout: Duration) -> Self {
        Self {
            inner: StdMutex::new(SessionState::default()),
            gate: tokio::sync::Mutex::new(()),
            timeout,
        }
    }

    /// Отменяет идущий разбор, не начиная нового (`cancel_probe`).
    ///
    /// Нужна отдельно от [`probe`], потому что очистка поля ссылки не
    /// сопровождается новым адресом, который можно было бы передать
    /// вместо неё (дизайн E2, «Управляющие вызовы»).
    pub async fn cancel(&self) {
        if let Some(previous) = self.begin().1 {
            previous.cancel().await;
        }
    }

    /// Объявляет новое поколение и забирает дескриптор предыдущего
    /// разбора, если он был. Убивает его уже вызывающий — под мьютексом
    /// `await` недопустим.
    fn begin(&self) -> (u64, Option<Arc<RunHandle>>) {
        let mut state = self.lock();
        state.generation += 1;
        (state.generation, state.current.take())
    }

    /// Объявляет `handle` текущим разбором, если поколение `generation`
    /// всё ещё последнее.
    ///
    /// `false` — пока вызов ждал очереди, пришёл следующий: запускать
    /// нечего и не нужно.
    fn install(&self, generation: u64, handle: &Arc<RunHandle>) -> bool {
        let mut state = self.lock();
        if state.generation != generation {
            return false;
        }
        state.current = Some(Arc::clone(handle));
        true
    }

    /// Снимает `handle` с роли текущего разбора — если его не сменил уже
    /// кто-то другой (сравнение по указателю, а не по значению: у двух
    /// разных запусков состояние дескриптора может совпадать).
    fn clear(&self, handle: &Arc<RunHandle>) {
        let mut state = self.lock();
        if state
            .current
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, handle))
        {
            state.current = None;
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SessionState> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Разбирает ссылку: валидация → запуск yt-dlp → классификация исхода →
/// лестница качеств (Ф-1, Ф-2, Ф-8).
///
/// Начинает разбор или **заменяет** идущий: предыдущий процесс убивается
/// до старта нового (см. doc модуля). Вернуться может только двумя
/// способами — карточкой или одним из девяти классов Ф-6.
pub async fn probe<L>(
    session: &ProbeSession,
    launcher: &L,
    input: &str,
) -> Result<ProbeResult, ProbeFailure>
where
    L: YtDlpLauncher + ?Sized,
{
    // Ф-2: до запуска процесса и до формирования аргументов. Порядок в
    // коде — часть требования, а не стиль.
    let url = validate_url(input)?;

    let (generation, previous) = session.begin();
    if let Some(previous) = previous {
        // Ф-8: предыдущий процесс убит до того, как стартует новый, —
        // `cancel` возвращает управление, когда убийство уже отправлено.
        previous.cancel().await;
    }

    let _queued = session.gate.lock().await;

    let handle = Arc::new(RunHandle::new());
    if !session.install(generation, &handle) {
        // Пока ждали очереди, пользователь ввёл ещё что-то. Свой запуск не
        // начинаем вовсе: один разбор — один запуск yt-dlp (Н-2).
        eprintln!("probe: вызов вытеснен более новым до запуска yt-dlp");
        return Err(preempted());
    }

    let args = probe_args(url);
    eprintln!("probe: запуск yt-dlp {}", args.join(" "));

    let started = Instant::now();
    let outcome = launcher.launch(&args, session.timeout, &handle).await;
    let elapsed = started.elapsed();

    session.clear(&handle);

    if handle.was_cancelled() {
        eprintln!(
            "probe: разбор вытеснен и процесс убит через {} мс",
            elapsed.as_millis()
        );
        return Err(preempted());
    }

    let result = interpret(outcome);
    match &result {
        Ok(card) => eprintln!(
            "probe: карточка за {} мс, пунктов качества {}",
            elapsed.as_millis(),
            card.qualities.len()
        ),
        Err(failure) => eprintln!(
            "probe: отказ за {} мс, класс {:?}: {failure}",
            elapsed.as_millis(),
            failure.kind()
        ),
    }
    result
}

/// Вытесненный (или отменённый) разбор. Почему именно этот класс — в doc
/// модуля, раздел «Чем становится вытесненный вызов».
fn preempted() -> ProbeFailure {
    ProbeFailure::YtDlpFailure {
        reason: YtDlpFailureReason::Generic,
        // Процесс убит нами, а не отказал: рассказывать в «Подробнее»
        // нечего, и пустые детали границу команды не пересекают.
        details: ProbeErrorDetails {
            stderr_tail: None,
            exit_code: None,
        },
    }
}

/// Превращает исход запуска в карточку или класс отказа.
///
/// Таймаут разбирается здесь, а не классификацией: [`classify`] отвечает
/// на вопрос «что сказал yt-dlp», а убитый по таймауту процесс не сказал
/// ничего — решение принято нами и по нашим часам.
fn interpret(outcome: Result<RunOutput, SidecarError>) -> Result<ProbeResult, ProbeFailure> {
    let (exit_code, stdout, stderr) = match outcome {
        Ok(RunOutput { stdout, stderr }) => (Some(0), stdout, stderr),
        Err(SidecarError::Timeout { ms, stderr }) => {
            return Err(ProbeFailure::Timeout {
                // Контракт говорит с пользователем в секундах, а порог
                // приходит в миллисекундах — округляем вверх, чтобы
                // «за 0 с» не появилось никогда.
                secs: ms.div_ceil(1_000),
                details: ProbeErrorDetails {
                    stderr_tail: stderr_tail(&stderr),
                    // Убитый процесс своего кода завершения не оставил.
                    exit_code: None,
                },
            });
        }
        // Процесс стартовал и отказал сам — код и stderr отдаёт
        // классификации, она и решит класс.
        Err(SidecarError::NonZeroExit { code, stderr }) => (Some(code), String::new(), stderr),
        // Процесс убит сигналом, не смог стартовать или бинарника нет
        // вовсе. Кода завершения нет ни в одном из случаев; всё, что
        // известно, — stderr, если он был. Класс определит классификация:
        // без узнаваемого текста это «сбой yt-dlp» (С-12) — честный
        // catch-all и для «yt-dlp пропал из каталога данных».
        Err(SidecarError::LaunchFailed { stderr, .. }) => (None, String::new(), stderr),
        Err(SidecarError::NotFound) => (None, String::new(), String::new()),
    };

    let metadata = classify(&YtDlpOutcome {
        exit_code,
        stdout: &stdout,
        stderr: &stderr,
    })?;

    card(&metadata, &stderr)
}

/// Собирает карточку из метаданных, признанных успехом.
///
/// Два исхода без выразимого успеха (нет длительности, пустая лестница)
/// становятся отказом класса «сбой yt-dlp» — см. doc модуля.
fn card(metadata: &Value, stderr: &str) -> Result<ProbeResult, ProbeFailure> {
    // Кода завершения в деталях этих трёх отказов нет намеренно, хотя он
    // известен и равен нулю: «Подробнее» под заголовком «Не удалось
    // получить данные о ролике», где написано, что процесс завершился
    // успешно, — это не диагностика, а противоречие. Процесс здесь
    // действительно отработал штатно, вопрос не к нему: полезен только
    // хвост stderr (предупреждения окружения yt-dlp), а чего именно не
    // хватило в метаданных, сказано строкой лога рядом.
    let details = ProbeErrorDetails {
        stderr_tail: stderr_tail(stderr),
        exit_code: None,
    };

    let Some(title) = text(metadata, "title") else {
        eprintln!("probe: в метаданных нет названия — карточку собрать не из чего");
        return Err(ProbeFailure::YtDlpFailure {
            reason: YtDlpFailureReason::Generic,
            details,
        });
    };

    let Some(duration_secs) = duration_secs(metadata) else {
        eprintln!("probe: в метаданных нет длительности — карточку собрать не из чего");
        return Err(ProbeFailure::YtDlpFailure {
            reason: YtDlpFailureReason::Generic,
            details,
        });
    };

    let qualities = build_quality_ladder(metadata);
    if qualities.is_empty() {
        eprintln!("probe: у ролика нет ни одного пункта качества — выбирать нечего");
        return Err(ProbeFailure::YtDlpFailure {
            reason: YtDlpFailureReason::Generic,
            details,
        });
    }

    Ok(ProbeResult {
        title: title.to_owned(),
        duration_secs,
        // Имя канала не обязательно: карточка без него остаётся полезной
        // (контракт, дизайн E2). `uploader` — запасной источник того же
        // самого имени, yt-dlp заполняет его не всегда одновременно с
        // `channel`.
        channel: text(metadata, "channel")
            .or_else(|| text(metadata, "uploader"))
            .map(str::to_owned),
        thumbnail_url: text(metadata, "thumbnail").map(str::to_owned),
        qualities,
    })
}

/// Непустая строка поля метаданных; `None` — поля нет, оно не строка или
/// в нём одни пробелы (для карточки это то же самое, что нет поля).
fn text<'a>(metadata: &'a Value, key: &str) -> Option<&'a str> {
    metadata
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// Длительность ролика в секундах.
///
/// yt-dlp отдаёт её числом, но не обязательно целым (у части роликов —
/// дробное), поэтому читается как `f64` и округляется. Ноль и
/// отрицательное значение — то же отсутствие данных, просто выраженное
/// числом: карточка «0:00» врала бы пользователю.
///
/// Отсечка стоит **после** округления, а не до: ролик короче полусекунды
/// прошёл бы проверку `> 0` и всё равно стал бы нулём — той самой
/// карточкой «0:00». На живом YouTube такой ролик почти недостижим, но
/// порядок двух строк не должен решать, соврём мы пользователю или нет.
fn duration_secs(metadata: &Value) -> Option<u64> {
    let secs = metadata.get("duration").and_then(Value::as_f64)?;
    if !secs.is_finite() || secs < 0.0 {
        return None;
    }

    let rounded = secs.round() as u64;
    (rounded > 0).then_some(rounded)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::VecDeque;
    use std::fs;
    use std::path::Path;

    use crate::types::{QualityItem, QualityKind, QualitySize};

    // ───────────────────────── подменяемый запускатель ─────────────────

    /// Что должен сделать очередной запуск.
    enum Behaviour {
        /// Мгновенно вернуть готовый исход.
        Immediate(Result<RunOutput, SidecarError>),
        /// Висеть, пока запуск не отменят снаружи, и вернуть то, что
        /// вернул бы убитый сигналом процесс (кода завершения нет).
        UntilCancelled,
    }

    /// Подменяемый [`YtDlpLauncher`]: считает вызовы, помнит argv и
    /// следит, чтобы двух одновременных запусков не случилось.
    ///
    /// Нужен ровно для тех требований, которые на настоящем процессе не
    /// проверить: «на не-ссылку процесс не порождается вовсе» и
    /// «второй разбор вытесняет первый».
    #[derive(Default)]
    struct FakeLauncher {
        state: StdMutex<FakeState>,
        script: StdMutex<VecDeque<Behaviour>>,
    }

    #[derive(Default)]
    struct FakeState {
        calls: Vec<Vec<String>>,
        timeouts: Vec<Duration>,
        live: usize,
        max_live: usize,
    }

    impl FakeLauncher {
        fn with_script(script: impl IntoIterator<Item = Behaviour>) -> Self {
            Self {
                state: StdMutex::new(FakeState::default()),
                script: StdMutex::new(script.into_iter().collect()),
            }
        }

        fn succeeding(stdout: &str) -> Self {
            Self::with_script([Behaviour::Immediate(Ok(RunOutput {
                stdout: stdout.to_owned(),
                stderr: String::new(),
            }))])
        }

        fn calls(&self) -> Vec<Vec<String>> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .calls
                .clone()
        }

        fn max_live(&self) -> usize {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .max_live
        }

        fn timeouts(&self) -> Vec<Duration> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .timeouts
                .clone()
        }

        fn enter(&self, args: &[&str], timeout: Duration) -> Behaviour {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state
                .calls
                .push(args.iter().map(|arg| (*arg).to_owned()).collect());
            state.timeouts.push(timeout);
            state.live += 1;
            state.max_live = state.max_live.max(state.live);
            drop(state);

            self.script
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .pop_front()
                .unwrap_or(Behaviour::Immediate(Ok(RunOutput {
                    stdout: String::new(),
                    stderr: String::new(),
                })))
        }

        fn leave(&self) {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.live -= 1;
        }
    }

    impl YtDlpLauncher for FakeLauncher {
        fn launch<'a>(
            &'a self,
            args: &'a [&'a str],
            timeout: Duration,
            handle: &'a RunHandle,
        ) -> Pin<Box<dyn Future<Output = Result<RunOutput, SidecarError>> + Send + 'a>> {
            Box::pin(async move {
                let behaviour = self.enter(args, timeout);
                let outcome = match behaviour {
                    Behaviour::Immediate(outcome) => outcome,
                    Behaviour::UntilCancelled => {
                        handle.cancelled().await;
                        Err(SidecarError::LaunchFailed {
                            reason: crate::types::LaunchFailedReason::Corrupted,
                            stderr: String::new(),
                        })
                    }
                };
                self.leave();
                outcome
            })
        }
    }

    // ───────────────────────────── фикстуры ────────────────────────────

    /// Конверт запуска, снятый **финальным** набором аргументов
    /// (см. README рядом с фикстурами).
    struct Capture {
        envelope: Value,
    }

    impl Capture {
        fn load(name: &str) -> Self {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/ytdlp-probe/orchestration")
                .join(format!("{name}.json"));
            let raw = fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("фикстура {} не читается: {err}", path.display()));
            Self {
                envelope: serde_json::from_str(&raw)
                    .unwrap_or_else(|err| panic!("фикстура {} — не JSON: {err}", path.display())),
            }
        }

        fn field<'a>(&'a self, key: &str) -> &'a Value {
            self.envelope
                .get(key)
                .unwrap_or_else(|| panic!("в конверте нет поля {key}"))
        }

        fn stdout(&self) -> String {
            match self.field("stdout") {
                Value::Null => String::new(),
                value => value.to_string(),
            }
        }

        /// Исход запуска в том виде, в каком его отдал бы домен `sidecar`.
        fn outcome(&self) -> Result<RunOutput, SidecarError> {
            let stderr = self
                .field("stderr")
                .as_str()
                .expect("stderr конверта — строка")
                .to_owned();
            match self.field("exitCode").as_i64() {
                Some(0) => Ok(RunOutput {
                    stdout: self.stdout(),
                    stderr,
                }),
                Some(code) => Err(SidecarError::NonZeroExit {
                    code: code as i32,
                    stderr,
                }),
                None => Err(SidecarError::LaunchFailed {
                    reason: crate::types::LaunchFailedReason::Corrupted,
                    stderr,
                }),
            }
        }

        fn argv(&self) -> Vec<String> {
            self.field("_capture")
                .get("argv")
                .and_then(Value::as_array)
                .expect("в конверте объявлен argv")
                .iter()
                .map(|arg| arg.as_str().expect("argv — строки").to_owned())
                .collect()
        }

        fn elapsed_secs(&self) -> f64 {
            self.field("_capture")
                .get("elapsedSecs")
                .and_then(Value::as_f64)
                .expect("в конверте объявлено время запуска")
        }
    }

    const CAPTURES: [&str; 5] = [
        "final-argv-success-4k",
        "final-argv-watch-with-list",
        "final-argv-short-form",
        "final-argv-playlist",
        "final-argv-channel",
    ];

    /// Фикстуры одного и того же ролика, снятые тремя формами адреса.
    /// Обоснование «набор аргументов один на все формы ссылки» держится
    /// ровно на том, сколько форм на нём проверено.
    const SAME_VIDEO: [&str; 3] = [
        "final-argv-success-4k",
        "final-argv-watch-with-list",
        "final-argv-short-form",
    ];

    const URL: &str = "https://www.youtube.com/watch?v=aqz-KE-bpKQ";

    fn session() -> ProbeSession {
        ProbeSession::new()
    }

    /// Прогоняет готовый исход через всю оркестрацию — то же, что сделал
    /// бы настоящий запуск, вернувший ровно это.
    async fn probe_outcome(
        outcome: Result<RunOutput, SidecarError>,
    ) -> Result<ProbeResult, ProbeFailure> {
        let launcher = FakeLauncher::with_script([Behaviour::Immediate(outcome)]);
        probe(&session(), &launcher, URL).await
    }

    // ───────────────────── Ф-2: не ссылка, разделитель ─────────────────

    #[tokio::test]
    async fn input_that_is_not_an_http_url_never_reaches_the_launcher() {
        // С-4 целиком: произвольный текст, путь, строка-флаг, чужая схема,
        // ссылка без хоста, пустой ввод.
        let rejected = [
            "просто текст",
            "-о--",
            "--flat-playlist",
            "/Users/me/video.mp4",
            "ftp://example.com/x",
            "javascript:alert(1)",
            "www.youtube.com/watch?v=x",
            "https://",
            "https:///watch?v=x",
            "",
            "   ",
        ];

        for input in rejected {
            let launcher = FakeLauncher::succeeding("{}");
            let failure = probe(&session(), &launcher, input)
                .await
                .expect_err("«{input}» — не http(s)-ссылка");

            assert_eq!(failure, ProbeFailure::NotAUrl, "ввод «{input}»");
            assert!(
                launcher.calls().is_empty(),
                "ввод «{input}»: процесс не должен порождаться вовсе (Ф-2, К-3а)"
            );
        }
    }

    #[tokio::test]
    async fn an_http_url_passes_validation_in_any_case_and_with_stray_spaces() {
        for input in [
            "https://www.youtube.com/watch?v=x",
            "http://youtu.be/x",
            "HTTPS://WWW.YOUTUBE.COM/watch?v=x",
            "  https://youtu.be/x  ",
        ] {
            let launcher = FakeLauncher::succeeding("{}");
            // Результат неважен: `{}` — не карточка. Важно, что запуск
            // состоялся, то есть валидация ввод пропустила.
            let _ = probe(&session(), &launcher, input).await;
            assert_eq!(
                launcher.calls().len(),
                1,
                "ввод «{input}» — валидная http(s)-ссылка"
            );
        }
    }

    #[test]
    fn the_url_goes_after_the_argument_separator_and_nothing_follows_it() {
        // Ф-2, второй пояс. Живая проверка (yt-dlp 2026.08.19): `-o--`
        // ДО разделителя съедается опцией `-o` — «You must provide at
        // least one URL»; ПОСЛЕ разделителя становится ссылкой —
        // «'-o--' is not a valid URL».
        for url in [URL, "https://youtu.be/-dashes-", "https://example.com/-o--"] {
            let args = probe_args(url);

            let separator = args
                .iter()
                .position(|arg| *arg == "--")
                .expect("разделитель аргументов обязателен (Ф-2)");
            assert_eq!(
                separator,
                args.len() - 2,
                "после разделителя стоит ровно один элемент — ссылка"
            );
            assert_eq!(args.last(), Some(&url));
        }
    }

    #[test]
    fn the_launch_arguments_are_the_ones_the_fixtures_were_captured_with() {
        // Набор аргументов и фикстуры обязаны разъезжаться громко: класс
        // «плейлист» достижим только с `--flat-playlist`, а лестница
        // строится по выводу того же запуска (обязанность TL-32 из
        // ревью TL-31).
        for name in CAPTURES {
            let capture = Capture::load(name);
            let argv = capture.argv();
            let url = argv.last().expect("в argv фикстуры есть ссылка");

            assert_eq!(
                argv,
                probe_args(url)
                    .iter()
                    .map(|arg| (*arg).to_owned())
                    .collect::<Vec<_>>(),
                "{name}: фикстура снята не тем набором аргументов, который \
                 формирует оркестрация. Набор сменили — переснимите фикстуры \
                 по README, а не правьте эту строку"
            );
        }
    }

    // ───────────────── лестница на финальном наборе аргументов ─────────

    #[tokio::test]
    async fn the_final_arguments_still_yield_the_whole_ladder() {
        // Главная проверка обязанности «покажи, что лестница на твоём
        // наборе строится целиком»: фикстура снята финальным argv
        // (с `--flat-playlist`), и лестница на ней полная.
        let card = probe_outcome(Capture::load("final-argv-success-4k").outcome())
            .await
            .expect("полная фикстура успеха обязана давать карточку");

        assert_eq!(
            card.title,
            "Big Buck Bunny 60fps 4K - Official Blender Foundation Short Film"
        );
        assert_eq!(card.duration_secs, 635);
        assert!(card.channel.is_some(), "имя канала есть в метаданных");
        assert!(card.thumbnail_url.is_some(), "превью есть в метаданных");

        let heights: Vec<Option<u32>> = card.qualities.iter().map(|item| item.height_px).collect();
        assert_eq!(
            heights,
            vec![Some(2160), Some(1440), Some(1080), Some(720), None],
            "К-1: 2160p / 1440p / 1080p / 720p и «только аудио»"
        );
        assert_eq!(
            card.qualities.last().map(|item| item.kind),
            Some(QualityKind::AudioOnly)
        );

        let sizes: Vec<u64> = card.qualities.iter().map(size_of).collect();
        assert!(
            sizes.windows(2).all(|pair| pair[0] > pair[1]),
            "К-1: оценки убывают от 2160p к 720p, «только аудио» меньше любой \
             видеостроки — получилось {sizes:?}"
        );
    }

    #[tokio::test]
    async fn the_final_arguments_change_the_ladder_of_the_same_video_in_no_way() {
        // Прямое сравнение с фикстурой TL-30, снятой базовой формой
        // (`-J --no-playlist`, без `--flat-playlist`): тот же ролик, тот же
        // пин, разный argv. Если добавленные флаги когда-нибудь начнут
        // обрезать список форматов, разойдётся именно этот тест — а не
        // карточка у пользователя.
        let final_argv = probe_outcome(Capture::load("final-argv-success-4k").outcome())
            .await
            .expect("карточка ролика");

        let base_form: Value = serde_json::from_str(
            &fs::read_to_string(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/ytdlp-probe/4k-full-ladder.json"),
            )
            .expect("фикстура лестницы TL-30 читается"),
        )
        .expect("фикстура лестницы TL-30 — JSON");

        assert_eq!(
            final_argv.qualities,
            build_quality_ladder(&base_form),
            "финальный набор аргументов обязан давать ту же лестницу, что \
             базовая форма, на которой сняты фикстуры TL-30"
        );
    }

    #[tokio::test]
    async fn every_shape_of_the_same_link_gives_exactly_the_same_card() {
        // Три формы одного адреса на одном наборе аргументов:
        // - `watch?v=…` — как копируют из адресной строки;
        // - `watch?v=…&list=…` — как копируют из открытого плейлиста;
        //   `--no-playlist` перебивает `--flat-playlist`, и это ролик, а не
        //   вкладка (решение анализа E2, С-9);
        // - `youtu.be/…` — как копируют с телефона и из «Поделиться».
        //
        // Карточка обязана быть одна и та же: ролик один.
        let plain = probe_outcome(Capture::load(SAME_VIDEO[0]).outcome())
            .await
            .expect("карточка ролика");

        for name in &SAME_VIDEO[1..] {
            let other = probe_outcome(Capture::load(name).outcome())
                .await
                .unwrap_or_else(|failure| {
                    panic!("{name}: ожидалась карточка, получен {failure:?}")
                });

            assert_eq!(other.title, plain.title, "{name}");
            assert_eq!(other.duration_secs, plain.duration_secs, "{name}");
            assert_eq!(other.channel, plain.channel, "{name}");
            assert_eq!(other.qualities, plain.qualities, "{name}");
        }
    }

    #[tokio::test]
    async fn a_playlist_and_a_channel_reach_their_class_instead_of_the_timeout() {
        for name in ["final-argv-playlist", "final-argv-channel"] {
            let capture = Capture::load(name);
            let failure = probe_outcome(capture.outcome())
                .await
                .expect_err("плейлист и канал карточкой не становятся");

            assert!(
                matches!(failure, ProbeFailure::PlaylistUnsupported { .. }),
                "{name}: ожидался класс «плейлист», получен {failure:?}"
            );

            // Смысл `--flat-playlist` и `--playlist-end`: без них тот же
            // адрес не укладывается в таймаут вовсе (45 с и больше на
            // канале), и пользователь получает «не уложились» вместо
            // «плейлисты не поддерживаются».
            assert!(
                capture.elapsed_secs() < PROBE_TIMEOUT_SECS as f64,
                "{name}: замер {} с не уложился в таймаут {PROBE_TIMEOUT_SECS} с",
                capture.elapsed_secs()
            );
        }
    }

    #[test]
    fn fixtures_are_real_output_of_the_pinned_yt_dlp() {
        // Та же связь, что у фикстур лестницы (TL-30) и исходов (TL-31):
        // фикстуры заморожены, а yt-dlp — нет, и смена пина обязана громко
        // ломать этот тест, а не тихо оставлять набор зелёным.
        let pinned = pinned_yt_dlp_version();

        for name in CAPTURES {
            let capture = Capture::load(name);

            assert_eq!(
                capture
                    .field("_capture")
                    .get("ytDlpVersion")
                    .and_then(Value::as_str),
                Some(pinned.as_str()),
                "{name}: снято не тем yt-dlp, который вложен в приложение \
                 ({pinned} по binaries.lock.json). Пин сменили — переснимите \
                 фикстуры по README, а не правьте эту строку"
            );
            assert_eq!(
                capture
                    .field("stdout")
                    .get("_version")
                    .and_then(|version| version.get("version"))
                    .and_then(Value::as_str),
                Some(pinned.as_str()),
                "{name}: версия внутри вывода не совпадает с объявленной в конверте"
            );
        }
    }

    // ───────────────────── Ф-8: один разбор, вытеснение ────────────────

    #[tokio::test]
    async fn a_second_probe_kills_the_first_one_before_starting_its_own() {
        let launcher = Arc::new(FakeLauncher::with_script([
            Behaviour::UntilCancelled,
            Behaviour::Immediate(Capture::load("final-argv-success-4k").outcome()),
        ]));
        let session = Arc::new(session());

        let first = tokio::spawn({
            let (session, launcher) = (Arc::clone(&session), Arc::clone(&launcher));
            async move { probe(&session, launcher.as_ref(), URL).await }
        });

        wait_for(|| !launcher.calls().is_empty()).await;

        let second = probe(&session, launcher.as_ref(), "https://youtu.be/aqz-KE-bpKQ")
            .await
            .expect("второй разбор доходит до карточки");
        assert_eq!(second.qualities.len(), 5);

        let first = first.await.expect("первый разбор завершается, а не виснет");
        assert!(
            matches!(first, Err(ProbeFailure::YtDlpFailure { .. })),
            "вытесненный разбор возвращается отказом, а не карточкой: {first:?}"
        );

        assert_eq!(launcher.calls().len(), 2, "два адреса — два запуска");
        assert_eq!(
            launcher.max_live(),
            1,
            "Ф-8: одновременно не более одного разбора — второй стартует \
             только после того, как первый убит"
        );
    }

    #[tokio::test]
    async fn of_two_probes_queued_behind_a_running_one_only_the_last_launches() {
        // Два адреса приходят, пока первый разбор ещё идёт. Первый
        // вытесняется и убивается, а из двух оставшихся запускается только
        // последний: тот, чьё поколение устарело, пока он ждал очереди,
        // не запускает ничего вовсе — «один разбор — один запуск» (Н-2),
        // а не три подряд.
        let launcher = Arc::new(FakeLauncher::with_script([
            Behaviour::UntilCancelled,
            Behaviour::Immediate(Capture::load("final-argv-success-4k").outcome()),
        ]));
        let session = Arc::new(session());

        let spawn_probe = |url: &'static str| {
            let (session, launcher) = (Arc::clone(&session), Arc::clone(&launcher));
            tokio::spawn(async move { probe(&session, launcher.as_ref(), url).await })
        };

        let first = spawn_probe(URL);
        wait_for(|| !launcher.calls().is_empty()).await;

        // Обе задачи ставятся в очередь, пока первый разбор ещё висит, и
        // ни одна из них не успевает ничего запустить до того, как он
        // будет убит.
        let second = spawn_probe("https://youtu.be/second");
        let third = spawn_probe("https://youtu.be/third");

        assert!(
            first.await.expect("первый завершается").is_err(),
            "вытесненный разбор карточкой не возвращается"
        );
        let second = second.await.expect("второй завершается");
        let third = third.await.expect("третий завершается");

        let calls = launcher.calls();
        assert_eq!(
            calls.len(),
            2,
            "три адреса — два запуска: один из двух поздних вызовов не \
             запускал yt-dlp вовсе"
        );
        assert_eq!(calls[0].last().map(String::as_str), Some(URL));

        // Какая из двух задач проснётся первой, решает планировщик; что бы
        // он ни решил, запущен ровно тот адрес, чей вызов дошёл до
        // карточки, — второй остался без запуска.
        let launched = calls[1].last().cloned().expect("в argv есть ссылка");
        match (&second, &third) {
            (Err(_), Ok(_)) => assert_eq!(launched, "https://youtu.be/third"),
            (Ok(_), Err(_)) => assert_eq!(launched, "https://youtu.be/second"),
            pair => panic!("ровно один из двух вызовов обязан дойти до карточки: {pair:?}"),
        }
    }

    #[tokio::test]
    async fn cancel_stops_the_running_probe_without_starting_a_new_one() {
        let launcher = Arc::new(FakeLauncher::with_script([Behaviour::UntilCancelled]));
        let session = Arc::new(session());

        let running = tokio::spawn({
            let (session, launcher) = (Arc::clone(&session), Arc::clone(&launcher));
            async move { probe(&session, launcher.as_ref(), URL).await }
        });
        wait_for(|| !launcher.calls().is_empty()).await;

        session.cancel().await;

        let outcome = running.await.expect("отменённый разбор завершается");
        assert!(
            outcome.is_err(),
            "после отмены карточка не возвращается: {outcome:?}"
        );
        assert_eq!(
            launcher.calls().len(),
            1,
            "`cancel_probe` не запускает ничего своего"
        );
    }

    #[tokio::test]
    async fn cancelling_an_idle_session_does_nothing() {
        // Очистка уже пустого поля — штатный случай, а не ошибка.
        let session = session();
        session.cancel().await;
        session.cancel().await;
    }

    // ─────────────────────────── таймаут ───────────────────────────────

    #[tokio::test]
    async fn a_process_that_outlives_the_timeout_is_killed_and_becomes_the_timeout_class() {
        // Управляемая задержка — настоящий процесс, который заведомо
        // переживает таймаут; убийство и снятие с реестра проверяет домен
        // `sidecar`, здесь — что оркестрация зовёт его с таймаутом сессии
        // и превращает исход в класс «таймаут» с порогом в секундах.
        let dir = tempfile::tempdir().expect("временный каталог");
        let script = dir.path().join("sleep.sh");
        fs::write(&script, "#!/bin/sh\nsleep 30\n").expect("скрипт-фикстура");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("chmod");
        }

        let registry = ChildRegistry::new();
        let launcher = SidecarLauncher::new(script, &registry);
        let session = ProbeSession::with_timeout(Duration::from_millis(300));

        let failure = probe(&session, &launcher, URL)
            .await
            .expect_err("процесс не уложился в таймаут");

        assert!(
            matches!(failure, ProbeFailure::Timeout { secs: 1, .. }),
            "ожидался класс «таймаут» с округлённым вверх порогом, получен {failure:?}"
        );
        assert!(
            registry.is_empty(),
            "убитый по таймауту процесс снимается с реестра (TL-10)"
        );
    }

    #[tokio::test]
    async fn the_production_timeout_is_the_one_the_launch_gets() {
        let launcher = FakeLauncher::succeeding("{}");
        let _ = probe(&session(), &launcher, URL).await;

        assert_eq!(
            launcher.timeouts(),
            vec![Duration::from_secs(PROBE_TIMEOUT_SECS)],
            "запуск получает таймаут, объявленный дизайном E2"
        );
    }

    #[tokio::test]
    async fn the_timeout_threshold_travels_to_the_contract_in_seconds() {
        let failure = probe_outcome(Err(SidecarError::Timeout {
            ms: PROBE_TIMEOUT_SECS * 1_000,
            stderr: "[youtube] Downloading player".to_string(),
        }))
        .await
        .expect_err("таймаут — отказ");

        let contract = failure.to_contract();
        assert_eq!(contract.timeout_secs, Some(PROBE_TIMEOUT_SECS));
        assert_eq!(
            contract.details.and_then(|details| details.exit_code),
            None,
            "убитый процесс своего кода завершения не оставляет"
        );
    }

    // ──────────── исходы без выразимого успеха и отказы запуска ────────

    #[tokio::test]
    async fn metadata_without_a_usable_duration_is_not_a_card() {
        // Собрано вручную, а не снято: у живого ролика длительность есть
        // всегда, а единственный живой случай без неё — идущий эфир,
        // который классификация отсекает раньше (находка TL-31).
        //
        // Ноль и «0,4 с» проверяются вместе с отсутствием поля не для
        // полноты: отсечка стоит после округления именно потому, что
        // полсекунды иначе превратились бы в карточку «0:00».
        for duration in [
            r#""duration":null,"#,
            r#""duration":0,"#,
            r#""duration":0.4,"#,
            "",
        ] {
            let stdout = format!(
                r#"{{"_type":"video","title":"Ролик без длительности",{duration}
                    "formats":[{{"format_id":"137","protocol":"https",
                    "vcodec":"avc1","acodec":"none","height":1080,
                    "format_note":"1080p","filesize":1000}}]}}"#
            );

            let Err(failure) = probe_outcome(Ok(RunOutput {
                stdout,
                stderr: String::new(),
            }))
            .await
            else {
                panic!("«{duration}» не длительность, а карточка всё равно получилась");
            };

            assert!(
                matches!(failure, ProbeFailure::YtDlpFailure { .. }),
                "«{duration}»: контракт не допускает карточку без длительности (Ф-5, К-1)"
            );
        }
    }

    #[tokio::test]
    async fn a_video_without_a_single_quality_item_is_not_a_card() {
        // Успех с пустой лестницей: завершённая трансляция, у которой
        // форматов не оказалось. Карточка, где нечего выбрать, — состояние
        // без честного выхода, поэтому это отказ.
        let failure = probe_outcome(Ok(RunOutput {
            stdout: r#"{"_type":"video","title":"Запись эфира без форматов",
                        "duration":3600,"live_status":"was_live","formats":[]}"#
                .to_string(),
            stderr: "WARNING: No supported JavaScript runtime could be found".to_string(),
        }))
        .await
        .expect_err("пустая лестница — не карточка");

        assert!(matches!(failure, ProbeFailure::YtDlpFailure { .. }));

        // Код завершения в детали таких отказов не кладётся, хотя известен
        // и равен нулю: «Подробнее» под заголовком «Не удалось получить
        // данные о ролике», где написано, что процесс завершился успешно, —
        // это противоречие, а не диагностика. Хвост stderr остаётся: он
        // единственное, что тут вообще может пригодиться.
        let details = failure
            .to_contract()
            .details
            .expect("хвост stderr для «Подробнее» остаётся");
        assert_eq!(details.exit_code, None);
        assert!(details
            .stderr_tail
            .is_some_and(|tail| tail.contains("JavaScript runtime")));
    }

    #[tokio::test]
    async fn a_video_without_a_title_is_not_a_card() {
        let failure = probe_outcome(Ok(RunOutput {
            stdout: r#"{"_type":"video","duration":100,"formats":[{"format_id":"137",
                        "protocol":"https","vcodec":"avc1","acodec":"none",
                        "height":1080,"format_note":"1080p","filesize":1000}]}"#
                .to_string(),
            stderr: String::new(),
        }))
        .await
        .expect_err("К-1 требует название посимвольно — показывать нечего");

        assert!(matches!(failure, ProbeFailure::YtDlpFailure { .. }));
    }

    #[tokio::test]
    async fn a_card_survives_missing_channel_and_thumbnail() {
        // Обратная сторона того же правила: необязательные поля отсутствуют
        // — карточка остаётся полезной (контракт, дизайн E2).
        let card = probe_outcome(Ok(RunOutput {
            stdout: r#"{"_type":"video","title":"Без канала и превью","duration":61.4,
                        "formats":[{"format_id":"137","protocol":"https","vcodec":"avc1",
                        "acodec":"none","height":1080,"format_note":"1080p",
                        "filesize":1000}]}"#
                .to_string(),
            stderr: String::new(),
        }))
        .await
        .expect("карточка без канала и превью — штатный успех");

        assert_eq!(card.channel, None);
        assert_eq!(card.thumbnail_url, None);
        assert_eq!(card.duration_secs, 61, "дробная длительность округляется");
    }

    #[tokio::test]
    async fn a_yt_dlp_that_could_not_be_launched_at_all_is_a_yt_dlp_failure() {
        // Практически недостижимо (фронтенд не даёт разбирать, пока yt-dlp
        // не `ok`), но молчаливого зависания здесь быть не должно.
        for outcome in [
            Err(SidecarError::NotFound),
            Err(SidecarError::LaunchFailed {
                reason: crate::types::LaunchFailedReason::PermissionDenied,
                stderr: String::new(),
            }),
        ] {
            let failure = probe_outcome(outcome)
                .await
                .expect_err("запускать нечего — карточки нет");
            assert!(matches!(failure, ProbeFailure::YtDlpFailure { .. }));
        }
    }

    #[tokio::test]
    async fn a_failing_run_keeps_the_exit_code_and_the_stderr_tail_for_the_details() {
        let failure = probe_outcome(Err(SidecarError::NonZeroExit {
            code: 1,
            stderr: "ERROR: [youtube] xxxxxxxxxxx: Video unavailable".to_string(),
        }))
        .await
        .expect_err("ненулевой код — отказ");

        let contract = failure.to_contract();
        let details = contract
            .details
            .expect("технические детали для «Подробнее»");
        assert_eq!(details.exit_code, Some(1));
        assert!(details
            .stderr_tail
            .is_some_and(|tail| tail.contains("Video unavailable")));
    }

    // ───────────────────────────── хэлперы ─────────────────────────────

    fn size_of(item: &QualityItem) -> u64 {
        match item.size {
            QualitySize::Known { bytes } => bytes,
            QualitySize::Unknown => panic!(
                "К-1 требует оценку размера у каждой строки; у пункта {:?} её нет",
                item.kind
            ),
        }
    }

    /// Ждёт условия, уступая исполнение другим задачам.
    ///
    /// Настоящего ожидания времени здесь нет: обе задачи живут в одном
    /// однопоточном рантайме теста, и `yield_now` достаточно, чтобы дать
    /// сопернику дойти до своей точки `await`.
    async fn wait_for(mut condition: impl FnMut() -> bool) {
        for _ in 0..1_000 {
            if condition() {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("условие не наступило за 1000 переключений задач");
    }

    /// Версия yt-dlp из пина `binaries.lock.json` — та, что реально
    /// вкладывается в приложение.
    fn pinned_yt_dlp_version() -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("binaries.lock.json");
        let raw = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("пин {} не читается: {err}", path.display()));
        let pin: Value = serde_json::from_str(&raw)
            .unwrap_or_else(|err| panic!("пин {} — не JSON: {err}", path.display()));

        pin.get("ytDlp")
            .and_then(|yt_dlp| yt_dlp.get("version"))
            .and_then(Value::as_str)
            .expect("в пине объявлена версия yt-dlp")
            .to_owned()
    }
}
