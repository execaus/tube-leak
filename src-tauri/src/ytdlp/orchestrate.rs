//! Оркестрация контура самообновления yt-dlp (TL-58): расписание,
//! конвейер, граница задач, переключение, откат и реакция на сломанное
//! извлечение (Ф-7, решения владельца Р-1, Р-2, Р-3 эпика E6).
//!
//! # Что здесь сходится
//!
//! Сходящийся узел (тот же приём, что `probe::orchestrate` в E2 и
//! `download::orchestrate` в E3): пять готовых кусков контура — проверка
//! по метаданным ([`super::release`]), приём и установка архива
//! ([`super::fetch`]), smoke-проверка ([`super::smoke`]), запись об
//! активной установке и уборка ([`super::state`]) и контракт границы
//! ([`crate::types`]) — соединяются здесь в один конвейер и в три
//! команды. Ни один из пяти не знает ни о расписании, ни о том, идёт ли
//! сейчас загрузка ролика: это знание живёт только здесь.
//!
//! # Конвейер целиком
//!
//! ```text
//! проверка метаданных                   ← один запрос, если новее нет
//!   └ есть Y ─ запреты (С-4, С-5)       ← до сети, а не после
//!       └ приём архива + sha256 + распаковка (TL-56)
//!           └ пауза между задачами (Н-3) ← прогрев не спорит с загрузкой
//!               └ smoke-проверка (TL-57)
//!                   └ пауза между задачами (Ф-7)
//!                       └ переключение активной записи (TL-54)
//!                           └ уборка (Ф-8)
//! ```
//!
//! Пауза между задачами спрашивается **дважды**, и это не описка.
//! Первая — перед smoke: она же первый запуск распакованного дерева,
//! то есть те самые 24–36 с дисковой работы, которые Н-3 запрещает
//! ставить в конкуренцию с активной загрузкой (замер TL-57). Вторая —
//! перед переключением: между smoke и переключением пользователь мог
//! начать новую задачу, а Ф-7 требует не менять версию посреди неё.
//!
//! # Чего в конвейере нет
//!
//! - **Вопросов пользователю.** Р-1: контур сам проверяет, сам ставит,
//!   подтверждения не спрашивает и уведомлением не прерывает. Всё, что
//!   он показывает, — одна строка состояния (Ф-10).
//! - **Автоматического отката.** Р-3: возврат на известно-хорошую — это
//!   [`UpdateController::begin_rollback`], то есть действие
//!   пользователя. Счётчика неудач, по которому контур откатился бы
//!   сам, здесь нет ни одного.
//! - **Понижения версии.** С-10: контур ставит только то, что новее уже
//!   имеющегося. Единственное исключение — тот же ручной откат.
//!
//! # Как удерживается С-8 («откатились — не ставим обратно сами»)
//!
//! Без единого нового поля на диске: точка сравнения — не активная
//! версия, а **самая новая из тех, что лежат на диске**
//! ([`comparison_base`]), то есть максимум из активной и
//! известно-хорошей. В обычной жизни это и есть активная (известно-
//! хорошая всегда старее, потому что ею становится вытесненная
//! активная). После отката они меняются местами, максимум остаётся на
//! отвергнутой версии — и она перестаёт быть «новее», то есть
//! обновлением. Следующий релиз апстрима больше их обеих и ставится как
//! обычно, ровно как обещает С-8.
//!
//! Отдельной памяти о том, «от чего откатились», не заводится
//! сознательно: она была бы вторым источником правды о том же факте, а
//! он уже записан раскладкой — обе установки лежат на диске, и Ф-8
//! обещает, что их не больше двух.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::sidecar::ChildRegistry;
use crate::types::{
    YtDlpUpdateCommandError, YtDlpUpdateCommandErrorKind, YtDlpUpdateFailure, YtDlpUpdatePercent,
    YtDlpUpdateSnapshot, YtDlpUpdateStatus,
};

use super::fetch::{self, ArchiveSource, FetchStage, PreparedCandidate};
use super::layout::{self, ArchiveIdentity, BuildId, Layout, RepairLog};
use super::release::{self, MetadataSource, ReleaseVersion};
use super::session::Session;
use super::smoke;
use super::state::{self, InUse, InstallEntry, InstallState};
use super::update::{UpdateAsset, UpdateCheck};

/// Через сколько после старта приложения контур впервые смотрит на
/// апстрим.
///
/// Не «сразу»: подготовка первого запуска (TL-12) в этот момент может
/// распаковывать и греть дерево — 24–36 с дисковой работы, и вклиниваться
/// в них проверкой, которая может кончиться скачиванием шестидесяти
/// мегабайт, значит соревноваться с самим собой за диск (Н-3).
///
/// Сохранность файлов задержка **не** обеспечивает: уборку остатков
/// `.staging-*` и `.download-*` в подготовке и прогон конвейера разводит
/// замок сеанса ([`Session::hold_root_for_update`], TL-66), а не время. До
/// TL-66 это была вторая причина задержки, и она не держала ни ручную
/// проверку, ни повтор подготовки посреди сеанса.
///
/// Две минуты — с запасом больше подготовки: она укладывается в 35 с даже
/// в худшем замере TL-12 (36,37 с), а обычный тёплый старт стоит доли
/// секунды.
pub const STARTUP_CHECK_DELAY: Duration = Duration::from_secs(2 * 60);

/// Как часто контур проверяет апстрим сам.
///
/// # Откуда шесть часов
///
/// Из бюджета запросов, а не из ощущения «часто/редко». Измерение
/// TL-55: анонимный доступ к API GitHub ограничен **60 запросами в час**
/// (`x-ratelimit-limit: 60`), токена у приложения нет и не будет.
/// Проверка «уже последняя» стоит один запрос, проверка, нашедшая
/// обновление, — два (метаданные плюс файл сумм).
///
/// Худший час одного экземпляра приложения: одна плановая проверка
/// (интервал шесть часов, но в час может попасть только одна) плюс две
/// внеплановых по С-13 ([`BROKEN_EXTRACTION_THROTTLE`] — 30 минут), то
/// есть три проверки, максимум шесть запросов. Даже если у пользователя
/// открыто несколько экземпляров приложения (#21), до шестидесяти
/// остаётся десятикратный запас.
///
/// Со стороны «не слишком ли редко»: апстрим выпускает релизы раз в
/// недели (136 релизов за пять с половиной лет по фикстуре TL-55), а
/// сломанное извлечение и не ждёт расписания — на него есть С-13.
/// Шесть часов означают, что новая версия доезжает до пользователя в
/// день выхода, и это на порядок быстрее, чем релиз приложения.
pub const PLANNED_CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// Пауза внеплановой проверки по сломанному извлечению (С-13).
///
/// Короче плановой — это правило дизайна, а не число: «общий счётчик
/// паузы между любой проверкой глушит именно тот случай, ради которого
/// С-13 существует». Тридцать минут — та величина, при которой
/// пользователь, поймавший поломку и жмущий «Повторить», получает
/// свежую проверку сразу, а десять неудачных задач подряд не
/// превращаются в десять запросов.
///
/// Соотношение с [`PLANNED_CHECK_INTERVAL`] сторожится компилятором
/// ниже: разъехавшись, эти два значения молча отменили бы решение
/// дизайна.
pub const BROKEN_EXTRACTION_THROTTLE: Duration = Duration::from_secs(30 * 60);

// Сторож правила дизайна «пауза С-13 короче паузы планового
// расписания». Число дизайн не назначал, соотношение — назначил.
const _: () = assert!(BROKEN_EXTRACTION_THROTTLE.as_secs() < PLANNED_CHECK_INTERVAL.as_secs());

/// Сколько раз подряд контур пробует установить один и тот же выпуск,
/// прежде чем отложить его надолго (С-4: «тот же ассет не
/// перекачивается в бесконечном цикле»).
///
/// Три, а не один: отказы приёма бывают преходящими — оборванное
/// соединение на середине шестидесяти мегабайт даёт ровно тот же класс,
/// что и битый архив, и объявлять выпуск негодным с первого раза
/// значило бы пропускать релизы из-за плохого Wi-Fi. Больше трёх нет
/// смысла: три подряд отказа — это уже не совпадение.
const MAX_FETCH_ATTEMPTS: u32 = 3;

/// Через сколько счётчик неудачных приёмов остывает.
///
/// Те же сутки, что у журнала починок (`super::prepare::REPAIR_COOLDOWN`),
/// и по той же причине: пользователь, освободивший диск или починивший
/// сеть, не должен ждать нового релиза апстрима, чтобы контур попробовал
/// снова. Второй выход из запрета — сам новый релиз: у него другой build
/// id и, значит, чистая история.
const FETCH_COOLDOWN: Duration = Duration::from_secs(24 * 60 * 60);

/// Куда уходит снимок состояния контура.
///
/// Абстракция ровно ради тестов — тот же приём, что
/// `super::prepare::ProgressSink`: конвейер нельзя проверить, поднимая
/// настоящее Tauri-приложение, а проверить его надо.
pub trait UpdateSink: Send + Sync {
    fn emit(&self, snapshot: YtDlpUpdateSnapshot);
}

/// Что контур знает о задачах скачивания ролика (E3).
///
/// Знает он ровно два факта, и оба — про границу задач: идёт ли сейчас
/// задача и как дождаться, когда не идёт. Ничего больше контуру не
/// нужно, и знать больше ему вредно: любое дополнительное знание о
/// задаче стало бы связью между двумя эпиками, которой сегодня нет.
pub trait TaskBoundary: Send + Sync {
    /// Идёт ли сейчас задача скачивания.
    fn is_busy(&self) -> bool;

    /// Ждёт паузы между задачами.
    fn wait(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>>;
}

/// Откуда контур берёт архивы.
///
/// Два источника, один путь установки (С-10): релизный ассет из сети и
/// вложенный в бандл ресурс различаются только тем, как открыть поток
/// байт, — дальше оба идут через [`fetch::fetch_and_install`].
/// `Send + Sync` у самих источников — не украшение: конвейер живёт
/// отдельной задачей рантайма, а между приёмом архива и переключением
/// стоят два ожидания (граница задач и smoke-проверка). Источник
/// переживает их, то есть уезжает через `await` вместе с будущим — и без
/// этих границ задача просто не спавнится.
pub trait ArchiveSupply: Send + Sync {
    /// Источник релизного ассета. Запрос делается при открытии потока, а
    /// не здесь.
    fn network<'a>(&'a self, asset: &UpdateAsset) -> Box<dyn ArchiveSource + Send + Sync + 'a>;

    /// Архив, вложенный в бандл. `None` — ресурса нет (в собранном
    /// приложении не бывает, но резолв ресурса — операция, которая
    /// умеет отказывать).
    fn bundled(&self) -> Option<Box<dyn ArchiveSource + Send + Sync + '_>>;
}

/// Окружение одного прогона конвейера.
///
/// Структура заимствований, а не трейт «всё сразу»: каждое поле — свой
/// шов со своим смыслом, и тест подставляет ровно те из них, которые
/// проверяет. Собирает её граница (`crate::commands::update`), потому
/// что только там есть `AppHandle`, каталог данных и состояние
/// приложения.
pub struct Pipeline<'a> {
    layout: &'a Layout,
    metadata: &'a (dyn MetadataSource + Send + Sync),
    archives: &'a dyn ArchiveSupply,
    registry: &'a ChildRegistry,
    in_use: &'a InUse,
    /// Память подготовки на время сеанса (TL-23): контур сбрасывает её,
    /// когда меняет корень установок, иначе служебный экран показал бы
    /// версию, которой приложение уже не работает.
    session: &'a Session,
    sink: &'a dyn UpdateSink,
    boundary: &'a dyn TaskBoundary,
}

/// Что заставило контур посмотреть на апстрим.
///
/// Различие нужно ровно в одном месте — в троттлинге, и различие там
/// принципиальное (решение владельца): у внеплановой проверки по
/// сломанному извлечению **свой** счётчик, а не общий с плановой.
/// Общий счётчик глушил бы именно тот случай, ради которого С-13
/// существует: плановая проверка была час назад и ничего не нашла, а
/// скачивание сломалось сейчас.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckTrigger {
    /// Первая проверка после старта приложения.
    Startup,
    /// Очередная плановая (Ф-2).
    Planned,
    /// «Проверить сейчас» — действие пользователя (С-12).
    Manual,
    /// Скачивание ролика упало классом `ytDlpFailure` (С-13).
    BrokenExtraction,
}

/// Куда откат отдаёт первый ответ команды `roll_back_ytdlp` (TL-66): строку
/// 14, если ждать придётся, иначе исход (см.
/// [`UpdateController::run_rollback`]).
pub type RollbackReply = tokio::sync::oneshot::Sender<YtDlpUpdateSnapshot>;

/// Состояние контура в памяти процесса.
struct ControllerState {
    status: YtDlpUpdateStatus,
    rollback_target: Option<String>,
    /// Когда контур в последний раз обращался к апстриму — любой
    /// причиной. По нему считается плановое расписание.
    last_check: Option<Instant>,
    /// Когда в последний раз проверяли по сломанному извлечению.
    /// Отдельное поле, а не то же самое: см. [`CheckTrigger`].
    last_broken_extraction_check: Option<Instant>,
    /// Выпуски, запрещённые к установке в пределах этого процесса.
    ///
    /// Существует потому, что запрет на диске может не записаться (нет
    /// прав, полный диск), а без него тот же кандидат приедет снова —
    /// узор долга #22: операция, стабильно упирающаяся в отказ,
    /// повторяется вечно. Журнал на диске остаётся источником правды
    /// между запусками, эта таблица закрывает окно, когда диск
    /// недоступен.
    paused: BTreeSet<BuildId>,
}

/// Контур обновления как состояние приложения: один на процесс.
pub struct UpdateController {
    state: Mutex<ControllerState>,
    /// Право вести конвейер. Отдельно от [`Self::state`], потому что
    /// держится поперёк `await`, а обычный мьютекс так держать нельзя.
    ///
    /// Второй сторож на ту же единственность, и это не избыточность:
    /// статус отклоняет команды мгновенно и с типизированной причиной
    /// (`busy`), а эта очередь удерживает инвариант «конвейер один» даже
    /// если статус кто-то однажды выставит мимо [`Self::begin`] — и
    /// уборка на это прямо опирается (её предусловие: «вызывающий не
    /// готовит установку параллельно»).
    pipeline: tokio::sync::Mutex<()>,
}

impl Default for UpdateController {
    fn default() -> Self {
        Self::new()
    }
}

impl UpdateController {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(ControllerState {
                status: YtDlpUpdateStatus::NeverChecked,
                rollback_target: None,
                last_check: None,
                last_broken_extraction_check: None,
                paused: BTreeSet::new(),
            }),
            pipeline: tokio::sync::Mutex::new(()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ControllerState> {
        // Отравленный мьютекс не повод ронять контур: внутри статус и
        // две отметки времени, паника чужого потока не делает их
        // противоречивыми (тот же приём, что в `ChildRegistry`).
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Держит ли контур границу задач прямо сейчас — вопрос очереди
    /// загрузок (Р-7 эпика E4).
    ///
    /// Истина означает ровно одно: установка уже принята и ждёт прогрева
    /// либо переключения, то есть работы, которую Н-3 запрещает ставить в
    /// конкуренцию с активной загрузкой. Очередь на этот ответ платит
    /// паузой между задачами в десятки секунд, поэтому шире брать нельзя:
    /// проверка апстрима и скачивание архива границы задач не требуют и
    /// очередь не держат — они идут параллельно загрузке и всегда шли.
    ///
    /// Три состояния, а не одно: прогрев (`preparing`) — это уже работа
    /// на границе, а ожидание границы бывает и у обновления
    /// (`readyWaiting`), и у отката (`rollbackWaiting`) — механизм
    /// переключения у них один (Ф-7 E6).
    ///
    /// Расписание, конвейер и экран контура этим не меняются — метод
    /// только читает статус (граница E4: «не меняет расписание, конвейер
    /// и UI контура»).
    pub fn holds_task_boundary(&self) -> bool {
        matches!(
            self.lock().status,
            YtDlpUpdateStatus::Preparing { .. }
                | YtDlpUpdateStatus::ReadyWaiting { .. }
                | YtDlpUpdateStatus::RollbackWaiting { .. }
        )
    }

    /// Ждёт, пока текущий прогон конвейера закончится.
    ///
    /// Ожидание — на том же замке, которым держится единственность
    /// конвейера ([`Self::pipeline`]): другого способа узнать «прогон
    /// закончился» у контура нет, а второй флаг рядом с замком разошёлся
    /// бы с ним на первой же новой ветке выхода.
    ///
    /// Зовётся очередью на границе задач и только после
    /// [`Self::holds_task_boundary`]: сам по себе замок держится весь
    /// прогон, включая скачивание архива, и ждать его без этой проверки
    /// значило бы задерживать старт следующей задачи на минуты вместо
    /// десятков секунд.
    pub async fn wait_until_idle(&self) {
        let _pipeline = self.pipeline.lock().await;
    }

    /// Снимок для команды `ytdlp_update_state` и для события.
    pub fn snapshot(&self) -> YtDlpUpdateSnapshot {
        let state = self.lock();
        YtDlpUpdateSnapshot::new(state.status.clone(), state.rollback_target.clone())
    }

    /// Перечитывает цель отката по каталогу данных приложения.
    ///
    /// Отдельный вход для границы: команда снимка обязана знать цель
    /// кнопки «Вернуться к …» до первого прогона конвейера, а собирать
    /// ради этого всё окружение незачем. Раскладка при этом наружу не
    /// уезжает — её собирает эта функция.
    pub fn refresh_rollback_target_in(&self, data_dir: &std::path::Path) {
        self.refresh_rollback_target(&Layout::new(data_dir));
    }

    /// Перечитывает цель отката из записи Ф-5.
    ///
    /// Цель — версия известно-хорошей установки, если она отличается от
    /// активной; иначе возвращаться некуда, и кнопки нет вовсе (дизайн
    /// E6). Читается с диска, а не хранится вечно, потому что запись
    /// меняют оба пути переключения — и обновление, и откат.
    fn refresh_rollback_target(&self, layout: &Layout) {
        let target = rollback_target(&InstallState::load(layout));
        self.lock().rollback_target = target;
    }

    /// Занимает контур под работу и отдаёт снимок с новым статусом.
    ///
    /// Проверка занятости и смена статуса — под одним замком: между
    /// «свободен ли» и «занял» не должно быть точки, где успевает
    /// пройти второй вызов. Занятость определяется контрактным
    /// [`YtDlpUpdateStatus::busy`], а не собственным флагом, — тем же
    /// правилом, по которому фронтенд гасит обе кнопки.
    fn begin(&self, status: YtDlpUpdateStatus) -> Result<YtDlpUpdateSnapshot, BusyError> {
        let mut state = self.lock();
        if state.status.busy() {
            return Err(BusyError);
        }
        state.status = status;
        Ok(YtDlpUpdateSnapshot::new(
            state.status.clone(),
            state.rollback_target.clone(),
        ))
    }

    /// «Проверить сейчас» (С-12): занимает контур и отдаёт снимок с уже
    /// переключённым `checking`, чтобы кнопка погасла, не дожидаясь
    /// первого события.
    pub fn begin_manual_check(&self) -> Result<YtDlpUpdateSnapshot, YtDlpUpdateCommandError> {
        self.begin(YtDlpUpdateStatus::Checking)
            .map_err(|_| busy_error())
    }

    /// Занимает контур под фоновую проверку, если её время пришло.
    ///
    /// `None` означает «не сейчас» и не является отказом: причин две —
    /// контур занят или троттлинг ещё не отпустил. Обе — норма, и
    /// сообщать о них пользователю нечем (Р-1: контур тихий).
    pub fn begin_background_check(&self, trigger: CheckTrigger, now: Instant) -> Option<()> {
        let mut state = self.lock();
        if state.status.busy() || !throttle_allows(&state, trigger, now) {
            return None;
        }
        state.status = YtDlpUpdateStatus::Checking;
        Some(())
    }

    /// «Вернуться к известно-хорошей» (Р-3).
    ///
    /// Отказывает типизированно в двух случаях: контур занят и
    /// возвращаться некуда. Второй — не гипотетический: кнопки без цели
    /// фронтенд не рисует, но защита на стороне ядра обязана быть
    /// настоящей (тот же довод, что у `DownloadCommandErrorKind`).
    ///
    /// Снимка не возвращает (TL-66): первый ответ команды — исход, который
    /// становится известен только внутри отката, под замком конвейера
    /// ([`Self::run_rollback`], «Первый ответ команды»). До TL-66 здесь
    /// возвращалась строка 14 всегда, и там, где загрузки нет, она
    /// вспыхивала на доли секунды до события с исходом.
    ///
    /// Статус при этом ставится [`YtDlpUpdateStatus::RollbackWaiting`] —
    /// единственный занятый статус отката в контракте. Он нужен ядру с
    /// этой самой секунды: гасит команды (`busy`) и держит границу задач
    /// для очереди ([`Self::holds_task_boundary`]). Наружу событием он
    /// уходит, только если ждать действительно придётся; снимок
    /// `ytdlp_update_state`, запрошенный посреди мгновенного отката, его
    /// покажет — экран, нажавший кнопку, снимок в это время не запрашивает.
    pub fn begin_rollback(&self) -> Result<(), YtDlpUpdateCommandError> {
        let target = {
            let state = self.lock();
            match state.rollback_target.clone() {
                Some(target) => target,
                None => {
                    return Err(YtDlpUpdateCommandError {
                        kind: YtDlpUpdateCommandErrorKind::NothingToRollBackTo,
                        message: "известно-хорошей установки, отличной от активной, на диске \
                                  нет — возвращаться некуда"
                            .to_string(),
                    })
                }
            }
        };

        self.begin(YtDlpUpdateStatus::RollbackWaiting { version: target })
            .map(|_| ())
            .map_err(|_| busy_error())
    }

    /// Ставит терминальный статус и отправляет его подписчикам.
    fn finish(&self, sink: &dyn UpdateSink, status: YtDlpUpdateStatus) {
        let snapshot = {
            let mut state = self.lock();
            state.status = status;
            YtDlpUpdateSnapshot::new(state.status.clone(), state.rollback_target.clone())
        };
        sink.emit(snapshot);
    }

    /// Промежуточный статус: тот же путь, что у терминального, отдельным
    /// именем ради читаемости вызовов.
    fn step(&self, sink: &dyn UpdateSink, status: YtDlpUpdateStatus) {
        self.finish(sink, status);
    }

    /// Отправляет подписчикам то, что уже стоит.
    ///
    /// Нужно на входе в конвейер: статус занятия ставит команда или
    /// расписание, а событие о нём обязано уйти всё равно — иначе
    /// служебный экран, открытый во время фоновой проверки, не увидел бы
    /// строку 2 «Проверяем обновления…» ни разу, а сразу её исход.
    fn emit_current(&self, sink: &dyn UpdateSink) {
        sink.emit(self.snapshot());
    }

    /// Отказ конвейера — один вход на все пять классов Ф-9.
    fn fail(&self, sink: &dyn UpdateSink, failure: YtDlpUpdateFailure) {
        // Версия в лог идёт отдельным полем, а не из текста: три класса
        // из пяти её знают, два — нет, и склеивать её в сообщение
        // значило бы придумывать версию тем двум, у которых её не бывает
        // (проверка метаданных падает раньше, чем становится известно, о
        // какой версии речь).
        match failure.version() {
            Some(version) => eprintln!("yt-dlp update ({version}): {}", failure.message()),
            None => eprintln!("yt-dlp update: {}", failure.message()),
        }
        self.finish(
            sink,
            YtDlpUpdateStatus::Failed {
                at: crate::clock::now_iso8601(),
                failure,
            },
        );
    }

    /// Возвращает контур в состояние, в котором он был до занятия, и
    /// **не** отправляет события.
    ///
    /// Нужно там, где рассказать пользователю нечего и незачем: контур
    /// сходил к апстриму, ничего делать не стал, и ни одно из четырнадцати
    /// состояний дизайна этого не описывает. Показать вместо этого
    /// «установлена последняя версия» было бы неправдой сразу после
    /// отката (последняя — как раз та, от которой отказались), а
    /// оставить `checking` — вечным спиннером, который С-12 запрещает
    /// прямо.
    fn restore(&self, sink: &dyn UpdateSink, previous: YtDlpUpdateStatus) {
        self.finish(sink, previous);
    }

    /// Отмечает факт обращения к апстриму.
    ///
    /// Плановый счётчик двигает **любая** проверка: запрос к API стоит
    /// одинаково, кто бы его ни затеял, и бюджет в 60 запросов в час
    /// один на всех. Счётчик С-13 двигает только своя причина — в этом
    /// и состоит раздельность троттлингов.
    fn note_checked(&self, trigger: CheckTrigger, now: Instant) {
        let mut state = self.lock();
        state.last_check = Some(now);
        if trigger == CheckTrigger::BrokenExtraction {
            state.last_broken_extraction_check = Some(now);
        }
    }

    fn is_paused(&self, build_id: &BuildId) -> bool {
        self.lock().paused.contains(build_id)
    }

    fn pause_in_memory(&self, build_id: &BuildId) {
        self.lock().paused.insert(build_id.clone());
    }

    /// Полный конвейер проверки и установки (С-1, С-2, С-3, С-12, С-13).
    ///
    /// Вызывается только после того, как контур занят (`begin_*`):
    /// разделение существует затем, чтобы команда «Проверить сейчас»
    /// могла вернуть снимок мгновенно, а работа шла дальше сама (Н-3 —
    /// ничего не блокирует).
    pub async fn run_check(&self, env: &Pipeline<'_>, trigger: CheckTrigger, now: Instant) {
        let _queue = self.pipeline.lock().await;
        // Весь прогон — под замком временных объектов корня: уборка
        // подготовки не должна снести `.download-*`/`.staging-*` этого
        // прогона (TL-66). Берётся после замка конвейера, а подготовка его
        // не ждёт — взаимной блокировке неоткуда взяться.
        let _root = env.session.hold_root_for_update().await;

        self.refresh_rollback_target(env.layout);
        self.emit_current(env.sink);
        self.note_checked(trigger, now);

        let install_state = InstallState::load(env.layout);
        let Some(base) = comparison_base(&install_state) else {
            // Разобрать собственную запись не удалось. Это не один из
            // пяти классов Ф-9 — он не про сеть, не про архив и не про
            // место, — и выдумывать ему класс значило бы соврать в
            // интерфейсе (тот же довод, по которому `release` не берёт
            // разбор локальной версии на себя). Молчанием это не
            // становится: строка в логе есть, а состояние блока
            // возвращается тем, чем было.
            eprintln!(
                "yt-dlp update: версию установленного yt-dlp не с чем сравнить — \
                 проверка обновления пропущена"
            );
            self.restore(env.sink, YtDlpUpdateStatus::NeverChecked);
            return;
        };

        let outcome = blocking(|| release::check_for_update(env.metadata, &base));

        match outcome {
            Err(error) => self.fail(env.sink, error.to_failure()),
            Ok(UpdateCheck::UpToDate) => {
                if base_is_active(&install_state, &base) {
                    self.finish(
                        env.sink,
                        YtDlpUpdateStatus::UpToDate {
                            at: crate::clock::now_iso8601(),
                        },
                    );
                } else {
                    // Новее ничего нет, но активная — не самая новая из
                    // лежащих на диске: пользователь откатился, и С-8
                    // держит нас от того, чтобы поставить обратно. Так
                    // выглядит успешная проверка, которой нечего
                    // сказать; см. doc `restore`.
                    eprintln!(
                        "yt-dlp update: новее {base} у апстрима нет, а {base} — та версия, \
                         от которой откатились (С-8): ставить обратно не будем"
                    );
                    self.restore(env.sink, previous_terminal_status(&install_state));
                }
            }
            Ok(UpdateCheck::Available(asset)) => {
                self.install_release(env, &asset, &install_state, trigger)
                    .await;
            }
        }
    }

    /// Вторая половина С-10: вшитый в бандл пин новее активной установки.
    ///
    /// Случай возникает после обновления **самого приложения** поверх
    /// самообновлённого yt-dlp: пользователь поставил новый релиз, в нём
    /// пин свежее того, что контур успел скачать сам. Тогда пин ставится
    /// тем же конвейером, что и сетевое обновление, — распаковка, smoke,
    /// переключение, уборка (С-10 требует буквально «тем же путём»).
    ///
    /// Обратный случай — пин **старее** активной — не делает ничего:
    /// приложение не откатывает yt-dlp, который само же обновило.
    /// # Почему эта ветка занимает контур сама
    ///
    /// В отличие от проверки, её никто не «заказывает»: она выполняется
    /// на каждом старте и почти всегда не находит работы (пин старее
    /// активной — обычное состояние приложения, которое хоть раз
    /// обновило yt-dlp). Займи она контур заранее, каждый запуск
    /// приложения мигал бы блоку строкой «Проверяем обновления…» и
    /// гасил бы кнопки ни за чем. Поэтому статус меняется только тогда,
    /// когда установка действительно начинается.
    pub async fn run_bundled_pin(&self, env: &Pipeline<'_>) {
        let _queue = self.pipeline.lock().await;
        // Весь прогон — под замком временных объектов корня: уборка
        // подготовки не должна снести `.download-*`/`.staging-*` этого
        // прогона (TL-66). Берётся после замка конвейера, а подготовка его
        // не ждёт — взаимной блокировке неоткуда взяться.
        let _root = env.session.hold_root_for_update().await;

        self.refresh_rollback_target(env.layout);

        let install_state = InstallState::load(env.layout);
        let identity = ArchiveIdentity::bundled();

        let Ok(pinned) = identity.build_id() else {
            eprintln!("yt-dlp update: пин бандла не годится в идентификатор установки");
            return;
        };

        let known = [install_state.active(), install_state.known_good()]
            .into_iter()
            .flatten()
            .any(|entry| entry.build_id() == &pinned);

        let newer = match (
            ReleaseVersion::parse(identity.version),
            active_version(&install_state),
        ) {
            (Ok(pin), Some(active)) => pin > active,
            // Активной записи нет вовсе — ставить пин отдельным
            // конвейером незачем: это первый запуск, и его делает
            // подготовка (TL-12).
            (Ok(_), None) => false,
            (Err(error), _) => {
                eprintln!("yt-dlp update: версия пина непонятна ({error})");
                false
            }
        };

        if !newer || known {
            return;
        }

        let Some(source) = env.archives.bundled() else {
            eprintln!("yt-dlp update: вложенный в бандл архив недоступен");
            return;
        };

        eprintln!(
            "yt-dlp update: пин бандла {} новее активной установки — ставлю его тем же \
             конвейером, что и сетевое обновление (С-10)",
            identity.version
        );

        if self
            .begin(YtDlpUpdateStatus::Downloading {
                version: identity.version.to_string(),
                percent: YtDlpUpdatePercent::new(0),
            })
            .is_err()
        {
            return;
        }
        self.step(
            env.sink,
            YtDlpUpdateStatus::Downloading {
                version: identity.version.to_string(),
                percent: YtDlpUpdatePercent::new(0),
            },
        );

        self.install(env, source.as_ref(), identity, &pinned, &install_state)
            .await;
    }

    /// Ручной возврат на известно-хорошую установку (Р-3, С-8).
    ///
    /// Вызывается после [`Self::begin_rollback`]. Цель одна по
    /// построению (Ф-8), поэтому параметра-версии нет: её читает та же
    /// запись Ф-5, которая только что нарисовала кнопку.
    ///
    /// # Первый ответ команды (TL-66)
    ///
    /// `reply` получает ровно один снимок — тот, что команда отката
    /// вернёт фронтенду. Решение о нём принимается здесь, где исход
    /// известен, а не в команде заранее:
    ///
    /// - идёт загрузка — строка 14 (`rollbackWaiting`) сразу, до ожидания
    ///   границы; исход приходит событием;
    /// - загрузки нет — ответ ждёт конца отката и несёт исход (строка 13
    ///   или отказ). Строки 14 нет ни в ответе, ни в событиях: объявлять
    ///   ожидание загрузки, которой нет, — та самая вспышка.
    ///
    /// Цена второго случая: промис команды живёт, пока идёт проверка
    /// запуска цели (на тёплом дереве — доли секунды, на холодном — до
    /// 24 с), и фронтенд всё это время не получает `busy`. Кнопки
    /// выглядят живыми, хотя ядро отклонит нажатие классом `busy`.
    ///
    /// Ветка выхода, не ответившая сама, получает ответ текущим снимком в
    /// конце: забыть про ответ нечем.
    pub async fn run_rollback(&self, env: &Pipeline<'_>, reply: RollbackReply) {
        let mut reply = Some(reply);
        self.roll_back(env, &mut reply).await;
        if let Some(reply) = reply {
            // Получатель мог уйти (окно закрыто) — исход уже ушёл событием.
            let _ = reply.send(self.snapshot());
        }
    }

    async fn roll_back(&self, env: &Pipeline<'_>, reply: &mut Option<RollbackReply>) {
        let _queue = self.pipeline.lock().await;
        // Весь прогон — под замком временных объектов корня: уборка
        // подготовки не должна снести `.download-*`/`.staging-*` этого
        // прогона (TL-66). Берётся после замка конвейера, а подготовка его
        // не ждёт — взаимной блокировке неоткуда взяться.
        let _root = env.session.hold_root_for_update().await;

        let install_state = InstallState::load(env.layout);
        let Some(target) = install_state.known_good().cloned() else {
            // Запись изменилась между командой и работой: возвращаться
            // уже некуда.
            eprintln!("yt-dlp update: возвращаться некуда — известно-хорошей установки нет");
            self.restore(env.sink, previous_terminal_status(&install_state));
            return;
        };

        let abandoned = install_state
            .active()
            .map(|entry| entry.version().to_string())
            .unwrap_or_default();

        // Граница задач — та же, что у обычного переключения (Ф-7, единый
        // механизм, не вторая реализация). Занятость спрашивается один
        // раз: ответ «ждём» и само ожидание обязаны опираться на одно
        // наблюдение. Спроси их порознь — и загрузка, начавшаяся между
        // вопросами, оставила бы промис команды висеть до её конца без
        // объявленной строки 14.
        if env.boundary.is_busy() {
            let waiting = self.snapshot();
            env.sink.emit(waiting.clone());
            if let Some(reply) = reply.take() {
                let _ = reply.send(waiting);
            }
            env.boundary.wait().await;
        }

        // Годность цели проверяет тот, кто откатывается, и проверяет
        // запуском. Дешевле нельзя: в резерв уезжает **вытесненная
        // активная** установка, а вытеснить её мог как раз отказ запуска
        // (`prepare` при негодной активной берёт пин, и вытесненная
        // становится известно-хорошей). Дерево при этом целое и манифест
        // сходится — `layout::validate` такую установку пропускает, а
        // `--version` нет. Внутрь `activate` эта проверка не кладётся
        // намеренно: там завелось бы второе место, решающее «годна ли
        // установка», рядом с `layout::validate`.
        let candidate = match prepared_from_installed(env.layout, &target) {
            Ok(candidate) => candidate,
            Err(reason) => {
                self.fail(
                    env.sink,
                    YtDlpUpdateFailure::SmokeCheckFailed {
                        version: target.version().to_string(),
                        message: reason,
                    },
                );
                return;
            }
        };

        if let Err(error) = smoke::smoke_check(&candidate, env.layout, env.registry).await {
            self.remember_if_journal_missing(env.layout, &candidate.build_id, &error);
            self.fail(env.sink, error.to_failure());
            return;
        }

        match self.switch_to(env, target.clone()) {
            Ok(()) => self.finish(
                env.sink,
                YtDlpUpdateStatus::RolledBack {
                    at: crate::clock::now_iso8601(),
                    active: target.version().to_string(),
                    abandoned,
                },
            ),
            Err(failure) => self.fail(env.sink, failure),
        }
    }

    /// Установка найденного релиза: запреты, приём, smoke, переключение.
    async fn install_release(
        &self,
        env: &Pipeline<'_>,
        asset: &UpdateAsset,
        install_state: &InstallState,
        trigger: CheckTrigger,
    ) {
        let identity = ArchiveIdentity::from(asset);
        let build_id = match identity.build_id() {
            Ok(build_id) => build_id,
            Err(error) => {
                // Метаданные назвали версию или сумму, из которых нельзя
                // составить имя каталога. Это тот же класс, что битый
                // архив: ставить нечего, лечится ожиданием следующего
                // релиза.
                self.fail(
                    env.sink,
                    YtDlpUpdateFailure::ArchiveCorrupted {
                        version: asset.version.clone(),
                        message: error.to_string(),
                    },
                );
                return;
            }
        };

        // Действие пользователя — второй и последний выход из запрета
        // С-5 (первый — новый релиз апстрима, у него другой build id).
        // Нажав «Проверить сейчас», пользователь просит попробовать ещё
        // раз именно то, что не вышло: молча ответить ему прежним
        // отказом значило бы сделать запрет вечным, а С-5 обещает
        // обратное. Плановая и внеплановая проверки запреты не трогают —
        // иначе они бы их и снимали, и ставили, то есть не запрещали бы
        // ничего.
        if trigger == CheckTrigger::Manual {
            self.lift_bans(env.layout, &build_id);
        }

        if let Some(failure) = self.forbidden(env.layout, &build_id, &asset.version, install_state)
        {
            self.fail(env.sink, failure);
            return;
        }

        self.step(
            env.sink,
            YtDlpUpdateStatus::Downloading {
                version: asset.version.clone(),
                percent: YtDlpUpdatePercent::new(0),
            },
        );

        let source = env.archives.network(asset);
        self.install(env, source.as_ref(), identity, &build_id, install_state)
            .await;
    }

    /// Общий хвост обоих источников (С-10: путь один).
    async fn install(
        &self,
        env: &Pipeline<'_>,
        source: &(dyn ArchiveSource + Send + Sync),
        identity: ArchiveIdentity<'_>,
        build_id: &BuildId,
        install_state: &InstallState,
    ) {
        let version = identity.version.to_string();

        let candidate = match self.obtain(env, source, identity, build_id) {
            Ok(candidate) => candidate,
            Err(failure) => {
                self.fail(env.sink, failure);
                return;
            }
        };

        // В корне установок появилось новое дерево (TL-23). Экрану оно ещё
        // не видно — активная запись прежняя, — но сеанс помнит только то,
        // что верно для корня целиком, и разбирать, какая установка на что
        // влияет, здесь не берутся: лишний сброс стоит одного запуска
        // `--version` на следующей проверке.
        env.session.invalidate();

        // Прогрев не соревнуется с активной загрузкой (Н-3): первый
        // запуск распакованного дерева стоит 24–36 с дисковой работы, и
        // smoke-проверка — это он и есть.
        self.wait_for_boundary(env).await;

        self.step(
            env.sink,
            YtDlpUpdateStatus::Preparing {
                version: version.clone(),
            },
        );

        if let Err(error) = smoke::smoke_check(&candidate, env.layout, env.registry).await {
            self.remember_if_journal_missing(env.layout, build_id, &error);
            self.fail(env.sink, error.to_failure());
            return;
        }

        // Между smoke и переключением пользователь мог начать новую
        // задачу: Ф-7 требует не менять версию посреди неё.
        self.wait_for_boundary(env).await;

        let entry = match InstallEntry::for_identity(identity) {
            Ok(entry) => entry,
            Err(error) => {
                self.fail(
                    env.sink,
                    YtDlpUpdateFailure::ArchiveCorrupted {
                        version,
                        message: error.to_string(),
                    },
                );
                return;
            }
        };

        match self.switch_to(env, entry) {
            Ok(()) => {
                // Установка удалась — история неудачных приёмов этого
                // выпуска больше ни о чём не говорит.
                RepairLog::clear(&env.layout.update_attempt_path(build_id));
                self.finish(
                    env.sink,
                    YtDlpUpdateStatus::Updated {
                        at: crate::clock::now_iso8601(),
                        version,
                    },
                );
            }
            Err(failure) => self.fail(env.sink, failure),
        }

        let _ = install_state;
    }

    /// Приём и распаковка — либо готовый кандидат, если он уже лежит на
    /// диске.
    ///
    /// Свойство «уже лежит» — это самовосстановление С-7 в чистом виде:
    /// приложение убили между распаковкой и переключением, дерево и
    /// манифест остались, и платить за те же шестьдесят мегабайт второй
    /// раз не за что. Проверяет это [`layout::validate`] — та же
    /// функция, которой резолв решает «есть ли чем работать», а не
    /// отдельное правило.
    fn obtain(
        &self,
        env: &Pipeline<'_>,
        source: &(dyn ArchiveSource + Send + Sync),
        identity: ArchiveIdentity<'_>,
        build_id: &BuildId,
    ) -> Result<PreparedCandidate, YtDlpUpdateFailure> {
        if let Ok(installed) = layout::validate(env.layout, build_id) {
            eprintln!(
                "yt-dlp update: {build_id} уже распакован — прерванная подготовка \
                 продолжается с проверки запуска, скачивать нечего"
            );
            return Ok(PreparedCandidate {
                build_id: build_id.clone(),
                version: identity.version.to_string(),
                dir: installed.dir,
                executable: installed.executable,
            });
        }

        let version = identity.version.to_string();
        let mut last_percent = u8::MAX;

        blocking(|| {
            fetch::fetch_and_install(source, identity, env.layout, &mut |stage, done, total| {
                match stage {
                    FetchStage::Downloading => {
                        let percent = percent_of(done, total);
                        // Событие на каждый изменившийся процент, а не
                        // на каждый мегабайт: тот же приём троттлинга,
                        // что у прогресса скачивания ролика в E3.
                        if percent != last_percent {
                            last_percent = percent;
                            self.step(
                                env.sink,
                                YtDlpUpdateStatus::Downloading {
                                    version: version.clone(),
                                    percent: YtDlpUpdatePercent::new(percent),
                                },
                            );
                        }
                    }
                    FetchStage::Unpacking => {
                        if last_percent != u8::MAX {
                            last_percent = u8::MAX;
                            self.step(
                                env.sink,
                                YtDlpUpdateStatus::Preparing {
                                    version: version.clone(),
                                },
                            );
                        }
                    }
                }
            })
        })
        .map_err(|error| error.to_failure(identity.version))
    }

    /// Переключение активной записи и уборка (Ф-8) — общий хвост
    /// обновления и отката.
    fn switch_to(&self, env: &Pipeline<'_>, entry: InstallEntry) -> Result<(), YtDlpUpdateFailure> {
        let version = entry.version().to_string();
        let mut install_state = InstallState::load(env.layout);

        install_state.activate(env.layout, entry).map_err(|error| {
            YtDlpUpdateFailure::ArchiveCorrupted {
                version,
                message: format!("активная установка не записана: {error}"),
            }
        })?;

        // Активная установка сменилась — обновлением или откатом (TL-23):
        // версия, которую помнит сеанс, больше не та, которой работает
        // приложение. Сразу после записи, до уборки: уборка может снести
        // дерево, путь к которому сеанс ещё помнит.
        env.session.invalidate();

        // Уборка — только после подтверждённого переключения: запись уже
        // называет обе установки, которые обязаны остаться, и всё
        // прочее лишнее по построению. Отметки занятости защищают
        // деревья, из которых прямо сейчас работают процессы (Ф-7), и
        // защищают безусловно.
        let report = state::cleanup(env.layout, &install_state, env.in_use, &[]);
        if !report.removed.is_empty() {
            eprintln!(
                "yt-dlp update: уборка сняла {} объект(ов) из каталога установок",
                report.removed.len()
            );
        }

        self.refresh_rollback_target(env.layout);
        Ok(())
    }

    /// Ждёт паузы между задачами, показав, что именно ждёт.
    ///
    /// Строка 6 таблицы состояний показывается только когда ждать
    /// действительно приходится: в тихом приложении переключение
    /// происходит сразу, и объявлять «применится, когда закончится
    /// текущая загрузка» было бы неправдой.
    async fn wait_for_boundary(&self, env: &Pipeline<'_>) {
        if !env.boundary.is_busy() {
            return;
        }

        let waiting = {
            let state = self.lock();
            match &state.status {
                // Откат сюда не приходит: границу он спрашивает сам, вместе
                // с первым ответом команды (`roll_back`).
                YtDlpUpdateStatus::Downloading { version, .. }
                | YtDlpUpdateStatus::Preparing { version }
                | YtDlpUpdateStatus::ReadyWaiting { version } => Some(version.clone()),
                _ => None,
            }
        };

        if let Some(version) = waiting {
            self.step(env.sink, YtDlpUpdateStatus::ReadyWaiting { version });
        }

        env.boundary.wait().await;
    }

    /// Запреты, которые обязаны сработать **до** обращения в сеть.
    ///
    /// Все три — про то, что платить за заведомо известный исход не
    /// надо, и все три стоят здесь, а не внутри шагов конвейера: к
    /// моменту, когда smoke могла бы отказать сама, шестьдесят
    /// мегабайт уже приняты и распакованы, то есть С-5 («тот же build id
    /// не устанавливается повторно») исполнен не был.
    fn forbidden(
        &self,
        layout: &Layout,
        build_id: &BuildId,
        version: &str,
        install_state: &InstallState,
    ) -> Option<YtDlpUpdateFailure> {
        // С-5: этот выпуск уже не прошёл проверку запуска.
        if let Some(previous) = smoke::previous_failure(layout, build_id) {
            return Some(YtDlpUpdateFailure::SmokeCheckFailed {
                version: version.to_string(),
                message: format!(
                    "yt-dlp {version} уже не прошёл проверку запуска ({}) — до нового релиза \
                     апстрима этот выпуск пропускается",
                    previous.last_reason
                ),
            });
        }

        // То же, но записанное только в памяти: журнал на диске мог не
        // записаться (см. `ControllerState::paused`).
        if self.is_paused(build_id) {
            return Some(YtDlpUpdateFailure::SmokeCheckFailed {
                version: version.to_string(),
                message: format!(
                    "yt-dlp {version} не прошёл проверку запуска в этом сеансе, а записать \
                     запрет на диск не удалось — до перезапуска приложения этот выпуск \
                     пропускается"
                ),
            });
        }

        // С-4: тот же ассет не перекачивается в бесконечном цикле.
        let attempts = RepairLog::read(&layout.update_attempt_path(build_id));
        if attempts.attempts >= MAX_FETCH_ATTEMPTS
            && crate::clock::now_unix_secs().saturating_sub(attempts.last_attempt_unix)
                < FETCH_COOLDOWN.as_secs()
        {
            return Some(YtDlpUpdateFailure::ArchiveCorrupted {
                version: version.to_string(),
                message: format!(
                    "обновление до {version} не удалось {} раз(а) подряд, последний — {} \
                     ({}); следующая попытка не раньше чем через сутки",
                    attempts.attempts, attempts.last_attempt_at, attempts.last_reason
                ),
            });
        }

        // Предусловие приёма (С-4): `fetch_and_install` нельзя звать с
        // идентификатором активной установки. Порядок шагов в
        // `prepare::install` эту гарантию уже держит конструкцией
        // (распаковка идёт до сноса прежнего дерева), но проверка
        // остаётся: ставить поверх того, что и так активно, — работа с
        // заведомо нулевым результатом, а гарантия, у которой два
        // независимых держателя, переживает правку любого из них.
        if install_state
            .active()
            .is_some_and(|active| active.build_id() == build_id)
        {
            return Some(YtDlpUpdateFailure::ArchiveCorrupted {
                version: version.to_string(),
                message: format!("{build_id} уже активная установка — ставить нечего"),
            });
        }

        None
    }

    /// Снимает все запреты на этот выпуск — «действие пользователя» из
    /// С-5.
    ///
    /// Снимаются все три сразу, а не только журнал smoke: пользователь
    /// просит попробовать заново, и остановить его на счётчике
    /// неудачных приёмов было бы тем же вечным запретом, только под
    /// другим именем.
    fn lift_bans(&self, layout: &Layout, build_id: &BuildId) {
        smoke::forget(layout, build_id);
        RepairLog::clear(&layout.update_attempt_path(build_id));
        self.lock().paused.remove(build_id);
    }

    /// Если запрет не доехал до диска — держим его в памяти.
    ///
    /// Спрашиваем не «удалась ли запись», а «виден ли запрет»: это тот
    /// же вопрос, который задаст следующая проверка, и заданный тем же
    /// способом. Утверждение о собственной записи проверяется чтением,
    /// а не доверием к возвращённому значению.
    fn remember_if_journal_missing(
        &self,
        layout: &Layout,
        build_id: &BuildId,
        error: &smoke::SmokeError,
    ) {
        if !error.was_attempted() {
            // Запуска не было — запрет уже стоял, писать было нечего.
            return;
        }

        if smoke::previous_failure(layout, build_id).is_none() {
            eprintln!(
                "yt-dlp update: запрет на {build_id} не сохранён на диске — держу его в \
                 памяти до перезапуска приложения"
            );
            self.pause_in_memory(build_id);
        }
    }
}

/// Внутренний отказ «контур занят»: наружу уходит контрактным
/// [`YtDlpUpdateCommandErrorKind::Busy`], но фоновым вызовам он не
/// ошибка, а «не сейчас».
struct BusyError;

fn busy_error() -> YtDlpUpdateCommandError {
    YtDlpUpdateCommandError {
        kind: YtDlpUpdateCommandErrorKind::Busy,
        message: "контур обновления занят: идёт проверка, подготовка или ожидание паузы \
                  между задачами"
            .to_string(),
    }
}

/// Пропускает ли троттлинг проверку этой причины прямо сейчас.
///
/// Чистая функция от состояния и момента — момент приходит аргументом, а
/// не читается внутри, ровно затем, чтобы расписание проверялось без
/// единого настоящего ожидания (приём TL-12, `repair_exhausted`).
fn throttle_allows(state: &ControllerState, trigger: CheckTrigger, now: Instant) -> bool {
    match trigger {
        // Действие пользователя не троттлится вовсе: С-12 требует, чтобы
        // ручная проверка работала независимо от расписания автопроверок.
        CheckTrigger::Manual => true,
        // Первая проверка сеанса: расписание в памяти пустое, и спрашивать
        // его не о чем.
        CheckTrigger::Startup => true,
        CheckTrigger::Planned => elapsed_at_least(state.last_check, now, PLANNED_CHECK_INTERVAL),
        // Единственное место, где раздельность троттлингов выражена
        // кодом: здесь спрашивается **свой** счётчик, и плановый на
        // решение не влияет никак.
        CheckTrigger::BrokenExtraction => elapsed_at_least(
            state.last_broken_extraction_check,
            now,
            BROKEN_EXTRACTION_THROTTLE,
        ),
    }
}

fn elapsed_at_least(last: Option<Instant>, now: Instant, interval: Duration) -> bool {
    last.is_none_or(|last| now.saturating_duration_since(last) >= interval)
}

/// С чем сравнивается последний релиз апстрима.
///
/// Не с активной установкой, а с **самой новой из лежащих на диске** —
/// см. «Как удерживается С-8» в doc модуля. `None` означает, что ни одну
/// из имеющихся версий не удалось разобрать: сравнивать не с чем, и
/// проверка не выполняется вовсе.
fn comparison_base(state: &InstallState) -> Option<ReleaseVersion> {
    [state.active(), state.known_good()]
        .into_iter()
        .flatten()
        .filter_map(|entry| match ReleaseVersion::parse(entry.version()) {
            Ok(version) => Some(version),
            Err(error) => {
                eprintln!("yt-dlp update: {error}");
                None
            }
        })
        .max()
        .or_else(|| {
            // Записи нет — работает пин, и он же точка отсчёта (С-10:
            // вложенный архив — начальное значение состояния).
            ReleaseVersion::parse(layout::BUNDLED_VERSION).ok()
        })
}

fn active_version(state: &InstallState) -> Option<ReleaseVersion> {
    let text = state.active().map(InstallEntry::version)?;
    ReleaseVersion::parse(text).ok()
}

/// Совпадает ли точка сравнения с активной установкой.
///
/// Расходятся они ровно в одном случае — после отката, когда
/// известно-хорошая новее активной. Именно этот случай и не должен
/// показываться как «установлена последняя версия».
fn base_is_active(state: &InstallState, base: &ReleaseVersion) -> bool {
    active_version(state).is_some_and(|active| active == *base)
}

/// Цель кнопки «Вернуться к …»: версия известно-хорошей установки, если
/// она отличается от активной.
fn rollback_target(state: &InstallState) -> Option<String> {
    let known_good = state.known_good()?;
    let active = state.active()?;
    (known_good.build_id() != active.build_id()).then(|| known_good.version().to_string())
}

/// Статус, к которому контур возвращается, когда рассказывать нечего.
///
/// После отката это строка 13 — она остаётся верной и объясняет, почему
/// обновления «нет»; в остальных случаях — «ещё не проверяли»,
/// единственное состояние, которое не врёт ни о чём.
fn previous_terminal_status(state: &InstallState) -> YtDlpUpdateStatus {
    match (state.active(), state.known_good()) {
        (Some(active), Some(known_good)) if known_good.build_id() != active.build_id() => {
            YtDlpUpdateStatus::RolledBack {
                at: crate::clock::now_iso8601(),
                active: active.version().to_string(),
                abandoned: known_good.version().to_string(),
            }
        }
        _ => YtDlpUpdateStatus::NeverChecked,
    }
}

/// Собирает «подготовленного кандидата» из уже установленного дерева.
///
/// Нужно откату: проверка запуска (TL-57) умеет говорить только о
/// кандидате, а откат запускает установку, которая кандидатом была
/// когда-то раньше. Тип тот же не ради удобства — «подготовлена, но не
/// активна» описывает обе ситуации буквально.
fn prepared_from_installed(
    layout: &Layout,
    entry: &InstallEntry,
) -> Result<PreparedCandidate, String> {
    let build_id = entry.build_id().clone();
    let installed = layout::validate(layout, &build_id)
        .map_err(|invalid| format!("установка {build_id} непригодна: {invalid}"))?;

    Ok(PreparedCandidate {
        build_id,
        version: entry.version().to_string(),
        dir: installed.dir,
        executable: installed.executable,
    })
}

fn percent_of(done: u64, total: u64) -> u8 {
    if total == 0 {
        return 0;
    }
    let percent = done.saturating_mul(100) / total;
    u8::try_from(percent.min(100)).unwrap_or(100)
}

/// Выполняет блокирующую работу, не занимая рабочий поток рантайма
/// целиком.
///
/// Конвейер обновления блокирует по-настоящему: `ureq` синхронный,
/// распаковка — тоже, и один такой шаг длится минуты. Без этой обёртки
/// он занял бы поток исполнителя, на котором в этот момент могли бы
/// исполняться задачи скачивания ролика.
///
/// Требование к рантайму названо прямо: [`tokio::task::block_in_place`]
/// работает только на многопоточном рантайме, а на однопоточном
/// паникует. Боевой путь ему удовлетворяет — Tauri поднимает свой
/// глобальный рантайм как `TokioRuntime::new()`, то есть многопоточный
/// (проверено в исходниках tauri 2.11.5, `async_runtime::default_runtime`).
/// Тесты этого модуля объявляют `#[tokio::test(flavor = "multi_thread")]`
/// по той же причине; забывший это тест паникует громко, а не работает
/// вполсилы.
fn blocking<R>(work: impl FnOnce() -> R) -> R {
    tokio::task::block_in_place(work)
}

/// Боевой источник архивов: сеть плюс ресурс бандла (С-10).
struct Archives<'a> {
    transport: &'a super::transport::GithubTransport,
    /// Путь к вложенному в бандл архиву; `None` — ресурс не резолвится.
    bundled: Option<PathBuf>,
}

impl ArchiveSupply for Archives<'_> {
    fn network<'a>(&'a self, asset: &UpdateAsset) -> Box<dyn ArchiveSource + Send + Sync + 'a> {
        Box::new(self.transport.archive_source(asset))
    }

    fn bundled(&self) -> Option<Box<dyn ArchiveSource + Send + Sync + '_>> {
        let path = self.bundled.as_ref()?;
        match fetch::BundledArchive::at(path) {
            Ok(archive) => Some(Box::new(archive)),
            Err(error) => {
                eprintln!("yt-dlp update: вложенный архив недоступен: {error}");
                None
            }
        }
    }
}

/// Боевое окружение конвейера, собранное из того, что есть у границы.
///
/// Существует ради двух вещей сразу. Первая — раскладка ([`Layout`])
/// остаётся внутренней: за пределы домена `ytdlp` она не уезжает ни
/// полем, ни аргументом, и собрать путь в каталоге данных мимо неё
/// по-прежнему нельзя. Вторая — домен не узнаёт ни о Tauri, ни об
/// эпике E3: приёмник событий и граница задач приходят сюда трейтами,
/// а их боевые реализации живут у границы (`crate::commands::update`),
/// где `AppHandle` и слот загрузки и так есть.
pub struct UpdateJob<'a> {
    layout: Layout,
    transport: &'a super::transport::GithubTransport,
    bundled: Option<PathBuf>,
    registry: &'a ChildRegistry,
    in_use: &'a InUse,
    session: &'a Session,
    sink: &'a dyn UpdateSink,
    boundary: &'a dyn TaskBoundary,
}

impl<'a> UpdateJob<'a> {
    // Каждый аргумент — отдельное состояние приложения со своим смыслом
    // (см. doc `Pipeline`); сворачивать их в структуру-контекст значило бы
    // завести у границы вторую сборку того же `Pipeline`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        data_dir: &std::path::Path,
        bundled: Option<PathBuf>,
        transport: &'a super::transport::GithubTransport,
        registry: &'a ChildRegistry,
        in_use: &'a InUse,
        session: &'a Session,
        sink: &'a dyn UpdateSink,
        boundary: &'a dyn TaskBoundary,
    ) -> Self {
        Self {
            layout: Layout::new(data_dir),
            transport,
            bundled,
            registry,
            in_use,
            session,
            sink,
            boundary,
        }
    }

    /// Полный конвейер проверки (плановой, ручной или по С-13).
    pub async fn check(&self, controller: &UpdateController, trigger: CheckTrigger) {
        let archives = self.archives();
        controller
            .run_check(
                &self.pipeline(&archives),
                trigger,
                crate::clock::monotonic_now(),
            )
            .await;
    }

    /// Вторая половина С-10: пин бандла новее активной установки.
    pub async fn bundled_pin(&self, controller: &UpdateController) {
        let archives = self.archives();
        controller.run_bundled_pin(&self.pipeline(&archives)).await;
    }

    /// Ручной возврат на известно-хорошую установку (Р-3); `reply` — первый
    /// ответ команды (см. [`UpdateController::run_rollback`]).
    pub async fn rollback(&self, controller: &UpdateController, reply: RollbackReply) {
        let archives = self.archives();
        controller
            .run_rollback(&self.pipeline(&archives), reply)
            .await;
    }

    fn archives(&self) -> Archives<'a> {
        Archives {
            transport: self.transport,
            bundled: self.bundled.clone(),
        }
    }

    fn pipeline<'p>(&'p self, archives: &'p Archives<'a>) -> Pipeline<'p> {
        Pipeline {
            layout: &self.layout,
            metadata: self.transport,
            archives,
            registry: self.registry,
            in_use: self.in_use,
            session: self.session,
            sink: self.sink,
            boundary: self.boundary,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::{Cursor, Read, Write};
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use tempfile::{tempdir, TempDir};
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    use super::super::fetch::{Origin, StreamArchive};
    use super::super::release::{
        MetadataError, MetadataRequest, LATEST_RELEASE_URL, UPSTREAM_ASSET,
    };

    const EXECUTABLE: &str = "yt-dlp_macos";

    /// Версии фикстур: активная, кандидат и «предыдущая известно-хорошая».
    const ACTIVE: &str = "2026.08.19";
    const CANDIDATE: &str = "2026.09.01";
    const OLDER: &str = "2026.07.11";

    // ─────────────────────────── окружение ───────────────────────────

    /// Приёмник, который помнит всё, что ему отдали.
    ///
    /// Помнит целиком снимки, а не «последний статус»: половина
    /// утверждений ниже — про **порядок** состояний, а не про итог, и
    /// проверить его по последнему значению нельзя в принципе.
    #[derive(Default)]
    struct RecordingSink(Mutex<Vec<YtDlpUpdateSnapshot>>);

    impl UpdateSink for RecordingSink {
        fn emit(&self, snapshot: YtDlpUpdateSnapshot) {
            self.0.lock().expect("замок приёмника").push(snapshot);
        }
    }

    impl RecordingSink {
        fn snapshots(&self) -> Vec<YtDlpUpdateSnapshot> {
            self.0.lock().expect("замок приёмника").clone()
        }

        fn statuses(&self) -> Vec<YtDlpUpdateStatus> {
            self.snapshots()
                .into_iter()
                .map(|snapshot| snapshot.status)
                .collect()
        }

        /// Имена состояний без полей — форма, в которой удобно говорить о
        /// последовательности этапов.
        fn stages(&self) -> Vec<&'static str> {
            self.statuses().iter().map(stage_name).collect()
        }

        fn last(&self) -> YtDlpUpdateStatus {
            self.statuses()
                .pop()
                .expect("приёмник обязан получить хотя бы один снимок")
        }
    }

    fn stage_name(status: &YtDlpUpdateStatus) -> &'static str {
        match status {
            YtDlpUpdateStatus::NeverChecked => "neverChecked",
            YtDlpUpdateStatus::Checking => "checking",
            YtDlpUpdateStatus::UpToDate { .. } => "upToDate",
            YtDlpUpdateStatus::Downloading { .. } => "downloading",
            YtDlpUpdateStatus::Preparing { .. } => "preparing",
            YtDlpUpdateStatus::ReadyWaiting { .. } => "readyWaiting",
            YtDlpUpdateStatus::RollbackWaiting { .. } => "rollbackWaiting",
            YtDlpUpdateStatus::Updated { .. } => "updated",
            YtDlpUpdateStatus::RolledBack { .. } => "rolledBack",
            YtDlpUpdateStatus::Failed { .. } => "failed",
        }
    }

    /// Граница задач, которой можно управлять.
    ///
    /// Помнит не только факт ожидания, но и **что было записано активной
    /// установкой в момент ожидания**: без этого «переключение произошло
    /// после паузы» проверялось бы по итогу, то есть не проверялось бы
    /// вовсе — итог одинаков в обоих случаях.
    struct Boundary {
        layout: Layout,
        busy: AtomicBool,
        waits: Mutex<Vec<Option<String>>>,
    }

    impl Boundary {
        fn idle(layout: &Layout) -> Self {
            Self {
                layout: layout.clone(),
                busy: AtomicBool::new(false),
                waits: Mutex::new(Vec::new()),
            }
        }

        fn busy(layout: &Layout) -> Self {
            let boundary = Self::idle(layout);
            boundary.busy.store(true, Ordering::SeqCst);
            boundary
        }

        fn waits(&self) -> Vec<Option<String>> {
            self.waits.lock().expect("замок границы").clone()
        }
    }

    impl TaskBoundary for Boundary {
        fn is_busy(&self) -> bool {
            self.busy.load(Ordering::SeqCst)
        }

        fn wait(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
            let active = InstallState::load(&self.layout)
                .active()
                .map(|entry| entry.version().to_string());
            self.waits.lock().expect("замок границы").push(active);
            // Задача «закончилась» ровно тем, что её дождались: второй
            // раз ждать будет нечего, и конвейер дойдёт до конца.
            self.busy.store(false, Ordering::SeqCst);
            Box::pin(std::future::ready(()))
        }
    }

    /// Граница, занятая до тех пор, пока её не отпустит сам тест (TL-66).
    ///
    /// Нужна там, где предмет — что происходит **во время** ожидания:
    /// [`Boundary`] отпускает себя сама в момент, когда её начали ждать.
    /// Порядок наблюдается рандеву, без часов: `entered` — конвейер уже
    /// ждёт, `release` — загрузка кончилась.
    struct GatedBoundary {
        busy: AtomicBool,
        entered: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }

    impl GatedBoundary {
        fn busy() -> Self {
            Self {
                busy: AtomicBool::new(true),
                entered: tokio::sync::Notify::new(),
                release: tokio::sync::Notify::new(),
            }
        }
    }

    impl TaskBoundary for GatedBoundary {
        fn is_busy(&self) -> bool {
            self.busy.load(Ordering::SeqCst)
        }

        fn wait(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
            Box::pin(async move {
                // `notify_one` без ждущего оставляет разрешение: тест,
                // подписавшийся позже, всё равно проснётся.
                self.entered.notify_one();
                self.release.notified().await;
                self.busy.store(false, Ordering::SeqCst);
            })
        }
    }

    /// Источник метаданных на снятых фикстурах формы, а не на живых
    /// ответах: форму разбирает и проверяет TL-55, здесь предмет — что
    /// конвейер с этой формой делает.
    struct Upstream {
        release_json: String,
        checksums: String,
        requests: Mutex<Vec<String>>,
    }

    impl Upstream {
        /// Апстрим, у которого последний релиз — `version` с архивом
        /// `body`.
        fn offering(version: &str, body: &[u8]) -> Self {
            let sha256 = sha256_of(body);
            let url = format!(
                "https://github.com/yt-dlp/yt-dlp/releases/download/{version}/{UPSTREAM_ASSET}"
            );
            let sums_url = format!(
                "https://github.com/yt-dlp/yt-dlp/releases/download/{version}/SHA2-256SUMS"
            );

            Self {
                release_json: format!(
                    r#"{{"tag_name":"{version}","draft":false,"prerelease":false,"assets":[
                        {{"name":"{UPSTREAM_ASSET}","size":{size},"browser_download_url":"{url}"}},
                        {{"name":"SHA2-256SUMS","size":1595,"browser_download_url":"{sums_url}"}}
                    ]}}"#,
                    size = body.len()
                ),
                checksums: format!("{sha256}  {UPSTREAM_ASSET}\n"),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn requests(&self) -> Vec<String> {
            self.requests.lock().expect("замок источника").clone()
        }
    }

    impl MetadataSource for Upstream {
        fn fetch(&self, request: &MetadataRequest) -> Result<Vec<u8>, MetadataError> {
            self.requests
                .lock()
                .expect("замок источника")
                .push(request.url.clone());

            if request.url == LATEST_RELEASE_URL {
                return Ok(self.release_json.clone().into_bytes());
            }
            if request.url.ends_with("SHA2-256SUMS") {
                return Ok(self.checksums.clone().into_bytes());
            }
            Err(MetadataError::Http {
                status: 404,
                reason: format!("фикстура не знает адреса {}", request.url),
            })
        }
    }

    /// Апстрим, до которого нет сети.
    struct Offline;

    impl MetadataSource for Offline {
        fn fetch(&self, _request: &MetadataRequest) -> Result<Vec<u8>, MetadataError> {
            Err(MetadataError::Offline {
                reason: "сеть выключена".to_string(),
            })
        }
    }

    /// Источник архивов, который считает открытия потока.
    ///
    /// Счётчик — не диагностика, а предмет половины утверждений: «не
    /// скачивает повторно» проверяется тем, что поток не открывался ни
    /// разу, и никаким другим способом изнутри не проверяется вовсе.
    struct Supply {
        body: Vec<u8>,
        opened: AtomicUsize,
        bundled_asked: AtomicUsize,
    }

    impl Supply {
        fn of(body: &[u8]) -> Self {
            Self {
                body: body.to_vec(),
                opened: AtomicUsize::new(0),
                bundled_asked: AtomicUsize::new(0),
            }
        }

        fn opened(&self) -> usize {
            self.opened.load(Ordering::SeqCst)
        }
    }

    impl ArchiveSupply for Supply {
        fn network<'a>(&'a self, asset: &UpdateAsset) -> Box<dyn ArchiveSource + Send + Sync + 'a> {
            Box::new(StreamArchive::new(
                Origin::Network,
                asset.url.clone(),
                asset.size_bytes,
                move || {
                    self.opened.fetch_add(1, Ordering::SeqCst);
                    Ok(Box::new(Cursor::new(self.body.clone())) as Box<dyn Read>)
                },
            ))
        }

        fn bundled(&self) -> Option<Box<dyn ArchiveSource + Send + Sync + '_>> {
            self.bundled_asked.fetch_add(1, Ordering::SeqCst);
            None
        }
    }

    /// Рандеву источника, который замирает посреди потока (TL-66).
    #[derive(Default)]
    struct Stall {
        parked: tokio::sync::Notify,
        released: Mutex<bool>,
        wake: std::sync::Condvar,
    }

    impl Stall {
        fn release(&self) {
            *self.released.lock().expect("замок рандеву") = true;
            self.wake.notify_all();
        }
    }

    /// Источник архива, который отдаёт половину тела и ждёт, пока тест его
    /// не отпустит: так на диске лежит `.download-*` идущего обновления.
    ///
    /// Приём архива синхронный (`blocking`), поэтому и ожидание здесь
    /// синхронное; о том, что поток замер, тест узнаёт через `Notify`.
    struct StalledSupply {
        body: Vec<u8>,
        stall: Arc<Stall>,
    }

    struct StallingReader {
        body: Vec<u8>,
        offset: usize,
        stalled: bool,
        stall: Arc<Stall>,
    }

    impl Read for StallingReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let half = self.body.len() / 2;
            if self.offset == half && !self.stalled {
                self.stalled = true;
                // `notify_one` без ждущего оставляет разрешение.
                self.stall.parked.notify_one();
                let mut released = self.stall.released.lock().expect("замок рандеву");
                while !*released {
                    released = self.stall.wake.wait(released).expect("замок рандеву");
                }
            }
            let end = if self.offset < half {
                half
            } else {
                self.body.len()
            };
            let n = buf.len().min(end - self.offset);
            buf[..n].copy_from_slice(&self.body[self.offset..self.offset + n]);
            self.offset += n;
            Ok(n)
        }
    }

    impl ArchiveSupply for StalledSupply {
        fn network<'a>(&'a self, asset: &UpdateAsset) -> Box<dyn ArchiveSource + Send + Sync + 'a> {
            Box::new(StreamArchive::new(
                Origin::Network,
                asset.url.clone(),
                asset.size_bytes,
                move || {
                    Ok(Box::new(StallingReader {
                        body: self.body.clone(),
                        offset: 0,
                        stalled: false,
                        stall: Arc::clone(&self.stall),
                    }) as Box<dyn Read>)
                },
            ))
        }

        fn bundled(&self) -> Option<Box<dyn ArchiveSource + Send + Sync + '_>> {
            None
        }
    }

    /// Каталог данных с раскладкой, реестром процессов и контуром.
    struct Fixture {
        _dir: TempDir,
        layout: Layout,
        registry: ChildRegistry,
        in_use: InUse,
        session: Session,
        controller: UpdateController,
    }

    fn fixture() -> Fixture {
        let dir = tempdir().expect("tempdir");
        let layout = Layout::new(&dir.path().join("app-data"));
        layout.create_root().expect("корень обязан создаваться");

        Fixture {
            _dir: dir,
            layout,
            registry: ChildRegistry::new(),
            in_use: InUse::new(),
            session: Session::new(),
            controller: UpdateController::new(),
        }
    }

    impl Fixture {
        /// Установка, которая распакована, проходит проверку дерева и
        /// отвечает на `--version` своей версией.
        fn install(&self, version: &str, sha256: &str) -> InstallEntry {
            let entry = InstallEntry::new(version, sha256).expect("идентификатор фикстуры");
            let dir = self.layout.install_dir(entry.build_id());
            write_file(
                &dir.join(EXECUTABLE),
                format!("#!/bin/sh\necho {version}\n").as_bytes(),
                true,
            );
            write_file(&dir.join("_internal/lib.so"), b"0123456789", false);

            let manifest =
                layout::manifest_for(&dir, EXECUTABLE, ArchiveIdentity { version, sha256 })
                    .expect("манифест обязан собираться");
            manifest
                .write_atomic(&self.layout.manifest_path(entry.build_id()))
                .expect("манифест обязан записываться");

            entry
        }

        /// Ломает исполняемый файл установки: дерево остаётся тем же (и
        /// проверку дерева проходит), а запуск падает. Так выглядит
        /// установка, которую вытеснил пин, — та самая, что уезжает в
        /// известно-хорошие без проверки годности.
        fn break_executable(&self, entry: &InstallEntry) {
            let path = self.layout.install_dir(entry.build_id()).join(EXECUTABLE);
            let size = fs::metadata(&path).expect("файл на месте").len() as usize;

            // Длина обязана совпасть до байта: манифест сверяет размеры
            // файлов, и разойдись она — дерево перестало бы проходить
            // `layout::validate`, то есть тест проверял бы отказ проверки
            // дерева вместо отказа запуска.
            let mut body = b"#!/bin/sh\nexit 1\n".to_vec();
            assert!(
                size >= body.len(),
                "фикстура сломанного файла не помещается в исходный размер"
            );
            body.resize(size, b'#');
            write_file(&path, &body, true);
        }

        fn record(&self, entries: &[InstallEntry]) {
            let mut state = InstallState::default();
            for entry in entries {
                state
                    .activate(&self.layout, entry.clone())
                    .expect("запись обязана сохраняться");
            }
        }

        fn state(&self) -> InstallState {
            InstallState::load(&self.layout)
        }

        /// Каталог данных, из которого собрана [`Self::layout`], — вход
        /// подготовки.
        fn data_dir(&self) -> PathBuf {
            self._dir.path().join("app-data")
        }

        /// Путь вложенного архива для подготовки. Файла нет: активная
        /// установка фикстуры готовится без него, а до резерва дело не
        /// доходит.
        fn bundled_archive(&self) -> PathBuf {
            self._dir.path().join("bundled-yt-dlp.zip")
        }

        fn active_version(&self) -> Option<String> {
            self.state()
                .active()
                .map(|entry| entry.version().to_string())
        }

        fn known_good_version(&self) -> Option<String> {
            self.state()
                .known_good()
                .map(|entry| entry.version().to_string())
        }

        fn install_dirs(&self) -> BTreeSet<String> {
            fs::read_dir(self.layout.root())
                .expect("корень читается")
                .filter_map(Result::ok)
                .filter(|entry| entry.path().is_dir())
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        }

        fn pipeline<'a>(
            &'a self,
            metadata: &'a (dyn MetadataSource + Send + Sync),
            archives: &'a dyn ArchiveSupply,
            sink: &'a dyn UpdateSink,
            boundary: &'a dyn TaskBoundary,
        ) -> Pipeline<'a> {
            Pipeline {
                layout: &self.layout,
                metadata,
                archives,
                registry: &self.registry,
                in_use: &self.in_use,
                session: &self.session,
                sink,
                boundary,
            }
        }

        /// Заполняет память сеанса так, как её оставил бы тёплый старт на
        /// активной установке `version` (TL-23).
        fn remember_warm_start(&self, version: &str) {
            let executable = self.layout.root().join("remembered").join(EXECUTABLE);
            self.session.remember_for_test(
                crate::types::YtDlpPrepared {
                    version: version.to_string(),
                    path: executable.display().to_string(),
                    prepared: false,
                    duration_ms: 1,
                },
                super::super::prepare::WarmLaunch {
                    executable,
                    output: crate::sidecar::RunOutput {
                        stdout: format!("{version}\n"),
                        stderr: String::new(),
                    },
                    checked_at: crate::clock::now_iso8601(),
                    duration_ms: 1,
                },
            );
            assert!(self.session.remembers_anything());
        }
    }

    fn write_file(path: &Path, contents: &[u8], executable: bool) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("каталог фикстуры");
        }
        fs::write(path, contents).expect("файл фикстуры");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = if executable { 0o755 } else { 0o644 };
            fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("права фикстуры");
        }
        #[cfg(not(unix))]
        let _ = executable;
    }

    fn sha256_of(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(bytes)
            .iter()
            .fold(String::new(), |mut hex, byte| {
                use std::fmt::Write as _;
                let _ = write!(hex, "{byte:02x}");
                hex
            })
    }

    /// Архив формы апстримного onedir-ассета, чей yt-dlp отвечает
    /// `version` на `--version`.
    fn onedir_zip(version: &str) -> Vec<u8> {
        onedir_zip_recording(version, None)
    }

    /// То же, но каждый запуск оставляет строку в `journal`.
    ///
    /// Нужен там, где предмет проверки — «кандидат больше не
    /// запускается»: другого способа увидеть это изнутри нет, а по
    /// косвенным признакам (скачался ли архив) утверждение получается
    /// ложным — распакованное дерево остаётся на диске и второй раз не
    /// качается независимо ни от каких запретов.
    fn onedir_zip_recording(version: &str, journal: Option<&Path>) -> Vec<u8> {
        let record = journal.map_or_else(String::new, |path| {
            format!("echo run >> {}\n", path.display())
        });
        onedir_zip_body(&format!("#!/bin/sh\n{record}echo {version}\n"))
    }

    fn onedir_zip_body(script: &str) -> Vec<u8> {
        let mut buffer = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut buffer);
            let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);

            zip.start_file(EXECUTABLE, stored.unix_permissions(0o755))
                .expect("start_file");
            zip.write_all(script.as_bytes()).expect("write");

            zip.add_directory("_internal/", stored.unix_permissions(0o755))
                .expect("add_directory");
            zip.start_file("_internal/lib.so", stored.unix_permissions(0o644))
                .expect("start_file");
            zip.write_all(b"shared-library-bytes").expect("write");

            zip.finish().expect("finish");
        }
        buffer.into_inner()
    }

    /// Идентификатор кандидата, который приедет из фикстуры апстрима.
    fn candidate_build_id(body: &[u8]) -> BuildId {
        BuildId::new(CANDIDATE, &sha256_of(body)).expect("идентификатор кандидата")
    }

    // ─────────────────── расписание и троттлинг ───────────────────

    /// Состояние контура с проставленными отметками времени — предмет
    /// проверок троттлинга.
    fn state_with(last_check: Option<Instant>, last_broken: Option<Instant>) -> ControllerState {
        ControllerState {
            status: YtDlpUpdateStatus::NeverChecked,
            rollback_target: None,
            last_check,
            last_broken_extraction_check: last_broken,
            paused: BTreeSet::new(),
        }
    }

    #[test]
    fn a_planned_check_waits_for_its_interval() {
        let start = Instant::now();
        let state = state_with(Some(start), None);

        assert!(
            !throttle_allows(
                &state,
                CheckTrigger::Planned,
                start + Duration::from_secs(60)
            ),
            "минуту спустя плановой проверке ещё рано"
        );
        assert!(
            throttle_allows(
                &state,
                CheckTrigger::Planned,
                start + PLANNED_CHECK_INTERVAL
            ),
            "по истечении интервала плановая проверка обязана пройти"
        );
    }

    #[test]
    fn a_broken_extraction_check_ignores_the_planned_counter() {
        // Критерий приёмки задачи и главное утверждение о раздельности
        // троттлингов: плановая проверка была только что и ничего не
        // нашла, а скачивание сломалось сейчас — именно ради этого
        // случая С-13 и существует. Общий счётчик заглушил бы его
        // полностью.
        let start = Instant::now();
        let state = state_with(Some(start), None);
        let moment = start + Duration::from_secs(60);

        assert!(
            !throttle_allows(&state, CheckTrigger::Planned, moment),
            "предусловие теста: плановой проверке в этот момент рано"
        );
        assert!(
            throttle_allows(&state, CheckTrigger::BrokenExtraction, moment),
            "внеплановая проверка по сломанному извлечению не должна зависеть от \
             планового счётчика"
        );
    }

    #[test]
    fn a_second_broken_extraction_check_waits_for_its_own_pause() {
        // Обратная сторона того же: раздельный счётчик не значит
        // «без счётчика». Десять неудачных задач подряд не превращаются
        // в десять запросов к апстриму.
        let start = Instant::now();
        let state = state_with(None, Some(start));

        assert!(
            !throttle_allows(
                &state,
                CheckTrigger::BrokenExtraction,
                start + Duration::from_secs(60)
            ),
            "минуту спустя второй внеплановой проверке рано"
        );
        assert!(
            throttle_allows(
                &state,
                CheckTrigger::BrokenExtraction,
                start + BROKEN_EXTRACTION_THROTTLE
            ),
            "по истечении своей паузы внеплановая проверка обязана пройти"
        );
    }

    #[test]
    fn a_manual_check_ignores_every_throttle() {
        // С-12: ручная проверка работает независимо от расписания
        // автопроверок. Обе отметки стоят «только что».
        let start = Instant::now();
        let state = state_with(Some(start), Some(start));

        assert!(throttle_allows(&state, CheckTrigger::Manual, start));
    }

    #[test]
    fn any_check_moves_the_planned_counter_but_only_c13_moves_its_own() {
        // Бюджет запросов один на всех (60 в час, замер TL-55), поэтому
        // плановый счётчик двигает любое обращение к апстриму. Счётчик
        // С-13 — только своя причина, иначе раздельность была бы
        // односторонней.
        let controller = UpdateController::new();
        let now = Instant::now();

        controller.note_checked(CheckTrigger::Manual, now);
        {
            let state = controller.lock();
            assert_eq!(state.last_check, Some(now));
            assert_eq!(state.last_broken_extraction_check, None);
        }

        controller.note_checked(CheckTrigger::BrokenExtraction, now);
        {
            let state = controller.lock();
            assert_eq!(state.last_broken_extraction_check, Some(now));
        }
    }

    #[test]
    fn a_busy_contour_starts_no_second_pipeline() {
        // Дизайн гасит обе кнопки, пока проверка или откат идут, и то же
        // правило обязано держать ядро: «не плодим параллельные пробы
        // поверх уже идущей».
        let controller = UpdateController::new();
        controller
            .begin_manual_check()
            .expect("первая проверка обязана занять контур");

        let second = controller
            .begin_manual_check()
            .expect_err("вторая обязана быть отклонена");
        assert_eq!(second.kind, YtDlpUpdateCommandErrorKind::Busy);
        assert!(
            controller
                .begin_background_check(CheckTrigger::Manual, Instant::now())
                .is_none(),
            "фоновая проверка на занятом контуре тоже не начинается"
        );
    }

    #[test]
    fn a_rollback_without_a_target_is_refused_by_the_core() {
        // Кнопки без цели фронтенд не рисует, но защита ядра обязана быть
        // настоящей: контракт объявляет для этого отдельный класс.
        let controller = UpdateController::new();
        let error = controller
            .begin_rollback()
            .expect_err("возвращаться некуда");
        assert_eq!(error.kind, YtDlpUpdateCommandErrorKind::NothingToRollBackTo);
    }

    // ─────────────────── точка сравнения и С-8 ───────────────────

    #[test]
    fn the_comparison_base_is_the_newest_installation_on_disk() {
        let fixture = fixture();
        let older = fixture.install(OLDER, &"a1".repeat(32));
        let newer = fixture.install(CANDIDATE, &"b2".repeat(32));

        // Обычная жизнь: активная новее известно-хорошей.
        fixture.record(&[older.clone(), newer.clone()]);
        let base = comparison_base(&fixture.state()).expect("точка сравнения");
        assert_eq!(base.as_str(), CANDIDATE);
        assert!(base_is_active(&fixture.state(), &base));

        // После отката: активной становится старая, а самой новой на
        // диске остаётся та, от которой отказались.
        fixture.record(&[older.clone(), newer.clone(), older.clone()]);
        assert_eq!(fixture.active_version().as_deref(), Some(OLDER));
        let base = comparison_base(&fixture.state()).expect("точка сравнения");
        assert_eq!(
            base.as_str(),
            CANDIDATE,
            "после отката сравнивать надо с отвергнутой версией, иначе контур \
             поставит её обратно на первой же проверке (С-8)"
        );
        assert!(
            !base_is_active(&fixture.state(), &base),
            "точка сравнения разошлась с активной — ровно этим и отличается откат"
        );
    }

    #[test]
    fn the_rollback_target_is_the_known_good_installation() {
        let fixture = fixture();
        let older = fixture.install(OLDER, &"a1".repeat(32));
        let newer = fixture.install(CANDIDATE, &"b2".repeat(32));

        fixture.record(std::slice::from_ref(&older));
        assert_eq!(
            rollback_target(&fixture.state()),
            None,
            "до первого переключения возвращаться некуда"
        );

        fixture.record(&[older, newer]);
        assert_eq!(
            rollback_target(&fixture.state()).as_deref(),
            Some(OLDER),
            "цель отката — известно-хорошая установка"
        );
    }

    #[test]
    fn the_percentage_never_leaves_the_hundred() {
        assert_eq!(percent_of(0, 100), 0);
        assert_eq!(percent_of(50, 100), 50);
        assert_eq!(percent_of(100, 100), 100);
        // Источник вправе отдать больше объявленного — на проводе это не
        // должно превращаться в «103 %».
        assert_eq!(percent_of(300, 100), 100);
        // Знаменателя нет: делить не на что, и полосе показывать нечего.
        assert_eq!(percent_of(10, 0), 0);
    }

    // ─────────────────── конвейер целиком ───────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn a_new_release_is_fetched_checked_and_switched_to() {
        // С-1 целиком: нашли, скачали, сверили сумму, распаковали,
        // запустили, переключили. Проверяется и порядок этапов (по нему
        // фронтенд рисует строки 2, 4, 5, 7), и итог на диске.
        let fixture = fixture();
        let active = fixture.install(ACTIVE, &"a1".repeat(32));
        fixture.record(&[active]);

        let body = onedir_zip(CANDIDATE);
        let upstream = Upstream::offering(CANDIDATE, &body);
        let supply = Supply::of(&body);
        let sink = RecordingSink::default();
        let boundary = Boundary::idle(&fixture.layout);

        fixture
            .controller
            .begin_manual_check()
            .expect("контур свободен");
        fixture
            .controller
            .run_check(
                &fixture.pipeline(&upstream, &supply, &sink, &boundary),
                CheckTrigger::Manual,
                Instant::now(),
            )
            .await;

        assert_eq!(
            sink.stages().first().copied(),
            Some("checking"),
            "первое, что видит подписчик, — что проверка идёт (строка 2)"
        );
        assert!(
            sink.stages().contains(&"downloading"),
            "этап скачивания обязан доехать до подписчика: {:?}",
            sink.stages()
        );
        assert!(
            sink.stages().contains(&"preparing"),
            "этап подготовки обязан доехать до подписчика: {:?}",
            sink.stages()
        );
        assert_eq!(sink.stages().last().copied(), Some("updated"));

        match sink.last() {
            YtDlpUpdateStatus::Updated { version, .. } => assert_eq!(version, CANDIDATE),
            other => panic!("ожидалось переключение, а не {other:?}"),
        }

        assert_eq!(fixture.active_version().as_deref(), Some(CANDIDATE));
        assert_eq!(fixture.known_good_version().as_deref(), Some(ACTIVE));
        assert_eq!(
            sink.snapshots()
                .last()
                .and_then(|s| s.rollback_target.clone()),
            Some(ACTIVE.to_string()),
            "после переключения кнопка отката обязана указывать на прежнюю версию"
        );
        assert_eq!(supply.opened(), 1, "архив скачивается ровно один раз");

        // Полоса: доходит до конца и ни разу не едет назад. Процент —
        // единственное число, которое контур показывает пользователю, и
        // «103 %» или откат назад были бы видны прямо на экране.
        let percents: Vec<u8> = sink
            .statuses()
            .iter()
            .filter_map(|status| match status {
                YtDlpUpdateStatus::Downloading { percent, .. } => Some(percent.value()),
                _ => None,
            })
            .collect();
        assert_eq!(percents.first().copied(), Some(0));
        assert_eq!(percents.last().copied(), Some(100));
        assert!(
            percents.windows(2).all(|pair| pair[0] <= pair[1]),
            "процент обязан только расти: {percents:?}"
        );
        assert_eq!(
            upstream.requests().len(),
            2,
            "проверка, нашедшая обновление, стоит два запроса: метаданные и файл сумм"
        );
        assert_eq!(
            fixture.install_dirs().len(),
            2,
            "на диске остаются ровно две установки (Ф-8): {:?}",
            fixture.install_dirs()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_active_download_delays_the_switch_until_the_task_boundary() {
        // Критерий приёмки: во время активной загрузки контур не
        // переключает активную запись раньше паузы между задачами (Ф-7,
        // Р-2). Проверяется не итогом — итог одинаков в обоих случаях, —
        // а тем, что было записано в момент ожидания.
        let fixture = fixture();
        let active = fixture.install(ACTIVE, &"a1".repeat(32));
        fixture.record(&[active]);

        let body = onedir_zip(CANDIDATE);
        let upstream = Upstream::offering(CANDIDATE, &body);
        let supply = Supply::of(&body);
        let sink = RecordingSink::default();
        let boundary = Boundary::busy(&fixture.layout);

        fixture
            .controller
            .begin_manual_check()
            .expect("контур свободен");
        fixture
            .controller
            .run_check(
                &fixture.pipeline(&upstream, &supply, &sink, &boundary),
                CheckTrigger::Manual,
                Instant::now(),
            )
            .await;

        let waits = boundary.waits();
        assert!(
            !waits.is_empty(),
            "конвейер обязан дождаться паузы между задачами, а не идти напролом"
        );
        for observed in &waits {
            assert_eq!(
                observed.as_deref(),
                Some(ACTIVE),
                "пока задача идёт, активной обязана оставаться прежняя версия"
            );
        }

        assert!(
            sink.stages().contains(&"readyWaiting"),
            "пользователю обязано быть видно, что обновление ждёт границы задач \
             (строка 6): {:?}",
            sink.stages()
        );
        assert_eq!(sink.stages().last().copied(), Some("updated"));
        assert_eq!(fixture.active_version().as_deref(), Some(CANDIDATE));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn nothing_newer_upstream_costs_one_request_and_touches_no_disk() {
        // С-2: проверка ограничивается метаданными — архив не
        // скачивается, диск не трогается.
        let fixture = fixture();
        let active = fixture.install(ACTIVE, &"a1".repeat(32));
        fixture.record(&[active]);

        let body = onedir_zip(ACTIVE);
        let upstream = Upstream::offering(ACTIVE, &body);
        let supply = Supply::of(&body);
        let sink = RecordingSink::default();
        let boundary = Boundary::idle(&fixture.layout);
        let before = fixture.install_dirs();

        fixture.controller.begin_manual_check().expect("свободен");
        fixture
            .controller
            .run_check(
                &fixture.pipeline(&upstream, &supply, &sink, &boundary),
                CheckTrigger::Manual,
                Instant::now(),
            )
            .await;

        assert_eq!(sink.stages().last().copied(), Some("upToDate"));
        assert_eq!(supply.opened(), 0, "качать нечего");
        assert_eq!(
            upstream.requests().len(),
            1,
            "проверка «уже последняя» стоит один запрос — на этом свойстве стоит \
             расписание"
        );
        assert_eq!(fixture.install_dirs(), before);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_abandoned_release_is_not_installed_again_by_itself() {
        // С-8: build id, с которого откатились, не переустанавливается
        // автоматически до следующего релиза апстрима. Апстрим при этом
        // честно отвечает, что последняя версия — та самая.
        let fixture = fixture();
        let older = fixture.install(OLDER, &"a1".repeat(32));
        let abandoned = fixture.install(CANDIDATE, &"b2".repeat(32));
        // Обновились, потом откатились: активной стала старая.
        fixture.record(&[older.clone(), abandoned, older]);

        let body = onedir_zip(CANDIDATE);
        let upstream = Upstream::offering(CANDIDATE, &body);
        let supply = Supply::of(&body);
        let sink = RecordingSink::default();
        let boundary = Boundary::idle(&fixture.layout);

        fixture.controller.begin_manual_check().expect("свободен");
        fixture
            .controller
            .run_check(
                &fixture.pipeline(&upstream, &supply, &sink, &boundary),
                CheckTrigger::Manual,
                Instant::now(),
            )
            .await;

        assert_eq!(supply.opened(), 0, "отвергнутая версия не качается заново");
        assert_eq!(
            fixture.active_version().as_deref(),
            Some(OLDER),
            "активной обязана остаться та, на которую откатились"
        );
        assert_eq!(
            sink.stages().last().copied(),
            Some("rolledBack"),
            "показывать «установлена последняя версия» здесь было бы неправдой: \
             последняя — как раз отвергнутая ({:?})",
            sink.stages()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_unreachable_upstream_leaves_the_active_installation_alone() {
        // С-3 и Н-2: отказ проверки не трогает работу на текущей версии
        // и не превращается в вечный спиннер.
        let fixture = fixture();
        let active = fixture.install(ACTIVE, &"a1".repeat(32));
        fixture.record(&[active]);

        let body = onedir_zip(CANDIDATE);
        let supply = Supply::of(&body);
        let sink = RecordingSink::default();
        let boundary = Boundary::idle(&fixture.layout);

        fixture.controller.begin_manual_check().expect("свободен");
        fixture
            .controller
            .run_check(
                &fixture.pipeline(&Offline, &supply, &sink, &boundary),
                CheckTrigger::Manual,
                Instant::now(),
            )
            .await;

        match sink.last() {
            YtDlpUpdateStatus::Failed {
                failure: YtDlpUpdateFailure::NetworkUnavailable { .. },
                ..
            } => {}
            other => panic!("ожидался класс «нет сети», а не {other:?}"),
        }
        assert!(
            !sink.last().busy(),
            "терминальный исход обязан снимать занятость — иначе кнопки останутся \
             мёртвыми навсегда"
        );
        assert_eq!(fixture.active_version().as_deref(), Some(ACTIVE));
        assert_eq!(supply.opened(), 0);
    }

    // ─────────────────── запреты до сети ───────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn a_release_that_already_failed_the_smoke_check_is_not_fetched_again() {
        // С-5: тот же build id не устанавливается повторно **сам**.
        // Спросить журнал обязан тот, кто затевает скачивание, — иначе к
        // моменту, когда smoke могла бы отказать сама, шестьдесят
        // мегабайт уже приняты и распакованы.
        //
        // Проверка здесь фоновая: ручная запрет снимает намеренно, и это
        // предмет соседнего теста.
        let fixture = fixture();
        let active = fixture.install(ACTIVE, &"a1".repeat(32));
        fixture.record(&[active]);

        let body = onedir_zip(CANDIDATE);
        let build_id = candidate_build_id(&body);
        RepairLog::empty()
            .with_attempt("не запускается", 1_000)
            .write_atomic(&fixture.layout.smoke_path(&build_id))
            .expect("журнал обязан записываться");

        let upstream = Upstream::offering(CANDIDATE, &body);
        let supply = Supply::of(&body);
        let sink = RecordingSink::default();
        let boundary = Boundary::idle(&fixture.layout);

        fixture.controller.begin_manual_check().expect("свободен");
        fixture
            .controller
            .run_check(
                &fixture.pipeline(&upstream, &supply, &sink, &boundary),
                CheckTrigger::Planned,
                Instant::now(),
            )
            .await;

        assert_eq!(supply.opened(), 0, "поток не должен открываться вовсе");
        match sink.last() {
            YtDlpUpdateStatus::Failed {
                failure: YtDlpUpdateFailure::SmokeCheckFailed { version, .. },
                ..
            } => assert_eq!(version, CANDIDATE),
            other => panic!("ожидался класс «не прошла проверку запуска», а не {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_manual_check_lifts_the_ban_and_tries_the_candidate_once_more() {
        // Второй выход из запрета С-5 — «действие пользователя» (первый:
        // новый релиз апстрима). Без него запрет вечен, а С-5 обещает
        // обратное: `smoke::forget` для того и объявлен публичным, а кто
        // именно его зовёт, TL-57 оставила решить этой задаче.
        let fixture = fixture();
        let active = fixture.install(ACTIVE, &"a1".repeat(32));
        fixture.record(&[active]);

        let body = onedir_zip(CANDIDATE);
        let build_id = candidate_build_id(&body);
        RepairLog::empty()
            .with_attempt("не запускается", 1_000)
            .write_atomic(&fixture.layout.smoke_path(&build_id))
            .expect("журнал обязан записываться");

        let upstream = Upstream::offering(CANDIDATE, &body);
        let supply = Supply::of(&body);
        let sink = RecordingSink::default();
        let boundary = Boundary::idle(&fixture.layout);

        fixture.controller.begin_manual_check().expect("свободен");
        fixture
            .controller
            .run_check(
                &fixture.pipeline(&upstream, &supply, &sink, &boundary),
                CheckTrigger::Manual,
                Instant::now(),
            )
            .await;

        assert_eq!(
            supply.opened(),
            1,
            "пользователь попросил проверить — кандидат обязан получить второй шанс"
        );
        assert_eq!(sink.stages().last().copied(), Some("updated"));
        assert_eq!(fixture.active_version().as_deref(), Some(CANDIDATE));
        assert!(
            smoke::previous_failure(&fixture.layout, &build_id).is_none(),
            "запрет обязан быть снят, а не обойдён: иначе следующая фоновая \
             проверка снова считала бы кандидата запрещённым"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_release_that_failed_three_times_is_not_fetched_until_it_cools_down() {
        // С-4: тот же ассет не перекачивается в бесконечном цикле.
        let fixture = fixture();
        let active = fixture.install(ACTIVE, &"a1".repeat(32));
        fixture.record(&[active]);

        let body = onedir_zip(CANDIDATE);
        let build_id = candidate_build_id(&body);
        let now = crate::clock::now_unix_secs();
        let exhausted = (0..MAX_FETCH_ATTEMPTS).fold(RepairLog::empty(), |log, _| {
            log.with_attempt("архив битый", now)
        });
        exhausted
            .write_atomic(&fixture.layout.update_attempt_path(&build_id))
            .expect("журнал обязан записываться");

        let upstream = Upstream::offering(CANDIDATE, &body);
        let supply = Supply::of(&body);
        let sink = RecordingSink::default();
        let boundary = Boundary::idle(&fixture.layout);

        fixture.controller.begin_manual_check().expect("свободен");
        fixture
            .controller
            .run_check(
                &fixture.pipeline(&upstream, &supply, &sink, &boundary),
                CheckTrigger::Planned,
                Instant::now(),
            )
            .await;

        assert_eq!(supply.opened(), 0, "исчерпанные попытки не открывают поток");
        assert!(
            matches!(
                sink.last(),
                YtDlpUpdateStatus::Failed {
                    failure: YtDlpUpdateFailure::ArchiveCorrupted { .. },
                    ..
                }
            ),
            "ожидался класс «архив повреждён», а не {:?}",
            sink.last()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_candidate_whose_ban_could_not_be_written_is_held_in_memory() {
        // Решение ведущего по итогам TL-57: если запись журнала не
        // удалась (нет прав, полный диск), паузу держит оркестрация —
        // иначе тот же кандидат приедет снова, и это узор долга #22.
        //
        // Отказ записи воспроизводится каталогом на месте файла журнала:
        // такой путь не открывается на запись ни при каких правах.
        //
        // Предмет проверки — **число запусков кандидата**, а не число
        // скачиваний, и это различие пришлось выяснить мутацией. По
        // скачиваниям утверждение получается ложным: распакованное
        // дерево провалившегося кандидата остаётся на диске (уборка
        // ходит только после удачного переключения), и следующая
        // проверка его не качает независимо ни от каких запретов —
        // она просто запускает его снова. Ровно это и есть #22:
        // тридцать секунд дисковой работы по расписанию до нового
        // релиза апстрима.
        let fixture = fixture();
        let active = fixture.install(ACTIVE, &"a1".repeat(32));
        fixture.record(&[active]);

        // Кандидат, который распакуется и запустится, но назовёт чужую
        // версию, — самый дешёвый способ провалить проверку запуска.
        let launches = fixture._dir.path().join("launches");
        let body = onedir_zip_recording("1999.01.01", Some(&launches));
        let build_id = candidate_build_id(&body);
        fs::create_dir_all(fixture.layout.smoke_path(&build_id))
            .expect("каталог на месте журнала обязан создаваться");

        let upstream = Upstream::offering(CANDIDATE, &body);
        let supply = Supply::of(&body);
        let boundary = Boundary::idle(&fixture.layout);

        let first = RecordingSink::default();
        fixture.controller.begin_manual_check().expect("свободен");
        fixture
            .controller
            .run_check(
                &fixture.pipeline(&upstream, &supply, &first, &boundary),
                CheckTrigger::Planned,
                Instant::now(),
            )
            .await;

        assert_eq!(supply.opened(), 1, "первый раз кандидат скачивается честно");
        assert_eq!(launched(&launches), 1, "и один раз запускается");
        assert!(
            matches!(
                first.last(),
                YtDlpUpdateStatus::Failed {
                    failure: YtDlpUpdateFailure::SmokeCheckFailed { .. },
                    ..
                }
            ),
            "ожидался провал проверки запуска, а не {:?}",
            first.last()
        );
        assert!(
            smoke::previous_failure(&fixture.layout, &build_id).is_none(),
            "предусловие теста: запрет на диск записать не удалось"
        );

        let second = RecordingSink::default();
        fixture.controller.begin_manual_check().expect("свободен");
        fixture
            .controller
            .run_check(
                &fixture.pipeline(&upstream, &supply, &second, &boundary),
                CheckTrigger::Planned,
                Instant::now(),
            )
            .await;

        assert_eq!(
            launched(&launches),
            1,
            "второй раз кандидат запускаться не должен: запрет держится в памяти"
        );
        assert_eq!(supply.opened(), 1, "и качаться заново тоже не должен");
        assert_eq!(fixture.active_version().as_deref(), Some(ACTIVE));
    }

    /// Сколько раз запускался кандидат — по строкам, которые он сам за
    /// собой пишет.
    fn launched(journal: &Path) -> usize {
        fs::read_to_string(journal)
            .map(|text| text.lines().count())
            .unwrap_or(0)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_already_unpacked_candidate_is_not_fetched_again() {
        // С-7: приложение убили между распаковкой и переключением.
        // Следующий запуск доводит обновление, не платя второй раз за
        // шестьдесят мегабайт и не требуя ручного вмешательства.
        let fixture = fixture();
        let active = fixture.install(ACTIVE, &"a1".repeat(32));
        fixture.record(&[active]);

        let body = onedir_zip(CANDIDATE);
        // «Подготовлена, но не переключена»: дерево и манифест на месте,
        // запись про кандидата молчит.
        fixture.install(CANDIDATE, &sha256_of(&body));

        let upstream = Upstream::offering(CANDIDATE, &body);
        let supply = Supply::of(&body);
        let sink = RecordingSink::default();
        let boundary = Boundary::idle(&fixture.layout);

        fixture.controller.begin_manual_check().expect("свободен");
        fixture
            .controller
            .run_check(
                &fixture.pipeline(&upstream, &supply, &sink, &boundary),
                CheckTrigger::Manual,
                Instant::now(),
            )
            .await;

        assert_eq!(
            supply.opened(),
            0,
            "распакованный кандидат не скачивается заново"
        );
        assert_eq!(sink.stages().last().copied(), Some("updated"));
        assert_eq!(fixture.active_version().as_deref(), Some(CANDIDATE));
    }

    // ─────────────────── откат (Р-3) ───────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn a_rollback_switches_to_the_known_good_installation_and_swaps_the_target() {
        // Р-3 и «цель отката меняется местами сама»: после возврата к X
        // известно-хорошей становится Y — та, от которой отказались.
        let fixture = fixture();
        let older = fixture.install(OLDER, &"a1".repeat(32));
        let newer = fixture.install(CANDIDATE, &"b2".repeat(32));
        fixture.record(&[older, newer]);

        let sink = RecordingSink::default();
        let boundary = Boundary::idle(&fixture.layout);
        let supply = Supply::of(b"");

        fixture.controller.refresh_rollback_target(&fixture.layout);
        fixture
            .controller
            .begin_rollback()
            .expect("возвращаться есть куда");
        fixture
            .controller
            .run_rollback(
                &fixture.pipeline(&Offline, &supply, &sink, &boundary),
                tokio::sync::oneshot::channel().0,
            )
            .await;

        match sink.last() {
            YtDlpUpdateStatus::RolledBack {
                active, abandoned, ..
            } => {
                assert_eq!(active, OLDER);
                assert_eq!(abandoned, CANDIDATE);
            }
            other => panic!("ожидался выполненный возврат, а не {other:?}"),
        }
        assert_eq!(fixture.active_version().as_deref(), Some(OLDER));
        assert_eq!(fixture.known_good_version().as_deref(), Some(CANDIDATE));
        assert_eq!(
            sink.snapshots()
                .last()
                .and_then(|s| s.rollback_target.clone()),
            Some(CANDIDATE.to_string()),
            "кнопка обязана вести обратно: откат — переключатель, а не действие в \
             одну сторону"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_rollback_makes_the_session_forget_the_version_it_remembered() {
        // TL-23: служебный экран берёт версию из запуска пробы подготовки.
        // После отката приложение работает другой установкой, и помнить
        // прежнюю версию сеанс не вправе. Откат не принимает архива, то есть
        // сброс здесь — только от переключения.
        let fixture = fixture();
        let older = fixture.install(OLDER, &"a1".repeat(32));
        let newer = fixture.install(CANDIDATE, &"b2".repeat(32));
        fixture.record(&[older, newer]);
        fixture.remember_warm_start(CANDIDATE);

        let sink = RecordingSink::default();
        let boundary = Boundary::idle(&fixture.layout);
        let supply = Supply::of(b"");

        fixture.controller.refresh_rollback_target(&fixture.layout);
        fixture
            .controller
            .begin_rollback()
            .expect("возвращаться есть куда");
        fixture
            .controller
            .run_rollback(
                &fixture.pipeline(&Offline, &supply, &sink, &boundary),
                tokio::sync::oneshot::channel().0,
            )
            .await;

        assert_eq!(stage_name(&sink.last()), "rolledBack");
        assert!(
            !fixture.session.remembers_anything(),
            "после отката сеанс обязан забыть версию прежней установки"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_update_makes_the_session_forget_the_version_it_remembered() {
        let fixture = fixture();
        let active = fixture.install(ACTIVE, &"a1".repeat(32));
        fixture.record(&[active]);
        fixture.remember_warm_start(ACTIVE);

        let body = onedir_zip(CANDIDATE);
        let upstream = Upstream::offering(CANDIDATE, &body);
        let supply = Supply::of(&body);
        let sink = RecordingSink::default();
        let boundary = Boundary::idle(&fixture.layout);

        fixture.controller.begin_manual_check().expect("свободен");
        fixture
            .controller
            .run_check(
                &fixture.pipeline(&upstream, &supply, &sink, &boundary),
                CheckTrigger::Manual,
                Instant::now(),
            )
            .await;

        assert_eq!(stage_name(&sink.last()), "updated");
        assert!(!fixture.session.remembers_anything());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_installed_candidate_makes_the_session_forget_even_if_it_is_not_switched_to() {
        // Кандидат распакован, но не прошёл smoke: активная запись прежняя,
        // а корень установок изменился — сеанс помнит только то, что верно
        // для корня целиком (см. комментарий в `install`). Переключения
        // здесь нет, поэтому сброс — только от установки.
        let fixture = fixture();
        let active = fixture.install(ACTIVE, &"a1".repeat(32));
        fixture.record(&[active]);
        fixture.remember_warm_start(ACTIVE);

        // Архив отвечает не той версией, которую обещают метаданные.
        let body = onedir_zip(OLDER);
        let upstream = Upstream::offering(CANDIDATE, &body);
        let supply = Supply::of(&body);
        let sink = RecordingSink::default();
        let boundary = Boundary::idle(&fixture.layout);

        fixture.controller.begin_manual_check().expect("свободен");
        fixture
            .controller
            .run_check(
                &fixture.pipeline(&upstream, &supply, &sink, &boundary),
                CheckTrigger::Manual,
                Instant::now(),
            )
            .await;

        assert!(
            matches!(
                sink.last(),
                YtDlpUpdateStatus::Failed {
                    failure: YtDlpUpdateFailure::SmokeCheckFailed { .. },
                    ..
                }
            ),
            "предусловие: кандидат не прошёл smoke, а не {:?}",
            sink.last()
        );
        assert_eq!(fixture.active_version().as_deref(), Some(ACTIVE));
        assert!(!fixture.session.remembers_anything());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_rollback_to_an_installation_that_does_not_launch_is_refused() {
        // Перенесено из TL-54: в резерв уезжает вытесненная активная, а
        // вытеснить её мог отказ запуска — значит известно-хорошей
        // способна оказаться заведомо сломанная установка. Проверку
        // годности делает тот, кто откатывается, и делает её запуском:
        // дерево у такой установки целое, и `layout::validate` её
        // пропускает.
        let fixture = fixture();
        let broken = fixture.install(OLDER, &"a1".repeat(32));
        let newer = fixture.install(CANDIDATE, &"b2".repeat(32));
        fixture.record(&[broken.clone(), newer]);
        fixture.break_executable(&broken);

        assert!(
            layout::validate(&fixture.layout, broken.build_id()).is_ok(),
            "предусловие теста: сломанная установка проходит проверку дерева — \
             иначе тест проверял бы не то"
        );

        let sink = RecordingSink::default();
        let boundary = Boundary::idle(&fixture.layout);
        let supply = Supply::of(b"");

        fixture.controller.refresh_rollback_target(&fixture.layout);
        fixture.controller.begin_rollback().expect("цель есть");
        fixture
            .controller
            .run_rollback(
                &fixture.pipeline(&Offline, &supply, &sink, &boundary),
                tokio::sync::oneshot::channel().0,
            )
            .await;

        match sink.last() {
            YtDlpUpdateStatus::Failed {
                failure: YtDlpUpdateFailure::SmokeCheckFailed { version, .. },
                ..
            } => assert_eq!(version, OLDER),
            other => panic!("ожидался отказ проверки запуска, а не {other:?}"),
        }
        assert_eq!(
            fixture.active_version().as_deref(),
            Some(CANDIDATE),
            "активная установка обязана остаться прежней"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_rollback_waits_for_the_task_boundary_too() {
        // Ф-7 не различает обновление и откат: версия не меняется
        // посреди задачи ни в ту, ни в другую сторону.
        let fixture = fixture();
        let older = fixture.install(OLDER, &"a1".repeat(32));
        let newer = fixture.install(CANDIDATE, &"b2".repeat(32));
        fixture.record(&[older, newer]);

        let sink = RecordingSink::default();
        let boundary = Boundary::busy(&fixture.layout);
        let supply = Supply::of(b"");

        fixture.controller.refresh_rollback_target(&fixture.layout);
        fixture.controller.begin_rollback().expect("цель есть");
        fixture
            .controller
            .run_rollback(
                &fixture.pipeline(&Offline, &supply, &sink, &boundary),
                tokio::sync::oneshot::channel().0,
            )
            .await;

        let waits = boundary.waits();
        assert_eq!(
            waits.len(),
            1,
            "откат обязан дождаться паузы между задачами"
        );
        assert_eq!(
            waits[0].as_deref(),
            Some(CANDIDATE),
            "пока задача идёт, активной обязана оставаться прежняя версия"
        );
        assert_eq!(fixture.active_version().as_deref(), Some(OLDER));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_rollback_without_a_download_answers_with_its_outcome_and_never_with_row_14() {
        // TL-66, критерий 1: загрузки нет — ждать нечего, и первый ответ
        // команды обязан быть исходом (строка 13). Строки 14 «применится,
        // когда закончится текущая загрузка» нет ни в ответе, ни в событиях.
        let fixture = fixture();
        let older = fixture.install(OLDER, &"a1".repeat(32));
        let newer = fixture.install(CANDIDATE, &"b2".repeat(32));
        fixture.record(&[older, newer]);

        let sink = RecordingSink::default();
        let boundary = Boundary::idle(&fixture.layout);
        let supply = Supply::of(b"");

        fixture.controller.refresh_rollback_target(&fixture.layout);
        fixture.controller.begin_rollback().expect("цель есть");
        let (reply, mut first) = tokio::sync::oneshot::channel();
        fixture
            .controller
            .run_rollback(
                &fixture.pipeline(&Offline, &supply, &sink, &boundary),
                reply,
            )
            .await;

        let first = first.try_recv().expect("ответ команды обязан прийти");
        match &first.status {
            YtDlpUpdateStatus::RolledBack { active, .. } => assert_eq!(active, OLDER),
            other => panic!("первый ответ без загрузки — исход, а не {other:?}"),
        }
        assert_eq!(
            sink.stages(),
            vec!["rolledBack"],
            "строки 14 не должно быть и в событиях"
        );
        assert!(boundary.waits().is_empty(), "ждать было нечего");
        assert_eq!(fixture.active_version().as_deref(), Some(OLDER));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_rollback_during_a_download_answers_row_14_first_and_the_outcome_by_event() {
        // TL-66, критерий 2: загрузка идёт — первый ответ строка 14, и он
        // обязан прийти **до** конца загрузки, а исход — событием после.
        let fixture = fixture();
        let older = fixture.install(OLDER, &"a1".repeat(32));
        let newer = fixture.install(CANDIDATE, &"b2".repeat(32));
        fixture.record(&[older, newer]);

        let sink = RecordingSink::default();
        let boundary = GatedBoundary::busy();
        let supply = Supply::of(b"");

        fixture.controller.refresh_rollback_target(&fixture.layout);
        fixture.controller.begin_rollback().expect("цель есть");
        let (reply, mut first) = tokio::sync::oneshot::channel();
        let pipeline = fixture.pipeline(&Offline, &supply, &sink, &boundary);

        tokio::join!(fixture.controller.run_rollback(&pipeline, reply), async {
            boundary.entered.notified().await;

            // Конвейер уже ждёт границы: ответ обязан быть отдан раньше.
            let first = first
                .try_recv()
                .expect("ответ команды обязан прийти до конца загрузки");
            assert_eq!(stage_name(&first.status), "rollbackWaiting");
            assert_eq!(sink.stages(), vec!["rollbackWaiting"]);
            assert_eq!(
                fixture.active_version().as_deref(),
                Some(CANDIDATE),
                "пока загрузка идёт, переключения нет"
            );

            boundary.release.notify_one();
        });

        assert_eq!(sink.stages(), vec!["rollbackWaiting", "rolledBack"]);
        assert_eq!(fixture.active_version().as_deref(), Some(OLDER));
    }

    // ─────────────── уборка подготовки против обновления ───────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn a_repeated_preparation_does_not_clean_away_the_download_of_a_running_update() {
        // TL-66: подготовка после сброса памяти сеанса (повтор проверки,
        // отказ фонового прогрева) случается посреди сеанса. Её уборка
        // `.download-*` не должна снести временный файл идущего обновления.
        let fixture = Arc::new(fixture());
        let active = fixture.install(ACTIVE, &"a1".repeat(32));
        fixture.record(&[active]);

        let body = onedir_zip(CANDIDATE);
        let stall = Arc::new(Stall::default());
        // Упавшее утверждение не должно оставить приём замершим навсегда:
        // рантайм теста на выходе ждал бы его поток.
        struct ReleaseOnDrop(Arc<Stall>);
        impl Drop for ReleaseOnDrop {
            fn drop(&mut self) {
                self.0.release();
            }
        }
        let _release = ReleaseOnDrop(Arc::clone(&stall));
        let upstream = Arc::new(Upstream::offering(CANDIDATE, &body));
        let supply = Arc::new(StalledSupply {
            body,
            stall: Arc::clone(&stall),
        });
        let sink = Arc::new(RecordingSink::default());
        let boundary = Arc::new(Boundary::idle(&fixture.layout));

        fixture.controller.begin_manual_check().expect("свободен");
        // Отдельная задача рантайма: приём архива идёт в `block_in_place`,
        // и в одной задаче с проверкой он не дал бы ей выполниться.
        let update = tokio::spawn({
            let fixture = Arc::clone(&fixture);
            let (upstream, supply, sink, boundary) = (
                Arc::clone(&upstream),
                Arc::clone(&supply),
                Arc::clone(&sink),
                Arc::clone(&boundary),
            );
            async move {
                let pipeline = fixture.pipeline(&*upstream, &*supply, &*sink, &*boundary);
                fixture
                    .controller
                    .run_check(&pipeline, CheckTrigger::Manual, Instant::now())
                    .await;
            }
        });

        stall.parked.notified().await;
        let downloads = fixture.layout.stale_downloads();
        assert_eq!(
            downloads.len(),
            1,
            "предусловие: временный файл идущего обновления на диске"
        );

        fixture.session.invalidate();
        let _ = fixture
            .session
            .prepare(
                &fixture.bundled_archive(),
                &fixture.data_dir(),
                &fixture.registry,
                &crate::ytdlp::testing::SilentSink,
            )
            .await;
        assert!(
            downloads[0].exists(),
            "уборка подготовки снесла временный файл идущего обновления"
        );

        stall.release();
        update.await.expect("конвейер не паникует");
        assert_eq!(stage_name(&sink.last()), "updated");

        // Контроль: без идущего обновления та же подготовка остаток убирает —
        // иначе тест выше проходил бы и у сломанной уборки.
        let leftover_id = BuildId::new(OLDER, &"c3".repeat(32)).expect("идентификатор");
        let (leftover, file) = fixture
            .layout
            .create_download_file(&leftover_id)
            .expect("файл приёма");
        drop(file);
        fixture.session.invalidate();
        let _ = fixture
            .session
            .prepare(
                &fixture.bundled_archive(),
                &fixture.data_dir(),
                &fixture.registry,
                &crate::ytdlp::testing::SilentSink,
            )
            .await;
        assert!(
            !leftover.exists(),
            "без обновления подготовка обязана убрать недокачанный архив"
        );
    }

    // ─────────────────── С-10: пин бандла ───────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn a_bundled_pin_older_than_the_active_installation_changes_nothing() {
        // Первая половина С-10: приложение не откатывает yt-dlp, который
        // само же обновило. Версия пина — из `binaries.lock.json`, и
        // активная здесь заведомо новее любого мыслимого пина.
        let fixture = fixture();
        let active = fixture.install("2099.12.31", &"a1".repeat(32));
        fixture.record(&[active]);

        let sink = RecordingSink::default();
        let boundary = Boundary::idle(&fixture.layout);
        let supply = Supply::of(b"");

        fixture
            .controller
            .run_bundled_pin(&fixture.pipeline(&Offline, &supply, &sink, &boundary))
            .await;

        assert!(
            sink.statuses().is_empty(),
            "работы нет — значит и событий нет: иначе каждый запуск приложения мигал \
             бы блоку состоянием ни за чем ({:?})",
            sink.stages()
        );
        assert_eq!(
            supply.bundled_asked.load(Ordering::SeqCst),
            0,
            "до вложенного архива дело не доходит вовсе"
        );
        assert_eq!(fixture.active_version().as_deref(), Some("2099.12.31"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_bundled_pin_newer_than_the_active_installation_goes_through_the_same_pipeline() {
        // Вторая половина С-10, не закрытая ни одной задачей до этой:
        // вшитый в новый релиз приложения пин новее самообновлённой
        // установки — и ставится тем же конвейером.
        //
        // Проверяется решение и вход в общий путь: источник у него —
        // вложенный архив. Дальше путь тот же, что у сетевого
        // обновления, и проверен на нём (`a_new_release_is_fetched…`):
        // подложить сюда настоящий архив нельзя, его сумма обязана
        // совпадать с пином, то есть это ровно те 54 МиБ из ресурсов
        // бандла.
        let fixture = fixture();
        let active = fixture.install("2000.01.01", &"a1".repeat(32));
        fixture.record(&[active]);

        let sink = RecordingSink::default();
        let boundary = Boundary::idle(&fixture.layout);
        let supply = Supply::of(b"");

        fixture
            .controller
            .run_bundled_pin(&fixture.pipeline(&Offline, &supply, &sink, &boundary))
            .await;

        assert_eq!(
            supply.bundled_asked.load(Ordering::SeqCst),
            1,
            "пин новее активной — обязан пойти в установку вложенным архивом"
        );
    }
}
