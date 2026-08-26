//! Политика повторов и сторож продвижения (Ф-5, С-6…С-8) — TL-43.
//!
//! Модуль чистый и без часов внутри: [`RetryPolicy`] принимает момент
//! аргументом и потому проверяется без единого настоящего ожидания —
//! приём TL-12 (`ytdlp::prepare::repair_exhausted`). Часы читает
//! оркестрация (TL-44) одной функцией [`crate::clock::monotonic_now`];
//! почему монотонные, а не настенные, написано там же.
//!
//! Здесь принимается **решение**, а не выполняется действие: убить
//! процесс, подождать паузу, запустить следующую попытку и эмитить
//! события — дело оркестрации. Политика отвечает на три вопроса: было ли
//! продвижение, замерла ли попытка и что делать с неудавшейся.
//!
//! # Сторож «нет продвижения» — таймер, а не счётчик строк
//!
//! Это находка предыдущей задачи (TL-41), и она подтверждена ещё раз уже
//! на снятом исходе TL-43. Замерший поток ведёт себя двумя способами
//! сразу, и оба обманывают счётчик строк вывода:
//!
//! - **молчит** — строка прогресса печатается на принятых данных, и при
//!   их отсутствии не выходит ни одна;
//! - **печатает, ничего не принимая** — в фикстуре
//!   `stalled-killed-by-watchdog` после последнего принятого байта стоят
//!   две строки «Read timed out … Retrying», то есть вывод есть, а
//!   загрузки нет.
//!
//! Поэтому отсчёт — по времени с последнего **принятого байта**
//! ([`RetryPolicy::stall_deadline`]), и опрашивать его в цикле не нужно:
//! оркестрация вооружает таймер на возвращённый момент и перевооружает
//! его после каждого продвижения.
//!
//! # Почему на входе сумма байт задачи, а не строка прогресса
//!
//! Декомпозиция эпика описывает политику как функцию от
//! последовательности [`crate::download::progress::ProgressSample`]. На
//! входе здесь всё же число — сумма принятых байт задачи
//! ([`crate::download::aggregate::ProgressAggregator::received_bytes`]),
//! и это не упрощение, а обход ловушки.
//!
//! У задачи с двумя потоками `downloaded_bytes` каждой строки — счётчик
//! **своего** потока. Видео заканчивается на 18 294 110 байтах, следом
//! начинается звук и первой же строкой сообщает 1024. Отметка «максимум,
//! достигнутый задачей», снятая с сырых строк, после такого перехода
//! перестаёт расти на **всё** время скачивания звука: на снятой живьём
//! фикстуре это 16 строк подряд — весь звуковой поток целиком (15 строк)
//! плюс `finished` видео перед ним, то есть половина вывода загрузки.
//!
//! Сколько это секунд, зависит от длины ролика, и здесь пределы честности
//! снятых данных: ролик фикстуры короткий, его звук укладывается в порог,
//! и **сработавшим** сторож на ней не увидеть. Но серия покрывает поток
//! целиком, а не его кусок, — значит на ролике, чей звук качается дольше
//! двадцати секунд (то есть на любом сколько-нибудь длинном), наивный
//! сторож объявит зависшей совершенно здоровую загрузку. Тест
//! `a_healthy_switch_between_streams_is_not_a_stall` показывает и то и
//! другое: механизм — на снятых байтах и снятой частоте строк,
//! срабатывание — на тех же байтах с растянутым шагом, и растяжение в нём
//! оговорено.
//!
//! Сумма по потокам растёт монотонно, и её уже считает агрегатор;
//! заводить второе место, где та же величина выводится иначе, — верный
//! способ их разойтись.
//!
//! # Числа
//!
//! Все четыре — из таблицы дизайна E3, и все четыре объявлены здесь
//! именованными константами, а не литералами по месту: калибровка на
//! собранном bundle (К-6/К-7) должна править одну строку, а не искать
//! число по коду. Что уже удалось откалибровать замером, а что осталось
//! стартовой точкой, написано у каждой константы отдельно.

use std::time::{Duration, Instant};

use crate::types::DownloadAttempt;

/// Сколько попыток скачать поток бывает подряд **без единого байта
/// продвижения между ними**: первая плюс семь повторов.
///
/// Дизайн E3 назвал стартовой точкой шесть и потребовал калибровки на
/// собранном bundle (К-6/К-7). Калибровка выполнена в TL-44, и число
/// изменено по её итогу — ниже замер, который этого потребовал.
///
/// # Замер (собранный `.app` со смонтированного образа, macOS 15,
/// 2026-08-27)
///
/// Сценарий К-6 буквально: скачивание 1080p, на 20-й секунде Wi-Fi
/// выключается (`networksetup -setairportpower en0 off`), через 45 секунд
/// включается обратно.
///
/// | Момент | Что произошло |
/// |---|---|
/// | 20 с | Wi-Fi выключен, попытка 1 обрывается |
/// | +5 с | попытка 2 — отказ |
/// | +10 с | попытка 3 — отказ |
/// | +20 с | попытка 4 — отказ |
/// | 65 с | Wi-Fi включён |
/// | +40 с | попытка 5 — **всё ещё отказ** |
/// | +60 с | попытка 6 — успех, докачка с байта 3 846 314 |
///
/// Два вывода, и оба неочевидны заранее.
///
/// **Наблюдаемый обрыв длиннее выключения.** Питание интерфейса вернулось
/// на 65-й секунде, а первая успешная попытка случилась только на ~155-й:
/// между ними ассоциация с точкой доступа, DHCP и DNS. Сорокапятисекундное
/// выключение обошлось приложению примерно в две минуты недоступной сети —
/// то есть считать бюджет по длине самого обрыва нельзя.
///
/// **Шести попыток на это хватило впритык.** Попытка без сети отказывает
/// за секунду-две, поэтому бюджет уходит почти целиком на лестницу пауз, а
/// она при шести попытках даёт 5 + 10 + 20 + 40 + 60 = 135 с. Замер
/// израсходовал пять попыток из шести: ещё один неудачный шаг — и
/// пользователь получил бы «Соединение потеряно» там, где сеть уже
/// вернулась. Запаса не было вовсе.
///
/// Отсюда восемь: лестница становится 5 + 10 + 20 + 40 + 60 + 60 + 60 =
/// 255 с, то есть почти вдвое перекрывает измеренную потребность и
/// покрывает перезагрузку роутера. Больше брать незачем: ожидание и так
/// объяснено — на панели виден номер попытки и обратный отсчёт, а
/// «Отменить» отвечает мгновенно (замер: 12 мс от команды до
/// терминального события), но задача, которой сеть не вернётся, обязана
/// честно сдаться, а не ждать вечно.
///
/// Пользовательской настройкой число станет в E5; здесь оно одно на всех.
pub const MAX_ATTEMPTS: u32 = 8;

/// Пауза перед первым повтором; дальше удваивается.
///
/// Стартовая точка дизайна E3.
pub const FIRST_RETRY_DELAY: Duration = Duration::from_secs(5);

/// Потолок паузы между повторами.
///
/// Стартовая точка дизайна E3.
pub const MAX_RETRY_DELAY: Duration = Duration::from_secs(60);

/// Сколько времени без роста принятых байт означает, что попытка не
/// удалась (С-8).
///
/// Стартовая точка дизайна E3 — **проверена замером и оставлена как
/// есть**, но с условием к запуску (см. [`SOCKET_TIMEOUT_SECS`]).
///
/// Замер (macOS 15, 2026-08-26, пин 2026.08.19, транспорт замолчал, не
/// закрыв сокет):
///
/// | Что | Когда |
/// |---|---|
/// | последний принятый байт | 0 с |
/// | первая строка «Read timed out» yt-dlp, повтор внутри процесса | +20,3 с |
/// | вторая, третья, … | каждые +20,2 с |
/// | yt-dlp сдался бы сам | ≈ +200 с |
///
/// Отсюда два вывода. Первый: **без сторожа пользователь смотрел бы на
/// замерший процент больше трёх минут** — С-8 не теоретический сценарий.
/// Второй: с таймаутом сокета по умолчанию (20 с) сторог сработал бы
/// ровно в момент первой внутренней попытки yt-dlp переподключиться, то
/// есть исход зависел бы от планировщика. Порог оставлен двадцатью
/// секундами, а развязано это со стороны запуска.
pub const NO_PROGRESS_TIMEOUT: Duration = Duration::from_secs(20);

/// Значение `--socket-timeout`, с которым запуск обязан звать yt-dlp
/// (требование к TL-44, как `--progress-template` в
/// [`crate::download::progress`]).
///
/// Не украшение и не вкус: это число разводит **свой** сторож и
/// **внутренний** механизм повторов yt-dlp по времени, вместо того чтобы
/// заставлять их сработать одновременно.
///
/// # Решение владельца ожидается
///
/// Число меняет **сетевое поведение** приложения, а инвариант проекта
/// требует вести себя как обычный пользователь: вдвое более короткий
/// таймаут — вдвое более частые переподключения на медленной связи.
/// Вопрос вынесен владельцу; до ответа значение остаётся здесь, и
/// вернуть умолчание (20 с) — правка одной строки. Что при этом
/// изменится, названо ниже: сторож и внутренние повторы yt-dlp снова
/// сойдутся в одну точку, и какой из них сработает первым, будет решать
/// планировщик.
///
/// Замерено на том же замолчавшем транспорте: с `--socket-timeout 10`
/// yt-dlp обнаруживает тишину на 10-й секунде и переподключается сам, а
/// на 20-й, если это не помогло, попытку забирает сторож
/// ([`NO_PROGRESS_TIMEOUT`]). То есть у процесса есть ровно одна дешёвая
/// попытка исправить положение без перезапуска — а перезапуск стоит
/// повторного разбора адреса (2–3,5 с по тем же замерам) плюс паузы.
/// С таймаутом по умолчанию (20 с) эта попытка либо не случалась бы
/// никогда, либо случалась через раз.
pub const SOCKET_TIMEOUT_SECS: u64 = 10;

/// Что делать с попыткой, которая не удалась.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryDecision {
    /// Подождать `delay` и начать попытку `attempt`.
    ///
    /// `attempt` — та, что начнётся **после** паузы: именно её номер
    /// показывает экран паузы («Ждём повторной попытки (2 из 8)»), и
    /// именно так его понимает [`DownloadAttempt`].
    Retry {
        attempt: DownloadAttempt,
        delay: Duration,
    },
    /// Попытки исчерпаны: задача падает классом
    /// [`crate::download::DownloadFailure::ConnectionLost`] с этим
    /// числом попыток.
    ///
    /// **Технических деталей решение не несёт, и это обязанность
    /// вызывающего.** `ConnectionLost` требует ещё и
    /// [`crate::types::DownloadErrorDetails`] — хвост stderr и код
    /// завершения, — а знает их не политика, а классификация: они
    /// приезжают в
    /// [`crate::download::classify::AttemptVerdict::Interrupted`] на
    /// каждой неудачной попытке. Оркестрации (TL-44) держать последние
    /// такие детали при себе и подставить их сюда; иначе в «Подробнее» у
    /// исчерпания попыток не будет ничего, кроме слова «попытки
    /// закончились», — а это ровно тот класс состояний без содержания,
    /// который правило Н-4 и запрещает.
    GiveUp { attempts: u32 },
}

/// Счётчик попыток и сторож продвижения одной задачи скачивания.
///
/// Живёт столько же, сколько задача, и переживает её повторы — как и
/// агрегатор прогресса рядом.
///
/// # Как этим пользуется оркестрация (TL-44)
///
/// ```ignore
/// let mut policy = RetryPolicy::new(clock::monotonic_now());
/// loop {
///     policy.attempt_started(clock::monotonic_now());
///     // …запустить yt-dlp; на каждую применённую строку прогресса:
///     policy.observe(aggregator.received_bytes(), clock::monotonic_now());
///     // …и параллельно — таймер, а не опрос:
///     //     stall_deadline() отдаёт момент, только пока попытка идёт;
///     //     таймер перевооружается после каждого продвижения.
///     match policy.attempt_failed() {
///         RetryDecision::Retry { attempt, delay } => { /* пауза, событие */ }
///         RetryDecision::GiveUp { attempts } => { /* connectionLost */ }
///     }
/// }
/// ```
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Номер идущей попытки, считая с единицы.
    attempt: u32,
    /// Наибольшее число принятых байт, которое задача видела за всю свою
    /// жизнь. Отметка, по которой различается «продвинулись» и
    /// «пересказали уже принятое» — докачка сообщает абсолютные байты
    /// (проверено живьём в TL-41), поэтому продолженная попытка
    /// перебивает отметку с первой же строки.
    high_water_bytes: u64,
    /// Момент последнего продвижения либо начала попытки — от него
    /// отсчитывается сторож С-8.
    last_progress_at: Instant,
    /// Идёт ли сейчас попытка.
    ///
    /// Сторожу продвижения нечего сторожить, пока процесса нет: между
    /// неудачей и следующим запуском выдерживается пауза до минуты, и
    /// таймер, вооружённый в этом промежутке, выстрелил бы **внутри
    /// паузы**. Поэтому состояние живёт в поле, а
    /// [`RetryPolicy::stall_deadline`] в этом промежутке не отдаёт
    /// момента вовсе — ошибиться нечем.
    attempt_running: bool,
}

impl RetryPolicy {
    /// Политика в начале задачи: идёт первая попытка, принято ноль байт.
    pub fn new(now: Instant) -> Self {
        Self {
            attempt: 1,
            high_water_bytes: 0,
            last_progress_at: now,
            attempt_running: false,
        }
    }

    /// Попытка, о которой идёт речь сейчас, и предел — «попытка 2 из 8».
    ///
    /// Снимать номер первой попытки (контракт требует не рисовать
    /// «попытка 1 из 8») — не дело политики: это делает
    /// [`crate::download::aggregate::ProgressAggregator::running`], один
    /// раз и на границе события.
    pub fn attempt(&self) -> DownloadAttempt {
        DownloadAttempt {
            number: self.attempt,
            total: MAX_ATTEMPTS,
        }
    }

    /// Начинается новая попытка: сторож продвижения отсчитывается заново.
    ///
    /// Без этого свежая попытка унаследовала бы отсчёт от последнего
    /// байта предыдущей — то есть родилась бы уже просроченной: до неё
    /// прошли и двадцать секунд тишины, и пауза перед повтором.
    pub fn attempt_started(&mut self, now: Instant) {
        self.last_progress_at = now;
        self.attempt_running = true;
    }

    /// Учесть суммарное число принятых байт задачи.
    ///
    /// Возвращает `true`, если это продвижение, — тогда счётчик попыток
    /// обнулён и сторож перезаведён. Вызывать нужно на каждую строку
    /// прогресса, отнесённую агрегатором к потоку задачи.
    ///
    /// **Счётчик попыток сбрасывает любое продвижение** (С-6), и это
    /// главное свойство политики. Иначе лимит наказывал бы за длину
    /// ролика, а не за качество связи: на часовой загрузке шесть редких
    /// обрывов подряд неизбежны, и задача падала бы там, где связь просто
    /// неидеальна.
    pub fn observe(&mut self, received_bytes: u64, now: Instant) -> bool {
        if received_bytes <= self.high_water_bytes {
            return false;
        }

        self.high_water_bytes = received_bytes;
        self.last_progress_at = now;
        self.attempt = 1;
        true
    }

    /// Момент, в который попытка считается зависшей, если до него не
    /// придёт ни одного байта (С-8).
    ///
    /// Отдаётся моментом, а не флагом, намеренно: по нему вооружается
    /// таймер. Опрос в цикле здесь работал бы, но требовал бы того, чего
    /// в этой фазе как раз и нет, — событий, на которых опрашивать.
    pub fn stall_deadline(&self) -> Option<Instant> {
        self.attempt_running
            .then(|| self.last_progress_at + NO_PROGRESS_TIMEOUT)
    }

    /// Замерла ли попытка к моменту `now`.
    ///
    /// Для тех мест, где момент уже под рукой и заводить таймер незачем.
    pub fn is_stalled(&self, now: Instant) -> bool {
        self.stall_deadline()
            .is_some_and(|deadline| now >= deadline)
    }

    /// Попытка не удалась — обрывом, зависанием или отказом процесса, по
    /// поводу которого повтор осмыслен.
    ///
    /// Вызывать **только** для [`crate::download::classify::AttemptVerdict::Interrupted`]
    /// и для сработавшего сторожа: класс отказа, который повтором не
    /// чинится, до политики не доходит вовсе.
    ///
    /// Момента здесь нет в аргументах, и это не упущение: решение
    /// «повторять или сдаться» зависит только от числа неудач подряд, а
    /// отсчёт сторожа заводится от **старта** следующей попытки
    /// ([`RetryPolicy::attempt_started`]), а не от неудачи предыдущей —
    /// иначе пауза до минуты съедала бы всё окно сторожа ещё до того,
    /// как процесс успеет запуститься.
    pub fn attempt_failed(&mut self) -> RetryDecision {
        // Попытки больше нет: сторожу нечего сторожить до тех пор, пока
        // `attempt_started` не объявит следующую.
        self.attempt_running = false;

        if self.attempt >= MAX_ATTEMPTS {
            return RetryDecision::GiveUp {
                attempts: self.attempt,
            };
        }

        let delay = delay_after(self.attempt);
        self.attempt += 1;
        RetryDecision::Retry {
            attempt: self.attempt(),
            delay,
        }
    }
}

/// Пауза после `failures`-й подряд неудачной попытки: 5 → 10 → 20 → 40 →
/// 60 (и дальше 60, если предел попыток когда-нибудь вырастет).
///
/// Растёт удвоением от [`FIRST_RETRY_DELAY`] с потолком
/// [`MAX_RETRY_DELAY`] — лестница из таблицы дизайна E3 получается
/// вычислением, а не выписыванием: выписанная, она разошлась бы с
/// константами при первой же калибровке.
fn delay_after(failures: u32) -> Duration {
    let doubled = FIRST_RETRY_DELAY
        .as_secs()
        .checked_shl(failures.saturating_sub(1))
        .unwrap_or(u64::MAX);

    Duration::from_secs(doubled.min(MAX_RETRY_DELAY.as_secs()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::download::aggregate::{ProgressAggregator, SampleOutcome};
    use crate::download::fixtures;
    use crate::download::progress::{parse_line, ProgressSample, StdoutLine};
    use crate::types::{QualitySize, QualityStreams};

    /// Момент «начала времён» теста: фиксированная точка, от которой
    /// отмеряются секунды. Настоящих ожиданий в тестах нет ни одного.
    fn t0() -> Instant {
        Instant::now()
    }

    fn at(base: Instant, secs: u64) -> Instant {
        base + Duration::from_secs(secs)
    }

    #[test]
    fn the_delay_ladder_matches_the_design_table() {
        // 5 → 10 → 20 → 40 → 60 — ровно пять пауз, потому что попыток
        // шесть. Шестая пауза не наступает никогда, но потолок обязан
        // держать и её.
        let ladder: Vec<u64> = (1..=6).map(|n| delay_after(n).as_secs()).collect();

        assert_eq!(ladder, vec![5, 10, 20, 40, 60, 60]);
    }

    #[test]
    fn failures_in_a_row_exhaust_the_attempts_and_no_more() {
        let base = t0();
        let mut policy = RetryPolicy::new(base);

        // Лестница выводится из констант, а не переписывается числом:
        // калибровка на bundle правит `MAX_ATTEMPTS` (TL-44 подняла его с
        // шести до восьми), и тест, знающий предел наизусть, краснел бы на
        // каждой такой правке, ничего при этом не проверяя.
        let expected: Vec<u64> = (1..MAX_ATTEMPTS)
            .map(|step| {
                let doubled = FIRST_RETRY_DELAY.as_secs() << (step - 1);
                doubled.min(MAX_RETRY_DELAY.as_secs())
            })
            .collect();

        let mut delays = Vec::new();
        for step in 1..MAX_ATTEMPTS {
            match policy.attempt_failed() {
                RetryDecision::Retry { attempt, delay } => {
                    assert_eq!(attempt.total, MAX_ATTEMPTS);
                    assert_eq!(attempt.number, step + 1);
                    delays.push(delay.as_secs());
                }
                RetryDecision::GiveUp { .. } => panic!("попытки кончились слишком рано"),
            }
        }

        assert_eq!(delays, expected);
        assert_eq!(
            delays.first(),
            Some(&5),
            "лестница начинается с пяти секунд"
        );
        assert_eq!(
            delays.last(),
            Some(&MAX_RETRY_DELAY.as_secs()),
            "и упирается в потолок"
        );
        assert_eq!(
            policy.attempt_failed(),
            RetryDecision::GiveUp {
                attempts: MAX_ATTEMPTS
            },
            "неудача номер MAX_ATTEMPTS — это исчерпание, а не попытка сверх предела"
        );
    }

    #[test]
    fn any_progress_resets_the_counter_of_attempts() {
        // С-6 буквально: час загрузки на нестабильной сети не исчерпывает
        // лимит суммированием редких обрывов. Здесь их девять — больше
        // предела, — и задача не падает ни разу.
        let base = t0();
        let mut policy = RetryPolicy::new(base);
        let mut received = 0u64;

        for round in 0..9 {
            let now = at(base, round * 600);
            received += 1_000_000;

            assert!(
                policy.observe(received, now),
                "круг {round}: байты выросли — это продвижение"
            );
            assert_eq!(
                policy.attempt().number,
                1,
                "круг {round}: продвижение обнуляет счётчик"
            );

            assert!(matches!(
                policy.attempt_failed(),
                RetryDecision::Retry { .. }
            ));
        }
    }

    #[test]
    fn a_reconnect_without_a_single_new_byte_is_not_progress() {
        // Обратная сторона того же: попытка поднялась, доложила уже
        // принятое и снова умерла. Продвижения не было — счётчик стоит.
        let base = t0();
        let mut policy = RetryPolicy::new(base);

        assert!(policy.observe(995_883, at(base, 10)));
        assert!(matches!(
            policy.attempt_failed(),
            RetryDecision::Retry { .. }
        ));
        assert_eq!(policy.attempt().number, 2);

        assert!(
            !policy.observe(995_883, at(base, 30)),
            "то же число байт — это пересказ, а не продвижение"
        );
        assert!(
            !policy.observe(1_000, at(base, 31)),
            "попытка, начавшая файл заново, тем более не продвижение"
        );
        assert_eq!(
            policy.attempt().number,
            2,
            "счётчик обнуляется ростом байт, а не фактом подключения"
        );
    }

    #[test]
    fn the_watchdog_counts_time_and_not_lines() {
        // С-8: порог отсчитывается от последнего принятого байта.
        let base = t0();
        let mut policy = RetryPolicy::new(base);
        policy.attempt_started(base);

        policy.observe(1024, at(base, 5));
        assert_eq!(policy.stall_deadline(), Some(at(base, 25)));

        assert!(!policy.is_stalled(at(base, 24)));
        assert!(
            policy.is_stalled(at(base, 25)),
            "ровно порог — уже зависание: секунда туда-сюда ничего не \
             решает, а строгое неравенство дало бы сторожу шанс не \
             сработать вовсе на грубых часах"
        );

        // Пришёл байт — отсчёт заводится заново, а не продолжается.
        policy.observe(2048, at(base, 24));
        assert_eq!(policy.stall_deadline(), Some(at(base, 44)));
        assert!(!policy.is_stalled(at(base, 43)));
    }

    #[test]
    fn the_watchdog_stays_silent_while_there_is_no_attempt_to_watch() {
        // Между неудачей и следующим запуском выдерживается пауза до
        // минуты. Таймер, вооружённый в этом промежутке от последнего
        // принятого байта, выстрелил бы **внутри паузы** — то есть
        // объявил бы зависшей попытку, которой ещё нет. Момента в этом
        // промежутке не выдаётся вовсе: вооружать нечем.
        let base = t0();
        let mut policy = RetryPolicy::new(base);
        policy.attempt_started(base);
        policy.observe(1024, at(base, 5));

        assert!(policy.stall_deadline().is_some(), "попытка идёт");
        policy.attempt_failed();

        assert_eq!(
            policy.stall_deadline(),
            None,
            "попытки нет — сторожить нечего"
        );
        assert!(
            !policy.is_stalled(at(base, 3600)),
            "и никакая пауза не превращается в зависание сама по себе"
        );

        policy.attempt_started(at(base, 65)); // после паузы в 60 с
        assert_eq!(policy.stall_deadline(), Some(at(base, 85)));
    }

    #[test]
    fn a_fresh_attempt_is_not_born_already_stalled() {
        // Между попытками проходит пауза, и до неё — двадцать секунд
        // тишины. Если бы сторож считал от последнего байта задачи,
        // новая попытка была бы объявлена зависшей, не успев ничего
        // принять.
        let base = t0();
        let mut policy = RetryPolicy::new(base);
        policy.attempt_started(base);

        policy.observe(1024, at(base, 5));
        assert!(policy.is_stalled(at(base, 25)));

        policy.attempt_failed();
        policy.attempt_started(at(base, 30)); // после паузы в 5 с

        assert!(!policy.is_stalled(at(base, 49)));
        assert!(policy.is_stalled(at(base, 50)));
    }

    #[test]
    fn a_healthy_switch_between_streams_is_not_a_stall() {
        // Ловушка, из-за которой вход политики — сумма байт задачи, а не
        // `downloaded_bytes` строки. Данные настоящие: снятый живьём
        // вывод загрузки `133+139`, где видео кончается на 18 МБ, а звук
        // начинается с килобайта.
        let samples = progress_samples("video-and-audio.json");
        let mut aggregator = video_and_audio_aggregator();

        let base = t0();
        let mut policy = RetryPolicy::new(base);
        policy.attempt_started(base);

        let mut naive_high_water = 0u64;
        let mut naive_run = 0u32;
        let mut longest_naive_run = 0u32;
        let mut policy_still = 0;

        for (index, sample) in samples.iter().enumerate() {
            // Строки идут раз в секунду — вдвое реже, чем на живой
            // загрузке (замер TL-41: медиана 0,18–0,46 с), и всё равно
            // втрое чаще порога сторожа.
            let now = at(base, index as u64 + 1);

            // Как считала бы наивная политика — по сырым байтам строки.
            if sample.downloaded_bytes > naive_high_water {
                naive_high_water = sample.downloaded_bytes;
                naive_run = 0;
            } else {
                naive_run += 1;
                longest_naive_run = longest_naive_run.max(naive_run);
            }

            assert!(matches!(
                aggregator.apply(sample),
                SampleOutcome::Applied { .. }
            ));
            if !policy.observe(aggregator.received_bytes(), now) {
                policy_still += 1;
            }
            assert!(
                !policy.is_stalled(now),
                "здоровая загрузка не должна выглядеть зависшей ни на одной \
                 строке (строка {index})"
            );
        }

        // Сколько именно строк держится наивная отметка — не «больше
        // десяти», а ровно весь второй поток: 16 строк из 33. Число
        // закреплено, потому что на нём стоит вывод «серия покрывает
        // поток целиком, а не его кусок».
        let audio_lines = samples
            .iter()
            .filter(|sample| sample.format_id == "139")
            .count();
        assert_eq!(
            longest_naive_run as usize,
            audio_lines + 1,
            "серия начинается на одну строку раньше звука — на `finished` \
             видео, где yt-dlp повторяет тот же счётчик, — и дальше идёт \
             весь звуковой поток целиком, без единого просвета"
        );
        assert_eq!(
            (longest_naive_run, samples.len()),
            (16, 33),
            "фикстура изменилась — пересчитайте числа в шапке модуля, они \
             сняты с неё"
        );

        assert_eq!(
            policy_still, 2,
            "по сумме потоков не растут ровно две строки — по одной \
             `finished` на поток: yt-dlp повторяет в ней тот же счётчик, \
             что в последней `downloading`. Это не зависание, а конец \
             потока, и сторож переживает его с запасом"
        );
    }

    #[test]
    fn the_naive_watchdog_fires_on_a_stream_that_takes_longer_than_the_threshold() {
        // Вторая половина того же: на коротком ролике фикстуры звук
        // укладывается в порог, и **сработавшим** наивный сторож на ней
        // не увидеть. Байты здесь снятые, а шаг растянут до двух секунд
        // на строку — растяжение и есть допущение теста, и оно тут
        // единственное. Оно же и есть обычная жизнь: звук ролика на час
        // качается минутами, и наивная отметка стоит всё это время.
        let samples = progress_samples("video-and-audio.json");
        let mut aggregator = video_and_audio_aggregator();

        let base = t0();
        let mut policy = RetryPolicy::new(base);
        policy.attempt_started(base);

        let mut naive = RetryPolicy::new(base);
        naive.attempt_started(base);

        let mut naive_stalled_at = None;
        for (index, sample) in samples.iter().enumerate() {
            let now = at(base, (index as u64 + 1) * 2);

            aggregator.apply(sample);
            policy.observe(aggregator.received_bytes(), now);
            naive.observe(sample.downloaded_bytes, now);

            if naive.is_stalled(now) && naive_stalled_at.is_none() {
                naive_stalled_at = Some(index);
            }
            assert!(
                !policy.is_stalled(now),
                "сумма по потокам растёт — зависания нет (строка {index})"
            );
        }

        let stalled_at = naive_stalled_at.expect(
            "наивный сторож обязан сработать: на растянутом шаге звук \
             качается дольше порога",
        );
        assert_eq!(
            samples[stalled_at].format_id, "139",
            "срабатывает он ровно там, где ловушка и живёт, — на втором \
             потоке, а не на дырке в выводе"
        );
    }

    #[test]
    fn a_resumed_attempt_counts_from_where_it_stopped() {
        // Докачка с места (Ф-5) глазами политики: вторая попытка первой
        // же строкой перебивает отметку первой — значит счётчик попыток
        // обнуляется сразу, а не после того, как заново наберётся
        // потерянное. Обе фикстуры сняты живьём одна за другой.
        let base = t0();
        let mut policy = RetryPolicy::new(base);

        let interrupted = last_downloaded_bytes("resume-interrupted.json");
        for (index, bytes) in downloaded_bytes("resume-interrupted.json")
            .into_iter()
            .enumerate()
        {
            policy.observe(bytes, at(base, index as u64));
        }
        assert_eq!(policy.attempt().number, 1);

        // Обрыв, повтор, пауза — счётчик пошёл вверх.
        assert!(matches!(
            policy.attempt_failed(),
            RetryDecision::Retry { .. }
        ));
        assert_eq!(policy.attempt().number, 2);
        policy.attempt_started(at(base, 105));

        let continued = downloaded_bytes("resume-continued.json");
        let first = continued[0];
        assert!(
            first > interrupted,
            "докачка сообщает абсолютные байты: {first} должно быть больше \
             {interrupted}, иначе фикстура перестала быть докачкой"
        );

        assert!(policy.observe(first, at(base, 106)));
        assert_eq!(
            policy.attempt().number,
            1,
            "первый же принятый байт возвращает задачу на нормальный путь"
        );
    }

    #[test]
    fn the_socket_timeout_of_the_constant_is_the_one_the_fixtures_were_shot_with() {
        // Симметрия с шаблоном вывода прогресса: тот сверяется с
        // фикстурами посимвольно, а таймаут сокета до сих пор не
        // сверялся ничем — и требование к запуску (TL-44) тихо
        // разошлось бы с числом, на котором стоят замеры.
        //
        // Сверять есть с чем: обе смоделированные фикстуры сняты
        // запуском с этим аргументом, и обе показывают именно то
        // поведение, ради которого он выбран.
        let expected = SOCKET_TIMEOUT_SECS.to_string();

        for name in [
            "connection-lost-mid-download.json",
            "stalled-killed-by-watchdog.json",
        ] {
            let argv = fixtures::outcome(name).capture.argv;
            let at = argv
                .iter()
                .position(|arg| arg == "--socket-timeout")
                .unwrap_or_else(|| {
                    panic!("{name}: фикстура снята без --socket-timeout, а замеры сделаны с ним")
                });

            assert_eq!(
                argv.get(at + 1).map(String::as_str),
                Some(expected.as_str()),
                "{name}: фикстура снята с другим таймаутом сокета. Число \
                 сменили — переснимите фикстуры обрыва и перепроверьте \
                 замеры в doc SOCKET_TIMEOUT_SECS, а не правьте эту строку"
            );
        }
    }

    #[test]
    fn the_socket_timeout_leaves_the_process_exactly_one_recovery_attempt() {
        // Связь двух чисел, ради которой они стоят рядом: у yt-dlp
        // должна быть ровно одна попытка переподключиться внутри окна
        // сторожа. Ноль означал бы перезапуск процесса на каждой
        // секундной ряби, две — что сторож простаивает.
        let window = NO_PROGRESS_TIMEOUT.as_secs();

        assert!(
            SOCKET_TIMEOUT_SECS < window,
            "с таймаутом сокета не меньше порога yt-dlp не успел бы \
             попробовать ни разу"
        );
        assert!(
            SOCKET_TIMEOUT_SECS * 2 >= window,
            "двух попыток в окне быть не должно: вторая уже не дешевле \
             честного повтора с паузой"
        );
    }

    /// Разобранные строки прогресса фикстуры — как есть, по порядку.
    fn progress_samples(fixture: &str) -> Vec<ProgressSample> {
        fixtures::stdout(fixture)
            .lines()
            .filter_map(|line| match parse_line(line) {
                StdoutLine::Progress(sample) => Some(sample),
                _ => None,
            })
            .collect()
    }

    fn video_and_audio_aggregator() -> ProgressAggregator {
        ProgressAggregator::new(
            &QualityStreams {
                video_format_id: Some("133".to_string()),
                audio_format_id: Some("139".to_string()),
            },
            QualitySize::Unknown,
        )
        .expect("два потока — агрегатор строится")
    }

    fn downloaded_bytes(fixture: &str) -> Vec<u64> {
        fixtures::stdout(fixture)
            .lines()
            .filter_map(|line| match parse_line(line) {
                StdoutLine::Progress(sample) => Some(sample.downloaded_bytes),
                _ => None,
            })
            .collect()
    }

    fn last_downloaded_bytes(fixture: &str) -> u64 {
        *downloaded_bytes(fixture)
            .last()
            .expect("в фикстуре есть строки прогресса")
    }
}
