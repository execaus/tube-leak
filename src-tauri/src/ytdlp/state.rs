//! Какая установка yt-dlp активна, какая — последняя известно-хорошая, и
//! что из лежащего в корне установок можно удалить (TL-54, требования Ф-5
//! и Ф-8 эпика E6).
//!
//! # Почему запись, а не «самая свежая по имени каталога»
//!
//! Эвристика по каталогу выглядит бесплатной ровно до первого
//! обновления. Имя каталога — `<версия>-<sha12>`, версия апстрима растёт
//! лексикографически, и «самая свежая» совпадала бы с активной почти
//! всегда — кроме двух случаев, ради которых контур вообще существует:
//! сразу после распаковки кандидата (он самый свежий, но ещё не прошёл
//! smoke — Ф-6) и после ручного отката (активна заведомо не самая
//! свежая — С-8). В первом случае эвристика назвала бы активной
//! непроверенную установку, во втором уборка снесла бы ту, на которую
//! только что откатились. Поэтому активная установка — **явная запись**,
//! а резолв пути для всех потребителей (служебный экран, разбор ссылки,
//! скачивание) идёт через неё.
//!
//! # Что здесь хранится
//!
//! ```text
//! <app_data>/yt-dlp/installs.json
//!   { "schemaVersion": 1,
//!     "active":    { "version": "2026.08.19", "sha256": "07e5…" },
//!     "knownGood": { "version": "2026.07.11", "sha256": "1f3c…" } }
//! ```
//!
//! Пара «версия + сумма», а не готовое имя каталога, и это не форма ради
//! формы: файл лежит в каталоге данных пользователя, то есть его
//! содержимое — такой же непроверенный ввод, как метаданные релиза
//! апстрима. Имя каталога из него не берётся — оно **собирается заново**
//! через [`BuildId::new`], который проверяет обе строки белым списком.
//! Отредактированная руками запись с `"version": "../PWNED"` не
//! превращается в путь: она отбрасывается на чтении, как отбрасывается
//! нечитаемый манифест.
//!
//! # Вложенный в бандл архив — начальное значение и резерв
//!
//! Записи нет (первый запуск вообще) — активной считается установка пина
//! (С-10, «вшитый архив — не хозяин состояния»). Запись есть, но
//! установка, на которую она указывает, не проходит проверку — [`resolve`]
//! спускается к известно-хорошей, затем к пину: работать на резерве
//! лучше, чем не работать вовсе.
//!
//! Резерв — не откат: запись [`resolve`] не меняет, и как только активная
//! установка станет пригодной, резолв вернётся к ней сам. Автоматического
//! отката в контуре нет (Р-3), и его здесь тоже нет.
//!
//! # Чего здесь нет
//!
//! - **Сравнения версий и решения «пора обновляться».** Это Ф-2 (TL-55) и
//!   оркестрация (TL-58). Здесь нет ни одного места, где две версии
//!   сравниваются между собой, — в частности, случай «пин новее активной»
//!   (вторая половина С-10) отсюда не решается: он требует установки,
//!   прогрева и smoke, то есть всего конвейера.
//! - **Момента переключения.** [`InstallState::activate`] меняет значение
//!   атомарно, но не знает, идёт ли сейчас загрузка; границу задач держит
//!   TL-58 (Ф-7, Р-2).
//! - **Расстановки отметок «в использовании».** [`InUse`] — механизм;
//!   отметку ставит резолв пути ([`super::prepare::installed_executable`],
//!   TL-58), и это не деталь размещения: путь к yt-dlp больше взять
//!   негде, поэтому «кто получил путь — тот и держит установку» держится
//!   на коде, а не на дисциплине вызывающих.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::error::PrepareError;
use super::layout::{self, ArchiveIdentity, BuildId, Installed, Layout};

/// Версия формата записи. Растёт, когда меняется смысл полей: чужую
/// версию проще считать отсутствующей (резолв уйдёт к пину, подготовка
/// восстановит запись), чем угадывать её семантику.
const STATE_SCHEMA_VERSION: u32 = 1;

/// Установка в записи: то, из чего собирается её [`BuildId`], плюс сам
/// идентификатор — уже проверенный.
///
/// Версия хранится отдельным полем, а не вычитается из идентификатора,
/// потому что идентификатор её не отдаёт: `BuildId` — одна строка без
/// внутренней структуры для читающего, и разбирать её обратно значило бы
/// завести второе место, знающее, как она собрана.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallEntry {
    build_id: BuildId,
    version: String,
    sha256: String,
}

impl InstallEntry {
    /// Единственный конструктор — через проверку [`BuildId::new`].
    ///
    /// Отдельной проверки здесь нет намеренно: вторая проверка того же
    /// ввода — это второе место, которое однажды разойдётся с первым.
    pub fn new(version: &str, sha256: &str) -> Result<Self, PrepareError> {
        Ok(Self {
            build_id: BuildId::new(version, sha256)?,
            version: version.to_string(),
            sha256: sha256.to_string(),
        })
    }

    /// То же из идентичности архива — форма, в которой версия и сумма
    /// приходят и от пина, и от релиза апстрима.
    pub fn for_identity(identity: ArchiveIdentity<'_>) -> Result<Self, PrepareError> {
        Self::new(identity.version, identity.sha256)
    }

    /// Идентификатор установки: им адресуются каталог и журналы рядом.
    pub fn build_id(&self) -> &BuildId {
        &self.build_id
    }

    /// Версия yt-dlp в апстримном формате — то, что показывается
    /// пользователю (в том числе как цель отката).
    pub fn version(&self) -> &str {
        &self.version
    }

    fn to_raw(&self) -> RawEntry {
        RawEntry {
            version: self.version.clone(),
            sha256: self.sha256.clone(),
        }
    }
}

/// Запись на диске. Отдельный тип от [`InstallEntry`], потому что на
/// диске лежат непроверенные строки, а в памяти — проверенный
/// идентификатор; смешать их в одном типе значило бы получить
/// `Deserialize`, создающий `BuildId` мимо [`BuildId::new`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawEntry {
    version: String,
    sha256: String,
}

impl RawEntry {
    /// Проверяет запись. `None` — не годится; молча это не проходит:
    /// испорченная запись означает, что резолв уйдёт на резерв, и понять
    /// потом, почему, можно только по логу.
    fn checked(self, slot: &str) -> Option<InstallEntry> {
        match InstallEntry::new(&self.version, &self.sha256) {
            Ok(entry) => Some(entry),
            Err(err) => {
                eprintln!("yt-dlp: запись «{slot}» не годится и отброшена: {err}");
                None
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawState {
    schema_version: u32,
    active: Option<RawEntry>,
    known_good: Option<RawEntry>,
}

/// Явная запись Ф-5, прочитанная в память.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstallState {
    active: Option<InstallEntry>,
    known_good: Option<InstallEntry>,
}

impl InstallState {
    /// Читает запись. Файла нет, он не разбирается или он от другой
    /// версии формата — состояние пустое: это в точности «ещё ни разу не
    /// переключались», и подготовка первого запуска восстановит его сама.
    pub fn load(layout: &Layout) -> Self {
        let path = layout.state_path();
        let Some(raw) = layout::read_json::<RawState>(&path) else {
            return Self::default();
        };

        if raw.schema_version != STATE_SCHEMA_VERSION {
            eprintln!(
                "yt-dlp: запись об установках версии формата {} (поддерживается \
                 {STATE_SCHEMA_VERSION}) — считаю, что записи нет",
                raw.schema_version
            );
            return Self::default();
        }

        Self {
            active: raw.active.and_then(|entry| entry.checked("активная")),
            known_good: raw
                .known_good
                .and_then(|entry| entry.checked("известно-хорошая")),
        }
    }

    /// Активная установка по записи. `None` — записи ещё нет.
    pub fn active(&self) -> Option<&InstallEntry> {
        self.active.as_ref()
    }

    /// Последняя известно-хорошая — цель ручного отката (С-8, Р-3).
    /// `None` — возвращаться некуда (ни разу не переключались).
    pub fn known_good(&self) -> Option<&InstallEntry> {
        self.known_good.as_ref()
    }

    /// Делает `entry` активной, а прежнюю активную — известно-хорошей, и
    /// записывает это на диск атомарно.
    ///
    /// Возвращает `false`, если менять было нечего (та же установка уже
    /// активна): тогда файл не переписывается вовсе.
    ///
    /// # Почему «прежняя активная» — единственное правило
    ///
    /// Из него же получается откат, и это не совпадение, а причина, по
    /// которой правило именно такое. Возврат на известно-хорошую X — это
    /// `activate(X)`: активной становится X, а известно-хорошей — Y, та
    /// самая, от которой отказались. Кнопка «Вернуться к …» превращается
    /// в переключатель между двумя удерживаемыми установками, как и
    /// описывает дизайн E6, — без второй ветки кода и без вопроса «а что
    /// теперь считать хорошим».
    ///
    /// Третьей установке в записи места нет по построению — отсюда Ф-8
    /// («не более двух») получается само, а не проверкой числа.
    ///
    /// # Чего здесь не проверяется
    ///
    /// Пригодности установки к запуску. Тот, кто переключает, обязан
    /// знать, что переключает: smoke-проверка стоит **до** этого вызова
    /// (Ф-6, TL-57), а не внутри него. Проверка здесь означала бы обход
    /// дерева на каждом переключении и, что хуже, второе место, решающее
    /// «годна ли установка», рядом с [`layout::validate`].
    pub fn activate(&mut self, layout: &Layout, entry: InstallEntry) -> Result<bool, PrepareError> {
        if self.active.as_ref().map(InstallEntry::build_id) == Some(entry.build_id()) {
            return Ok(false);
        }

        let displaced = self.active.replace(entry);
        // Прежняя известно-хорошая забывается: удерживаются ровно две
        // установки, и третья не помещается ни в запись, ни на диск
        // (Ф-8). `None` бывает ровно один раз — при самой первой записи.
        self.known_good = displaced;

        self.save(layout)?;
        Ok(true)
    }

    fn save(&self, layout: &Layout) -> Result<(), PrepareError> {
        let raw = RawState {
            schema_version: STATE_SCHEMA_VERSION,
            active: self.active.as_ref().map(InstallEntry::to_raw),
            known_good: self.known_good.as_ref().map(InstallEntry::to_raw),
        };

        // Через временный файл и `rename` — тем же способом, что манифест
        // и журнал починок. Половина JSON под именем записи означала бы
        // «активной установки нет» на следующем старте, то есть откат к
        // пину без единой команды пользователя.
        layout::write_json_atomic(&layout.state_path(), &raw, "записи об установках")
    }

    /// Установки, которые уборка не удаляет **из-за записи** (к ним
    /// добавляются занятые процессом и только что подготовленные — см.
    /// [`cleanup`]).
    ///
    /// Когда записи нет, в защите стоит пин: именно его вернёт [`resolve`]
    /// на первом запуске, и удалить то, что резолв сейчас отдаёт, уборка
    /// не должна ни при каком стечении обстоятельств. Когда запись есть,
    /// пин не защищён ничем — иначе после трёх переключений на диске
    /// осталось бы три установки вместо двух, а вернуть его на диск умеет
    /// вложенный в бандл архив.
    fn retained(&self) -> BTreeSet<BuildId> {
        let mut ids = BTreeSet::new();

        match self.active.as_ref() {
            Some(active) => {
                ids.insert(active.build_id().clone());
            }
            None => {
                if let Ok(pinned) = layout::bundled_build_id() {
                    ids.insert(pinned);
                }
            }
        }

        if let Some(known_good) = self.known_good.as_ref() {
            ids.insert(known_good.build_id().clone());
        }

        ids
    }
}

/// Откуда взялся путь, который получил потребитель.
///
/// Различие видно в логе и нужно там: «работаем на резерве» — это не
/// отказ, но и не норма, и разбираться с ним владельцу придётся по
/// записи в логе, а не по поведению приложения (оно как раз не меняется).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// Активная установка по записи — обычный путь.
    Active,
    /// Известно-хорошая: активная не прошла проверку.
    KnownGood,
    /// Вложенный в бандл пин: записи нет (первый запуск) либо ни активная,
    /// ни известно-хорошая не годятся.
    Bundled,
}

impl Slot {
    fn describe(self) -> &'static str {
        match self {
            Self::Active => "активная",
            Self::KnownGood => "известно-хорошая",
            Self::Bundled => "вложенная в бандл",
        }
    }
}

/// Установка, на которой приложение работает прямо сейчас.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub build_id: BuildId,
    pub installed: Installed,
    pub slot: Slot,
}

/// Путь к yt-dlp через запись Ф-5 — единственный резолв на всех
/// потребителей (служебный экран, разбор ссылки, скачивание).
///
/// Порядок попыток: активная → известно-хорошая → пин. Каждая проверяется
/// [`layout::validate`] (манифест, исполняемый файл, сверка дерева), то
/// есть «есть запись» и «есть чем работать» — разные утверждения, и
/// путается их здесь нечем.
pub fn resolve(layout: &Layout, state: &InstallState) -> Result<Resolved, PrepareError> {
    let mut candidates: Vec<(Slot, BuildId)> = Vec::with_capacity(3);
    let mut reasons: Vec<String> = Vec::new();

    if let Some(active) = state.active() {
        candidates.push((Slot::Active, active.build_id().clone()));
    }
    if let Some(known_good) = state.known_good() {
        candidates.push((Slot::KnownGood, known_good.build_id().clone()));
    }
    match layout::bundled_build_id() {
        Ok(pinned) => candidates.push((Slot::Bundled, pinned)),
        Err(err) => reasons.push(format!("вложенная в бандл — {err}")),
    }

    let mut tried: BTreeSet<BuildId> = BTreeSet::new();
    for (slot, build_id) in candidates {
        // Одна и та же установка в двух ролях (запись указывает на пин —
        // обычное дело до первого обновления) проверяется один раз: два
        // одинаковых обхода дерева и две одинаковых строки в диагностике
        // не добавляют ничего.
        if !tried.insert(build_id.clone()) {
            continue;
        }

        match layout::validate(layout, &build_id) {
            Ok(installed) => {
                return Ok(Resolved {
                    build_id,
                    installed,
                    slot,
                })
            }
            Err(invalid) => reasons.push(format!("{} {build_id} — {invalid}", slot.describe())),
        }
    }

    Err(PrepareError::LayoutUnexpected {
        reason: format!("готовой установки нет: {}", reasons.join("; ")),
    })
}

/// Отметки «этой установкой прямо сейчас пользуется процесс».
///
/// Существует ради безусловной защиты из Ф-8 и Ф-7: установка, из которой
/// запущен работающий процесс, не удаляется, даже если она не активна и
/// не известно-хорошая. Такое состояние — не экзотика, а прямое следствие
/// Р-2: задача доходит до конца на той версии, на которой началась, а
/// переключение и уборка происходят на границе задач; между ними
/// работающий процесс и запись расходятся ровно на одну установку.
///
/// Счётчик, а не флаг: одну и ту же установку одновременно держат
/// несколько процессов (разбор новой ссылки идёт рядом с загрузкой, и у
/// каждого свой yt-dlp),
/// и снятие отметки первым из них сняло бы защиту с остальных.
#[derive(Debug, Default)]
pub struct InUse {
    marks: Mutex<BTreeMap<BuildId, u32>>,
}

impl InUse {
    pub fn new() -> Self {
        Self::default()
    }

    /// Отмечает установку занятой на время жизни возвращённого стража.
    #[must_use = "отметка держится, пока жив страж: `let _ = mark(..)` снимает её \
                  тем же выражением, в котором поставил"]
    pub fn mark(&self, build_id: &BuildId) -> InUseGuard<'_> {
        let mut marks = self.lock();
        *marks.entry(build_id.clone()).or_insert(0) += 1;
        drop(marks);

        InUseGuard {
            registry: self,
            build_id: build_id.clone(),
        }
    }

    /// Занятые прямо сейчас установки.
    ///
    /// `pub(super)` ради одного проверяющего за пределами модуля: резолв
    /// пути ([`super::prepare::installed_executable`]) обязан ставить
    /// отметку, и утверждение «ставит» проверяется чтением реестра, а не
    /// поведением уборки. Уборка уважает отметки — свойство **этого**
    /// модуля, и оно проверено здесь же; приписывать его чужому тесту
    /// значило бы проверять дважды одно и не проверить другого.
    pub(super) fn snapshot(&self) -> BTreeSet<BuildId> {
        self.lock().keys().cloned().collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<BuildId, u32>> {
        // Отравленный мьютекс — не повод ронять уборку или запуск
        // процесса: внутри обычная таблица счётчиков, паника чужого
        // потока не делает её противоречивой (тот же приём, что в
        // `crate::sidecar::ChildRegistry`).
        self.marks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Пока он жив, установка считается занятой.
#[derive(Debug)]
pub struct InUseGuard<'a> {
    registry: &'a InUse,
    build_id: BuildId,
}

impl Drop for InUseGuard<'_> {
    fn drop(&mut self) {
        let mut marks = self.registry.lock();
        if let Some(count) = marks.get_mut(&self.build_id) {
            *count -= 1;
            if *count == 0 {
                marks.remove(&self.build_id);
            }
        }
    }
}

/// Что уборка сделала. Нужен вызывающему для лога: сама она ничего не
/// печатает построчно, чтобы не превращать штатный проход в шум.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct CleanupReport {
    /// Удалённое — каталоги установок, их манифесты и журналы, остатки
    /// `.staging-*`/`.download-*`, временные файлы записи.
    pub removed: Vec<PathBuf>,
    /// То, что удалить не удалось. Не ошибка уборки: неудаляемый остаток
    /// занимает место, но работе не мешает, и следующий проход попробует
    /// снова.
    pub failed: Vec<PathBuf>,
}

/// Убирает из корня установок всё, что не защищено (Ф-8).
///
/// Защищены безусловно, то есть без единого условия, которое могло бы их
/// из защиты вывести:
///
/// - активная по записи (а когда записи нет — пин, потому что его и
///   вернёт [`resolve`]);
/// - последняя известно-хорошая — основа отката С-8;
/// - занятые процессом прямо сейчас (`in_use`) — Ф-7;
/// - только что подготовленные, ещё не переключённые (`prepared`) —
///   кандидат между распаковкой и smoke-проверкой.
///
/// Вместе с каталогом установки сохраняются её манифест и журналы рядом:
/// что чему принадлежит, знает [`Layout::belongs_to`], а не эта функция.
///
/// Отдельно от них — журнал проваленной smoke-проверки
/// ([`Layout::is_smoke_journal`], TL-57): он сохраняется безусловно,
/// **включая** журналы удаляемых установок, потому что говорит не о
/// дереве, а о том, что этот build id ставить больше не надо (С-5).
///
/// # Белый список, а не чёрный
///
/// Удаляется всё, чего нет в списке защищённого, — включая то, чего мы не
/// знаем: имя не в UTF-8, каталог от неизвестной версии приложения,
/// `…json.tmp` от прерванной записи. Обратный порядок («удалять
/// перечисленное») означал бы, что любой не предусмотренный сегодня
/// объект копится в каталоге данных вечно, а чёрный список в этом проекте
/// подводил дважды подряд (урок E3).
///
/// Цена ошибки при этом несимметрична, и потому список защищённого
/// строится из записи и отметок, а не из имён на диске: лишнее удалённое
/// стоит одной переустановки (124 МиБ, ~35 с), удалённая активная —
/// неработающего приложения.
///
/// # Предусловие
///
/// Вызывающий не готовит установку параллельно этому вызову — либо
/// готовит ту, которую передал в `prepared`. `.staging-<id>-*` защищённой
/// установки переживает уборку (потому и защищён по префиксу), а вот
/// распаковка **чужого** идентификатора, начатая параллельно, потеряет
/// свой каталог. В контуре это держит оркестрация (TL-58): конвейер
/// обновления в приложении один. Межпроцессная гонка двух экземпляров
/// приложения на одном каталоге данных здесь, как и во всей раскладке, не
/// решается — это отдельная задача (#21).
pub fn cleanup(
    layout: &Layout,
    state: &InstallState,
    in_use: &InUse,
    prepared: &[BuildId],
) -> CleanupReport {
    let mut retained = state.retained();
    retained.extend(in_use.snapshot());
    retained.extend(prepared.iter().cloned());

    let mut report = CleanupReport::default();

    let Ok(entries) = fs::read_dir(layout.root()) else {
        // Корня нет или он не читается: убирать нечего и незачем
        // отличать одно от другого — подготовка первого запуска создаёт
        // его заново.
        return report;
    };

    let state_path = layout.state_path();

    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        // Сама запись — не установка и не остаток: удалив её, уборка
        // отправила бы следующий запуск к пину, то есть выполнила бы
        // откат, которого никто не просил.
        if path == state_path {
            continue;
        }

        let name = entry.file_name();
        let name_str = name.to_str();

        // Запись о проваленной smoke-проверке (TL-57) — не установка и не
        // остаток, а память о том, что этот build id ставить не надо
        // (С-5). Она относится к установке, которую уборка сносит прямо
        // сейчас: провалившийся кандидат не активен и не известно-хорош.
        // Удали её вместе с деревом — и следующая же проверка скачает те
        // же шестьдесят мегабайт, распакует их и провалит запуск снова,
        // и так по расписанию до нового релиза апстрима.
        if name_str.is_some_and(Layout::is_smoke_journal) {
            continue;
        }

        let protected =
            name_str.is_some_and(|name| retained.iter().any(|id| Layout::belongs_to(name, id)));
        if protected {
            continue;
        }

        match remove(&path, entry.file_type().ok()) {
            Ok(()) => report.removed.push(path),
            Err(err) => {
                eprintln!("yt-dlp: не удалось убрать {}: {err}", path.display());
                report.failed.push(path);
            }
        }
    }

    report
}

/// Удаляет запись каталога, чем бы она ни была.
///
/// Тип берётся из [`fs::DirEntry::file_type`], который **не** идёт по
/// символической ссылке, поэтому [`fs::remove_dir_all`] на ссылку здесь
/// не попадает никогда: ссылка снимается сама, а не то, на что она
/// указывает. Подложенная в каталог данных ссылка на чужой каталог не
/// превращает уборку в удаление чужих файлов.
///
/// Утверждение проверено мутацией, и заодно выяснено, чего оно **не**
/// значит. Разрешение ссылки перед удалением (`canonicalize` над путём)
/// ломает уборку сразу — сторож
/// `cleanup_unlinks_a_symlink_instead_of_following_it` краснеет. А вот
/// подмена `DirEntry::file_type` на следующую по ссылке `fs::metadata`
/// его **не** ломает: `std::fs::remove_dir_all` сама делает `lstat` и на
/// ссылке вызывает `remove_file` вместо обхода. То есть от следования по
/// ссылке защищают здесь две вещи, а не одна, и вторая — чужая. Полагаться
/// на неё эта функция не собирается (в ней и стоит явная развилка), но
/// читатель, который решит проверить первую мутацией, должен знать, почему
/// она зелёная.
fn remove(path: &std::path::Path, file_type: Option<fs::FileType>) -> std::io::Result<()> {
    match file_type {
        Some(file_type) if file_type.is_dir() => fs::remove_dir_all(path),
        Some(file_type) if file_type.is_symlink() => {
            // Ссылка на каталог: Unix снимает её `unlink`, Windows —
            // `RemoveDirectory`. Который из двух, известно только по
            // платформе, поэтому пробуются оба; порядок такой, потому что
            // на Unix верен всегда первый.
            fs::remove_file(path).or_else(|err| fs::remove_dir(path).map_err(|_| err))
        }
        _ => fs::remove_file(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::{tempdir, TempDir};

    const EXECUTABLE: &str = "yt-dlp-test";

    struct Fixture {
        _dir: TempDir,
        layout: Layout,
    }

    fn fixture() -> Fixture {
        let dir = tempdir().expect("tempdir");
        let layout = Layout::new(&dir.path().join("app-data"));
        layout.create_root().expect("корень обязан создаваться");
        Fixture { _dir: dir, layout }
    }

    impl Fixture {
        /// Правдоподобная установка: дерево из двух файлов и манифест,
        /// который с ним сходится.
        fn install(&self, entry: &InstallEntry) {
            let dir = self.layout.install_dir(entry.build_id());
            write_file(&dir.join(EXECUTABLE), b"#!/bin/sh\n", true);
            write_file(&dir.join("_internal/lib.so"), b"0123456789", false);

            let manifest = layout::manifest_for(
                &dir,
                EXECUTABLE,
                ArchiveIdentity {
                    version: entry.version(),
                    sha256: &entry.sha256,
                },
            )
            .expect("манифест обязан собираться");
            manifest
                .write_atomic(&self.layout.manifest_path(entry.build_id()))
                .expect("манифест обязан записываться");
        }

        /// Что лежит прямо в корне установок — состояние диска, а не
        /// мнение уборки о нём.
        fn root_names(&self) -> BTreeSet<String> {
            fs::read_dir(self.layout.root())
                .expect("корень обязан читаться")
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        }

        /// Каталоги установок (то, что считается «установкой на диске»).
        fn install_dirs(&self) -> BTreeSet<String> {
            self.root_names()
                .into_iter()
                .filter(|name| !name.starts_with('.') && self.layout.root().join(name).is_dir())
                .collect()
        }

        fn touch(&self, name: &str) {
            write_file(&self.layout.root().join(name), b"x", false);
        }

        fn mkdir(&self, name: &str) {
            fs::create_dir_all(self.layout.root().join(name)).expect("каталог обязан создаваться");
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

    /// Установка с предсказуемым идентификатором: версия и повторяющийся
    /// байт суммы.
    fn entry(version: &str, fill: &str) -> InstallEntry {
        InstallEntry::new(version, &fill.repeat(32)).expect("образец обязан проходить проверку")
    }

    fn pinned_entry() -> InstallEntry {
        InstallEntry::for_identity(ArchiveIdentity::bundled())
            .expect("пин обязан проходить проверку идентификатора")
    }

    #[test]
    fn the_record_survives_a_restart_and_is_rebuilt_through_the_checked_type() {
        let fixture = fixture();
        let mut state = InstallState::default();
        let first = entry("2026.07.11", "1a");
        let second = entry("2026.08.19", "2b");

        assert!(state
            .activate(&fixture.layout, first.clone())
            .expect("запись обязана сохраняться"));
        assert!(state
            .activate(&fixture.layout, second.clone())
            .expect("запись обязана сохраняться"));

        // «Переживает перезапуск» — это чтение с диска в новом значении,
        // а не то же значение в памяти.
        let reloaded = InstallState::load(&fixture.layout);
        assert_eq!(reloaded.active(), Some(&second));
        assert_eq!(reloaded.known_good(), Some(&first));
        assert_eq!(reloaded, state);
    }

    #[test]
    fn activating_the_same_installation_twice_changes_nothing() {
        let fixture = fixture();
        let mut state = InstallState::default();
        let only = entry("2026.08.19", "2b");

        assert!(state
            .activate(&fixture.layout, only.clone())
            .expect("первая активация обязана записаться"));
        assert!(
            !state
                .activate(&fixture.layout, only.clone())
                .expect("повтор обязан быть безобидным"),
            "повторная активация той же установки не должна ничего менять"
        );
        assert_eq!(
            state.known_good(),
            None,
            "повтор не должен объявлять активную установку собственным резервом: \
             откатываться было бы некуда, а кнопка отката появилась бы"
        );
    }

    #[test]
    fn rolling_back_swaps_the_two_installations() {
        // С-8/Р-3: возврат — это активация известно-хорошей, и после него
        // роль резерва достаётся той версии, от которой отказались.
        let fixture = fixture();
        let mut state = InstallState::default();
        let old = entry("2026.07.11", "1a");
        let new = entry("2026.08.19", "2b");

        state
            .activate(&fixture.layout, old.clone())
            .expect("запись");
        state
            .activate(&fixture.layout, new.clone())
            .expect("запись");
        state
            .activate(&fixture.layout, old.clone())
            .expect("запись");

        assert_eq!(state.active(), Some(&old));
        assert_eq!(state.known_good(), Some(&new));
    }

    #[test]
    fn a_hostile_record_never_becomes_a_path() {
        // Файл записи лежит в каталоге пользователя: его содержимое —
        // такой же непроверенный ввод, как метаданные релиза. Проверка
        // одна и та же, обойти её через `serde` нечем.
        let fixture = fixture();
        let hostile = format!(
            r#"{{"schemaVersion":{STATE_SCHEMA_VERSION},
                 "active":{{"version":"../PWNED","sha256":"{sha}"}},
                 "knownGood":{{"version":"/tmp/OWNED","sha256":"{sha}"}}}}"#,
            sha = "ab".repeat(32)
        );
        fs::write(fixture.layout.state_path(), hostile).expect("запись фикстуры");

        let state = InstallState::load(&fixture.layout);
        assert_eq!(state.active(), None, "«../PWNED» обязана быть отброшена");
        assert_eq!(
            state.known_good(),
            None,
            "«/tmp/OWNED» обязана быть отброшена"
        );

        // И следствие, ради которого всё это: резолв не построил пути из
        // отброшенного, а ушёл к пину.
        fixture.install(&pinned_entry());
        let resolved = resolve(&fixture.layout, &state).expect("пин обязан резолвиться");
        assert_eq!(resolved.slot, Slot::Bundled);
        assert_eq!(&resolved.build_id, pinned_entry().build_id());
    }

    #[test]
    fn a_record_from_another_schema_version_is_treated_as_absent() {
        let fixture = fixture();
        let alien = format!(
            r#"{{"schemaVersion":{},"active":{{"version":"2026.08.19","sha256":"{}"}},
                 "knownGood":null}}"#,
            STATE_SCHEMA_VERSION + 1,
            "ab".repeat(32)
        );
        fs::write(fixture.layout.state_path(), alien).expect("запись фикстуры");

        assert_eq!(InstallState::load(&fixture.layout), InstallState::default());
    }

    #[test]
    fn resolving_follows_the_record_and_not_the_freshest_directory_name() {
        // Ф-5 целиком: на диске две установки, свежая по имени — не
        // активная. Эвристика «самая свежая» вернула бы 2026.08.19.
        let fixture = fixture();
        let old = entry("2026.07.11", "1a");
        let new = entry("2026.08.19", "2b");
        fixture.install(&old);
        fixture.install(&new);

        let mut state = InstallState::default();
        state
            .activate(&fixture.layout, new.clone())
            .expect("запись");
        state
            .activate(&fixture.layout, old.clone())
            .expect("запись");

        let resolved = resolve(&fixture.layout, &state).expect("активная обязана резолвиться");
        assert_eq!(resolved.slot, Slot::Active);
        assert_eq!(&resolved.build_id, old.build_id());
        assert_eq!(
            resolved.installed.executable,
            fixture.layout.install_dir(old.build_id()).join(EXECUTABLE)
        );
        assert_eq!(resolved.installed.version, old.version());
    }

    #[test]
    fn without_a_record_the_bundled_pin_is_the_initial_value() {
        // С-10: вложенный архив — начальное значение, пока записи нет.
        let fixture = fixture();
        let pinned = pinned_entry();
        fixture.install(&pinned);

        let resolved = resolve(&fixture.layout, &InstallState::default())
            .expect("пин обязан резолвиться без записи");
        assert_eq!(resolved.slot, Slot::Bundled);
        assert_eq!(&resolved.build_id, pinned.build_id());
    }

    #[test]
    fn a_broken_active_falls_back_to_the_known_good_without_changing_the_record() {
        let fixture = fixture();
        let old = entry("2026.07.11", "1a");
        let new = entry("2026.08.19", "2b");
        fixture.install(&old);
        fixture.install(&new);

        let mut state = InstallState::default();
        state
            .activate(&fixture.layout, old.clone())
            .expect("запись");
        state
            .activate(&fixture.layout, new.clone())
            .expect("запись");

        // Активная перестала сходиться с манифестом — ровно то, что
        // ловит дешёвая сверка `validate`.
        fs::remove_file(fixture.layout.install_dir(new.build_id()).join(EXECUTABLE))
            .expect("исполняемый файл обязан удаляться");

        let resolved = resolve(&fixture.layout, &state).expect("резерв обязан находиться");
        assert_eq!(resolved.slot, Slot::KnownGood);
        assert_eq!(&resolved.build_id, old.build_id());

        // Резерв — не откат: запись осталась прежней и на диске тоже.
        assert_eq!(state.active(), Some(&new));
        assert_eq!(InstallState::load(&fixture.layout).active(), Some(&new));
    }

    #[test]
    fn resolving_fails_with_every_reason_when_nothing_is_installed() {
        let fixture = fixture();
        let mut state = InstallState::default();
        state
            .activate(&fixture.layout, entry("2026.08.19", "2b"))
            .expect("запись");

        let err = resolve(&fixture.layout, &state).expect_err("резолвить нечего");
        let message = err.to_string();
        assert!(
            message.contains("активная") && message.contains("вложенная в бандл"),
            "в отказе обязаны быть перечислены все испробованные слоты: {message}"
        );
    }

    #[test]
    fn cleanup_keeps_the_active_the_known_good_and_the_record() {
        let fixture = fixture();
        let old = entry("2026.07.11", "1a");
        let new = entry("2026.08.19", "2b");
        let dropped = entry("2025.01.01", "3c");
        for install in [&old, &new, &dropped] {
            fixture.install(install);
        }

        let mut state = InstallState::default();
        state
            .activate(&fixture.layout, old.clone())
            .expect("запись");
        state
            .activate(&fixture.layout, new.clone())
            .expect("запись");

        cleanup(&fixture.layout, &state, &InUse::new(), &[]);

        let names = fixture.root_names();
        assert!(names.contains(&format!("{}", new.build_id())));
        assert!(names.contains(&format!("{}.json", new.build_id())));
        assert!(names.contains(&format!("{}", old.build_id())));
        assert!(names.contains(&format!("{}.json", old.build_id())));
        assert!(
            !names.contains(&format!("{}", dropped.build_id())),
            "установка, которой нет в записи, обязана уйти: {names:?}"
        );
        assert!(
            !names.contains(&format!("{}.json", dropped.build_id())),
            "манифест удалённой установки обязан уйти вместе с ней: {names:?}"
        );
        assert!(
            fixture.layout.state_path().exists(),
            "уборка не должна трогать саму запись: иначе следующий запуск \
             уехал бы на пин без единой команды пользователя"
        );
    }

    #[test]
    fn cleanup_never_removes_an_installation_that_is_in_use() {
        // Критерий приёмки TL-54: занятая процессом установка не активна,
        // не известно-хорошая, и всё равно переживает уборку (Ф-7).
        let fixture = fixture();
        let busy = entry("2026.05.05", "4d");
        let active = entry("2026.08.19", "2b");
        fixture.install(&busy);
        fixture.install(&active);

        let mut state = InstallState::default();
        state
            .activate(&fixture.layout, active.clone())
            .expect("запись");

        let in_use = InUse::new();
        let guard = in_use.mark(busy.build_id());
        cleanup(&fixture.layout, &state, &in_use, &[]);

        assert!(
            fixture
                .layout
                .install_dir(busy.build_id())
                .join(EXECUTABLE)
                .exists(),
            "установка под работающим процессом не удаляется ни при каких условиях"
        );

        // А после снятия отметки — уходит: защита держится стражем, а не
        // тем, что уборка вообще ничего не удаляет.
        drop(guard);
        cleanup(&fixture.layout, &state, &in_use, &[]);
        assert!(
            !fixture.layout.install_dir(busy.build_id()).exists(),
            "снятая отметка обязана снимать и защиту"
        );
    }

    #[test]
    fn marks_are_counted_so_the_first_process_to_finish_does_not_free_the_others() {
        let fixture = fixture();
        let busy = entry("2026.05.05", "4d");
        fixture.install(&busy);
        let state = InstallState::default();

        let in_use = InUse::new();
        let first = in_use.mark(busy.build_id());
        let second = in_use.mark(busy.build_id());
        drop(first);

        cleanup(&fixture.layout, &state, &in_use, &[]);
        assert!(
            fixture.layout.install_dir(busy.build_id()).exists(),
            "вторая отметка обязана продолжать держать защиту"
        );

        drop(second);
        assert!(
            in_use.snapshot().is_empty(),
            "снятые отметки не должны оставлять записей в реестре"
        );
    }

    #[test]
    fn cleanup_keeps_a_freshly_prepared_candidate_with_its_staging_dir() {
        // Кандидат между распаковкой и smoke-проверкой: в записи его ещё
        // нет, и защищает его только явное перечисление.
        let fixture = fixture();
        let candidate = entry("2026.09.01", "5e");
        let active = entry("2026.08.19", "2b");
        fixture.install(&candidate);
        fixture.install(&active);
        let staging = fixture
            .layout
            .create_staging_dir(candidate.build_id())
            .expect("каталог распаковки");

        let mut state = InstallState::default();
        state
            .activate(&fixture.layout, active.clone())
            .expect("запись");

        cleanup(
            &fixture.layout,
            &state,
            &InUse::new(),
            &[candidate.build_id().clone()],
        );

        assert!(
            fixture.layout.install_dir(candidate.build_id()).exists(),
            "только что подготовленная установка защищена безусловно"
        );
        assert!(
            staging.exists(),
            "каталог распаковки защищённой установки не должен исчезать у неё из-под ног"
        );
    }

    #[test]
    fn cleanup_sweeps_leftovers_orphans_and_anything_unknown() {
        let fixture = fixture();
        let active = entry("2026.08.19", "2b");
        let gone = entry("2025.01.01", "3c");
        fixture.install(&active);

        // Остаток распаковки чужой установки, недокачанный архив,
        // осиротевшие манифест и журналы, временный файл записи и просто
        // неизвестный объект.
        fixture.mkdir(&format!(".staging-{}-0123456789abcdef", gone.build_id()));
        fixture.touch(&format!(".download-{}-0123456789abcdef", gone.build_id()));
        fixture.touch(&format!("{}.json", gone.build_id()));
        fixture.touch(&format!("{}.repair.json", gone.build_id()));
        fixture.touch(&format!("{}.update.json", gone.build_id()));
        fixture.touch(&format!("{}.json.tmp", active.build_id()));
        fixture.touch("installs.json.tmp");
        fixture.mkdir("who-put-this-here");

        let mut state = InstallState::default();
        state
            .activate(&fixture.layout, active.clone())
            .expect("запись");

        cleanup(&fixture.layout, &state, &InUse::new(), &[]);

        let names = fixture.root_names();
        assert_eq!(
            names,
            BTreeSet::from([
                format!("{}", active.build_id()),
                format!("{}.json", active.build_id()),
                "installs.json".to_string(),
            ]),
            "в корне обязаны остаться только активная установка с манифестом и сама запись"
        );
    }

    #[test]
    fn cleanup_keeps_the_journals_of_a_protected_installation() {
        // Журнал починок и журнал попыток обновления переживают уборку
        // вместе со своей установкой — в них весь смысл: они помнят то,
        // что не должно повторяться (`RepairLog`, TL-56).
        let fixture = fixture();
        let active = entry("2026.08.19", "2b");
        fixture.install(&active);
        fixture.touch(&format!("{}.repair.json", active.build_id()));
        fixture.touch(&format!("{}.update.json", active.build_id()));

        let mut state = InstallState::default();
        state
            .activate(&fixture.layout, active.clone())
            .expect("запись");

        cleanup(&fixture.layout, &state, &InUse::new(), &[]);

        let names = fixture.root_names();
        assert!(names.contains(&format!("{}.repair.json", active.build_id())));
        assert!(names.contains(&format!("{}.update.json", active.build_id())));
    }

    #[test]
    fn cleanup_unlinks_a_symlink_instead_of_following_it() {
        #[cfg(unix)]
        {
            let fixture = fixture();
            let outside = fixture
                .layout
                .root()
                .parent()
                .expect("родитель")
                .join("precious");
            write_file(&outside.join("keep-me.txt"), b"precious", false);

            std::os::unix::fs::symlink(
                &outside,
                fixture.layout.root().join("2026.08.19-aaaaaaaaaaaa"),
            )
            .expect("ссылка обязана создаваться");

            cleanup(
                &fixture.layout,
                &InstallState::default(),
                &InUse::new(),
                &[],
            );

            assert!(
                !fixture
                    .layout
                    .root()
                    .join("2026.08.19-aaaaaaaaaaaa")
                    .symlink_metadata()
                    .is_ok(),
                "ссылка обязана быть снята"
            );
            assert!(
                outside.join("keep-me.txt").exists(),
                "уборка не должна ходить по ссылкам наружу каталога данных"
            );
        }
    }

    #[test]
    fn three_switches_leave_no_more_than_two_installations() {
        // Критерий приёмки TL-54 и цена контура из Ф-8: диск не растёт с
        // каждым релизом апстрима.
        let fixture = fixture();
        let mut state = InstallState::default();
        let versions = [
            entry("2026.05.05", "4d"),
            entry("2026.06.06", "5e"),
            entry("2026.07.11", "1a"),
            entry("2026.08.19", "2b"),
        ];

        for next in &versions {
            // Так это и происходит в контуре: распаковали кандидата,
            // переключились, убрали лишнее.
            fixture.install(next);
            state
                .activate(&fixture.layout, next.clone())
                .expect("запись");
            cleanup(&fixture.layout, &state, &InUse::new(), &[]);

            assert!(
                fixture.install_dirs().len() <= 2,
                "после переключения на {} на диске {:?}",
                next.version(),
                fixture.install_dirs()
            );
        }

        assert_eq!(
            fixture.install_dirs(),
            BTreeSet::from([
                format!("{}", versions[2].build_id()),
                format!("{}", versions[3].build_id()),
            ]),
            "остаться обязаны ровно активная и предыдущая"
        );
    }

    #[test]
    fn cleanup_without_a_record_protects_the_pin_it_would_resolve_to() {
        // Запись потеряна (снесена, не разобралась) — резолв уйдёт к
        // пину, значит и уборка обязана его сохранить. Иначе первый же
        // проход уборки стоил бы пользователю переустановки на 124 МиБ.
        let fixture = fixture();
        let pinned = pinned_entry();
        let stranger = entry("2025.01.01", "3c");
        fixture.install(&pinned);
        fixture.install(&stranger);

        cleanup(
            &fixture.layout,
            &InstallState::default(),
            &InUse::new(),
            &[],
        );

        assert!(fixture.layout.install_dir(pinned.build_id()).exists());
        assert!(!fixture.layout.install_dir(stranger.build_id()).exists());
    }

    #[test]
    fn the_pin_is_not_protected_once_the_record_names_someone_else() {
        // Обратная сторона предыдущего сторожа и прямое следствие Ф-8:
        // защищать пин всегда значило бы держать на диске три установки
        // вместо двух. Вернуть его умеет вложенный в бандл архив.
        let fixture = fixture();
        let pinned = pinned_entry();
        let old = entry("2026.07.11", "1a");
        let new = entry("2026.08.19", "2b");
        for install in [&pinned, &old, &new] {
            fixture.install(install);
        }

        let mut state = InstallState::default();
        state
            .activate(&fixture.layout, old.clone())
            .expect("запись");
        state
            .activate(&fixture.layout, new.clone())
            .expect("запись");

        cleanup(&fixture.layout, &state, &InUse::new(), &[]);

        assert!(
            !fixture.layout.install_dir(pinned.build_id()).exists(),
            "пин не защищён, когда запись называет активной другую установку"
        );
    }
}
