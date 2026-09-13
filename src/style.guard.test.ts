import { readFileSync, readdirSync } from 'node:fs'
import { dirname, join, relative, sep } from 'node:path'
import { fileURLToPath } from 'node:url'

import { parse as parseSFC } from '@vue/compiler-sfc'
import postcss, { type AtRule, type Root as PostcssRoot } from 'postcss'
import ts from 'typescript'
import { describe, expect, it } from 'vitest'

/**
 * Сторож палитры (TL-22, issue execaus/tube-leak#23), четвёртый раунд.
 *
 * Вся палитра проекта живёт в `src/style.css` как CSS-переменные (см.
 * doc-комментарий того файла). Всё остальное в `src/` не должно заводить
 * собственный цветовой литерал. Первые две версии сторожа были одним
 * регулярным выражением/самодельным разбором комментариев по всему тексту
 * файла — черновой синтаксический чёрный список, который либо пропускал
 * формы (`style="color: red"`, `stop-color`, `oklch()`, `el.style.color=`),
 * либо ложно срабатывал на прозу (`it's` в тексте шаблона, `#100` в
 * комментарии) или терял код между обманчивым `/*` внутри строки и
 * настоящим комментарием дальше по файлу.
 *
 * # Решение ревью — настоящие парсеры, логика по значениям
 *
 * Вместо самодельного разбора текста — три парсера, уже присутствующие в
 * графе зависимостей (`@vue/compiler-sfc`, `postcss`, `typescript`):
 *
 * 1. **CSS** — `postcss.parse()` для `.css`-файлов, для каждого блока
 *    `<style>` компонента (`descriptor.styles[]`) и для значения атрибута
 *    `style="..."` (обёрнутого в фиктивный `a{...}`, чтобы переиспользовать
 *    один и тот же обход деклараций). Ходим по декларациям `walkDecls`:
 *    любая декларация `--*` — нарушение сама по себе (свою палитру заводить
 *    нельзя, независимо от значения); любое ЗНАЧЕНИЕ любой декларации,
 *    содержащее цветовой литерал (см. п. 4 «Грамматика цвета» ниже), —
 *    нарушение. Никакого списка «цветовых свойств»: `border-radius: 4px`
 *    не совпадает с грамматикой цвета сам по себе, свойство можно не
 *    называть явно.
 * 2. **Шаблон** — `@vue/compiler-sfc` `parse()` даёт готовый AST
 *    (`descriptor.template.ast`), без собственного HTML-разбора. Обход
 *    дерева: у статических атрибутов (кроме `style`, который уходит в п.1)
 *    значение целиком прогоняется через ту же грамматику цвета — это
 *    покрывает `fill`/`stroke`/`stop-color`/`flood-color` и любой будущий
 *    SVG/HTML-атрибут без явного перечисления имён. У директив
 *    (`:style`, `:fill`, `v-bind`, `v-if`, …) и интерполяций `{{ }}` берём
 *    сырой текст выражения и уходим в п. 3.
 * 3. **Скрипты и выражения** — `typescript` `createSourceFile` +
 *    рекурсивный обход AST. Каждый строковый литерал и
 *    no-substitution template literal проверяется той же грамматикой
 *    цвета (п. 4). Выражение динамической привязки (`exp.content` из
 *    шаблона) заворачивается в скобки (`(${expr})`), чтобы `{ color:
 *    'red' }` разобрался как объектный литерал, а не как блок
 *    statement'ов, и уходит в тот же разбор. Комментарии, регулярные
 *    выражения и вложенные шаблонные строки (`` `${'`'}` ``) отличает сам
 *    парсер — никакого ручного разбора кавычек/комментариев в проекте
 *    больше нет.
 * 4. **Грамматика цвета** — hex (`#rgb`/`#rgba`/`#rrggbb`/`#rrggbbaa`),
 *    именованный цвет CSS (148 имён Level 4), системный цвет (`Canvas`,
 *    `ButtonFace`, …, включая устаревшие алиасы вроде `ActiveCaption`),
 *    вызов цветовой функции (`rgb`/`rgba`/`hsl`/`hsla`/`hwb`/`lab`/`lch`/
 *    `oklab`/`oklch`/`color`/`color-mix`/`light-dark`). Перед поиском из
 *    значения вырезаются простые `var(--x)` без fallback и содержимое
 *    `url(...)` (иначе `url(red-arrow.png)` и `var(--color-teal)` ловили
 *    бы сами себя). Границей токена служит не `\b`, а класс
 *    `[A-Za-z0-9_-]` — дефис нарочно считается частью «слова»:
 *    `red-arrow.png` и `darkred-theme` не совпадают с именем
 *    `red`/`darkred` (дефис/буква сразу после не даёт границы), а
 *    `border: 1px solid red;` и `0 0 2px red` совпадают (перед `red`
 *    пробел, после — конец строки/`;`). Функция цвета `color(...)` и
 *    `color-mix(...)` дополнительно требуют, чтобы перед именем функции
 *    не было `.` — иначе `theme.color('x')` (реальный вызов метода в TS)
 *    ловился бы как CSS-функция. Разбор по AST избавляет от этой проблемы
 *    для самого вызова (`theme.color(...)` — не строковый литерал, парсер
 *    до него не доходит), проверка на `.` оставлена как отдельный барьер
 *    на случай, если то же самое встретится внутри строки-значения
 *    (`'theme.color(...)'`).
 *
 *    Грамматика применяется НЕ ОДИНАКОВО ко всем трём слоям — измеренная
 *    асимметрия, не оплошность: для CSS-деклараций (п. 1) и статических
 *    атрибутов шаблона (п. 2) используется полная грамматика (включая
 *    голые имена цветов), а для строковых литералов в скриптах/выражениях
 *    (п. 3) голые ИМЕНА цветов проверяются только как «значение целиком»
 *    (`isWholeValueColor`), а «где-то внутри строки» — только по hex/
 *    функции (`containsHexOrFunctionColor`), без имён. Причина — не
 *    гипотеза, а измеренный факт первого прогона: `Field`/`Window` —
 *    легитимные системные цвета CSS **и** обычные английские слова,
 *    массово встречающиеся в описаниях тестов (`ProbeSection.test.ts`,
 *    `windowExitPort.ts`, `downloadTask.test.ts` — см. `git log` этой
 *    задачи, первый прогон сторожа на новом коде дал 12 ложных
 *    срабатываний именно на этом). Подробности и границы компромисса — в
 *    doc-комментарии `findScriptColorLiteral`.
 *
 * # Честный компромисс, а не маскировка (п. 3 брифа)
 *
 * `const id = '#add'` **ловится** этим сторожем: `#add` синтаксически ---
 * ровно валидный 3-значный hex-цвет (`a`,`d`,`d` — валидные hex-цифры), и
 * грамматика цвета не отличает «id, который выглядит как hex» от
 * настоящего цвета — отличить их без знания контекста (это в принципе
 * значение CSS-свойства или нет) невозможно, а строковый литерал в TS
 * такого контекста не несёт. Решение: ловить оба, а не рисковать
 * пропущенным настоящим цветом; см. тест-фиксацию этого компромисса ниже.
 * Обратная сторона того же выбора: отдельно стоящее английское слово-цвет
 * в произвольной строке TS (гипотетическое `'Mark as read'` — нет, `red`
 * там часть `read`, реальный пример — `'in the red zone'`) тоже поймается,
 * если оно не часть более длинного идентификатора. В проекте весь
 * пользовательский текст на русском (`src/utils/*Texts.ts`), поэтому это
 * не бьёт по реальному коду; если понадобится текст на английском со
 * словом-цветом, обходной путь — не создавать его как отдельный токен
 * (окружить другим словом/знаком препинания не поможет, нужно перефразировать).
 *
 * # Что сторож сознательно не ловит
 *
 * - Шаблонные литералы **с** подстановкой (`` `${x}px solid red` ``) — по
 *   тексту брифа проверяются только строковые литералы и
 *   no-substitution template literals. У статических частей такого
 *   литерала (`TemplateHead`/`Middle`/`Tail`) в TS свой тип узла, сюда не
 *   входящий; в проекте такие литералы сегодня используются только для
 *   чисел (`` `${percent}%` ``), не для цвета.
 * - Внутреннее содержимое HTML-комментариев, обычного текста узлов и имён
 *   идентификаторов — не проверяется никогда, это не отдельное правило, а
 *   следствие того, что обход трогает только атрибуты/директивы/строковые
 *   литералы AST, а не сырой текст файла.
 * - Комментарии в CSS (`/* ... *\/`) и в TS — постольку, поскольку они не
 *   являются узлами `Declaration`/`StringLiteral` в соответствующих AST,
 *   их содержимое никогда не видит грамматика цвета.
 * - `.svg`/`.html`, вложенные `<script>`/`<style>` внутри них: контент
 *   `<style>` уходит в п. 1, `<script>` — в п. 3 (см. `scanTemplateAst`).
 *   Единственное сохранённое исключение вне общего правила — сам корневой
 *   `index.html`: он не проходит общий обход (лежит вне `src/`), для него
 *   отдельная проверка синхронности фона с `--color-bg` (без изменений
 *   логики со второй итерации).
 *
 * # Самоисключение
 *
 * Единственный файл с легальными цветовыми литералами — `src/style.css`
 * (белый список из одного файла, не чёрный список «плохих» файлов).
 * Собственный исходник этого теста исключён из сканирования отдельно
 * (`SELF_PATH`): он неизбежно содержит образцы синтаксиса цвета как саму
 * грамматику (список именованных цветов) и как текст документации — это
 * код проверки, а не палитра компонента, и не то же самое, что скрывать
 * файл с настоящим нарушением.
 *
 * # Почему по исходному тексту, а не по вычисленному стилю
 *
 * jsdom не применяет `scoped`-стили SFC — единственный способ проверить
 * исходники без полной сборки Vite (тот же приём, что в `YtDlpUpdateBlock`/
 * `DownloadPanel`/`SidecarStatusRow`).
 *
 * # Падение при неразобранном файле
 *
 * `@vue/compiler-sfc` `parse()` не бросает на синтаксической ошибке, а
 * возвращает `errors` — сторож сам решает бросить, если `errors.length >
 * 0` (иначе сломанный `<template>`/`<style>`/`<script>` тихо дал бы пустой
 * AST и «зелёный» тест, ничего не проверив). `postcss.parse()` бросает
 * `CssSyntaxError` сам — это не перехватывается. `ts.createSourceFile()`
 * никогда не бросает даже на нечитаемом коде — здесь строгая ручная
 * проверка `parseDiagnostics` после разбора.
 *
 * # Четвёртый раунд — имена внутри составных строк, интерполяция, обработчики
 *
 * Третье ревью намеренно ограничило `findScriptColorLiteral` (п. 3) —
 * составная строка в скрипте проверялась только на hex/функцию, голые
 * имена цветов ловились лишь как «значение строки целиком». Это
 * пропускало реальный класс кода: строковые CSS-значения, которые
 * заведомо являются CSS-значением по СИНТАКСИЧЕСКОМУ контексту, а не по
 * догадке об их содержимом. Решение ведущего — не расширять поиск имён
 * на все строки проекта (риск `Field`/`Window` из П-1 никуда не делся),
 * а точно очертить, где строка гарантированно CSS-значение, и включать
 * поиск имён только там:
 *
 * 1. **Ключ объекта, похожий на CSS-свойство, внутри «стилевого» объекта.**
 *    «Стилевой объект» — это (а) весь объект(-ы), до которых можно дойти
 *    внутри выражения директивы `:style`/`v-bind:style` (сама директива —
 *    однозначный признак: значение обязано быть объектом стилей или
 *    строкой инлайн-CSS, см. п. 2), либо (б) любой объектный литерал в
 *    файле `.ts`/`.js`, просканированном целиком (`scanFile` — не
 *    `<script>`/`<script setup>` внутри `.vue`: у директивы `:style` уже
 *    есть свой явный признак контекста, а для остального скрипта SFC
 *    оставлен прежний, более узкий охват — за пределами прямого требования
 *    брифа). «Похожий на CSS-свойство» ключ — без списка известных
 *    CSS-свойств: попытка использовать транзитивный `mdn-data` (в графе
 *    только как зависимость `jsdom → css-tree`, не объявлен ни в одном
 *    `package.json` проекта) отклонена — не наша зависимость, может
 *    исчезнуть при любом обновлении `jsdom`, никак не защищено. Вместо
 *    этого — честная эвристика по форме: kebab-форма ключа (camelCase
 *    приводится к kebab, уже kebab остаётся как есть) совпадает с
 *    `^-?[a-z]+(-[a-z]+)*$` (`isCssPropertyLikeKey`). Она пропустит и
 *    некоторые не-CSS ключи с «похожей» формой (`{ status: 'dark red' }`) —
 *    названный компромисс, симметричный решению п. 3 предыдущего раунда
 *    («лучше поймать лишнее, чем пропустить настоящий цвет», см. «Честный
 *    компромисс» ниже).
 * 2. **Присваивание в цепочку `....style` и вызов `....setProperty(...)`.**
 *    `el.style = ...`, `el.style.cssText = ...`, `el.style.color = ...` —
 *    правая часть присваивания гарантированно CSS-значение вне
 *    зависимости от имени конкретного свойства (`isStylePropertyAccessChain`
 *    ищет сегмент `style` где угодно в цепочке доступа слева). Аналогично
 *    второй аргумент вызова, где свойство доступа названо `setProperty`
 *    (`el.style.setProperty('color', value)` — CSSOM API, синтаксически
 *    однозначен независимо от приёмника слева).
 * 3. **Директива `:style`/`v-bind:style`, чьё выражение целиком — строка.**
 *    `:style="'border: 1px solid red'"` — это не JS-объект, а инлайн-CSS
 *    текст, синтаксически неотличимый от статического `style="..."`.
 *    `scanStyleDirectiveExpression` разбирает выражение директивы,
 *    снимает обёрточные скобки и, если под ними — ровно строковый литерал
 *    (`StringLiteral`/`NoSubstitutionTemplateLiteral` без подстановок),
 *    отдаёт его текст в тот же CSS-путь (`scanInlineStyleValue`/postcss),
 *    что и статический атрибут `style="..."`, — а не в скриптовый разбор
 *    строковых литералов. Если же под скобками не строка (объект,
 *    тернарник, …), выражение уходит в обычный скриптовый разбор с
 *    признаком «стилевой объект» (п. 1а).
 *
 * Вне этих трёх точек имена цветов внутри составной строки по-прежнему не
 * ищутся — асимметрия с CSS-декларациями (см. предыдущий раздел) остаётся
 * в силе для прозы, `v-if`/`v-bind`(не `style`)/произвольных вызовов и
 * объектов внутри `<script setup>`: `it('Field and red window', …)`
 * (проза в описании теста) по-прежнему зелёная, закреплено фикстурой ниже.
 *
 * # Шаблонные литералы с подстановкой — статические части
 *
 * Второй раунд сознательно не проверял `` `${x}px solid red` `` — у
 * статических частей шаблонного литерала с подстановкой (`TemplateHead`/
 * `TemplateMiddle`/`TemplateTail`) свой тип узла, не входящий в
 * `StringLiteral`/`NoSubstitutionTemplateLiteral`. Четвёртый раунд достаёт
 * их тем же обходом (`ts.forEachChild` и так спускается в `head`/
 * `templateSpans[].literal` — они настоящие дочерние узлы AST, отдельного
 * обхода не нужно) и применяет к тексту КАЖДОЙ части ту же грамматику, что
 * и к обычной строке в том же контексте, — с одним отличием: `checkFragmentLiteral`
 * никогда не включает `isWholeValueColor` (проверку «весь текст — это
 * цвет»), потому что статическая часть — это не «всё значение», а его
 * кусок; для неё «функции цвета и hex — всегда, имена — только в
 * контекстах п. 1–3 выше» в чистом виде, без искажения от совпадения
 * куска с именем цвета целиком вне этих контекстов.
 *
 * # Обработчики `v-on`/`@`, `v-for`, `v-slot` — разбор без ложного padения
 *
 * Прежняя реализация заворачивала ЛЮБОЕ выражение директивы в `(...)`,
 * чтобы `{ color: 'red' }` разобрался как объектный литерал. Это ломало
 * три формы, валидные для Vue, но не являющиеся одиночным выражением в
 * скобках:
 *
 * - `v-on`/`@` — значение directive это список STATEMENT'ов
 *   (`open = false; emit('close')`), не выражение; `(a; b)` — синтаксическая
 *   ошибка TS. Такие директивы (`prop.name === 'on'`) разбираются как
 *   обычный модуль, без обёрточных скобок (`scanScriptModule` и так не
 *   оборачивает — этим и отличается от `scanScriptExpression`).
 * - `v-for` — `item of items`/`(item, index) in items` не выражение
 *   (левая часть — паттерн объявления, не значение). `@vue/compiler-sfc`
 *   уже разбирает форму сам и кладёт готовый результат в
 *   `prop.forParseResult.source` — проверяется только он (правая часть,
 *   после `in`/`of`, ровно как просит бриф), без ручного разбора текста
 *   по `in`/`of` и без риска ошибиться на форме с деструктуризацией слева.
 * - `v-slot`/`#slot` с деструктуризацией (`{ a, b }`, `{ a: renamed }`,
 *   `{ a, ...rest }`, значения по умолчанию `{ a = 1 }`) — измерено
 *   отдельно (см. отчёт по задаче): обёртка в скобки `({ ... })` уже
 *   разбирается TS без ошибки для всех этих форм (валидный синтаксис
 *   ObjectLiteralExpression с сокращёнными свойствами), падения не было —
 *   фикстуры ниже это закрепляют как регресс-барьер, без изменения кода.
 *
 * # Пятый раунд (TL-96, issue execaus/tube-leak#103) — ложные срабатывания и less
 *
 * Третье ревью нашло три формы, где сторож либо ловил то, что не является
 * цветом презентационно, либо тихо пропускал `<style lang="less">`:
 *
 * 1. **Статический атрибут — только белый список презентационных
 *    атрибутов.** `role="menu"`, `aria-haspopup="menu"` (системные цвета
 *    `menu`/`background` — обычные слова в НЕ-цветовых атрибутах) и
 *    `<MyBadge tone="green" />` (проп компонента) раньше ловились
 *    `isWholeValueColor` наравне с `fill="red"` — п. 2 doc-комментария
 *    четвёртого раунда специально исключал только `class`, остальное
 *    считалось презентационным по умолчанию. Теперь «целиком является
 *    цветом» проверяется только для `COLOR_PRESENTATION_ATTRIBUTES`
 *    (SVG presentation attributes, которые несут цвет, + `style`) — белый
 *    список, не чёрный: `role`/`aria-*`/пропсы компонентов/`class` не
 *    входят и не проверяются вовсе, чем бы они ни оказались. Мутация,
 *    доказывающая границы списка (см. отчёт по задаче): расширение до
 *    «всех атрибутов» красит `role="menu"`/`tone="green"`; сужение до
 *    пустого зеленит `fill="red"` (тест на этот случай тогда краснеет).
 * 2. **`:class`/`v-bind:class` не проверяется.** Значение этой привязки —
 *    список имён CSS-классов, а не CSS-значение; `:class="'red'"` ловился
 *    как обычный строковый литерал в скрипте (`isWholeValueColor` в
 *    `findScriptColorLiteral` не знает про контекст «это атрибут class»).
 *    Симметрично статическому `class` (уже не в белом списке п. 1) —
 *    отдельная проверка `directiveName === 'bind' && argOrName === 'class'`
 *    в `scanTemplateProp` останавливает разбор до того, как значение
 *    попадёт в скриптовый путь.
 * 3. **Имя CSS-свойства в значении `transition`/`transition-property`/
 *    `will-change` — не цвет.** `transition: background 0.2s ease`
 *    называет свойство `background` для анимации; `background` — валидный
 *    (устаревший) системный цвет CSS2, и `findCssColorLiteral` ловил его
 *    как обычное значение. Ни у одного из этих трёх свойств нет позиции,
 *    где голое ИМЯ цвета было бы допустимым значением (список: имена
 *    CSS-свойств, `all`, `none`, время, timing-функция) — `findDeclarationColorLiteral`
 *    для них использует только `containsHexOrFunctionColor` (hex/функции
 *    без имён), минуя `findCssColorLiteral` целиком. Другие свойства не
 *    затронуты: асимметрия точечная, по имени свойства декларации, а не
 *    по позиции внутри значения (что потребовало бы разбора формата
 *    transition-shorthand, который CSS не запрещает записывать в любом
 *    порядке компонентов).
 * 4. **`<style lang="less">` падает, а не молчит.** `@brand: #ff0000` —
 *    валидный less, но для `postcss.parse()` это просто at-rule без блока
 *    (`@brand` — имя, `: #ff0000` — параметры), значение никогда не
 *    попадает в `walkDecls`, тест был «зелёным», ничего не проверив. Less
 *    не установлен в проекте и разбирать его синтаксис (переменные `@x`,
 *    вложенность, миксины) не планируется — вместо этого `lang`,
 *    отличный от `css`/не заданного/`scss` (белый список
 *    `SUPPORTED_STYLE_LANGS`, scss уже используется в проекте), приводит к
 *    падению с понятным сообщением ДО вызова `postcss.parse` — тот же
 *    принцип «неразобранное — падение», что и для синтаксических ошибок
 *    (см. выше). Проверяется в обеих точках, где в коде вызывается
 *    `scanCssText` для содержимого `<style>`: `descriptor.styles[]`
 *    настоящих `.vue`-файлов (`style.lang`) и вложенный `<style>` внутри
 *    `.svg`/`.html`, разобранный как обычный элемент шаблона
 *    (`getStaticAttrValue(node, 'lang')` — для него нет отдельного поля
 *    `lang`, только атрибут).
 *
 * # Шестой раунд (TL-103, issue execaus/tube-leak#110) — ссылка на issue не hex
 *
 * Найдено при TL-98 (issue #105): заголовок теста `it('… issue #105 …', …)`
 * краснел — трёхзначный номер issue после `#` синтаксически неотличим от
 * 3-значного hex-цвета, а `findScriptColorLiteral` (пятый раунд и раньше)
 * искал hex ГДЕ УГОДНО в составной строке скрипта без оглядки на контекст.
 * Тот же класс ложного совпадения, что уже был решён для имён цветов в
 * четвёртом раунде (Б-1: `Field`/`Window` — легитимные системные цвета и
 * обычные слова) — решение ведущего распространяет ту же границу на hex, а
 * не изобретает отдельную:
 *
 * 1. **Голый hex внутри составной строки скрипта ищется только в тех же
 *    трёх контекстах, где уже ищутся голые имена (Б-1, «Четвёртый раунд»):**
 *    ключ объекта, похожий на CSS-свойство, внутри стилевого объекта;
 *    присваивание в цепочку `....style`/вызов `....setProperty(...)`;
 *    выражение директивы `:style`, целиком являющееся строкой. Реализовано
 *    без нового признака контекста — `findScriptColorLiteral` (вызывается,
 *    когда `allowNames === false`) больше не вызывает `containsHexOrFunctionColor`,
 *    только новый `containsFunctionColor` (вызовы цветовых функций, всегда,
 *    без ограничения контекста — синтаксис `rgb(...)` не совпадает со
 *    случайной прозой). Голый hex «где угодно» доступен только через
 *    `findCssColorLiteral`/`containsHexOrFunctionColor` (новый
 *    `containsHexColor` + `containsFunctionColor`), которые вызываются
 *    именно и только в контекстах п. 1–3 (`allowNames === true`,
 *    `checkWholeLiteral`) и для CSS-деклараций (п. 1 «Четвёртого раунда»,
 *    где поверхность узкая заведомо). Симметрично применено и к статическим
 *    частям шаблонных литералов с подстановкой (`checkFragmentLiteral`) —
 *    тот же класс совпадения не более обоснован внутри `TemplateHead`, чем
 *    внутри обычного строкового литерала.
 * 2. **Значение ЦЕЛИКОМ равное hex по-прежнему нарушение** (`'#f00'`,
 *    `'#105'`) — не затронуто, это честное ограничение из «Честного
 *    компромисса» (раздел выше, п. 3 брифа третьего раунда): строковый
 *    литерал не несёт контекста, отличающего id от цвета, когда ничего,
 *    кроме самого hex, в строке нет. Обходной путь для такого id — не
 *    писать его отдельным литералом (см. п. 3 ниже — заголовки тестов уже
 *    решают эту проблему иначе).
 * 3. **Первый аргумент `describe`/`it`/`test` не проверяется вовсе** —
 *    новый, отдельный от Б-1 признак контекста, а не расширение грамматики:
 *    заголовок теста — не CSS-значение и не значение вообще, это метаданные
 *    для отчёта тестраннера, программе к разбору не предназначенные.
 *    `isTestTitleCall`/`isEachTableCall`/`unwrapCalleeRootName` находят
 *    вызов по имени корневого идентификатора цепочки (`describe`/`it`/
 *    `test`), включая `.each(...)`/`.skip`/`.only`/`.concurrent` в любой
 *    комбинации и глубине; `.each([...])` сам по себе — не заголовочный
 *    вызов (его аргумент — таблица данных, которая по-прежнему
 *    проверяется, если фикстура данных содержит настоящий код с цветом).
 *    Остальные аргументы такого вызова (обычно функция теста) проверяются
 *    как обычно — исключён только индекс 0.
 *
 * Вызовы цветовых функций (`rgb(`, `oklch(`, …) исключение не затрагивает
 * нигде: они ищутся где угодно в любой строке независимо от контекста, как
 * и раньше, — синтаксис вызова функции не совпадает со случайной прозой
 * (единственный барьер для этого класса — точка перед именем функции,
 * отличающая `theme.color('x')` от `color('x')`, см. выше).
 */

const SRC_DIR = join(dirname(fileURLToPath(import.meta.url)))
const SELF_PATH = fileURLToPath(import.meta.url)
const STYLE_CSS_PATH = join(SRC_DIR, 'style.css')
const INDEX_HTML_PATH = join(SRC_DIR, '..', 'index.html')

const ALLOWED_RELATIVE_PATHS = new Set(['style.css'])
const SCANNED_EXTENSIONS = ['.vue', '.css', '.ts', '.js', '.html', '.svg']

// ---------------------------------------------------------------------------
// Грамматика цвета (общая для CSS-значений, атрибутов и строковых литералов)
// ---------------------------------------------------------------------------

// 148 именованных цветов CSS Color Module Level 4. Намеренно нет
// 'transparent'/'currentcolor' — это разрешённые ключевые слова, не
// литералы палитры (см. doc-комментарий выше).
const NAMED_COLORS = [
  'aliceblue', 'antiquewhite', 'aqua', 'aquamarine', 'azure', 'beige', 'bisque', 'black',
  'blanchedalmond', 'blue', 'blueviolet', 'brown', 'burlywood', 'cadetblue', 'chartreuse',
  'chocolate', 'coral', 'cornflowerblue', 'cornsilk', 'crimson', 'cyan', 'darkblue', 'darkcyan',
  'darkgoldenrod', 'darkgray', 'darkgreen', 'darkgrey', 'darkkhaki', 'darkmagenta',
  'darkolivegreen', 'darkorange', 'darkorchid', 'darkred', 'darksalmon', 'darkseagreen',
  'darkslateblue', 'darkslategray', 'darkslategrey', 'darkturquoise', 'darkviolet', 'deeppink',
  'deepskyblue', 'dimgray', 'dimgrey', 'dodgerblue', 'firebrick', 'floralwhite', 'forestgreen',
  'fuchsia', 'gainsboro', 'ghostwhite', 'gold', 'goldenrod', 'gray', 'green', 'greenyellow',
  'grey', 'honeydew', 'hotpink', 'indianred', 'indigo', 'ivory', 'khaki', 'lavender',
  'lavenderblush', 'lawngreen', 'lemonchiffon', 'lightblue', 'lightcoral', 'lightcyan',
  'lightgoldenrodyellow', 'lightgray', 'lightgreen', 'lightgrey', 'lightpink', 'lightsalmon',
  'lightseagreen', 'lightskyblue', 'lightslategray', 'lightslategrey', 'lightsteelblue',
  'lightyellow', 'lime', 'limegreen', 'linen', 'magenta', 'maroon', 'mediumaquamarine',
  'mediumblue', 'mediumorchid', 'mediumpurple', 'mediumseagreen', 'mediumslateblue',
  'mediumspringgreen', 'mediumturquoise', 'mediumvioletred', 'midnightblue', 'mintcream',
  'mistyrose', 'moccasin', 'navajowhite', 'navy', 'oldlace', 'olive', 'olivedrab', 'orange',
  'orangered', 'orchid', 'palegoldenrod', 'palegreen', 'paleturquoise', 'palevioletred',
  'papayawhip', 'peachpuff', 'peru', 'pink', 'plum', 'powderblue', 'purple', 'rebeccapurple',
  'red', 'rosybrown', 'royalblue', 'saddlebrown', 'salmon', 'sandybrown', 'seagreen', 'seashell',
  'sienna', 'silver', 'skyblue', 'slateblue', 'slategray', 'slategrey', 'snow', 'springgreen',
  'steelblue', 'tan', 'teal', 'thistle', 'tomato', 'turquoise', 'violet', 'wheat', 'white',
  'whitesmoke', 'yellow', 'yellowgreen',
]

// Системные цвета CSS (Level 4 + устаревшие алиасы, тоже валидные
// значения) — завязаны на палитру ОС, а не на токены проекта, поэтому
// тоже под запретом.
const SYSTEM_COLORS = [
  'accentcolor', 'accentcolortext', 'activetext', 'buttonborder', 'buttonface', 'buttontext',
  'canvas', 'canvastext', 'field', 'fieldtext', 'graytext', 'highlight', 'highlighttext',
  'linktext', 'mark', 'marktext', 'selecteditem', 'selecteditemtext', 'visitedtext',
  'activeborder', 'activecaption', 'appworkspace', 'background', 'buttonhighlight',
  'buttonshadow', 'captiontext', 'inactiveborder', 'inactivecaption', 'inactivecaptiontext',
  'infobackground', 'infotext', 'menu', 'menutext', 'scrollbar', 'threeddarkshadow',
  'threedface', 'threedhighlight', 'threedlightshadow', 'threedshadow', 'window',
  'windowframe', 'windowtext',
]

// Функции CSS, которые всегда производят литеральный цвет.
const COLOR_FUNCTION_NAMES = [
  'rgba', 'rgb', 'hsla', 'hsl', 'hwb', 'lab', 'lch', 'oklab', 'oklch', 'color-mix', 'color',
  'light-dark',
]

// Статические атрибуты, презентационно несущие цвет (белый список из
// SVG presentation attributes + HTML `style`), — только они проверяются на
// «значение целиком является цветом» (TL-96, issue execaus/tube-leak#103).
// `role="menu"`/`aria-haspopup="menu"` (системный цвет как обычное слово в
// не-цветовом атрибуте) и `<MyBadge tone="green" />` (проп компонента) не
// входят в белый список и не проверяются вовсе — они не задают цвет
// презентационно, а совпадение слова со значением палитры для них
// случайно. `class` тоже не проверяется (см. doc-комментарий
// `scanTemplateProp`, `:class`) — здесь достаточно не включать его в
// список.
const COLOR_PRESENTATION_ATTRIBUTES = new Set([
  'fill', 'stroke', 'stop-color', 'flood-color', 'lighting-color', 'color',
])

// Дефис — часть «слова» для наших целей: `red-arrow.png`/`darkred-theme`
// не должны совпадать с именем `red`/`darkred` целиком (см. doc-комментарий
// выше, п. 4).
const BOUNDARY_CHARS = 'A-Za-z0-9_-'

const NAMED_OR_SYSTEM_COLOR_RE = new RegExp(
  `(?<![${BOUNDARY_CHARS}])(?:${[...NAMED_COLORS, ...SYSTEM_COLORS].join('|')})(?![${BOUNDARY_CHARS}])`,
  'gi',
)

// Доп. барьер `.` перед именем функции — иначе `theme.color('x')` (реальный
// вызов метода) совпал бы с CSS-функцией `color(...)`, если бы этот текст
// когда-то оказался внутри проверяемой строки (сам вызов как код AST
// сторож не видит вовсе — см. doc-комментарий выше).
const COLOR_FUNCTION_CALL_RE = new RegExp(
  `(?<![.${BOUNDARY_CHARS}])(?:${COLOR_FUNCTION_NAMES.join('|')})\\(`,
  'gi',
)

const HEX_RUN_RE = /#[0-9a-fA-F]+/g
const VALID_HEX_DIGIT_COUNTS = new Set([3, 4, 6, 8])
const HEX_WHOLE_RE = /^#(?:[0-9a-fA-F]{3}|[0-9a-fA-F]{4}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$/
const COLOR_FUNCTION_WHOLE_RE = new RegExp(`^(?:${COLOR_FUNCTION_NAMES.join('|')})\\(.*\\)$`, 'is')
const NAMED_COLOR_SET = new Set(NAMED_COLORS)
const SYSTEM_COLOR_SET = new Set(SYSTEM_COLORS)

// Убираются перед поиском — простая ссылка на токен без fallback и
// содержимое `url(...)` не могут сами быть литералом палитры компонента.
const VAR_NO_FALLBACK_RE = /var\(\s*--[A-Za-z0-9_-]+\s*\)/gi
const URL_FUNCTION_RE = /url\(\s*(?:"[^"]*"|'[^']*'|[^)]*)\s*\)/gi

function stripNonPaletteConstructs(rawValue: string): string {
  return rawValue.replace(VAR_NO_FALLBACK_RE, ' ').replace(URL_FUNCTION_RE, ' ')
}

/**
 * Значение (после trim) ЦЕЛИКОМ является цветом: hex, ЛЮБОЕ из 148+41
 * имён (обычных или системных), либо вызов цветовой функции, охватывающий
 * всю строку. Используется там, где по требованию нужна именно полная
 * форма (статический атрибут шаблона — «целиком является цветом» — и
 * часть проверки строковых литералов в скриптах, см.
 * `findScriptColorLiteral` ниже).
 */
function isWholeValueColor(rawValue: string): boolean {
  const value = rawValue.trim()
  if (value === '') return false
  if (HEX_WHOLE_RE.test(value)) return true
  const lower = value.toLowerCase()
  if (NAMED_COLOR_SET.has(lower) || SYSTEM_COLOR_SET.has(lower)) return true
  if (COLOR_FUNCTION_WHOLE_RE.test(value)) return true
  return false
}

/**
 * Вызов цветовой функции ГДЕ УГОДНО в значении (после вырезания
 * `var()`/`url()`) — не требует, чтобы значение было ЦЕЛИКОМ цветом.
 * Единственный поиск «где угодно», применяемый БЕЗ оглядки на контекст (см.
 * `findScriptColorLiteral` шестого раунда, doc-комментарий файла — «Шестой
 * раунд»): вызов функции синтаксически однозначен сам по себе, в отличие от
 * голого hex-фрагмента или имени, которые совпадают со случайной прозой
 * (номер issue, идентификатор).
 */
function containsFunctionColor(rawValue: string): string | null {
  const value = stripNonPaletteConstructs(rawValue)
  const functionCall = value.match(COLOR_FUNCTION_CALL_RE)
  return functionCall && functionCall[0] !== undefined ? functionCall[0] : null
}

/**
 * hex ГДЕ УГОДНО в значении (после вырезания `var()`/`url()`) — не требует,
 * чтобы значение было ЦЕЛИКОМ цветом. Используется только там, где строка
 * синтаксически гарантированно CSS-значение (см. doc-комментарий файла —
 * «Шестой раунд»): голый hex-фрагмент неотличим от номера issue (`#105`) без
 * такого контекста.
 */
function containsHexColor(rawValue: string): string | null {
  const value = stripNonPaletteConstructs(rawValue)
  const hexRuns = value.match(HEX_RUN_RE)
  if (!hexRuns) return null
  const validHexRun = hexRuns.find((run) => VALID_HEX_DIGIT_COUNTS.has(run.length - 1))
  return validHexRun ?? null
}

/**
 * hex или вызов цветовой функции ГДЕ УГОДНО в значении (после вырезания
 * `var()`/`url()`) — не требует, чтобы значение было ЦЕЛИКОМ цветом.
 * Именованные/системные цвета сюда намеренно не входят (см.
 * `findScriptColorLiteral` — там объяснено, почему). Используется только в
 * контекстах, где строка синтаксически гарантированно CSS-значение
 * (CSS-декларации, п. 1–3 «Четвёртого раунда» через `findCssColorLiteral`) —
 * hex-часть здесь безопасна именно потому, что поверхность узкая, в отличие
 * от произвольной строки TS (см. `containsHexColor`).
 */
function containsHexOrFunctionColor(rawValue: string): string | null {
  const hexRun = containsHexColor(rawValue)
  if (hexRun !== null) return hexRun
  return containsFunctionColor(rawValue)
}

/**
 * Полная грамматика «где угодно в значении»: hex/функция (как
 * `containsHexOrFunctionColor`) плюс именованные/системные цвета —
 * используется ТОЛЬКО для CSS-деклараций (п. 1), где поверхность узкая и
 * контролируемая (значения CSS-свойств, а не произвольная проза).
 */
function findCssColorLiteral(rawValue: string): string | null {
  const hexOrFunction = containsHexOrFunctionColor(rawValue)
  if (hexOrFunction !== null) return hexOrFunction

  const value = stripNonPaletteConstructs(rawValue)
  const namedColor = value.match(NAMED_OR_SYSTEM_COLOR_RE)
  if (namedColor && namedColor[0] !== undefined) return namedColor[0]

  return null
}

/**
 * Проверка строкового литерала в скриптах/выражениях (п. 3): «целиком
 * цвет» — полной грамматикой (`isWholeValueColor`), «составное
 * CSS-подобное значение» — только по вызову цветовой функции
 * (`containsFunctionColor`), БЕЗ голых имён цветов и БЕЗ голого hex.
 *
 * Причина асимметрии с CSS-декларациями измерена, не предположена: первая
 * версия применяла ту же полную грамматику (включая системные цвета) к
 * каждому строковому литералу проекта — и `Field`/`Window`, легитимные
 * системные цвета CSS, оказались обычными английскими словами в описаниях
 * тестов («... field ...», «... window ...», см. `ProbeSection.test.ts`,
 * `windowExitPort.ts`, `downloadTask.test.ts`). Голое имя обычного
 * (не системного) цвета внутри длинной строки — тот же риск («in the red
 * zone»), просто пока не встретившийся в русскоязычном UI-тексте проекта.
 * Отсюда сознательный выбор: составное совпадение по именам цветов ловится
 * только там, где поверхность узкая и предсказуемая (CSS-значение), а не
 * в произвольной строке TS.
 *
 * Шестой раунд (TL-103, issue execaus/tube-leak#110) распространил то же
 * рассуждение на голый hex: `issue #105`/`см. #105` — трёхзначное число
 * после `#`, случайно являющееся валидным hex, тот же класс совпадения, что
 * и `Field`/`Window` для имён. Вызов цветовой функции (`rgb(`, `oklch(`, …)
 * синтаксически однозначен сам по себе (случайная проза не порождает
 * `rgb(...)`) и по-прежнему ищется где угодно в строке без ограничения по
 * контексту — асимметрия только для hex и имён, не для функций.
 */
function findScriptColorLiteral(text: string): string | null {
  if (isWholeValueColor(text)) return text.trim()
  return containsFunctionColor(text)
}

/**
 * Проверка ЦЕЛОГО строкового литерала (`StringLiteral`/
 * `NoSubstitutionTemplateLiteral`) с учётом контекста (см. doc-комментарий
 * файла, «Четвёртый раунд»): в контекстах п. 1–3 — полная грамматика
 * (`findCssColorLiteral`, имена включены), иначе — прежний ограниченный
 * разбор (`findScriptColorLiteral`).
 */
function checkWholeLiteral(text: string, allowNames: boolean): string | null {
  return allowNames ? findCssColorLiteral(text) : findScriptColorLiteral(text)
}

/**
 * Проверка СТАТИЧЕСКОЙ ЧАСТИ шаблонного литерала с подстановкой
 * (`TemplateHead`/`TemplateMiddle`/`TemplateTail`, текст без `${…}`).
 * В отличие от `checkWholeLiteral`, никогда не применяет `isWholeValueColor`:
 * часть — это кусок значения, а не всё значение, и совпадение куска с
 * именем цвета целиком вне контекстов п. 1–3 не должно ложно сработать
 * (см. doc-комментарий файла, «Шаблонные литералы с подстановкой»).
 * Функции цвета ищутся всегда; hex и имена — только если `allowNames`
 * (шестой раунд симметрично распространил ограничение hex из
 * `findScriptColorLiteral` и на статические части шаблонных литералов —
 * тот же класс ложного совпадения, `#105` в статической части не более
 * контекстно-обоснован, чем в обычном строковом литерале).
 */
function checkFragmentLiteral(text: string, allowNames: boolean): string | null {
  const functionColor = containsFunctionColor(text)
  if (functionColor !== null) return functionColor
  if (!allowNames) return null

  return findCssColorLiteral(text)
}

// ---------------------------------------------------------------------------
// Контексты «строка гарантированно CSS-значение» (Б-1, четвёртый раунд)
// ---------------------------------------------------------------------------

// Ключ объекта, похожий на CSS-свойство: без списка известных свойств (см.
// doc-комментарий файла — почему транзитивный `mdn-data` отклонён), честная
// эвристика по форме после приведения camelCase к kebab-case.
const CSS_PROPERTY_KEY_RE = /^-?[a-z]+(-[a-z]+)*$/

function toKebabCase(key: string): string {
  return key.replace(/[A-Z]/g, (ch) => `-${ch.toLowerCase()}`)
}

function isCssPropertyLikeKey(key: string): boolean {
  return CSS_PROPERTY_KEY_RE.test(toKebabCase(key))
}

/** Имя простого/строкового/числового свойства объекта; `null` для вычисляемого ключа (`[expr]: value`) — тогда контекст не форсируется. */
function getObjectPropertyKeyText(name: ts.PropertyName): string | null {
  if (ts.isIdentifier(name) || ts.isStringLiteral(name) || ts.isNumericLiteral(name)) return name.text
  return null
}

/**
 * true, если цепочка доступа к свойству где-то содержит сегмент `style`
 * (`el.style = …`, `el.style.cssText = …`, `el.style.color = …`) — тогда
 * правая часть присваивания достоверно CSS-значение независимо от имени
 * конкретного свойства, без эвристики по ключу.
 */
function isStylePropertyAccessChain(expr: ts.Expression): boolean {
  let current: ts.Expression = expr
  while (ts.isPropertyAccessExpression(current)) {
    if (current.name.text === 'style') return true
    current = current.expression
  }
  return false
}

/** `x.style.setProperty(name, value)` — CSSOM API, синтаксически однозначен независимо от приёмника `x`. */
function isSetPropertyCall(node: ts.CallExpression): boolean {
  return ts.isPropertyAccessExpression(node.expression) && node.expression.name.text === 'setProperty'
}

// Имена функций объявления теста из vitest — единственный источник, чей
// первый аргумент вообще не является CSS-значением или прозой, требующей
// разбора: это заголовок теста (TL-103, issue execaus/tube-leak#110).
const TEST_DEFINITION_NAMES = new Set(['describe', 'it', 'test'])

/**
 * Имя переменной в корне цепочки вызова/доступа к свойству: снимает
 * `PropertyAccessExpression` (`.skip`, `.only`, …) и `CallExpression`
 * (`.each([...])`) слой за слоем, пока не останется голый идентификатор.
 * `null`, если цепочка не сводится к идентификатору (например, вызов
 * результата другого вызова, не связанного с `describe`/`it`/`test`).
 */
function unwrapCalleeRootName(expr: ts.Expression): string | null {
  let current: ts.Expression = expr
  while (true) {
    if (ts.isIdentifier(current)) return current.text
    if (ts.isPropertyAccessExpression(current)) {
      current = current.expression
      continue
    }
    if (ts.isCallExpression(current)) {
      current = current.expression
      continue
    }
    return null
  }
}

/**
 * `describe.each([...])`/`it.each([...])` — вызов, порождающий функцию
 * заголовка, а не сам вызов с заголовком: его единственный аргумент —
 * таблица данных, а не название теста, и должен проверяться как обычно (в
 * т. ч. если фикстура данных содержит настоящий код с цветом).
 */
function isEachTableCall(node: ts.CallExpression): boolean {
  return ts.isPropertyAccessExpression(node.expression)
    && node.expression.name.text === 'each'
    && TEST_DEFINITION_NAMES.has(unwrapCalleeRootName(node.expression) ?? '')
}

/**
 * Вызов `describe(...)`/`it(...)`/`test(...)` (включая `.each(...)`, `.skip`,
 * `.only`, `.concurrent` в любой комбинации и глубине) с заголовком в первом
 * аргументе. Не совпадает с самим `it.each([...])` (см. `isEachTableCall`) —
 * тот вызов возвращает функцию заголовка, но заголовком не является.
 */
function isTestTitleCall(node: ts.CallExpression): boolean {
  if (isEachTableCall(node)) return false
  return TEST_DEFINITION_NAMES.has(unwrapCalleeRootName(node.expression) ?? '')
}

// ---------------------------------------------------------------------------
// CSS: `.css`-файлы, `<style>`-блоки SFC, инлайн `style="..."`
// ---------------------------------------------------------------------------

// Свойства, у которых идентификатор в значении стоит в позиции ИМЕНИ
// CSS-свойства, а не цвета (TL-96, issue execaus/tube-leak#103):
// `transition: background 0.2s ease` называет свойство `background` для
// анимации, а не значение — устаревший системный цвет CSS2 `Background`
// тут ни при чём, только случайное совпадение слова. Ни `transition`/
// `transition-property`, ни `will-change` не имеют положения, где голое
// ИМЯ цвета было бы допустимым значением (список: имена свойств, `all`,
// `none`, время, timing-функция) — поэтому для них голые имена цветов не
// ищутся вовсе. hex/цветовые функции по-прежнему проверяются
// `containsHexOrFunctionColor` (защитный барьер: они тоже не валидны в
// этих свойствах, но раз уж грамматика их ищет "где угодно в значении",
// сознательно не сужаем эту часть проверки).
const PROPERTY_NAME_POSITION_PROPS = new Set(['transition', 'transition-property', 'will-change'])

function findDeclarationColorLiteral(prop: string, value: string): string | null {
  if (PROPERTY_NAME_POSITION_PROPS.has(prop.toLowerCase())) {
    return containsHexOrFunctionColor(value)
  }
  return findCssColorLiteral(value)
}

/**
 * Разбирает CSS через postcss и проверяет каждую декларацию: `--*` —
 * нарушение сама по себе, любое значение — через грамматику цвета.
 * `postcss.parse` бросает `CssSyntaxError` на некорректном CSS — не
 * перехватывается нарочно (см. doc-комментарий файла, «Падение при
 * неразобранном файле»).
 */
function scanCssText(cssText: string, label: string): string[] {
  const root = postcss.parse(cssText)
  const violations: string[] = []

  root.walkDecls((decl) => {
    if (decl.prop.startsWith('--')) {
      violations.push(
        `${label}: объявлена собственная переменная "${decl.prop}" — палитра только в src/style.css`,
      )
      return
    }
    const literal = findDeclarationColorLiteral(decl.prop, decl.value)
    if (literal !== null) {
      violations.push(`${label}: "${decl.prop}: ${decl.value}" содержит цвет "${literal}"`)
    }
  })

  return violations
}

/** Инлайн `style="..."` — та же проверка деклараций, обёрнутая в фиктивный селектор. */
function scanInlineStyleValue(value: string, label: string): string[] {
  return scanCssText(`a{${value}}`, label)
}

// less не установлен в проекте (TL-96, issue execaus/tube-leak#103):
// `@brand: #ff0000;` в less синтаксически валиден и для postcss (это просто
// at-rule без блока) — сторож молча пропустил бы значение. Вместо разбора
// less-специфичного синтаксиса (переменные `@x`, вложенность, миксины) —
// падение с понятным сообщением, «неразобранное — падение» (тот же принцип,
// что и для синтаксических ошибок CSS/шаблона/скрипта). scss оставлен
// разрешённым белым списком (не «всё, кроме less» — «всё, кроме
// перечисленного»): его синтаксис, отличный от plain CSS (вложенность,
// `&`, свои переменные), сторожем специально не разбирается — как и
// раньше, соответствующие ошибки ловит сам `postcss.parse`, если до них
// дойдёт.
const SUPPORTED_STYLE_LANGS = new Set(['css', 'scss'])

function assertSupportedStyleLang(lang: string | undefined, label: string): void {
  if (lang !== undefined && !SUPPORTED_STYLE_LANGS.has(lang.toLowerCase())) {
    throw new Error(`${label}: неподдерживаемый язык стилей "${lang}"`)
  }
}

// ---------------------------------------------------------------------------
// Скрипты и выражения: `.ts`/`.js`, `<script>`-блоки SFC, выражения привязок
// ---------------------------------------------------------------------------

interface ParseDiagnosticsHost {
  parseDiagnostics?: readonly ts.Diagnostic[]
}

/**
 * `ts.createSourceFile` не бросает даже на мусоре — синтаксические ошибки
 * оседают в необнародованном (но стабильном на практике) поле
 * `parseDiagnostics`. Без этой проверки сломанный `<script>` тихо дал бы
 * дерево без единого `StringLiteral` и «зелёный» тест.
 */
function assertNoParseErrors(sourceFile: ts.SourceFile, label: string): void {
  const diagnostics = (sourceFile as unknown as ParseDiagnosticsHost).parseDiagnostics
  if (diagnostics !== undefined && diagnostics.length > 0) {
    const messages = diagnostics
      .map((d) => ts.flattenDiagnosticMessageText(d.messageText, '; '))
      .join(' | ')
    throw new Error(`не удалось разобрать ${label}: ${messages}`)
  }
}

/**
 * Рекурсивный обход AST скрипта с распространением контекста «строка —
 * гарантированно CSS-значение» (`allowNames`, см. doc-комментарий файла,
 * «Четвёртый раунд»). `objectStyleContext` — входной признак «мы внутри
 * `.ts`-модуля целиком или внутри выражения директивы `:style`»: только
 * тогда ключ объекта, похожий на CSS-свойство, форсирует `allowNames` для
 * своего значения (п. 1). Присваивание в цепочку `....style` и вызов
 * `....setProperty(...)` (п. 2) форсируют `allowNames` независимо от
 * `objectStyleContext` — это отдельный, более узкий и самодостаточный
 * признак. Как только `allowNames` стал `true` для узла, он остаётся
 * `true` для всего поддерева (сброса вниз по дереву не бывает — CSS-текст
 * внутри CSS-текста не превращается обратно в прозу).
 */
function collectColorLiterals(
  node: ts.Node,
  allowNames: boolean,
  objectStyleContext: boolean,
  label: string,
  violations: string[],
): void {
  if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) {
    const literal = checkWholeLiteral(node.text, allowNames)
    if (literal !== null) {
      violations.push(`${label}: строковый литерал "${node.text}" содержит цвет "${literal}"`)
    }
    return
  }

  if (ts.isTemplateHead(node) || ts.isTemplateMiddleOrTemplateTail(node)) {
    const literal = checkFragmentLiteral(node.text, allowNames)
    if (literal !== null) {
      violations.push(`${label}: часть шаблонного литерала "${node.text}" содержит цвет "${literal}"`)
    }
    return
  }

  if (ts.isBinaryExpression(node) && node.operatorToken.kind === ts.SyntaxKind.EqualsToken
    && isStylePropertyAccessChain(node.left)) {
    collectColorLiterals(node.left, allowNames, objectStyleContext, label, violations)
    collectColorLiterals(node.right, true, objectStyleContext, label, violations)
    return
  }

  if (ts.isCallExpression(node) && isSetPropertyCall(node)) {
    collectColorLiterals(node.expression, allowNames, objectStyleContext, label, violations)
    node.arguments.forEach((arg, index) => {
      collectColorLiterals(arg, index === 1 ? true : allowNames, objectStyleContext, label, violations)
    })
    return
  }

  // Заголовок теста (TL-103, issue execaus/tube-leak#110) — не CSS-значение
  // и не подлежит проверке грамматикой цвета вовсе, каким бы ни было его
  // содержимое (`it('… issue #105 …', …)` не должен путать номер issue с
  // hex-цветом). Callee (`node.expression`, включая `.each([...])` — там
  // проверяется таблица данных) и остальные аргументы (обычно функция теста)
  // по-прежнему обходятся как всегда.
  if (ts.isCallExpression(node) && isTestTitleCall(node)) {
    collectColorLiterals(node.expression, allowNames, objectStyleContext, label, violations)
    node.arguments.forEach((arg, index) => {
      if (index === 0) return
      collectColorLiterals(arg, allowNames, objectStyleContext, label, violations)
    })
    return
  }

  if (ts.isPropertyAssignment(node)) {
    const keyText = getObjectPropertyKeyText(node.name)
    const valueAllowNames = allowNames
      || (objectStyleContext && keyText !== null && isCssPropertyLikeKey(keyText))
    collectColorLiterals(node.initializer, valueAllowNames, objectStyleContext, label, violations)
    return
  }

  ts.forEachChild(node, (child) => collectColorLiterals(child, allowNames, objectStyleContext, label, violations))
}

interface ScanScriptOptions {
  /** См. doc-комментарий `collectColorLiterals` — п. 1 «Четвёртого раунда». */
  objectStyleContext?: boolean
}

/** Полный модуль/скрипт (`.ts`/`.js`, содержимое `<script>`/`<script setup>`). */
function scanScriptModule(sourceText: string, label: string, options: ScanScriptOptions = {}): string[] {
  const sourceFile = ts.createSourceFile(label, sourceText, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS)
  assertNoParseErrors(sourceFile, label)

  const violations: string[] = []
  collectColorLiterals(sourceFile, false, options.objectStyleContext ?? false, label, violations)
  return violations
}

/**
 * Фрагмент выражения из шаблона (`exp.content` директивы/интерполяции —
 * не полноценный файл, а кусок вроде `{ color: 'red' }` или `a ? b : c`).
 * Заворачивается в скобки, чтобы `{ ... }` разобрался как объектный
 * литерал (выражение), а не как statement-блок. Используется для всех
 * директив, КРОМЕ `:style` (свой путь — `scanStyleDirectiveExpression`),
 * `v-on`/`@` (список statement'ов — `scanScriptModule` без обёртки) и
 * `v-for` (проверяется только `forParseResult.source`, см.
 * `scanTemplateProp`).
 */
function scanScriptExpression(exprText: string, label: string): string[] {
  return scanScriptModule(`(${exprText})`, label)
}

/** Снимает обёрточные скобки: `((expr))` → `expr`. */
function unwrapParens(expr: ts.Expression): ts.Expression {
  let current = expr
  while (ts.isParenthesizedExpression(current)) {
    current = current.expression
  }
  return current
}

/**
 * Выражение директивы `:style`/`v-bind:style` (см. doc-комментарий файла,
 * «Четвёртый раунд», п. 3): если выражение целиком — строковый литерал
 * (`:style="'border: 1px solid red'"`), это инлайн-CSS текст, а не JS —
 * разбирается тем же путём, что статический атрибут `style="..."`
 * (`scanInlineStyleValue`/postcss). Иначе — обычный скриптовый разбор с
 * признаком «стилевой объект» (п. 1а): любой объектный литерал внутри
 * этого выражения (включая ветки тернарника) — стилевой.
 */
function scanStyleDirectiveExpression(exprText: string, label: string): string[] {
  const sourceFile = ts.createSourceFile(label, `(${exprText})`, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS)
  assertNoParseErrors(sourceFile, label)

  const [firstStatement] = sourceFile.statements
  if (firstStatement !== undefined && ts.isExpressionStatement(firstStatement)) {
    const inner = unwrapParens(firstStatement.expression)
    if (ts.isStringLiteral(inner) || ts.isNoSubstitutionTemplateLiteral(inner)) {
      return scanInlineStyleValue(inner.text, `${label} (строка стиля)`)
    }
  }

  const violations: string[] = []
  collectColorLiterals(sourceFile, false, true, label, violations)
  return violations
}

// ---------------------------------------------------------------------------
// Шаблон: AST из `@vue/compiler-sfc`, используется и для `.vue`, и (обёрнутый
// в `<template>`) для `.svg`/`.html`
// ---------------------------------------------------------------------------

// Компактные локальные типы под форму AST `@vue/compiler-core` — без
// собственного импорта этого пакета (он не в package.json проекта, только
// транзитивная зависимость `@vue/compiler-sfc`; см. бриф — «других
// зависимостей не добавлять»). Числа `type` — коды `NodeTypes` этого
// пакета: 0 ROOT, 1 ELEMENT, 2 TEXT, 3 COMMENT, 5 INTERPOLATION,
// 6 ATTRIBUTE, 7 DIRECTIVE (проверено эмпирически, см. отчёт по задаче).
interface RawExprNode {
  content?: string
}

interface RawTemplateNode {
  type: number
  tag?: string
  props?: RawTemplateProp[]
  children?: RawTemplateNode[]
  content?: RawExprNode | string
}

interface RawTemplateProp {
  type: number
  name?: string
  value?: RawExprNode
  arg?: RawExprNode
  exp?: RawExprNode
  // Только у `v-for` (`prop.name === 'for'`) — `@vue/compiler-sfc` сам
  // разбирает форму `значение in/of источник` (в т. ч. с деструктуризацией
  // и индексом слева) и кладёт готовую правую часть сюда. Проверяется
  // только `source` — левая часть является паттерном объявления, не
  // значением, ей нечего вычислять (см. doc-комментарий файла, «Четвёртый
  // раунд»).
  forParseResult?: { source?: RawExprNode }
}

function collectElementText(node: RawTemplateNode): string {
  return (node.children ?? [])
    .filter((child) => child.type === 2 && typeof child.content === 'string')
    .map((child) => child.content as string)
    .join('')
}

/** Значение статического атрибута элемента по имени, `undefined` если атрибута нет или он динамический. */
function getStaticAttrValue(node: RawTemplateNode, attrName: string): string | undefined {
  const prop = (node.props ?? []).find((p) => p.type === 6 && p.name === attrName)
  return prop?.value?.content
}

function scanTemplateProp(prop: RawTemplateProp, label: string, violations: string[]): void {
  if (prop.type === 6) {
    // Статический атрибут.
    const name = prop.name ?? ''
    const value = prop.value?.content
    if (value === undefined) return

    if (name.toLowerCase() === 'style') {
      violations.push(...scanInlineStyleValue(value, `${label} style="${value}"`))
      return
    }

    // Только презентационные цветовые атрибуты проверяются на «значение
    // целиком является цветом» (TL-96, белый список
    // `COLOR_PRESENTATION_ATTRIBUTES`) — `role`, `aria-*`, пропсы
    // компонентов (`tone`, …) и `class` не несут цвет презентационно и не
    // проверяются вовсе, что бы ни оказалось их значением.
    if (!COLOR_PRESENTATION_ATTRIBUTES.has(name.toLowerCase())) return

    // «Целиком является цветом» — строгое равенство, не «содержит» (см.
    // doc-комментарий файла, п. 2): `class="btn primary"` не должен
    // ловиться только из-за того, что где-то есть слово-цвет.
    if (isWholeValueColor(value)) {
      violations.push(`${label}: атрибут ${name}="${value}" целиком является цветом`)
    }
    return
  }

  if (prop.type === 7) {
    // Директива. `prop.name` — имя самой директивы (`bind`/`on`/`for`/
    // `slot`/`if`/…, без `v-`), не аргумента: `:style` и `v-on:click` дают
    // 'bind' и 'on' соответственно, а имя атрибута/события — в `prop.arg`.
    const directiveName = prop.name ?? ''
    const argOrName = prop.arg?.content ?? directiveName

    if (directiveName === 'for') {
      // `item of items`/`(item, index) in items` — не выражение (левая
      // часть — паттерн объявления), проверяется только правая часть,
      // уже разобранная самим `@vue/compiler-sfc` (см. `RawTemplateProp`).
      const source = prop.forParseResult?.source?.content
      if (source !== undefined) {
        violations.push(...scanScriptExpression(source, `${label} v-for (источник) "${source}"`))
      }
      return
    }

    const exprText = prop.exp?.content
    if (exprText === undefined) return

    if (directiveName === 'on') {
      // Список statement'ов (`open = false; emit('close')`), не выражение —
      // обёртка в скобки здесь синтаксическая ошибка, нужен обычный
      // модульный разбор без обёртки.
      violations.push(...scanScriptModule(exprText, `${label} @${argOrName}="${exprText}"`))
      return
    }

    if (directiveName === 'bind' && argOrName.toLowerCase() === 'style') {
      violations.push(...scanStyleDirectiveExpression(exprText, `${label} :style="${exprText}"`))
      return
    }

    // `:class`/`v-bind:class` не несёт цвет презентационно (TL-96): значение
    // — список имён CSS-классов, а не CSS-значение, поэтому слово-цвет в
    // нём (`:class="'red'"`) не проверяется вовсе — симметрично статическому
    // `class`, не входящему в `COLOR_PRESENTATION_ATTRIBUTES`.
    if (directiveName === 'bind' && argOrName.toLowerCase() === 'class') return

    violations.push(...scanScriptExpression(exprText, `${label} :${argOrName}="${exprText}"`))
  }
}

/**
 * Рекурсивный обход дерева шаблона. `<style>`/`<script>` как обычные
 * элементы (актуально для обёрнутых `.svg`/`.html`, см.
 * `scanMarkupFile` — в настоящих `.vue` эти блоки `@vue/compiler-sfc`
 * извлекает отдельно от AST шаблона, сюда не попадают) уходят в CSS/скрипт
 * разбор своего текстового содержимого, а не в разбор атрибутов/детей как
 * элемент. Текстовые узлы (тип 2) и комментарии (тип 3) никогда не
 * проверяются — этим и объясняется, что «red»/«green» в обычном тексте
 * шаблона не ловится: они не строковые литералы и не значения атрибута,
 * им попросту некуда попасть в этом обходе.
 */
function scanTemplateAst(node: RawTemplateNode, label: string, violations: string[]): void {
  if (node.type === 1) {
    const tag = (node.tag ?? '').toLowerCase()
    if (tag === 'style') {
      const lang = getStaticAttrValue(node, 'lang')
      assertSupportedStyleLang(lang, `${label} <style lang="${lang ?? 'css'}">`)
      violations.push(...scanCssText(collectElementText(node), `${label} <style>`))
      return
    }
    if (tag === 'script') {
      violations.push(...scanScriptModule(collectElementText(node), `${label} <script>`))
      return
    }
    for (const prop of node.props ?? []) {
      scanTemplateProp(prop, label, violations)
    }
  } else if (node.type === 5) {
    const content = node.content
    const exprText = typeof content === 'object' && content !== null ? content.content : undefined
    if (exprText !== undefined) {
      violations.push(...scanScriptExpression(exprText, `${label} {{ ${exprText} }}`))
    }
    return
  }

  for (const child of node.children ?? []) {
    scanTemplateAst(child, label, violations)
  }
}

// ---------------------------------------------------------------------------
// Разбор файла целиком, по расширению
// ---------------------------------------------------------------------------

function scanVueFile(source: string, label: string): string[] {
  const { descriptor, errors } = parseSFC(source, { filename: label })
  if (errors.length > 0) {
    throw new Error(`не удалось разобрать ${label}: ${errors.map((e) => e.message).join('; ')}`)
  }

  const violations: string[] = []

  if (descriptor.template) {
    scanTemplateAst(descriptor.template.ast as unknown as RawTemplateNode, label, violations)
  }
  for (const style of descriptor.styles) {
    assertSupportedStyleLang(style.lang, `${label} <style lang="${style.lang ?? 'css'}">`)
    violations.push(...scanCssText(style.content, `${label} <style>`))
  }
  if (descriptor.script) {
    violations.push(...scanScriptModule(descriptor.script.content, `${label} <script>`))
  }
  if (descriptor.scriptSetup) {
    violations.push(...scanScriptModule(descriptor.scriptSetup.content, `${label} <script setup>`))
  }

  return violations
}

/**
 * `.svg`/`.html` — не полноценный SFC, поэтому содержимое заворачивается в
 * фиктивный `<template>`, чтобы переиспользовать тот же разбор и тот же
 * обход дерева, что и для `.vue`-компонентов (`scanTemplateAst` сама
 * умеет вылущивать вложенные `<style>`/`<script>` как текстовые блоки).
 */
function scanMarkupFile(source: string, label: string): string[] {
  return scanVueFile(`<template>${source}</template>`, label)
}

function scanFile(absolutePath: string, label: string): string[] {
  const source = readFileSync(absolutePath, 'utf-8')

  if (absolutePath.endsWith('.css')) return scanCssText(source, label)
  if (absolutePath.endsWith('.ts') || absolutePath.endsWith('.js')) {
    return scanScriptModule(source, label, { objectStyleContext: true })
  }
  if (absolutePath.endsWith('.vue')) return scanVueFile(source, label)
  if (absolutePath.endsWith('.svg') || absolutePath.endsWith('.html')) return scanMarkupFile(source, label)

  throw new Error(`сторож не знает, как разбирать файл: ${label}`)
}

// ---------------------------------------------------------------------------
// Обход файлового дерева
// ---------------------------------------------------------------------------

function listFilesRecursively(dir: string): string[] {
  const files: string[] = []
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const fullPath = join(dir, entry.name)
    if (entry.isDirectory()) {
      files.push(...listFilesRecursively(fullPath))
    } else {
      files.push(fullPath)
    }
  }
  return files
}

describe('палитра проекта — сторож литералов цвета вне src/style.css (TL-22)', () => {
  const allFiles = listFilesRecursively(SRC_DIR)
  const scannedFiles = allFiles.filter(
    (path) => path !== SELF_PATH && SCANNED_EXTENSIONS.some((ext) => path.endsWith(ext)),
  )

  it('обход видит файлы во вложенных подкаталогах (src/components/)', () => {
    // Без этой проверки сторож можно было бы по ошибке ограничить только
    // корнем `src/` и не заметить, что рекурсия сломана, — тест ниже упал
    // бы «зелёным» просто потому, что ничего не проверил (урок проекта).
    const sawNestedVueFile = scannedFiles.some(
      (path) => relative(SRC_DIR, path).startsWith(`components${sep}`) && path.endsWith('.vue'),
    )
    expect(sawNestedVueFile).toBe(true)
  })

  it.each(scannedFiles.map((path) => [relative(SRC_DIR, path), path] as const))(
    '%s не содержит литерал цвета вне src/style.css',
    (relativePath, absolutePath) => {
      if (ALLOWED_RELATIVE_PATHS.has(relativePath.split('\\').join('/'))) {
        return
      }

      const violations = scanFile(absolutePath, relativePath)

      expect(
        violations,
        `найдены литералы цвета — используй переменную из src/style.css:\n${violations.join('\n')}`,
      ).toEqual([])
    },
  )
})

// ---------------------------------------------------------------------------
// Фикстуры — нарушения, которые предыдущие раунды ревью ловили руками
// ---------------------------------------------------------------------------

function cssViolations(cssText: string): string[] {
  return scanCssText(cssText, 'fixture.css')
}

function templateViolations(templateInner: string): string[] {
  return scanVueFile(`<template>${templateInner}</template>`, 'fixture.vue')
}

function scriptViolations(scriptBody: string, objectStyleContext = false): string[] {
  return scanScriptModule(scriptBody, 'fixture.ts', { objectStyleContext })
}

/**
 * Симулирует реальное сканирование `.ts`-файла: `scanFile` передаёт
 * `objectStyleContext: true` для `.ts`/`.js` (см. doc-комментарий файла,
 * «Четвёртый раунд», п. 1б) — обычный `scriptViolations` без аргумента
 * симулирует более узкий контекст (фрагмент/`<script setup>`).
 */
function moduleScriptViolations(scriptBody: string): string[] {
  return scriptViolations(scriptBody, true)
}

describe('сторож — фикстуры-мутации ловятся (два предыдущих раунда ревью)', () => {
  it.each([
    ['своя переменная в <style>', () => cssViolations(':root { --brand: #ff0000; }')],
    ['переопределение токена проекта', () => cssViolations(':root { --color-accent: red; }')],
    ['голый цвет без завершающей ; (последняя декларация)', () => cssViolations('.a { color: red }')],
    ['scrollbar-color', () => cssViolations('.a { scrollbar-color: red blue; }')],
    ['text-shadow', () => cssViolations('.a { text-shadow: 0 0 2px red; }')],
    ['drop-shadow() внутри filter', () => cssViolations('.a { filter: drop-shadow(0 0 4px red); }')],
    ['stop-color как декларация', () => cssViolations('.a { stop-color: red; }')],
    ['flood-color', () => cssViolations('.a { flood-color: red; }')],
    ['color-mix()', () => cssViolations('.a { background: color-mix(in srgb, white, black 20%); }')],
    ['oklch()', () => cssViolations('.a { color: oklch(62% 0.2 29); }')],
    ['color(display-p3 ...)', () => cssViolations('.a { color: color(display-p3 1 0 0); }')],
    ['light-dark()', () => cssViolations('.a { color: light-dark(black, white); }')],
    ['hwb()', () => cssViolations('.a { color: hwb(0 0% 0%); }')],
    ['var() с fallback не прячет литерал', () => cssViolations('.a { color: var(--missing, red); }')],
    ['SVG fill/stroke как статические атрибуты', () => templateViolations('<path fill="red" stroke="black" />')],
    ['stop-color как атрибут SVG', () => templateViolations('<stop stop-color="red" />')],
    [':style-объект в шаблоне', () => templateViolations('<div :style="{ color: \'red\' }" />')],
    ['тернарник в :style', () => templateViolations('<div :style="cond ? { color: \'red\' } : { color: \'blue\' }" />')],
    [':fill с тернарником', () => templateViolations('<path :fill="isActive ? \'red\' : \'blue\'" />')],
    ['интерполяция {{ }}', () => templateViolations('<p>{{ isActive ? \'red\' : \'blue\' }}</p>')],
    ['style-литерал в .ts', () => scriptViolations("const s = { color: 'red' }")],
    ['el.style.color = ...', () => scriptViolations("el.style.color = 'red'")],
    ['setProperty с hex', () => scriptViolations("el.style.setProperty('color', '#f00')")],
    ['голый hex-литерал строкой', () => scriptViolations("const c = '#ff0000'")],
    ['голое имя цвета строкой', () => scriptViolations("const c = 'red'")],
  ])('краснеет: %s', (_label, run) => {
    expect(run()).not.toEqual([])
  })

  it('честный компромисс (см. doc-комментарий файла): "#add" синтаксически валиден как hex и тоже ловится', () => {
    // Это единственное расхождение с исходным списком «не должно ловить» —
    // названо и протестировано осознанно, а не замаскировано (требование
    // брифа, раздел 3): `#add` неотличим от настоящего 3-значного hex-цвета
    // без внешнего контекста, а строковый литерал в TS его не несёт.
    expect(scriptViolations("const id = '#add'")).not.toEqual([])
  })
})

describe('сторож — падает, если файл не удалось разобрать, а не молчит', () => {
  it('незакрытый комментарий в CSS — postcss бросает, сторож не глушит', () => {
    expect(() => cssViolations('.a { color: red /* oops')).toThrow()
  })

  it('незакрытый блочный комментарий в скрипте — ts.createSourceFile не бросает сам, сторож проверяет диагностику', () => {
    expect(() => scriptViolations('const a = 1; /* oops')).toThrow()
  })

  it('незакрытый HTML-комментарий в шаблоне — errors из @vue/compiler-sfc не игнорируются', () => {
    expect(() => templateViolations('<div><!-- oops</div>')).toThrow()
  })

  it('незакрытая одинарная кавычка в скрипте', () => {
    expect(() => scriptViolations("const a = 'oops")).toThrow()
  })
})

// ---------------------------------------------------------------------------
// Фикстуры — ложные срабатывания, которых быть не должно
// ---------------------------------------------------------------------------

describe('сторож — не даёт заявленных ложных срабатываний', () => {
  it.each([
    ['background: url(...)', () => cssViolations('.a { background: url(red-arrow.png); }')],
    ['граница слова: border-radius — не border-color', () => cssViolations('.a { border-radius: 4px; }')],
    ['граница слова: outline-offset — не литерал', () => cssViolations('.a { outline-offset: 2px; }')],
    ['допустимая ссылка на токен', () => cssViolations('.a { color: var(--color-accent); }')],
    ['допустимое служебное значение', () => cssViolations('.a { background: transparent; }')],
    ['currentColor разрешён', () => cssViolations('.a { border-color: currentColor; }')],
    ['реальная композиция токенов проходит чисто', () => cssViolations('.a { border: 1px solid var(--color-border); }')],
    ['вызов метода .color(...) — не CSS-функция', () => scriptViolations("theme.color('x')")],
    ['issue-номер в комментарии', () => scriptViolations('// see issue #100')],
    ['тернарник с именами — идентификаторы, не строки', () => scriptViolations('const x = b ? tan : red')],
    ['имя функции содержит "rgb"', () => scriptViolations("function parseRgb(input: string) {}")],
    ['имя файла с цветом внутри как часть слова', () => scriptViolations("const icon = 'red-arrow.png'")],
    ['слова "red"/"green" в обычном тексте шаблона', () => templateViolations('<p>The red panda eats green bamboo</p>')],
    [
      'апостроф в тексте шаблона не путает разбор ("it\'s")',
      () => templateViolations("<p>hi it's a test</p>"),
    ],
    [
      'вложенная шаблонная строка с обратной кавычкой не роняет разбор',
      () => scriptViolations('const s = `${\'`\'}`'),
    ],
    ['реальный :style на ширину прогресса — не цвет', () => templateViolations(':style="{ width: `${percent}%` }"')],
    ['класс с несколькими словами — не цвет целиком', () => templateViolations('<div class="btn primary" />')],
  ])('не краснеет: %s', (_label, run) => {
    expect(run()).toEqual([])
  })
})

// ---------------------------------------------------------------------------
// Б-1 (четвёртый раунд) — имена цветов внутри составных строк, по контексту
// ---------------------------------------------------------------------------

describe('Б-1: имена цветов внутри составной строки ловятся в контекстах п. 1–3', () => {
  it.each([
    [
      'ключ объекта в :style похож на CSS-свойство (border)',
      () => templateViolations('<div :style="{ border: \'1px solid red\' }" />'),
    ],
    [
      'ключ объекта в .ts-модуле похож на CSS-свойство (camelCase boxShadow)',
      () => moduleScriptViolations("export const cardStyle = { boxShadow: '0 0 0 2px white' }"),
    ],
    [
      'выражение :style целиком — строка инлайн-CSS',
      () => templateViolations('<div :style="\'border: 1px solid red\'" />'),
    ],
  ])('краснеет: %s', (_label, run) => {
    expect(run()).not.toEqual([])
  })

  it('не краснеет: проза с системными цветами вне контекстов п. 1–3 ("Field and red window")', () => {
    // Требование ведущего — закрепление конкретно этого случая: `Field` и
    // `Window` легитимные системные цвета CSS **и** обычные английские
    // слова (см. doc-комментарий `findScriptColorLiteral`). Проверяется в
    // контексте целого `.ts`-модуля (`moduleScriptViolations`, как реально
    // сканируется `*.test.ts`), чтобы доказать: новый поиск имён по ключу
    // объекта не расширился до произвольного текстового аргумента вызова —
    // здесь нет ни одного объектного литерала с CSS-подобным ключом, ни
    // `.style`-цепочки, ни `setProperty`.
    expect(moduleScriptViolations("it('Field and red window', () => {})")).toEqual([])
  })

  it('.style-цепочка ловит составное имя независимо от объектного контекста', () => {
    expect(scriptViolations("el.style.border = '1px solid red'")).not.toEqual([])
  })

  it('setProperty ловит составное имя во втором аргументе', () => {
    expect(scriptViolations("el.style.setProperty('border', '1px solid red')")).not.toEqual([])
  })

  it('ключ объекта, НЕ похожий на CSS-свойство по форме, не форсирует имена (снаружи стилевого контекста)', () => {
    // `2fast` — не проходит `^-?[a-z]+(-[a-z]+)*$` (цифра), поэтому даже в
    // .ts-модуле не считается CSS-свойством — составное имя внутри не ищется.
    expect(moduleScriptViolations("const x = { '2fast': 'go red now' }")).toEqual([])
  })
})

// ---------------------------------------------------------------------------
// Интерполяция шаблонных литералов — статические части (дешёвый пункт 1)
// ---------------------------------------------------------------------------

describe('интерполяция: статические части шаблонного литерала с подстановкой', () => {
  it('функция цвета в статической части :style-объекта (hsl(${hue}, ...))', () => {
    const violations = templateViolations(
      '<div :style="{ background: `hsl(${hue}, 70%, 45%)` }" />',
    )
    expect(violations).not.toEqual([])
  })

  it('составное имя в хвосте шаблонного литерала внутри .style-присваивания', () => {
    const violations = scriptViolations(
      'el.style.cssText = `color: ${c}; border: 1px solid red`',
    )
    expect(violations).not.toEqual([])
  })

  it('не краснеет: статическая часть без цвета, вне контекстов п. 1–3 (ширина прогресса)', () => {
    expect(templateViolations('<div :style="{ width: `${percent}%` }" />')).toEqual([])
  })

  it('не краснеет: голое имя цвета как ЧАСТЬ (не всё значение) статического текста вне контекста', () => {
    // `checkFragmentLiteral` не применяет `isWholeValueColor` — иначе кусок
    // "red" в `${x}red` ловился бы как «имя целиком» даже вне контекстов
    // п. 1–3, а это не то же самое, что «имя внутри составной строки».
    expect(scriptViolations('const label = `${prefix}red`')).toEqual([])
  })
})

// ---------------------------------------------------------------------------
// Обработчики v-on/v-for/v-slot — разбор не падает (дешёвый пункт 2)
// ---------------------------------------------------------------------------

describe('v-on/v-for/v-slot: выражение разбирается без падения', () => {
  it.each([
    ['v-on со списком statement\'ов через ;', () => templateViolations('<button @click="open = false; emit(\'close\')">x</button>')],
    ['v-for с item of items', () => templateViolations('<div v-for="item of items" :key="item.id">{{ item }}</div>')],
    ['v-for с деструктуризацией и индексом', () => templateViolations('<div v-for="(item, index) in items" :key="index">{{ item }}</div>')],
    ['v-slot с деструктуризацией', () => templateViolations('<template v-slot="{ a, b }"><span>{{ a }}</span></template>')],
    ['#slot (сокращение) с переименованием', () => templateViolations('<template #default="{ item: renamedItem }"><span>{{ renamedItem }}</span></template>')],
    ['v-slot со значением по умолчанию при деструктуризации', () => templateViolations('<template v-slot="{ a = 1 }"><span>{{ a }}</span></template>')],
  ])('не падает и не краснеет: %s', (_label, run) => {
    expect(run()).toEqual([])
  })

  it('v-on: реальное нарушение внутри statement всё равно ловится (парсинг не глушит проверку)', () => {
    const violations = templateViolations(
      '<button @click="el.style.color = \'red\'; emit(\'close\')">x</button>',
    )
    expect(violations).not.toEqual([])
  })

  it('v-for: цвет в источнике (правой части) ловится', () => {
    // Надуманный, но валидный случай: показывает, что проверяется именно
    // `forParseResult.source`, а не отбрасывается совсем.
    const violations = templateViolations('<div v-for="item of (\'#ff0000\')" />')
    expect(violations).not.toEqual([])
  })
})

// ---------------------------------------------------------------------------
// TL-96 (issue execaus/tube-leak#103), пятый раунд — ложные срабатывания и less
// ---------------------------------------------------------------------------

function sfcViolations(source: string): string[] {
  return scanVueFile(source, 'fixture.vue')
}

describe('TL-96: статический атрибут — только белый список презентационных атрибутов', () => {
  it.each([
    ['role="menu" — не цветовой атрибут', () => templateViolations('<div role="menu" />')],
    ['aria-haspopup="menu" — не цветовой атрибут', () => templateViolations('<button aria-haspopup="menu" />')],
    ['проп компонента tone="green"', () => templateViolations('<MyBadge tone="green" />')],
    ['class="green" — class не проверяется', () => templateViolations('<div class="green" />')],
  ])('не краснеет: %s', (_label, run) => {
    expect(run()).toEqual([])
  })

  it.each([
    ['fill="red" остаётся нарушением (белый список не ослеплён)', () => templateViolations('<path fill="red" />')],
    ['stroke="blue" остаётся нарушением', () => templateViolations('<path stroke="blue" />')],
    ['color="red" остаётся нарушением', () => templateViolations('<font color="red" />')],
    ['style="color: red" остаётся нарушением', () => templateViolations('<div style="color: red" />')],
  ])('краснеет: %s', (_label, run) => {
    expect(run()).not.toEqual([])
  })
})

describe('TL-96: :class/v-bind:class не проверяется', () => {
  it.each([
    [':class="\'red\'" — статическая строка', () => templateViolations('<div :class="\'red\'" />')],
    ['v-bind:class с тем же значением (полная форма)', () => templateViolations('<div v-bind:class="\'red\'" />')],
    [':class с объектной формой', () => templateViolations('<div :class="{ red: isActive }" />')],
  ])('не краснеет: %s', (_label, run) => {
    expect(run()).toEqual([])
  })

  it(':style на том же элементе продолжает проверяться, когда рядом :class', () => {
    const violations = templateViolations('<div :class="\'red\'" :style="{ color: \'red\' }" />')
    expect(violations).not.toEqual([])
  })
})

describe('TL-96: имя CSS-свойства в transition/transition-property/will-change — не цвет', () => {
  it.each([
    ['transition: background 0.2s ease', () => cssViolations('.a { transition: background 0.2s ease; }')],
    ['transition-property: список свойств', () => cssViolations('.a { transition-property: background, color; }')],
    ['will-change: список свойств', () => cssViolations('.a { will-change: transform, background; }')],
    ['transition: color 0.2s — "color" тоже имя свойства', () => cssViolations('.a { transition: color 0.2s linear; }')],
  ])('не краснеет: %s', (_label, run) => {
    expect(run()).toEqual([])
  })

  it.each([
    // Регресс-барьер: асимметрия точечная (по имени СВОЙСТВА декларации),
    // а не глобальное ослабление грамматики — `background: red` (реальное
    // цветовое свойство, не из PROPERTY_NAME_POSITION_PROPS) по-прежнему
    // ловится.
    ['background: red — реальное свойство вне исключения', () => cssViolations('.a { background: red; }')],
    // Защитный барьер: hex/цветовая функция в этих трёх свойствах всё
    // равно ловится — исключены только голые ИМЕНА, а не вся грамматика.
    ['will-change с hex где угодно в значении (защитный барьер)', () => cssViolations('.a { will-change: #ff0000; }')],
    ['transition-property с цветовой функцией (защитный барьер)', () => cssViolations('.a { transition-property: rgb(255, 0, 0); }')],
  ])('краснеет: %s', (_label, run) => {
    expect(run()).not.toEqual([])
  })
})

describe('TL-96: <style lang="less"> падает как неподдерживаемый язык стилей', () => {
  it('SFC .vue со <style lang="less"> падает с понятным сообщением', () => {
    const source = [
      '<template><div /></template>',
      '<style lang="less">',
      '@brand: #ff0000;',
      '.a { color: @brand; }',
      '</style>',
    ].join('\n')

    expect(() => sfcViolations(source)).toThrow(/неподдерживаемый язык стилей/)
  })

  it('вложенный <style lang="less"> внутри .svg/.html (обёрнутый в <template>) тоже падает', () => {
    const source = '<svg><style lang="less">@brand: #ff0000;</style></svg>'
    expect(() => scanMarkupFile(source, 'fixture.svg')).toThrow(/неподдерживаемый язык стилей/)
  })

  it('не краснеет и не падает: <style> без lang (по умолчанию css) работает как раньше', () => {
    const source = '<template><div /></template>\n<style>\n.a { border: 1px solid var(--color-border); }\n</style>\n'
    expect(sfcViolations(source)).toEqual([])
  })

  it('не падает: <style lang="scss"> разрешён и по-прежнему ловит настоящие нарушения', () => {
    const source = [
      '<template><div /></template>',
      '<style lang="scss">',
      '.a { color: red; }',
      '</style>',
    ].join('\n')

    expect(sfcViolations(source)).not.toEqual([])
  })
})

// ---------------------------------------------------------------------------
// TL-103 (issue execaus/tube-leak#110), шестой раунд — ссылка на issue не hex
// ---------------------------------------------------------------------------

describe('TL-103: заголовок describe/it/test не проверяется вовсе', () => {
  it.each([
    ['it с номером issue в прозе', () => scriptViolations("it('… issue #105 …', () => {})")],
    ['describe с номером issue в начале строки', () => scriptViolations("describe('#110 заголовок', () => {})")],
    [
      'it с заголовком, ЦЕЛИКОМ совпадающим с hex (#105) — различает исключение заголовка от гейта hex по контексту',
      () => scriptViolations("it('#105', () => {})"),
    ],
    ['test(...)', () => scriptViolations("test('#105 тоже не проверяется', () => {})")],
    ['it.skip(...)', () => scriptViolations("it.skip('issue #105', () => {})")],
    ['it.only(...)', () => scriptViolations("it.only('issue #105', () => {})")],
    ['describe.skip(...)', () => scriptViolations("describe.skip('issue #105', () => {})")],
    [
      'it.each([...])(title, fn) — заголовок из .each не проверяется',
      () => scriptViolations("it.each([[1]])('#105 случай %i', (n) => {})"),
    ],
    [
      'шаблонная подпись с подстановкой в заголовке',
      () => scriptViolations('it(`issue #${n}`, () => {})'),
    ],
  ])('не краснеет: %s', (_label, run) => {
    expect(run()).toEqual([])
  })

  it('не краснеет: expect(x, "см. #105") — сообщение assertion, не заголовок теста, но hex вне CSS-контекста', () => {
    // `expect` не входит в `TEST_DEFINITION_NAMES` — это доказывает, что
    // зелёный цвет здесь получен именно ограничением поиска hex CSS-контекстом
    // (см. `findScriptColorLiteral`), а не совпадением с исключением
    // заголовка теста.
    expect(scriptViolations("expect(x, 'см. #105')")).toEqual([])
  })

  it('таблица данных it.each([...]) по-прежнему проверяется (не заголовок)', () => {
    // `.each([...])` сам по себе — не заголовочный вызов: его единственный
    // аргумент — таблица данных, а не название теста, реальный цвет внутри
    // неё обязан ловиться.
    const violations = scriptViolations("it.each([['#ff0000']])('%s', (hex) => {})")
    expect(violations).not.toEqual([])
  })

  it('функция-обработчик (второй аргумент it/describe) по-прежнему проверяется', () => {
    const violations = scriptViolations("it('заголовок без цвета', () => { el.style.color = 'red' })")
    expect(violations).not.toEqual([])
  })
})

describe('TL-103: hex внутри составной строки скрипта — только в CSS-контексте', () => {
  it.each([
    ['ключ объекта похож на CSS-свойство (color) — модуль', () => moduleScriptViolations("const s = { color: '#105' }")],
    ['ключ объекта похож на CSS-свойство (border) — модуль', () => moduleScriptViolations("const s = { border: '1px solid #fff' }")],
    ['присваивание в .style.color', () => scriptViolations("el.style.color = '#f00'")],
    ['setProperty с составным hex во втором аргументе', () => scriptViolations("el.style.setProperty('color', '#abc')")],
    ['значение целиком равно hex', () => scriptViolations("const c = '#fff'")],
    [':style-объект в шаблоне (background hex)', () => templateViolations('<div :style="{ background: \'#000\' }" />')],
    ['вызов цветовой функции в любой строке (не hex)', () => scriptViolations("const s = '0 0 2px rgb(0,0,0)'")],
  ])('краснеет: %s', (_label, run) => {
    expect(run()).not.toEqual([])
  })

  it('не краснеет: составной hex вне CSS-контекста и вне заголовка теста', () => {
    expect(scriptViolations("const message = 'см. issue #105 в отчёте'")).toEqual([])
  })
})

// ---------------------------------------------------------------------------
// index.html — фон синхронизирован с --color-bg (сохранённое исключение)
// ---------------------------------------------------------------------------

describe('index.html — фон синхронизирован с --color-bg', () => {
  function extractRootVar(styleCssSource: string, dark: boolean, name: string): string | null {
    const [lightBlock, darkBlock] = styleCssSource.split('@media (prefers-color-scheme: dark)')
    const block = dark ? darkBlock : lightBlock
    if (block === undefined) return null
    const m = block.match(new RegExp(`--${name}\\s*:\\s*([^;]+);`))
    return m?.[1]?.trim().toLowerCase() ?? null
  }

  function extractHtmlBackground(indexHtmlSource: string, dark: boolean): string | null {
    const [lightBlock, darkBlock] = indexHtmlSource.split('@media (prefers-color-scheme: dark)')
    const block = dark ? darkBlock : lightBlock
    if (block === undefined) return null
    const m = block.match(/html\s*\{[^}]*background\s*:\s*([^;]+);/)
    return m?.[1]?.trim().toLowerCase() ?? null
  }

  it('светлый и тёмный фон index.html равны --color-bg из src/style.css', () => {
    const styleCss = readFileSync(STYLE_CSS_PATH, 'utf-8')
    const indexHtml = readFileSync(INDEX_HTML_PATH, 'utf-8')

    const lightToken = extractRootVar(styleCss, false, 'color-bg')
    const darkToken = extractRootVar(styleCss, true, 'color-bg')
    const lightHtml = extractHtmlBackground(indexHtml, false)
    const darkHtml = extractHtmlBackground(indexHtml, true)

    expect(lightToken, '--color-bg (светлая) должен быть объявлен').not.toBeNull()
    expect(darkToken, '--color-bg (тёмная) должен быть объявлен').not.toBeNull()
    expect(lightHtml, 'index.html должен задавать светлый фон html').not.toBeNull()
    expect(darkHtml, 'index.html должен задавать тёмный фон html').not.toBeNull()

    expect(lightHtml).toBe(lightToken)
    expect(darkHtml).toBe(darkToken)
  })
})

// ---------------------------------------------------------------------------
// Контраст обводки фокуса на баннере очереди в тёмной теме (П-4)
// ---------------------------------------------------------------------------

describe('контраст кольца фокуса на баннере очереди в тёмной теме', () => {
  // WCAG 2.1 relative luminance / contrast ratio.
  function relativeLuminance([r, g, b]: readonly [number, number, number]): number {
    const channel = (c: number) => {
      const v = c / 255
      return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4
    }
    return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
  }

  function contrastRatio(a: readonly [number, number, number], b: readonly [number, number, number]): number {
    const lA = relativeLuminance(a)
    const lB = relativeLuminance(b)
    const lighter = Math.max(lA, lB)
    const darker = Math.min(lA, lB)
    return (lighter + 0.05) / (darker + 0.05)
  }

  /**
   * Разбирает значение CSS-цвета (hex 3/4/6/8, `rgb()`/`rgba()`,
   * `hsl()`/`hsla()`) в канал RGB 0..255. Бросает на любой другой форме —
   * П-4: если токен в тёмном блоке присутствует, но не разобрался, тест
   * обязан упасть, а не молча взять светлое значение.
   */
  function parseCssColorToRgb(raw: string): readonly [number, number, number] {
    const value = raw.trim()

    const hex = /^#([0-9a-fA-F]{3}|[0-9a-fA-F]{4}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$/.exec(value)
    if (hex?.[1] !== undefined) {
      const digits = hex[1]
      const expand = (s: string) => (s.length === 1 ? s + s : s)
      if (digits.length <= 4) {
        return [
          parseInt(expand(digits[0] ?? '0'), 16),
          parseInt(expand(digits[1] ?? '0'), 16),
          parseInt(expand(digits[2] ?? '0'), 16),
        ]
      }
      return [
        parseInt(digits.slice(0, 2), 16),
        parseInt(digits.slice(2, 4), 16),
        parseInt(digits.slice(4, 6), 16),
      ]
    }

    const rgb = /^rgba?\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)\s*(?:,\s*[\d.]+\s*)?\)$/i.exec(value)
    if (rgb?.[1] !== undefined && rgb[2] !== undefined && rgb[3] !== undefined) {
      return [Number(rgb[1]), Number(rgb[2]), Number(rgb[3])]
    }

    const hsl = /^hsla?\(\s*([\d.]+)\s*,\s*([\d.]+)%\s*,\s*([\d.]+)%\s*(?:,\s*[\d.]+\s*)?\)$/i.exec(value)
    if (hsl?.[1] !== undefined && hsl[2] !== undefined && hsl[3] !== undefined) {
      return hslToRgb(Number(hsl[1]), Number(hsl[2]), Number(hsl[3]))
    }

    throw new Error(`не удалось разобрать цвет для расчёта контраста: "${raw}"`)
  }

  function hslToRgb(h: number, s: number, l: number): readonly [number, number, number] {
    const sNorm = s / 100
    const lNorm = l / 100
    const c = (1 - Math.abs(2 * lNorm - 1)) * sNorm
    const hPrime = ((h % 360) + 360) % 360 / 60
    const x = c * (1 - Math.abs((hPrime % 2) - 1))
    const m = lNorm - c / 2
    let [r1, g1, b1] = [0, 0, 0]
    if (hPrime < 1) [r1, g1, b1] = [c, x, 0]
    else if (hPrime < 2) [r1, g1, b1] = [x, c, 0]
    else if (hPrime < 3) [r1, g1, b1] = [0, c, x]
    else if (hPrime < 4) [r1, g1, b1] = [0, x, c]
    else if (hPrime < 5) [r1, g1, b1] = [x, 0, c]
    else [r1, g1, b1] = [c, 0, x]
    return [Math.round((r1 + m) * 255), Math.round((g1 + m) * 255), Math.round((b1 + m) * 255)]
  }

  function isInsideDarkMedia(rule: { parent?: unknown }): boolean {
    const parent = rule.parent as AtRule | undefined
    return parent !== undefined && parent.type === 'atrule' && parent.name === 'media'
      && /prefers-color-scheme:\s*dark/.test(parent.params)
  }

  function extractRootDeclValue(cssRoot: PostcssRoot, name: string, dark: boolean): string | null {
    let found: string | null = null
    cssRoot.walkRules(':root', (rule) => {
      if (isInsideDarkMedia(rule) !== dark) return
      rule.walkDecls(`--${name}`, (decl) => {
        found = decl.value.trim()
      })
    })
    return found
  }

  /** Тёмное значение, если токен переопределён в тёмном блоке, иначе светлое (обычный каскад `:root`). */
  function resolveVarForContrast(cssRoot: PostcssRoot, name: string): readonly [number, number, number] {
    const darkValue = extractRootDeclValue(cssRoot, name, true)
    const lightValue = extractRootDeclValue(cssRoot, name, false)
    const raw = darkValue ?? lightValue
    if (raw === null) throw new Error(`--${name} не найден в src/style.css`)
    return parseCssColorToRgb(raw)
  }

  it('обводка фокуса (--color-accent) держит ≥3:1 на фоне баннера (--color-accent-soft)', () => {
    // Ревьюер измерил 2.91:1 у прежнего значения `--color-accent-soft`
    // (`#17324d`) — ниже порога WCAG 1.4.11 (3:1 для нетекстовых
    // UI-элементов вроде кольца фокуса на фоне баннера
    // `QueueSection.vue`). `--color-accent` не переопределён в тёмной
    // теме (см. doc `style.css`) — код ниже честно берёт светлое значение
    // через обычный каскад, а не хардкодит его.
    const cssRoot = postcss.parse(readFileSync(STYLE_CSS_PATH, 'utf-8'))
    const accent = resolveVarForContrast(cssRoot, 'color-accent')
    const banner = resolveVarForContrast(cssRoot, 'color-accent-soft')

    expect(contrastRatio(accent, banner)).toBeGreaterThanOrEqual(3)
  })

  it('П-4: падает, если токен есть в тёмном блоке, но в неразобранном формате — не подставляет светлое', () => {
    const cssRoot = postcss.parse(`
      :root { --x: #ffffff; }
      @media (prefers-color-scheme: dark) {
        :root { --x: hwb(0 0% 0%); }
      }
    `)
    expect(() => resolveVarForContrast(cssRoot, 'x')).toThrow()
  })

  it('разбирает rgb()/hsl()/8-значный hex в тёмном блоке (а не только 6-значный hex)', () => {
    const cssRoot = postcss.parse(`
      :root { --a: #ffffff; --b: #ffffff; --c: #ffffff; }
      @media (prefers-color-scheme: dark) {
        :root {
          --a: rgb(18, 18, 18);
          --b: hsl(0, 0%, 7%);
          --c: #121212ff;
        }
      }
    `)
    expect(resolveVarForContrast(cssRoot, 'a')).toEqual([18, 18, 18])
    expect(resolveVarForContrast(cssRoot, 'b')).toEqual([18, 18, 18])
    expect(resolveVarForContrast(cssRoot, 'c')).toEqual([18, 18, 18])
  })

  it('если токен не переопределён в тёмном блоке, честно берёт светлое значение', () => {
    const cssRoot = postcss.parse(`
      :root { --only-light: #1a73e8; }
      @media (prefers-color-scheme: dark) {
        :root { --other: #000000; }
      }
    `)
    expect(resolveVarForContrast(cssRoot, 'only-light')).toEqual([0x1a, 0x73, 0xe8])
  })
})
