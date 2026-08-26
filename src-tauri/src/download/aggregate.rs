//! Агрегация видео- и аудиопотока в один процент (Ф-2, TL-41).
//!
//! Модуль чистый: принимает разобранные строки прогресса
//! ([`crate::download::progress::ProgressSample`]) и отдаёт состояние фазы
//! «Скачивание» ([`DownloadingState`]). Ни процессов, ни таймеров, ни
//! событий Tauri — троттлинг эмита, повторы и фазы задачи это TL-43/TL-44.
//!
//! # Что здесь считается и почему именно так
//!
//! Дизайн E3 («Агрегация видео+аудио внутри „Скачивание“») задаёт правило:
//! показываемый процент — это `(байт видео получено + байт аудио получено)
//! / (ожидаемый размер видео + ожидаемый размер аудио)`, а при неизвестном
//! размере одного из потоков веса берутся 50/50. Отсюда три величины,
//! которые модуль ведёт по каждому потоку: принято байт, лучший известный
//! знаменатель и признак завершённости.
//!
//! ## Знаменатель уточняется, а не подпирается обрезанием
//!
//! Контракт обрезает процент сотней ([`DownloadPercent::new`]), но это
//! страховка от выезда полосы, а не способ считать. Показанные подряд «99,
//! 100, 100, 100 %» — такой же дефект «нет движения», как застывшая
//! скорость, поэтому знаменатель здесь уточняется тремя способами, и
//! обрезание ни в одном из них не участвует:
//!
//! 1. **Точный размер потока от yt-dlp вытесняет любую оценку.** У прямых
//!    потоков он приходит с первой же строки прогресса, у потоков через
//!    манифест — в строке `finished`.
//! 2. **Оценка E2 остаётся знаменателем только для тех потоков, у которых
//!    точного размера ещё нет.** Как только точный размер части потоков
//!    известен, из оценки вычитается их доля, а остаток достаётся
//!    остальным; если факт уже перерос оценку, знаменатель идёт за фактом,
//!    а не упирается в оценку.
//! 3. **Ста процентов не бывает, пока хоть один поток не закрыт.**
//!    Значение обрезается 99 до тех пор, пока каждый поток не завершён или
//!    не добран до своего точного размера. Это и есть настоящая защита от
//!    «100 % и тишина»: она не даёт заниженному знаменателю досрочно
//!    упереться в сотню.
//!
//! **Прикидка `total_bytes_estimate` знаменателем не служит.** На живом
//! выводе потока через манифест она пересчитывается на каждом чанке и
//! гуляет на порядок в обе стороны (закреплено тестом
//! `the_estimate_of_a_manifest_stream_swings_by_an_order_of_magnitude` в
//! [`crate::download::progress`]); делить на неё значило бы показывать
//! пляшущий процент. У таких потоков доля берётся по номеру фрагмента —
//! величина грубая (фрагменты неодинаковы), зато монотонная и точная на
//! концах.
//!
//! ## Откат назад
//!
//! Дизайн разрешает немонотонность «на долю процента» при уточнении
//! знаменателя и запрещает обнуление (К-6). Держится это двумя правилами
//! разного веса:
//!
//! - принятые байты потока не убывают (`max` с предыдущим значением) —
//!   байты на диске не исчезают, и продолженная попытка считает их вместе
//!   с накопленным (проверено живьём, см. doc `ProgressSample`);
//! - показанный процент не убывает (храповик [`ProgressAggregator::percent`]).
//!
//! Храповик здесь — страховочная сетка, а не источник приличного вида
//! числа: тест `the_ratchet_never_has_to_engage_on_live_output` требует,
//! чтобы на снятом выводе он не срабатывал ни разу. Если он начнёт
//! срабатывать, значит знаменатель считается неверно, и краснеть должно
//! именно это, а не полоса на экране пользователя.
//!
//! ## Признак потока — обязанность этого модуля
//!
//! `stream` у [`DownloadingState::Running`] опционален, и его отсутствие
//! на проводе означает ровно одно: поток единственный. Это единственное
//! место контракта, где смысл отсутствия поля держится на дисциплине
//! производителя, поэтому производитель здесь один: поле выдаётся строго
//! по [`DownloadPlan`] агрегатора, а не по тому, знает ли он сейчас
//! текущий поток.

use crate::download::progress::{ProgressSample, SampleStatus};
use crate::types::{
    DownloadAttempt, DownloadPercent, DownloadPlan, DownloadStream, DownloadingState,
    QualityStreams,
};

/// Что стало со строкой прогресса.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleOutcome {
    /// Строка отнесена к потоку задачи.
    Applied {
        /// Выросло ли суммарное число принятых байт задачи.
        ///
        /// Это и есть «продвижение» из С-6 и С-8: им сбрасывается счётчик
        /// попыток и им же кормится сторож «ни одного байта за 20 с».
        /// Решение принимает политика повторов (TL-43) — модуль только
        /// сообщает факт.
        advanced: bool,
    },
    /// `format_id` строки не совпал ни с одним потоком выбранного пункта.
    ///
    /// Молча проглотить такую строку нельзя: она означает, что yt-dlp
    /// качает не то, что заказано, — а именно на обещании «скачивание не
    /// делает повторного разбора и берёт ровно выбранные идентификаторы»
    /// (Ф-1, Ф-3 E2) стоит весь эпик. Что с этим делать — залогировать,
    /// отнести к сбою — решает оркестрация.
    UnknownFormat,
}

/// Состояние одного потока задачи.
#[derive(Debug, Clone)]
struct StreamState {
    format_id: String,
    /// Принято байт. Не убывает — см. раздел «Откат назад» в шапке.
    received_bytes: u64,
    /// Точный полный размер, когда его сообщил yt-dlp.
    total_bytes: Option<u64>,
    /// Фрагменты потока через манифест: номер и общее число.
    fragments: Option<(u64, u64)>,
    finished: bool,
}

impl StreamState {
    fn new(format_id: String) -> Self {
        Self {
            format_id,
            received_bytes: 0,
            total_bytes: None,
            fragments: None,
            finished: false,
        }
    }

    /// Поток забран целиком: либо yt-dlp сказал `finished`, либо принято
    /// не меньше точного размера.
    fn is_complete(&self) -> bool {
        self.finished
            || self
                .total_bytes
                .is_some_and(|total| total > 0 && self.received_bytes >= total)
    }

    /// Доля этого потока, если её есть из чего вывести.
    ///
    /// Порядок источников — от точного к грубому; `None` означает «поток
    /// качается, а во сколько байт он обойдётся, не знает никто».
    fn fraction(&self) -> Option<f64> {
        if self.finished {
            return Some(1.0);
        }
        if let Some(total) = self.total_bytes.filter(|total| *total > 0) {
            return Some(ratio(self.received_bytes, total));
        }
        if let Some((index, count)) = self.fragments.filter(|(_, count)| *count > 0) {
            // Индекс живьём доходит до 124 при 123 фрагментах: yt-dlp
            // считает начатые, а не законченные, — отсюда обрезание.
            return Some(ratio(index, count));
        }
        // Поток, о котором ещё не пришло ни байта, честно стоит на нуле;
        // поток с байтами, но без знаменателя, — это отсутствие данных.
        (self.received_bytes == 0).then_some(0.0)
    }
}

/// Агрегатор прогресса одной задачи скачивания.
///
/// Живёт столько же, сколько задача, и переживает её повторы: накопленные
/// байты и показанный процент между попытками не сбрасываются — на этом
/// стоит требование К-6.
#[derive(Debug, Clone)]
pub struct ProgressAggregator {
    video: Option<StreamState>,
    audio: Option<StreamState>,
    /// Оценка размера пункта из разбора E2 — **на оба потока сразу**
    /// (`QualitySize` считается по пункту лестницы, а не по потоку).
    ///
    /// `None` — оценки нет; тогда веса берутся 50/50 (правило дизайна для
    /// неизвестного размера).
    estimated_total_bytes: Option<u64>,
    current: Option<DownloadStream>,
    speed_bytes_per_sec: Option<u64>,
    eta_secs: Option<u64>,
    shown_percent: Option<u8>,
}

impl ProgressAggregator {
    /// Собрать агрегатор по потокам выбранного пункта карточки.
    ///
    /// `estimated_total_bytes` — оценка размера пункта из разбора E2
    /// (`QualitySize::Known`), если она была. Величина одна на пункт:
    /// разбор считает её суммой потоков и по потокам не разносит.
    ///
    /// `None` вместо агрегатора — объект без единого потока: инвариант
    /// [`QualityStreams::has_any`] запрещает такой пункт, и команда старта
    /// отклоняет его классом `noStreamsSelected` раньше (TL-44). Возврат
    /// `Option` здесь — чтобы невозможность была видна в типе, а не
    /// держалась на том, что кто-то раньше проверил.
    pub fn new(streams: &QualityStreams, estimated_total_bytes: Option<u64>) -> Option<Self> {
        let video = streams.video_format_id.clone().map(StreamState::new);
        let audio = streams.audio_format_id.clone().map(StreamState::new);

        (video.is_some() || audio.is_some()).then_some(Self {
            video,
            audio,
            estimated_total_bytes,
            current: None,
            speed_bytes_per_sec: None,
            eta_secs: None,
            shown_percent: None,
        })
    }

    /// Один поток или два — то есть будет ли шаг «Склейка» (Ф-3).
    pub fn plan(&self) -> DownloadPlan {
        if self.video.is_some() && self.audio.is_some() {
            DownloadPlan::VideoAndAudio
        } else {
            DownloadPlan::SingleStream
        }
    }

    /// Учесть строку прогресса.
    pub fn apply(&mut self, sample: &ProgressSample) -> SampleOutcome {
        let Some(stream) = self.stream_of(&sample.format_id) else {
            return SampleOutcome::UnknownFormat;
        };
        let before = self.received_bytes();

        let state = self
            .state_mut(stream)
            .expect("поток найден по своему же идентификатору");
        // Байты не убывают: продолженная попытка считает вместе с
        // накопленным, а попытка, начавшая файл заново, всё равно не
        // повод показывать пользователю откат к нулю (К-6).
        state.received_bytes = state.received_bytes.max(sample.downloaded_bytes);
        if let Some(total) = sample.exact_total_bytes() {
            state.total_bytes = Some(total);
        }
        if let (Some(index), Some(count)) = (sample.fragment_index, sample.fragment_count) {
            state.fragments = Some((index, count));
        }
        if sample.status == SampleStatus::Finished {
            state.finished = true;
            // У `finished` точный размер приходит всегда (обе фикстуры
            // это показывают), но если апстрим его когда-нибудь перестанет
            // слать — размером законченного потока честно считается то,
            // что принято.
            state.total_bytes.get_or_insert(state.received_bytes);
        }

        self.current = Some(stream);
        // У законченного потока нет ни текущей скорости, ни оставшегося
        // времени: yt-dlp кладёт в `speed` строки `finished` среднюю за
        // весь поток, и показать её как мгновенную значило бы соврать —
        // тот же класс, что застывшая скорость из дизайна.
        let live = sample.status == SampleStatus::Downloading;
        self.speed_bytes_per_sec = live.then_some(sample.speed_bytes_per_sec).flatten();
        self.eta_secs = live.then_some(sample.eta_secs).flatten();

        self.update_percent();

        SampleOutcome::Applied {
            advanced: self.received_bytes() > before,
        }
    }

    /// Закрыть поток, который качать не пришлось.
    ///
    /// Нужно ровно для одного случая: yt-dlp печатает
    /// `[download] … has already been downloaded`
    /// ([`crate::download::progress::StdoutLine::AlreadyDownloaded`]) и
    /// **не присылает по такому потоку ни одной строки прогресса, включая
    /// `finished`**. Без этого метода задача, у которой один поток уже
    /// лежал на диске (повтор после неудачной склейки — С-11), навсегда
    /// упиралась бы в 99 %: правило «сто процентов только когда закрыты
    /// все потоки» ждало бы строки, которой не будет.
    ///
    /// Строка `AlreadyDownloaded` несёт путь, а не `format_id`, — сопоставить
    /// их может только тот, кто задавал `-o` (TL-44), поэтому решение
    /// принимает он, а знание о последствиях живёт здесь.
    ///
    /// Возвращает `false`, если такого потока у задачи нет.
    pub fn mark_complete(&mut self, format_id: &str) -> bool {
        let Some(stream) = self.stream_of(format_id) else {
            return false;
        };
        let Some(state) = self.state_mut(stream) else {
            return false;
        };
        state.finished = true;
        self.update_percent();
        true
    }

    /// Показываемый процент — тот, что уйдёт в событие.
    ///
    /// `None`, пока считать не из чего: ни у одного потока нет ни размера,
    /// ни оценки. Ноль в этом случае означал бы «ничего не скачано», что
    /// неправда (правило Ф-2 «отсутствующие данные опускаются»).
    pub fn percent(&self) -> Option<DownloadPercent> {
        self.shown_percent.map(DownloadPercent::new)
    }

    /// Сколько байт задачи принято суммарно по всем потокам.
    ///
    /// Для политики повторов: продвижение сбрасывает счётчик попыток
    /// (С-6), а его отсутствие — повод признать попытку зависшей (С-8).
    pub fn received_bytes(&self) -> u64 {
        self.states()
            .iter()
            .fold(0, |sum, state| sum.saturating_add(state.received_bytes))
    }

    /// Все потоки задачи забраны целиком — качать больше нечего.
    pub fn is_complete(&self) -> bool {
        self.states().iter().all(|state| state.is_complete())
    }

    /// Состояние фазы «Скачивание» для события прогресса.
    ///
    /// `attempt` приходит от политики повторов; номер первой попытки
    /// снимается здесь по правилу контракта («только начиная со второй»),
    /// чтобы обычный путь не рисовал «попытка 1 из 6».
    pub fn running(&self, attempt: Option<DownloadAttempt>) -> DownloadingState {
        DownloadingState::Running {
            stream: self.current_stream(),
            percent: self.percent(),
            speed_bytes_per_sec: self.speed_bytes_per_sec,
            eta_secs: self.eta_secs,
            attempt: attempt.filter(|attempt| attempt.number > 1),
        }
    }

    /// Состояние паузы перед следующей попыткой (С-6).
    ///
    /// Процент замораживается на последнем показанном — К-6 требует, чтобы
    /// он не пропадал и не откатывался; ни скорости, ни оценки времени
    /// здесь нет по устройству контракта.
    pub fn waiting_retry(
        &self,
        attempt: DownloadAttempt,
        delay_secs: u64,
        remaining_secs: u64,
    ) -> DownloadingState {
        DownloadingState::WaitingRetry {
            percent: self.percent(),
            attempt,
            delay_secs,
            remaining_secs,
        }
    }

    /// Какой поток принимается сейчас — только у задачи с двумя потоками.
    fn current_stream(&self) -> Option<DownloadStream> {
        match self.plan() {
            DownloadPlan::VideoAndAudio => self.current,
            DownloadPlan::SingleStream => None,
        }
    }

    fn stream_of(&self, format_id: &str) -> Option<DownloadStream> {
        let matches = |state: &Option<StreamState>| {
            state
                .as_ref()
                .is_some_and(|state| state.format_id == format_id)
        };
        if matches(&self.video) {
            Some(DownloadStream::Video)
        } else if matches(&self.audio) {
            Some(DownloadStream::Audio)
        } else {
            None
        }
    }

    fn state_mut(&mut self, stream: DownloadStream) -> Option<&mut StreamState> {
        match stream {
            DownloadStream::Video => self.video.as_mut(),
            DownloadStream::Audio => self.audio.as_mut(),
        }
    }

    fn states(&self) -> Vec<&StreamState> {
        [self.video.as_ref(), self.audio.as_ref()]
            .into_iter()
            .flatten()
            .collect()
    }

    /// Пересчитать процент и провести его через храповик.
    fn update_percent(&mut self) {
        let Some(computed) = self.computed_percent() else {
            return;
        };
        self.shown_percent = Some(match self.shown_percent {
            Some(shown) => shown.max(computed),
            None => computed,
        });
    }

    /// Честный процент по текущим данным, без храповика.
    fn computed_percent(&self) -> Option<u8> {
        let states = self.states();
        let received: u64 = states
            .iter()
            .fold(0, |sum, state| sum.saturating_add(state.received_bytes));
        let exact: u64 = states
            .iter()
            .filter_map(|state| state.total_bytes)
            .fold(0, u64::saturating_add);
        let unknown: Vec<&&StreamState> = states
            .iter()
            .filter(|state| state.total_bytes.is_none())
            .collect();

        let share = if unknown.is_empty() {
            // Точный размер известен у всех потоков — оценке здесь делать
            // нечего.
            (exact > 0).then(|| ratio(received, exact))?
        } else if let Some(estimate) = self.estimated_total_bytes {
            // Оценка E2 — на пункт целиком, поэтому доля уже измеренных
            // потоков из неё вычитается, а остаток достаётся остальным.
            // `max` с уже принятым по ним не даёт заниженной оценке
            // выдать сотню раньше времени.
            let received_unknown: u64 = unknown
                .iter()
                .fold(0, |sum, state| sum.saturating_add(state.received_bytes));
            let residual = estimate.saturating_sub(exact).max(received_unknown);
            let denominator = exact.saturating_add(residual);
            (denominator > 0).then(|| ratio(received, denominator))?
        } else {
            // Размер хотя бы одного потока неизвестен и оценки нет: веса
            // поровну (50/50 у двух потоков), доли — кто чем может.
            let fractions: Vec<Option<f64>> = states.iter().map(|state| state.fraction()).collect();
            if fractions.iter().all(Option::is_none) {
                return None;
            }
            // Поток без доли голосует нулём, а не выпадает из расчёта:
            // иначе появление у него данных скачком меняло бы вес
            // остальных. Занизить он может только временно, и от отката
            // на экране страхует храповик.
            let sum: f64 = fractions.iter().map(|share| share.unwrap_or(0.0)).sum();
            sum / states.len() as f64
        };

        Some(to_percent(share, self.is_complete()))
    }
}

/// Доля с обрезанием в 0..=1: делитель всегда положителен у вызывающих.
fn ratio(numerator: u64, denominator: u64) -> f64 {
    (numerator as f64 / denominator as f64).clamp(0.0, 1.0)
}

/// Доля в проценты.
///
/// Округление вниз, а не к ближайшему: 99,6 % это ещё не сто, и показывать
/// сотню на незакрытой задаче — ровно тот дефект «нет движения», от
/// которого предостерегает контракт. Сотня выдаётся только когда закрыты
/// все потоки.
fn to_percent(share: f64, complete: bool) -> u8 {
    if complete {
        return 100;
    }
    let percent = (share.clamp(0.0, 1.0) * 100.0).floor();
    (percent as u8).min(99)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::download::fixtures;
    use crate::download::progress::{parse_line, StdoutLine};

    /// Все строки прогресса фикстуры по порядку.
    fn samples(name: &str) -> Vec<ProgressSample> {
        fixtures::stdout(name)
            .lines()
            .filter_map(|line| match parse_line(line) {
                StdoutLine::Progress(sample) => Some(sample),
                _ => None,
            })
            .collect()
    }

    fn streams(video: Option<&str>, audio: Option<&str>) -> QualityStreams {
        QualityStreams {
            video_format_id: video.map(str::to_owned),
            audio_format_id: audio.map(str::to_owned),
        }
    }

    /// Прогнать фикстуру целиком и собрать показанные проценты.
    fn percents(aggregator: &mut ProgressAggregator, name: &str) -> Vec<u8> {
        samples(name)
            .iter()
            .filter_map(|sample| {
                aggregator.apply(sample);
                aggregator.percent().map(DownloadPercent::value)
            })
            .collect()
    }

    /// Строка прогресса, собранная руками.
    ///
    /// Нужна там, где живой вывод нужного случая не даёт: сегодня yt-dlp
    /// сообщает точный размер прямого потока первой же строкой, и
    /// «знаменатель приехал заниженным» на снятом выводе не встречается
    /// (см. тест `the_estimate_of_e2_matches_the_byte_count_yt_dlp_reports`).
    fn sample(format_id: &str, downloaded: u64, total: Option<u64>) -> ProgressSample {
        ProgressSample {
            status: SampleStatus::Downloading,
            format_id: format_id.to_owned(),
            downloaded_bytes: downloaded,
            total_bytes: total,
            total_bytes_estimate: None,
            speed_bytes_per_sec: Some(1_000_000),
            eta_secs: Some(10),
            fragment_index: None,
            fragment_count: None,
        }
    }

    fn finished(format_id: &str, total: u64) -> ProgressSample {
        ProgressSample {
            status: SampleStatus::Finished,
            format_id: format_id.to_owned(),
            downloaded_bytes: total,
            total_bytes: Some(total),
            total_bytes_estimate: None,
            speed_bytes_per_sec: Some(500_000),
            eta_secs: None,
            fragment_index: None,
            fragment_count: None,
        }
    }

    #[test]
    fn an_item_without_a_single_stream_has_no_aggregator() {
        assert!(ProgressAggregator::new(&streams(None, None), Some(100)).is_none());
    }

    #[test]
    fn two_streams_are_weighted_by_bytes_when_the_estimate_of_e2_is_known() {
        // Живой пункт: видео 133 (9 323 483) плюс звук 139 (3 871 021),
        // оценка E2 — их сумма.
        let mut aggregator = ProgressAggregator::new(
            &streams(Some("133"), Some("139")),
            Some(9_323_483 + 3_871_021),
        )
        .expect("потоки заданы");
        assert_eq!(aggregator.plan(), DownloadPlan::VideoAndAudio);

        let shown = percents(&mut aggregator, "video-and-audio.json");

        // Видео весит 70,7 % пункта — на его конце столько и показано, а
        // не 50 % и не 100 %.
        let at_end_of_video = shown
            .iter()
            .copied()
            .find(|percent| *percent >= 70)
            .expect("процент доходит до доли видео");
        assert_eq!(at_end_of_video, 70);
        assert_eq!(shown.last().copied(), Some(100));
        assert!(
            shown.windows(2).all(|pair| pair[0] <= pair[1]),
            "процент не убывает: {shown:?}"
        );
    }

    #[test]
    fn two_streams_without_an_estimate_fall_back_to_equal_weights() {
        // Правило дизайна для неизвестного размера: 50/50 условно. Это
        // огрубление видно прямо в числах — конец видео даёт ровно
        // половину вместо честных 70 %.
        let mut aggregator =
            ProgressAggregator::new(&streams(Some("133"), Some("139")), None).expect("потоки");

        let mut shown = Vec::new();
        let mut at_end_of_video = None;
        for sample in samples("video-and-audio.json") {
            let video_finished =
                sample.format_id == "133" && sample.status == SampleStatus::Finished;
            aggregator.apply(&sample);
            let percent = aggregator.percent().map(DownloadPercent::value);
            if video_finished {
                at_end_of_video = percent;
            }
            shown.extend(percent);
        }

        assert_eq!(
            at_end_of_video,
            Some(50),
            "видео весит 70,7 % пункта, но без оценки размера его вес — \
             условная половина: {shown:?}"
        );
        assert_eq!(
            shown.first().copied(),
            Some(0),
            "шкала начинается с нуля, а не с середины"
        );
        assert_eq!(shown.last().copied(), Some(100));
        assert!(
            shown.windows(2).all(|pair| pair[0] <= pair[1]),
            "и при огрублённых весах не откатывается: {shown:?}"
        );
    }

    #[test]
    fn a_single_stream_never_names_a_stream_on_the_wire() {
        // Единственное место контракта, где смысл отсутствия поля держится
        // на дисциплине производителя, — и производитель здесь один.
        let mut aggregator =
            ProgressAggregator::new(&streams(None, Some("140")), Some(10_271_496)).expect("поток");
        assert_eq!(aggregator.plan(), DownloadPlan::SingleStream);

        for sample in samples("audio-only.json") {
            aggregator.apply(&sample);
            match aggregator.running(None) {
                DownloadingState::Running { stream, .. } => {
                    assert_eq!(stream, None, "у единственного потока признака нет");
                }
                other => panic!("ожидалось Running, пришло {other:?}"),
            }
        }
        assert_eq!(aggregator.percent().map(DownloadPercent::value), Some(100));
    }

    #[test]
    fn two_streams_always_name_the_stream_being_received() {
        let mut aggregator =
            ProgressAggregator::new(&streams(Some("133"), Some("139")), None).expect("потоки");

        let mut seen = Vec::new();
        for sample in samples("video-and-audio.json") {
            aggregator.apply(&sample);
            match aggregator.running(None) {
                DownloadingState::Running { stream, .. } => {
                    let stream = stream.expect("у задачи с двумя потоками признак обязателен");
                    if seen.last() != Some(&stream) {
                        seen.push(stream);
                    }
                }
                other => panic!("ожидалось Running, пришло {other:?}"),
            }
        }

        assert_eq!(
            seen,
            vec![DownloadStream::Video, DownloadStream::Audio],
            "сначала видео, потом звук — и ни одного возврата обратно"
        );
    }

    #[test]
    fn a_refined_denominator_replaces_a_low_estimate_instead_of_pinning_at_a_hundred() {
        // Сердце требования ревью: обрезание сотней — страховка, а не
        // способ считать. Оценка занижена вдвое; если бы знаменатель
        // остался ею, процент упёрся бы в сотню на середине потока и
        // простоял бы там до конца.
        let mut aggregator =
            ProgressAggregator::new(&streams(None, Some("140")), Some(5_000_000)).expect("поток");

        // Первая же строка приносит точный размер — вдвое больший.
        aggregator.apply(&sample("140", 4_000_000, Some(10_271_496)));
        assert_eq!(
            aggregator.percent().map(DownloadPercent::value),
            Some(38),
            "38 % от точного размера, а не 80 % от заниженной оценки"
        );

        aggregator.apply(&sample("140", 10_000_000, Some(10_271_496)));
        assert_eq!(
            aggregator.percent().map(DownloadPercent::value),
            Some(97),
            "и по-прежнему не сотня: поток не закрыт"
        );

        aggregator.apply(&finished("140", 10_271_496));
        assert_eq!(aggregator.percent().map(DownloadPercent::value), Some(100));
    }

    #[test]
    fn a_hundred_is_not_shown_while_a_stream_is_still_pending() {
        // Тот же дефект с другой стороны: оценка на пункт занижена
        // настолько, что вся она укладывается в один поток. Знаменатель
        // обязан идти за фактом, а «сто» — дождаться второго потока.
        let mut aggregator =
            ProgressAggregator::new(&streams(Some("133"), Some("139")), Some(1_000))
                .expect("потоки");

        aggregator.apply(&finished("133", 9_323_483));
        let shown = aggregator
            .percent()
            .map(DownloadPercent::value)
            .expect("процент есть");
        assert_eq!(
            shown, 99,
            "видео забрано целиком, звук не начат — это ещё не сто"
        );

        aggregator.apply(&finished("139", 3_871_021));
        assert_eq!(aggregator.percent().map(DownloadPercent::value), Some(100));
    }

    #[test]
    fn the_estimate_of_a_pending_stream_survives_the_exact_size_of_the_other() {
        // Оценка E2 — на пункт целиком; когда точный размер одного потока
        // приехал, остаток обязан достаться другому, а не пропасть.
        let estimate = 9_323_483 + 3_871_021;
        let mut aggregator =
            ProgressAggregator::new(&streams(Some("133"), Some("139")), Some(estimate))
                .expect("потоки");

        aggregator.apply(&finished("133", 9_323_483));
        assert_eq!(
            aggregator.percent().map(DownloadPercent::value),
            Some(70),
            "знаменатель остался суммой пункта: 9 323 483 из 13 194 504"
        );
    }

    #[test]
    fn a_manifest_stream_is_measured_by_fragments_not_by_the_swinging_estimate() {
        // Прикидка на этой фикстуре гуляет на порядок; если бы делили на
        // неё, процент прыгал бы вверх-вниз десятками.
        let mut aggregator =
            ProgressAggregator::new(&streams(Some("602"), None), None).expect("поток");

        let shown = percents(&mut aggregator, "hls-fragmented.json");

        assert!(
            shown.windows(2).all(|pair| pair[0] <= pair[1]),
            "по фрагментам процент монотонен"
        );
        assert_eq!(shown.last().copied(), Some(100));
        // Середина фикстуры показывает середину потока, а не случайное
        // число: 123 фрагмента, значит около 50 % на 60-м.
        let middle = shown[shown.len() / 2];
        assert!(
            (35..=65).contains(&middle),
            "на середине вывода ожидается середина потока, показано {middle}"
        );
    }

    #[test]
    fn a_resumed_attempt_continues_the_percent_instead_of_zeroing_it() {
        // К-6 буквально: обрыв на 5 %, продолжение — с достигнутого.
        let mut aggregator =
            ProgressAggregator::new(&streams(Some("134"), None), None).expect("поток");

        let interrupted = percents(&mut aggregator, "resume-interrupted.json");
        let stopped_at = *interrupted.last().expect("оборванная попытка что-то дала");
        assert!(stopped_at > 0);

        let continued = percents(&mut aggregator, "resume-continued.json");
        assert!(
            continued.iter().all(|percent| *percent >= stopped_at),
            "ни одно значение продолженной попытки не ниже достигнутого \
             ({stopped_at} %): {continued:?}"
        );
        assert_eq!(continued.last().copied(), Some(100));
    }

    #[test]
    fn the_ratchet_never_has_to_engage_on_live_output() {
        // Храповик — страховочная сетка. Если он начинает что-то держать,
        // значит знаменатель считается неверно, и это должно краснеть
        // здесь, а не выглядеть ровной полосой у пользователя.
        //
        // Проверяется на всех фикстурах со строками прогресса и в обоих
        // режимах веса — с оценкой E2 и без неё.
        let cases: [(&str, QualityStreams, Option<u64>); 4] = [
            (
                "video-and-audio.json",
                streams(Some("133"), Some("139")),
                Some(9_323_483 + 3_871_021),
            ),
            (
                "video-and-audio.json",
                streams(Some("133"), Some("139")),
                None,
            ),
            ("hls-fragmented.json", streams(Some("602"), None), None),
            (
                "audio-only.json",
                streams(None, Some("140")),
                Some(10_271_496),
            ),
        ];

        for (name, item, estimate) in cases {
            let mut aggregator = ProgressAggregator::new(&item, estimate).expect("потоки заданы");
            let mut previous = 0u8;

            for sample in samples(name) {
                aggregator.apply(&sample);
                let computed = aggregator
                    .computed_percent()
                    .expect("на снятом выводе процент считается всегда");
                assert!(
                    computed >= previous,
                    "{name} (оценка {estimate:?}): честный процент упал с \
                     {previous} до {computed} — храповик прикрыл бы это, но \
                     держать его должен верный знаменатель, а не он"
                );
                previous = computed;
            }
        }
    }

    #[test]
    fn a_stream_that_never_reports_is_closed_by_hand_or_the_task_hangs_at_99() {
        // Повтор после неудачной склейки (С-11): оба потока уже на диске,
        // yt-dlp печатает «has already been downloaded» и молчит.
        let mut aggregator = ProgressAggregator::new(
            &streams(Some("133"), Some("139")),
            Some(9_323_483 + 3_871_021),
        )
        .expect("потоки");

        aggregator.apply(&finished("133", 9_323_483));
        assert_eq!(aggregator.percent().map(DownloadPercent::value), Some(70));

        assert!(aggregator.mark_complete("139"));
        assert!(aggregator.is_complete());
        assert_eq!(aggregator.percent().map(DownloadPercent::value), Some(100));

        assert!(
            !aggregator.mark_complete("251"),
            "закрыть можно только поток этой задачи"
        );
    }

    #[test]
    fn a_line_about_a_format_nobody_asked_for_is_reported_not_absorbed() {
        let mut aggregator =
            ProgressAggregator::new(&streams(Some("133"), Some("139")), None).expect("потоки");

        assert_eq!(
            aggregator.apply(&sample("251", 1024, Some(9_000_000))),
            SampleOutcome::UnknownFormat
        );
        assert_eq!(aggregator.received_bytes(), 0);
        assert_eq!(aggregator.percent(), None);
    }

    #[test]
    fn progress_of_the_task_is_the_sum_over_streams_and_it_only_grows() {
        // Вход политики повторов: продвижение сбрасывает счётчик (С-6),
        // его отсутствие кормит сторож «ни байта за 20 с» (С-8).
        let mut aggregator =
            ProgressAggregator::new(&streams(Some("133"), Some("139")), None).expect("потоки");

        assert_eq!(
            aggregator.apply(&sample("133", 1024, Some(9_323_483))),
            SampleOutcome::Applied { advanced: true }
        );
        assert_eq!(
            aggregator.apply(&sample("133", 1024, Some(9_323_483))),
            SampleOutcome::Applied { advanced: false },
            "та же величина — это не продвижение"
        );
        assert_eq!(
            aggregator.apply(&sample("133", 512, Some(9_323_483))),
            SampleOutcome::Applied { advanced: false },
            "меньшая величина — тем более"
        );
        assert_eq!(aggregator.received_bytes(), 1024);

        aggregator.apply(&sample("139", 2048, Some(3_871_021)));
        assert_eq!(
            aggregator.received_bytes(),
            3072,
            "байты складываются по потокам, а не заменяются"
        );
    }

    #[test]
    fn a_finished_stream_reports_no_speed_and_no_eta() {
        // Средняя за весь поток из строки `finished` — не мгновенная
        // скорость; показать её значило бы соврать ровно так же, как
        // застывшим числом при нулевом движении.
        let mut aggregator =
            ProgressAggregator::new(&streams(None, Some("140")), None).expect("поток");

        aggregator.apply(&sample("140", 1_000_000, Some(10_271_496)));
        match aggregator.running(None) {
            DownloadingState::Running {
                speed_bytes_per_sec,
                eta_secs,
                ..
            } => {
                assert_eq!(speed_bytes_per_sec, Some(1_000_000));
                assert_eq!(eta_secs, Some(10));
            }
            other => panic!("ожидалось Running, пришло {other:?}"),
        }

        aggregator.apply(&finished("140", 10_271_496));
        match aggregator.running(None) {
            DownloadingState::Running {
                speed_bytes_per_sec,
                eta_secs,
                ..
            } => {
                assert_eq!(speed_bytes_per_sec, None);
                assert_eq!(eta_secs, None);
            }
            other => panic!("ожидалось Running, пришло {other:?}"),
        }
    }

    #[test]
    fn the_first_attempt_is_not_numbered_and_the_pause_freezes_the_percent() {
        let mut aggregator =
            ProgressAggregator::new(&streams(None, Some("140")), None).expect("поток");
        aggregator.apply(&sample("140", 5_000_000, Some(10_271_496)));

        let first = DownloadAttempt {
            number: 1,
            total: 6,
        };
        match aggregator.running(Some(first)) {
            DownloadingState::Running { attempt, .. } => assert_eq!(
                attempt, None,
                "«попытка 1 из 6» на обычном пути не рисуется"
            ),
            other => panic!("ожидалось Running, пришло {other:?}"),
        }

        let second = DownloadAttempt {
            number: 2,
            total: 6,
        };
        match aggregator.running(Some(second)) {
            DownloadingState::Running { attempt, .. } => assert_eq!(attempt, Some(second)),
            other => panic!("ожидалось Running, пришло {other:?}"),
        }

        assert_eq!(
            aggregator.waiting_retry(second, 8, 5),
            DownloadingState::WaitingRetry {
                percent: Some(DownloadPercent::new(48)),
                attempt: second,
                delay_secs: 8,
                remaining_secs: 5,
            },
            "в паузе виден последний известный процент — «сохранено» (К-6)"
        );
    }

    #[test]
    fn nothing_is_shown_until_there_is_something_to_divide_by() {
        // Ноль означал бы «ничего не скачано»; правильный ответ здесь —
        // «числа нет» (Ф-2).
        let mut aggregator =
            ProgressAggregator::new(&streams(Some("602"), None), None).expect("поток");
        assert_eq!(aggregator.percent(), None);

        // Байты идут, но ни размера, ни фрагментов нет.
        let blind = ProgressSample {
            status: SampleStatus::Downloading,
            format_id: "602".to_owned(),
            downloaded_bytes: 4096,
            total_bytes: None,
            total_bytes_estimate: Some(1_000_000),
            speed_bytes_per_sec: Some(1024),
            eta_secs: None,
            fragment_index: None,
            fragment_count: None,
        };
        aggregator.apply(&blind);
        assert_eq!(
            aggregator.percent(),
            None,
            "прикидка знаменателем не служит, и придумывать число не из чего"
        );
    }

    #[test]
    fn the_estimate_of_e2_matches_the_byte_count_yt_dlp_reports() {
        // Почему заниженный знаменатель проверяется собранными руками
        // строками, а не фикстурой: на живых данных занижения нет.
        // `filesize` из метаданных разбора E2 совпадает с `total_bytes`
        // строки прогресса до байта — сверено по фикстуре лестницы того же
        // ролика. Если апстрим разведёт эти величины, тест покраснеет, и
        // случай станет фикстурным.
        let metadata = fixtures::probe_metadata("4k-full-ladder.json");
        let formats = metadata
            .get("formats")
            .and_then(serde_json::Value::as_array)
            .expect("в фикстуре лестницы есть форматы");

        for (format_id, reported) in [
            ("133", 9_323_483_u64),
            ("139", 3_871_021),
            ("140", 10_271_496),
        ] {
            let declared = formats
                .iter()
                .find(|format| {
                    format.get("format_id").and_then(serde_json::Value::as_str) == Some(format_id)
                })
                .and_then(|format| format.get("filesize"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_else(|| panic!("у формата {format_id} объявлен размер"));

            assert_eq!(
                declared, reported,
                "оценка размера из разбора E2 и число байт из строки прогресса \
                 у формата {format_id} обязаны совпадать — на этом стоит выбор \
                 «занижение знаменателя проверяем собранными строками»"
            );
        }
    }
}
