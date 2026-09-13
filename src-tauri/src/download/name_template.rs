//! Шаблон имени файла: разбор, проверка и подстановка (Ф-12 E5) — TL-86.
//!
//! Шаблон — строка из литералов и переменных вида `{имя}`. Например,
//! `{id} — {title}` даёт `dQw4w9WgXcQ — Как приручить дракона`. Модуль
//! чистый: диска не трогает, часов не читает, процессов не запускает.
//!
//! # Белый список, а не чёрный
//!
//! Переменных ровно четыре ([`Variable::ALL`]), и они выписаны из Ф-12
//! буквально: `{title}`, `{id}`, `{quality}`, `{date}`. Всё, что не
//! совпадает с одним из четырёх имён байт в байт, отклоняется, в том числе
//! `{Title}`, `{ title }`, `{}` и `{channel}`. Одиночная `}` и незакрытая
//! `{` тоже отклоняются, а не становятся литералом. Экранирования скобок
//! нет: Ф-12 его не называет, и литеральная скобка в имени файла не стоит
//! ещё одного правила разбора.
//!
//! Разбор идёт слева направо, отказ — первая найденная проблема. Шаблон без
//! единой переменной ([`TemplateProblem::NoVariables`], С-9) проверяется
//! после синтаксиса: иначе все файлы получили бы одно имя с суффиксами
//! « (N)».
//!
//! # Один конвейер имени, а не два
//!
//! Подстановка не санитизирует ничего сама. Строка после подстановки
//! **целиком** уходит в [`sanitized_stem`] E3 — тот же код, что строил имя
//! из названия до E5 (Ф-12, анализ E5: «шаблон — через существующую
//! санитизацию, не через свою»). Отсюда все гарантии E3 для литералов
//! шаблона и значений сразу: разделители путей, `..`, символы Windows,
//! зарезервированные имена, невидимые символы, края, бюджет длины.
//!
//! Сырая подстановка наружу не отдаётся. Единственный публичный выход —
//! [`NameTemplate::file_stem`], и он зовёт [`sanitized_stem`] сам. Так
//! «забыть санитизировать» нечем. Это отступление от сигнатуры
//! `render -> String` в тексте задачи: там результат рендера был публичным
//! и несанитизированным.
//!
//! Следствия, закреплённые тестами:
//!
//! - шаблон не создаёт подпапку и не выводит файл из папки назначения —
//!   корпус инъекций проверяется обходом ФС после настоящей записи файла;
//! - значение переменной не разбирается как шаблон: `{id}` в названии —
//!   это текст, а не переменная. Разбор идёт один раз, по шаблону, и
//!   данные в него не попадают по построению;
//! - пустая основа после санитизации получает запасное имя из id, как в
//!   E3. Это не ошибка шаблона: `{quality}` пуст не у каждой задачи;
//! - умолчание `{title}` даёт байт в байт прежние имена E3.
//!
//! # Чего модуль не делает
//!
//! Он не знает, какой сегодня день по локальному времени, и не извлекает
//! id из ссылки. Оба значения приносит вызывающий в [`TemplateContext`]:
//! оркестрация (TL-89) — дату завершения и id задачи, команда
//! предпросмотра (TL-91) — образец. Дата при этом — не строка, а
//! [`TemplateDate`], и форму `ГГГГ-ММ-ДД` строит этот модуль.

use std::fmt;

use super::filename::sanitized_stem;
use crate::types::{QualityKind, SelectedQuality, TemplateProblem};

/// Шаблон по умолчанию (Ф-10). Даёт имена, тождественные E3.
pub const DEFAULT_TEMPLATE: &str = "{title}";

/// Переменная шаблона — одна из четырёх в белом списке Ф-12.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variable {
    /// `{title}` — название ролика.
    Title,
    /// `{id}` — id ролика. Тот же, что идёт в запасное имя.
    Id,
    /// `{quality}` — подпись пункта качества, см. [`quality_label`].
    Quality,
    /// `{date}` — дата в форме `ГГГГ-ММ-ДД`, см. [`TemplateDate`].
    Date,
}

impl Variable {
    /// Белый список целиком. Разбор сверяется только с ним.
    pub const ALL: [Self; 4] = [Self::Title, Self::Id, Self::Quality, Self::Date];

    /// Имя переменной между скобками.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Id => "id",
            Self::Quality => "quality",
            Self::Date => "date",
        }
    }

    /// Переменная по имени — только точное совпадение с белым списком.
    fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|variable| variable.name() == name)
    }
}

/// Кусок разобранного шаблона.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Segment {
    /// Текст шаблона как есть. Санитизируется вместе со всем результатом.
    Literal(String),
    /// Место подстановки.
    Variable(Variable),
}

/// Разобранный и проверенный шаблон имени.
///
/// Поля приватные, конструктор — [`NameTemplate::parse`] (и `Default`).
/// Значит у каждого существующего `NameTemplate` синтаксис верен и есть
/// хотя бы одна переменная: проверять его повторно негде и незачем.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameTemplate {
    source: String,
    segments: Vec<Segment>,
}

impl NameTemplate {
    /// Разобрать и проверить шаблон.
    ///
    /// # Errors
    ///
    /// Первая проблема слева направо, позиция — номер символа Unicode с
    /// единицы (doc [`TemplateProblem`]):
    ///
    /// - [`TemplateProblem::UnknownVariable`] — `{имя}` не из белого списка,
    ///   позиция открывающей скобки;
    /// - [`TemplateProblem::UnclosedBrace`] — `{` без `}` до конца строки
    ///   или до следующей `{`, позиция открывающей скобки;
    /// - [`TemplateProblem::StrayClosingBrace`] — `}` без `{`;
    /// - [`TemplateProblem::NoVariables`] — синтаксис верен, но переменных
    ///   нет.
    pub fn parse(template: &str) -> Result<Self, TemplateProblem> {
        let chars: Vec<char> = template.chars().collect();
        let mut segments = Vec::new();
        let mut literal = String::new();
        let mut index = 0;

        while let Some(&ch) = chars.get(index) {
            match ch {
                '{' => {
                    let rest = chars.get(index + 1..).unwrap_or_default();
                    // Имя кончается на первой скобке любого вида. `{` раньше
                    // `}` значит, что текущая скобка так и не закрылась:
                    // `{ti{tle}` — незакрытая в символе 1, а не переменная
                    // с именем `ti{tle`.
                    let end = rest.iter().position(|&c| c == '{' || c == '}');
                    match end.map(|offset| (offset, rest.get(offset))) {
                        Some((offset, Some(&'}'))) => {
                            let name: String =
                                rest.get(..offset).unwrap_or_default().iter().collect();
                            let Some(variable) = Variable::from_name(&name) else {
                                return Err(TemplateProblem::UnknownVariable {
                                    position: position_of(index),
                                    name,
                                });
                            };
                            if !literal.is_empty() {
                                segments.push(Segment::Literal(std::mem::take(&mut literal)));
                            }
                            segments.push(Segment::Variable(variable));
                            index += offset + 2;
                        }
                        _ => {
                            return Err(TemplateProblem::UnclosedBrace {
                                position: position_of(index),
                            })
                        }
                    }
                }
                '}' => {
                    return Err(TemplateProblem::StrayClosingBrace {
                        position: position_of(index),
                    })
                }
                other => {
                    literal.push(other);
                    index += 1;
                }
            }
        }

        if !literal.is_empty() {
            segments.push(Segment::Literal(literal));
        }

        if !segments
            .iter()
            .any(|segment| matches!(segment, Segment::Variable(_)))
        {
            return Err(TemplateProblem::NoVariables);
        }

        Ok(Self {
            source: template.to_string(),
            segments,
        })
    }

    /// Текст шаблона, как его ввёл пользователь, — для хранения.
    pub fn as_str(&self) -> &str {
        &self.source
    }

    /// Разобранные куски шаблона.
    #[cfg(test)]
    pub(super) fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// Основа имени файла (без расширения и суффикса коллизии).
    ///
    /// Подстановка, затем [`sanitized_stem`] E3 целиком. Пустая после
    /// санитизации основа получает запасное имя из `ctx.video_id` — это
    /// делает сама [`sanitized_stem`]. Результат годится везде, где до E5
    /// годился выход [`sanitized_stem`]: в склейку и в финализацию.
    pub fn file_stem(&self, ctx: &TemplateContext<'_>) -> String {
        sanitized_stem(&self.substituted(ctx), ctx.video_id)
    }

    /// Сырая подстановка. Наружу не отдаётся — см. шапку модуля.
    fn substituted(&self, ctx: &TemplateContext<'_>) -> String {
        let mut out = String::new();
        for segment in &self.segments {
            match segment {
                Segment::Literal(text) => out.push_str(text),
                Segment::Variable(Variable::Title) => out.push_str(ctx.title),
                Segment::Variable(Variable::Id) => out.push_str(ctx.video_id),
                Segment::Variable(Variable::Quality) => out.push_str(&quality_label(ctx.quality)),
                Segment::Variable(Variable::Date) => out.push_str(&ctx.date.to_string()),
            }
        }
        out
    }
}

impl Default for NameTemplate {
    /// [`DEFAULT_TEMPLATE`] в разобранном виде. Собран руками, без `parse`,
    /// чтобы в продакшен-пути не было `unwrap`; совпадение с разбором
    /// проверяет тест.
    fn default() -> Self {
        Self {
            source: DEFAULT_TEMPLATE.to_string(),
            segments: vec![Segment::Variable(Variable::Title)],
        }
    }
}

/// Значения переменных для одной подстановки.
///
/// Строковые поля — непроверенный ввод, так и задумано: их чистит
/// [`sanitized_stem`] вместе со всем результатом.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemplateContext<'a> {
    /// Название ролика.
    pub title: &'a str,
    /// Id ролика: значение `{id}` и основа запасного имени. Одно поле на
    /// оба места намеренно. Тождество `{title}` с E3 держится, только если
    /// вызывающий передаёт тот же id, что E3 брал для запасного имени, —
    /// `video_id_of(url)`, а не канонический id TL-72 (решение ведущего
    /// по ревью TL-86, записано в #97).
    pub video_id: &'a str,
    /// Выбранный пункт качества.
    pub quality: SelectedQuality,
    /// Дата для `{date}`: у загрузки — дата завершения по локальному
    /// времени (Ф-12), у предпросмотра — сегодняшняя.
    pub date: TemplateDate,
}

/// Подпись пункта качества для `{quality}` (Ф-12).
///
/// Ступень — `1080p`, `2160p`; «максимальное доступное» — тоже `NNNp`,
/// потому что в имени файла важна ступень, а не то, как пункт назван в
/// списке. «Только аудио» — `audio`. Видеопункт без ступени (контракт
/// такого не присылает) даёт пустую подпись, а не выдуманную.
pub fn quality_label(quality: SelectedQuality) -> String {
    match (quality.kind, quality.height_px) {
        (QualityKind::AudioOnly, _) => "audio".to_string(),
        (QualityKind::Standard | QualityKind::MaxAvailable, Some(height)) => format!("{height}p"),
        (QualityKind::Standard | QualityKind::MaxAvailable, None) => String::new(),
    }
}

/// Календарная дата для `{date}`.
///
/// Тип, а не строка: форма `ГГГГ-ММ-ДД` строится здесь, а вызывающий не
/// может подсунуть под видом даты произвольный текст. Конструктор проверяет
/// дату целиком, включая 29 февраля.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemplateDate {
    year: u16,
    month: u8,
    day: u8,
}

impl TemplateDate {
    /// Дата, если такая есть в григорианском календаре; год 1…9999, чтобы
    /// форма всегда была из четырёх цифр.
    // Дату строят оркестрация (TL-89) и предпросмотр (TL-91); хранилище
    // настроек (TL-87) её не строит.
    #[allow(dead_code)]
    pub fn new(year: u16, month: u8, day: u8) -> Option<Self> {
        let days_in_month = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if is_leap_year(year) => 29,
            2 => 28,
            _ => return None,
        };
        ((1..=9999).contains(&year) && (1..=days_in_month).contains(&day)).then_some(Self {
            year,
            month,
            day,
        })
    }
}

impl fmt::Display for TemplateDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

fn is_leap_year(year: u16) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Проверить шаблон и построить по нему основу имени на образце.
///
/// Единый валидатор Ф-12 для трёх вызовов: сохранение (TL-87), чтение файла
/// настроек (TL-87) и предпросмотр (TL-91). Образец — `sample`: команда
/// предпросмотра собирает его из констант `crate::commands::settings` и
/// сегодняшней даты. Результат — `TemplatePreview::result`.
///
/// # Errors
///
/// Те же, что у [`NameTemplate::parse`]. Пустого результата среди них нет:
/// пустая основа получает запасное имя (Ф-12).
// Сохранение (TL-87) зовёт `NameTemplate::parse` напрямую — ему нужен
// разобранный шаблон, а не пример; эту функцию зовёт предпросмотр (TL-91).
#[allow(dead_code)]
pub fn validate_for_save(
    template: &str,
    sample: &TemplateContext<'_>,
) -> Result<String, TemplateProblem> {
    NameTemplate::parse(template).map(|parsed| parsed.file_stem(sample))
}

/// Номер символа для показа: с единицы, насыщение на `u32::MAX`.
fn position_of(index: usize) -> u32 {
    u32::try_from(index.saturating_add(1)).unwrap_or(u32::MAX)
}

#[cfg(test)]
#[path = "name_template_tests.rs"]
mod tests;
