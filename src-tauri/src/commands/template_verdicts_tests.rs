//! Фикстура вердиктов шаблона имени для клиентского валидатора (TL-100).
//!
//! # Зачем
//!
//! Кнопку «Сохранить» на экране настроек держит только клиентская проверка
//! `src/utils/nameTemplateClientCheck.ts`, которая повторяет правила ядра.
//! Если клиент окажется строже ядра, допустимый шаблон станет несохраняемым
//! навсегда: до сервера такой ввод не доходит. Этот модуль выгружает корпус
//! шаблонов с вердиктом ядра в `tests/fixtures/name-template-verdicts.json`,
//! а ui-тест (TL-101, #108) сверяет с ним клиента.
//!
//! # Цепочка вердикта: та же функция, что у команды
//!
//! Вердикт строит [`preview_on`], то есть тело `preview_name_template`, без
//! копии цепочки: сначала `check_template_length` (предел
//! `NAME_TEMPLATE_MAX_CHARS`), затем `validate_for_save` (разбор и
//! `noVariables`), затем перевод в контракт `set_error_to_contract`. Поэтому
//! `verdict` в фикстуре — сериализованный контрактный [`TemplateProblem`],
//! тот же JSON, что получает webview, а не форма, написанная руками. Дата
//! образца на вердикт не влияет: она участвует только в примере имени.
//!
//! # Сторож и генератор
//!
//! Приём зеркала TL-51 (`types::bindings`). По умолчанию тест **сверяет**
//! построенную фикстуру с закоммиченной, байт в байт, и при расхождении
//! падает, называя первый расходящийся шаблон. Переписывает файл только
//! `TUBE_LEAK_UPDATE_TEMPLATE_VERDICTS=1`, и именно `=1`: пустое значение
//! в окружении не должно молча выключать сторожа.
//!
//! # Корпус
//!
//! Корпус не зависит от проверяемых правил. Границы длины записаны числами
//! 199/200/201, а не выведены из `NAME_TEMPLATE_MAX_CHARS`, иначе мутация
//! предела сдвинула бы корпус вместе с вердиктами. Состав:
//!
//! 1. все последовательности из 0…3 атомов [`ATOMS`];
//! 2. [`LONG_SAMPLES`] длинных шаблонов из кусков [`COMMON_PIECES`] и
//!    [`RARE_PIECES`], LCG с зерном [`SEED`];
//! 3. шаблоны длиной 199/200/201/1000 символов Unicode из пяти видов
//!    заполнителя ([`FILLERS`]) в шести формах ([`SHAPES`]). Заполнитель
//!    режется по скалярам, не по графемам: ZWJ-семья и `e`+U+0301 на границе
//!    бывают разорваны, как у пользователя, который упёрся в предел;
//! 4. точечные случаи [`SPECIALS`].
//!
//! Повторы убираются с сохранением первого вхождения.
//!
//! # Чего сторож не видит
//!
//! - Шаблоны вне корпуса. Полнота здесь не доказывается, доказывается
//!   только совпадение на корпусе.
//! - Одиночные суррогаты UTF-16. Строка Rust их не содержит, поэтому в
//!   фикстуре их нет. До ядра такой ввод тоже не доходит (doc
//!   `preview_name_template`), но что с ним сделает клиент, эта фикстура не
//!   проверяет.
//! - Сам клиент: сверку делает ui-тест #108, а этот модуль гарантирует
//!   только то, что фикстура говорит правду о ядре.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use serde::Serialize;
use serde_json::Value;

use super::*;

/// Переменная, по которой тест перестаёт быть сторожем и переписывает
/// фикстуру.
const UPDATE_ENV: &str = "TUBE_LEAK_UPDATE_TEMPLATE_VERDICTS";

/// Единственное значение [`UPDATE_ENV`], включающее перезапись (почему не
/// «задана ли» — doc модуля).
const UPDATE_ON: &str = "1";

/// Команда перегенерации для текста падения и шапки фикстуры.
const UPDATE_HINT: &str =
    "cd src-tauri && TUBE_LEAK_UPDATE_TEMPLATE_VERDICTS=1 cargo test --locked template_verdicts";

/// Путь фикстуры: от `CARGO_MANIFEST_DIR`, а не от cwd.
fn fixture_path() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/name-template-verdicts.json"
    ))
}

/// Алфавит полного перебора. Непечатное записано экранами, чтобы оно было
/// видно в исходнике.
const ATOMS: [&str; 12] = [
    "{",
    "}",
    "title",
    "id",
    "quality",
    "date",
    "Title",
    " ",
    "x",
    "\u{1F600}",
    // Семья: мужчина ZWJ женщина ZWJ девочка — 5 скаляров, 8 единиц UTF-16.
    "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}",
    // `e` и комбинирующий акут — 2 скаляра, одна графема.
    "e\u{301}",
];

/// Наибольшая длина полного перебора в атомах.
const EXHAUSTIVE_MAX_ATOMS: u32 = 3;

/// Зерно генератора длинных шаблонов.
const SEED: u64 = 0x5475_6265_4c65_616b;

/// Число длинных шаблонов.
const LONG_SAMPLES: usize = 400;

/// Куски длинных шаблонов, не ломающие синтаксис: атомы без скобок и
/// переменные белого списка целиком. Иначе почти каждый длинный шаблон
/// отказывал бы на первых символах, и далёкие позиции остались бы
/// непроверенными.
const COMMON_PIECES: [&str; 14] = [
    "title",
    "id",
    "quality",
    "date",
    "Title",
    " ",
    "x",
    "\u{1F600}",
    "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}",
    "e\u{301}",
    "{title}",
    "{id}",
    "{quality}",
    "{date}",
];

/// Редкие куски: каждый даёт отказ разбора в своём месте.
const RARE_PIECES: [&str; 4] = ["{", "}", "{Title}", "{}"];

/// Доля редкого куска: один из стольких.
const RARE_ONE_IN: usize = 16;

/// Длины длинных шаблонов в кусках, включительно.
const LONG_PIECES_MIN: usize = 4;
const LONG_PIECES_MAX: usize = 60;

/// Длины шаблонов на границе предела, в символах Unicode. Числа, а не
/// производные от предела (doc модуля). 1000 — далеко за пределом.
const BOUNDARY_LENGTHS: [usize; 4] = [199, 200, 201, 1000];

/// Заполнители шаблонов на границе.
const FILLERS: [&str; 5] = [
    "a",
    "я",
    "\u{1F600}",
    "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}",
    "e\u{301}",
];

/// Формы шаблонов на границе: `(префикс, суффикс)` вокруг заполнителя.
const SHAPES: [(&str, &str); 6] = [
    // допустим, переменная в начале
    ("{title}", ""),
    // допустим, переменная в конце: разбор проходит всю строку
    ("", "{title}"),
    // одиночная `}` в последнем символе: позиция считается через заполнитель
    ("", "}"),
    // неизвестная переменная в конце
    ("", "{x}"),
    // одни литералы: `noVariables`
    ("", ""),
    // отказ разбора в символе 1: за пределом длина обязана победить
    ("}", ""),
];

/// Точечные случаи из требований и ревью.
const SPECIALS: [&str; 12] = [
    "{{title}}",
    "{}",
    "{ title}",
    "{ti{tle}",
    "{title }",
    "{title}}",
    "{title",
    "title}",
    "{Title}",
    "{channel}",
    "{title}{id}{quality}{date}",
    "видео",
];

/// Размер корпуса после удаления повторов. Нижняя граница у сторожа: пустой
/// или выродившийся корпус не должен проходить сверку вхолостую. Меняется
/// осознанно вместе с составом корпуса.
const CORPUS_SIZE: usize = 2_413;

/// Линейный конгруэнтный генератор (константы MMIX, Кнут). Старшие биты:
/// у младших короткий период.
struct Lcg(u64);

impl Lcg {
    fn below(&mut self, bound: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        usize::try_from(self.0 >> 33).unwrap_or(0) % bound
    }
}

/// Корпус шаблонов в порядке фикстуры.
fn corpus() -> Vec<String> {
    let mut templates = Vec::new();

    for atoms in 0..=EXHAUSTIVE_MAX_ATOMS {
        for mut code in 0..ATOMS.len().pow(atoms) {
            let mut digits = Vec::new();
            for _ in 0..atoms {
                digits.push(code % ATOMS.len());
                code /= ATOMS.len();
            }
            // Старший разряд первым: порядок лексикографический по индексу.
            templates.push(digits.iter().rev().map(|&index| ATOMS[index]).collect());
        }
    }

    let mut rng = Lcg(SEED);
    for _ in 0..LONG_SAMPLES {
        let pieces = LONG_PIECES_MIN + rng.below(LONG_PIECES_MAX - LONG_PIECES_MIN + 1);
        let template: String = (0..pieces)
            .map(|_| {
                if rng.below(RARE_ONE_IN) == 0 {
                    RARE_PIECES[rng.below(RARE_PIECES.len())]
                } else {
                    COMMON_PIECES[rng.below(COMMON_PIECES.len())]
                }
            })
            .collect();
        templates.push(template);
    }

    for length in BOUNDARY_LENGTHS {
        for filler in FILLERS {
            for (prefix, suffix) in SHAPES {
                let fill = length - prefix.chars().count() - suffix.chars().count();
                let template: String = prefix
                    .chars()
                    .chain(filler.chars().cycle().take(fill))
                    .chain(suffix.chars())
                    .collect();
                assert_eq!(template.chars().count(), length, "{template:?}");
                templates.push(template);
            }
        }
    }

    templates.extend(SPECIALS.iter().map(|&s| s.to_owned()));

    let mut seen = HashSet::new();
    templates.retain(|template| seen.insert(template.clone()));
    templates
}

fn sample_date() -> TemplateDate {
    TemplateDate::new(2026, 9, 14).expect("дата")
}

/// Вердикт ядра: `None` — шаблон допустим, иначе проблема контракта.
fn verdict_of(template: &str) -> Option<TemplateProblem> {
    match preview_on(template, sample_date()) {
        Ok(_) => None,
        Err(SettingsCommandError {
            kind: SettingsCommandErrorKind::InvalidTemplate { problem },
            ..
        }) => Some(problem),
        Err(other) => panic!("предпросмотр {template:?} отказал не классом шаблона: {other:?}"),
    }
}

/// Запись фикстуры. `verdict: None` сериализуется в `null`.
#[derive(Serialize)]
struct Record {
    template: String,
    verdict: Option<TemplateProblem>,
}

/// Номер класса вердикта для счёта покрытия. `match` исчерпывающий: новый
/// вариант `TemplateProblem` не соберётся, пока его не впишут сюда.
fn class_of(verdict: Option<&TemplateProblem>) -> usize {
    match verdict {
        None => 0,
        Some(TemplateProblem::UnknownVariable { .. }) => 1,
        Some(TemplateProblem::UnclosedBrace { .. }) => 2,
        Some(TemplateProblem::StrayClosingBrace { .. }) => 3,
        Some(TemplateProblem::NoVariables) => 4,
        Some(TemplateProblem::TooLong { .. }) => 5,
    }
}

const CLASS_NAMES: [&str; 6] = [
    "допустим",
    "unknownVariable",
    "unclosedBrace",
    "strayClosingBrace",
    "noVariables",
    "tooLong",
];

fn records() -> Vec<Record> {
    corpus()
        .into_iter()
        .map(|template| {
            let verdict = verdict_of(&template);
            Record { template, verdict }
        })
        .collect()
}

/// Текст фикстуры: шапка и по записи на строку, чтобы дифф был построчным.
fn render(records: &[Record]) -> String {
    let header = |value: &str| serde_json::to_string(value).expect("строка сериализуется");
    let lines: Vec<String> = records
        .iter()
        .map(|record| {
            format!(
                "    {}",
                serde_json::to_string(record).expect("запись сериализуется")
            )
        })
        .collect();
    format!(
        "{{\n  \"about\": {},\n  \"regenerate\": {},\n  \"count\": {},\n  \"verdicts\": [\n{}\n  ]\n}}\n",
        header(
            "СГЕНЕРИРОВАНО ядром (src-tauri/src/commands/template_verdicts_tests.rs, TL-100). Руками не править. verdict: null — шаблон допустим, иначе TemplateProblem в форме провода. Цепочка — preview_on: длина, затем разбор."
        ),
        header(UPDATE_HINT),
        records.len(),
        lines.join(",\n"),
    )
}

/// Записи закоммиченной фикстуры: `(template, verdict)`.
fn parse_committed(text: &str) -> Result<Vec<(String, Value)>, String> {
    let root: Value = serde_json::from_str(text).map_err(|err| err.to_string())?;
    let list = root
        .get("verdicts")
        .and_then(Value::as_array)
        .ok_or("нет массива verdicts")?;
    list.iter()
        .enumerate()
        .map(|(index, entry)| {
            let template = entry
                .get("template")
                .and_then(Value::as_str)
                .ok_or(format!("запись #{index} без строки template"))?;
            let verdict = entry
                .get("verdict")
                .ok_or(format!("запись #{index} без verdict"))?;
            Ok((template.to_owned(), verdict.clone()))
        })
        .collect()
}

/// Первое расхождение записей, если оно есть.
fn first_divergence(expected: &[Record], committed: &[(String, Value)]) -> Option<String> {
    for (index, record) in expected.iter().enumerate() {
        let core = serde_json::to_value(&record.verdict).expect("вердикт сериализуется");
        match committed.get(index) {
            None => {
                return Some(format!(
                    "в фикстуре {} записей, у корпуса больше; первая недостающая #{index}: {:?}",
                    committed.len(),
                    record.template
                ))
            }
            Some((template, _)) if *template != record.template => {
                return Some(format!(
                    "запись #{index}: корпус {:?}, фикстура {template:?} — корпус изменился",
                    record.template
                ))
            }
            Some((_, verdict)) if *verdict != core => {
                return Some(format!(
                    "запись #{index}, шаблон {:?}: ядро {core}, фикстура {verdict}",
                    record.template
                ))
            }
            Some(_) => {}
        }
    }
    committed.get(expected.len()).map(|(template, _)| {
        format!(
            "в фикстуре {} записей, у корпуса {}; первая лишняя: {template:?}",
            committed.len(),
            expected.len()
        )
    })
}

/// Сторож и генератор фикстуры вердиктов (doc модуля).
#[test]
fn template_verdict_fixture_matches_the_core_chain() {
    let records = records();

    assert_eq!(
        records.len(),
        CORPUS_SIZE,
        "корпус выродился или изменился: {} шаблонов вместо {CORPUS_SIZE}. Если состав меняли осознанно — поправьте константу и перегенерируйте: {UPDATE_HINT}",
        records.len()
    );
    let mut classes = [0_usize; 6];
    for record in &records {
        classes[class_of(record.verdict.as_ref())] += 1;
    }
    for (count, name) in classes.iter().zip(CLASS_NAMES) {
        assert!(*count > 0, "в корпусе нет ни одного вердикта «{name}»");
    }

    let expected = render(&records);
    let path = fixture_path();

    if std::env::var(UPDATE_ENV).as_deref() == Ok(UPDATE_ON) {
        fs::write(&path, &expected)
            .unwrap_or_else(|err| panic!("не удалось записать {}: {err}", path.display()));
        return;
    }

    let committed = fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "фикстура {} не читается ({err}). Перегенерация: {UPDATE_HINT}",
            path.display()
        )
    });
    if committed == expected {
        return;
    }
    let reason = match parse_committed(&committed) {
        Err(reason) => format!("фикстура не разбирается: {reason}"),
        Ok(list) => first_divergence(&records, &list).unwrap_or_else(|| {
            "записи совпали, но текст фикстуры разошёлся (шапка или форматирование)".to_owned()
        }),
    };
    panic!(
        "фикстура вердиктов шаблона разошлась с ядром: {reason}.\nЕсли правило ядра меняли осознанно — перегенерируйте и проверьте клиентский валидатор (#108): {UPDATE_HINT}"
    );
}
