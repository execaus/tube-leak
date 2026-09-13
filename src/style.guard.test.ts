import { readFileSync, readdirSync } from 'node:fs'
import { dirname, join, relative, sep } from 'node:path'
import { fileURLToPath } from 'node:url'

import { describe, expect, it } from 'vitest'

/**
 * Сторож палитры (TL-22, issue execaus/tube-leak#23). Вся палитра проекта
 * живёт в `src/style.css` как CSS-переменные (см. doc-комментарий того
 * файла). Новые компоненты не должны заводить собственный литерал цвета.
 *
 * # Правки ревью (белый список по значениям, не по синтаксису)
 *
 * Первая версия сторожа искала цветовые литералы одним общим регэкспом по
 * всему тексту файла — черновой синтаксический чёрный список, который либо
 * пропускал формы (`style="color: red"`, `border: 1px solid red`,
 * `<path fill="red">`, `oklch(...)`, `color-mix(...)`), либо ложно
 * срабатывал на прозу (issue-номер `#100` внутри `//`-комментария после
 * сломанного вырезания комментариев). Ревью потребовало другой принцип:
 * **у свойств/ключей, несущих цвет, разрешено только ограниченное множество
 * значений** (`var(--color-*)`, `transparent`, `currentColor`, `inherit`,
 * `initial`, `unset`, `none`; числа и ключевые слова стиля вроде `solid` в
 * `border`/`box-shadow`/`outline` не запрещены явно — они и так не похожи
 * ни на один цветовой токен). Реализация ниже:
 *
 * 1. **Контекстно-свободный бэкстоп** — вызов цветовой CSS-функции
 *    (`rgb()`/`rgba()`/`hsl()`/`hsla()`/`hwb()`/`lab()`/`lch()`/`oklab()`/
 *    `oklch()`/`color()`/`color-mix()`) запрещён где угодно в файле: вызов
 *    функции — это не то, что случайно появляется в постороннем тексте
 *    (`parseRgb(` не совпадает — до `Rgb(` нет границы слова после `e`),
 *    а через него годно провести даже то, что не всегда лежит в
 *    обычной CSS-декларации.
 * 2. **Контекстно-зависимая проверка** — hex-литерал или голое имя
 *    цвета (`red`, `black`, `white`, …) нарушение **только** в значении
 *    свойства/ключа, который реально красит: CSS-декларации
 *    (`color`/`background*`/`border*`/`outline*`/`fill`/`stroke`/
 *    `box-shadow`/`text-decoration*`/`caret-color`/`accent-color`/
 *    `column-rule*`), инлайн `style="..."`, объекты `:style`/style-литералы
 *    в `.ts` (camelCase-ключи вроде `backgroundColor`) и SVG-атрибуты
 *    `fill`/`stroke`. Вне такого контекста `'#add'`, `b ? tan : red`,
 *    `function parseRgb(` не значат ничего цветового и не краснеют —
 *    «по построению», а не отдельным списком исключений (П-2).
 *
 * Перед любым поиском внутри `var(...)` **простая** ссылка без fallback
 * (`var(--color-accent)`) вырезается из значения — иначе имя токена вроде
 * гипотетического `--color-teal` само содержало бы слово «teal» и ловило
 * бы себя. Ссылка с fallback (`var(--x, red)`) не вырезается: там `red`
 * может быть настоящим зашитым литералом и обязан продолжать ловиться.
 *
 * # Вырезание комментариев — с учётом строк (П-1)
 *
 * Первая версия резала комментарии одним регэкспом `/\/\*[\s\S]*?\*\//`,
 * который не отличает `/*`/`//`/`<!--` внутри строкового литерала от
 * настоящего начала комментария: строка `'src/*'` содержит подстроку `/*`,
 * и жадный (точнее, ленивый, но всё равно слепой к строкам) поиск съедал
 * весь код до ближайшего настоящего `*\/` дальше по файлу — включая
 * реальные нарушения между ними. Здесь вместо регэкспа — посимвольный
 * разбор с состоянием «внутри строки» (`'`/`"`/` \` `, с учётом `\`-
 * экранирования): находясь внутри строки, `/*`, `//` и `<!--` не считаются
 * началом комментария.
 *
 * # Белый список файлов, не чёрный
 *
 * Разрешён ровно один файл — `src/style.css`. Собственный исходник этого
 * теста в сканирование не входит отдельно (`SELF_PATH`) — он неизбежно
 * содержит образцы синтаксиса цвета как текст регулярных выражений и как
 * документацию, это код проверки, а не палитра компонента.
 *
 * # Почему по исходному тексту, а не по вычисленному стилю
 *
 * jsdom не применяет `scoped`-стили SFC (тот же приём и то же ограничение,
 * что в сторожах `YtDlpUpdateBlock.test.ts` и `DownloadPanel`/
 * `SidecarStatusRow`) — единственный способ проверить исходники без сборки
 * Vite.
 *
 * # Обход подкаталогов — урок проекта (см. CLAUDE.md)
 *
 * Сторож уже один раз был ослеплён собственной выборкой файлов в этом
 * проекте. Обход здесь рекурсивный вручную (`fs.readdirSync` без опции
 * `recursive`, чтобы не зависеть от версии Node) и явно тестируется на
 * подкаталоге `src/components/`.
 */

const SRC_DIR = join(dirname(fileURLToPath(import.meta.url)))
const SELF_PATH = fileURLToPath(import.meta.url)
const STYLE_CSS_PATH = join(SRC_DIR, 'style.css')
const INDEX_HTML_PATH = join(SRC_DIR, '..', 'index.html')

// Белый список задачи — единственный файл палитры, а не список «плохих»
// файлов. Собственный исходник сторожа исключён отдельно, через
// `SELF_PATH` (см. doc-комментарий выше), концептуально это другая вещь:
// не «где разрешена палитра», а «не проверяй проверяющего на себе».
const ALLOWED_RELATIVE_PATHS = new Set(['style.css'])
const SCANNED_EXTENSIONS = ['.vue', '.css', '.ts', '.js', '.html', '.svg'] // М-1

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
  // намеренно нет 'transparent'/'currentcolor': это разрешённые значения,
  // а не литералы цвета.
]

// Функции CSS, которые всегда возвращают литеральный цвет — вызов такой
// функции сам по себе нарушение независимо от контекста (Б-1).
const COLOR_FUNCTION_NAMES = [
  'rgba', 'rgb', 'hsla', 'hsl', 'hwb', 'lab', 'lch', 'oklab', 'oklch', 'color-mix', 'color',
]

const HEX_RE = /#[0-9a-fA-F]{3,8}\b/g
const FUNCTION_CALL_RE = new RegExp(`\\b(?:${COLOR_FUNCTION_NAMES.join('|')})\\(`, 'gi')
const NAMED_COLOR_RE = new RegExp(`\\b(?:${NAMED_COLORS.join('|')})\\b`, 'gi')
const SIMPLE_VAR_RE = /var\(--[a-zA-Z0-9-]+\)/gi // без fallback — не трогаем var(--x, red)

// kebab-case CSS-свойства и camelCase ключи style-объектов, которые могут
// нести цвет (задание Б-1, дословный список). Проверяются префиксом
// (`background*`, `border*`, …), а не точным совпадением: длинные формы
// (`border-top-color`, `borderTopColor`) обязаны попадать под то же
// правило, что и короткие.
const KEBAB_COLOR_PROPERTY_PREFIXES = [
  'color', 'background', 'border', 'outline', 'fill', 'stroke',
  'box-shadow', 'text-decoration', 'caret-color', 'accent-color', 'column-rule',
]
const CAMEL_COLOR_PROPERTY_PREFIXES = [
  'color', 'background', 'border', 'outline', 'fill', 'stroke',
  'boxShadow', 'textDecoration', 'caretColor', 'accentColor', 'columnRule',
]

function isKebabColorProperty(name: string): boolean {
  const lower = name.toLowerCase()
  return KEBAB_COLOR_PROPERTY_PREFIXES.some((p) => lower === p || lower.startsWith(`${p}-`))
}

function isCamelColorProperty(name: string): boolean {
  return CAMEL_COLOR_PROPERTY_PREFIXES.some(
    (p) => name === p || (name.startsWith(p) && /^[A-Z]/.test(name.slice(p.length))),
  )
}

/**
 * Посимвольное вырезание комментариев (П-1) — в отличие от регэкспа, не
 * путает `/*`/`//`/`<!--` внутри строкового литерала с настоящим началом
 * комментария: пока разбор находится «внутри строки» (между открывающей и
 * закрывающей `'`/`"`/`` ` ``, с учётом `\`-экранирования), эти
 * последовательности не интерпретируются как комментарий.
 */
function stripComments(source: string): string {
  let result = ''
  let i = 0
  const n = source.length
  let stringChar: string | null = null

  while (i < n) {
    const c = source[i]

    if (stringChar) {
      result += c
      if (c === '\\' && i + 1 < n) {
        result += source[i + 1]
        i += 2
        continue
      }
      if (c === stringChar) stringChar = null
      i += 1
      continue
    }

    if (c === '"' || c === "'" || c === '`') {
      stringChar = c
      result += c
      i += 1
      continue
    }

    if (c === '/' && source[i + 1] === '/') {
      while (i < n && source[i] !== '\n') i += 1
      continue
    }

    if (c === '/' && source[i + 1] === '*') {
      i += 2
      while (i < n && !(source[i] === '*' && source[i + 1] === '/')) i += 1
      i = Math.min(i + 2, n)
      continue
    }

    if (source.startsWith('<!--', i)) {
      const end = source.indexOf('-->', i + 4)
      i = end === -1 ? n : end + 3
      continue
    }

    result += c
    i += 1
  }

  return result
}

/** Убирает из значения только простые ссылки `var(--x)` без fallback. */
function stripSimpleVarRefs(value: string): string {
  return value.replace(SIMPLE_VAR_RE, '')
}

/** Ищет hex/голое имя цвета в уже вычлененном значении цветового свойства. */
function findLiteralColorsInValue(value: string): string[] {
  const cleaned = stripSimpleVarRefs(value)
  const found: string[] = []
  for (const m of cleaned.matchAll(HEX_RE)) found.push(m[0])
  for (const m of cleaned.matchAll(NAMED_COLOR_RE)) found.push(m[0])
  return found
}

/** Контекстно-свободный бэкстоп (Б-1): вызов цветовой функции — везде нарушение. */
function findColorFunctionCalls(source: string): string[] {
  return [...source.matchAll(FUNCTION_CALL_RE)].map((m) => {
    const start = m.index ?? 0
    return source.slice(start, start + 32).split('\n')[0] ?? m[0]
  })
}

/** CSS-декларации: `<style>` блоков SFC, `.css`-файлов, инлайн `style="..."`. */
function scanCssDeclarations(text: string): string[] {
  const violations: string[] = []
  const declarationRe = /([a-zA-Z-]+)\s*:\s*([^;{}]+);/g
  for (const m of text.matchAll(declarationRe)) {
    const prop = m[1]
    const value = m[2]
    if (!prop || !value || !isKebabColorProperty(prop)) continue
    for (const literal of findLiteralColorsInValue(value)) {
      violations.push(`${prop}: ${value.trim()} (литерал «${literal}»)`)
    }
  }
  return violations
}

function scanInlineStyleAttributes(text: string): string[] {
  const violations: string[] = []
  const attrRe = /\bstyle\s*=\s*(?:"([^"]*)"|'([^']*)')/g
  for (const m of text.matchAll(attrRe)) {
    const value = m[1] ?? m[2] ?? ''
    // Дописываем `;`, чтобы переиспользовать разбор CSS-деклараций для
    // статического `style="color: red"` без завершающей точки с запятой.
    violations.push(...scanCssDeclarations(`${value};`))
  }
  return violations
}

/**
 * `:style="{ color: 'red' }"` в шаблоне и style-литералы в `.ts`/`.js`
 * (camelCase- и kebab-ключи в кавычках — оба варианта валидны как ключ
 * JS-объекта). Значение обязано быть строкой в кавычках: `b ? tan : red`
 * не совпадает (после `:` нет кавычки) — этим и отличается от настоящего
 * литерала, ложных срабатываний не даёт «по построению» (П-2).
 */
function scanColorObjectLiterals(text: string): string[] {
  const violations: string[] = []
  const pairRe = /(['"]?)([a-zA-Z_$][a-zA-Z0-9_$-]*)\1\s*:\s*(['"`])((?:(?!\3)[\s\S])*?)\3/g
  for (const m of text.matchAll(pairRe)) {
    const key = m[2]
    const value = m[4]
    if (key === undefined || value === undefined) continue
    if (!isCamelColorProperty(key) && !isKebabColorProperty(key)) continue
    for (const literal of findLiteralColorsInValue(value)) {
      violations.push(`${key}: '${value}' (литерал «${literal}»)`)
    }
  }
  return violations
}

/** SVG `fill="..."`/`stroke="..."`. */
function scanSvgColorAttributes(text: string): string[] {
  const violations: string[] = []
  const attrRe = /\b(fill|stroke)\s*=\s*(?:"([^"]*)"|'([^']*)')/g
  for (const m of text.matchAll(attrRe)) {
    const attr = m[1]
    const value = m[2] ?? m[3] ?? ''
    if (!attr) continue
    for (const literal of findLiteralColorsInValue(value)) {
      violations.push(`${attr}="${value}" (литерал «${literal}»)`)
    }
  }
  return violations
}

/** Полный разбор одного файла: бэкстоп + все контекстно-зависимые проверки. */
function collectViolations(source: string): string[] {
  const withoutComments = stripComments(source)
  return [
    ...findColorFunctionCalls(withoutComments),
    ...scanCssDeclarations(withoutComments),
    ...scanInlineStyleAttributes(withoutComments),
    ...scanColorObjectLiterals(withoutComments),
    ...scanSvgColorAttributes(withoutComments),
  ]
}

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
    // корнем `src/` и не заметить, что рекурсия сломана, — тест выше упал
    // бы «зелёным» просто потому, что ничего не проверил.
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

      const source = readFileSync(absolutePath, 'utf-8')
      const violations = collectViolations(source)

      expect(
        violations,
        `найдены литералы цвета — используй переменную из src/style.css:\n${violations.join('\n')}`,
      ).toEqual([])
    },
  )
})

describe('сторож — детектор ловит формы из ревью (Б-1), мутации на строках-фикстурах', () => {
  it.each([
    ['статический style, свойство color', 'style="color: red"'],
    [':style-объект в шаблоне', ':style="{ color: \'red\' }"'],
    ['style-литерал в .ts', "const s = { color: 'red' }"],
    ['голый цвет в border', 'border: 1px solid red;'],
    ['голый цвет в box-shadow', 'box-shadow: 0 0 4px black;'],
    ['голый цвет в outline', 'outline: 2px solid white;'],
    ['SVG fill/stroke', '<path fill="red" stroke="black">'],
    ['color-mix()', 'background: color-mix(in srgb, var(--color-bg), black 20%);'],
    ['oklch()', 'color: oklch(62% 0.2 29);'],
    ['color(display-p3 ...)', 'color: color(display-p3 1 0 0);'],
  ])('краснеет: %s', (_label, fixture) => {
    expect(collectViolations(fixture)).not.toEqual([])
  })

  it('var(--x) с fallback не прячет литерал в fallback-значении', () => {
    expect(collectViolations('color: var(--missing-token, red);')).not.toEqual([])
  })
})

describe('сторож — не путает строки с комментариями (П-1)', () => {
  it('литерал между обманчивым `/*` внутри строки и настоящим комментарием не пропадает', () => {
    // Старый регэксп `/\/\*[\s\S]*?\*\//` видит в `'src/*'` начало
    // комментария (подстрока `/*` внутри строки) и режет всё до ближайшего
    // настоящего `*/` — в том числе строку с `color: '#ff0000'` между
    // ними. Значение оформлено как ключ style-объекта (`color`), чтобы
    // проверка была содержательной и после Б-1 (голый hex вне контекста
    // цветового свойства сам по себе не нарушение, см. П-2).
    const fixture = [
      "const path = 'src/*'",
      "const style = { color: '#ff0000' }",
      '/** doc */',
    ].join('\n')

    expect(collectViolations(fixture)).not.toEqual([])
  })
})

describe('сторож — белый список по значениям не даёт ложных срабатываний (П-2)', () => {
  it.each([
    ['issue-номер в комментарии', '// see issue #100'],
    ['строка вне цветового ключа', "const id = '#add'"],
    ['тернарник с именами вне ключа', 'const x = b ? tan : red'],
    ['имя функции содержит "rgb"', 'function parseRgb(input: string) {}'],
    ['граница слова: border-radius — не border-color', 'border-radius: 4px;'],
    ['граница слова: outline-offset — не литерал', 'outline-offset: 2px;'],
    ['допустимые ключевые слова значения', 'color: var(--color-accent);'],
    ['допустимое служебное значение', 'background: transparent;'],
  ])('не краснеет: %s', (_label, fixture) => {
    expect(collectViolations(fixture)).toEqual([])
  })
})

describe('index.html — фон синхронизирован с --color-bg (П-3)', () => {
  function extractRootVar(styleCssSource: string, dark: boolean, name: string): string | null {
    const [lightBlock, darkBlock] = styleCssSource.split('@media (prefers-color-scheme: dark)')
    const block = dark ? darkBlock : lightBlock
    if (!block) return null
    const m = block.match(new RegExp(`--${name}\\s*:\\s*([^;]+);`))
    return m?.[1]?.trim().toLowerCase() ?? null
  }

  function extractHtmlBackground(indexHtmlSource: string, dark: boolean): string | null {
    const [lightBlock, darkBlock] = indexHtmlSource.split('@media (prefers-color-scheme: dark)')
    const block = dark ? darkBlock : lightBlock
    if (!block) return null
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

describe('контраст кольца фокуса на баннере очереди в тёмной теме (П-4)', () => {
  // WCAG 2.1 relative luminance / contrast ratio — та же формула, что и в
  // doc-комментарии `src/style.css` (акцент проверен тем же способом).
  function relativeLuminance(hex: string): number {
    const clean = hex.replace('#', '')
    const [r, g, b] = [0, 2, 4].map((offset) => parseInt(clean.slice(offset, offset + 2), 16) / 255)
    const channel = (c: number) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4)
    return 0.2126 * channel(r ?? 0) + 0.7152 * channel(g ?? 0) + 0.0722 * channel(b ?? 0)
  }

  function contrastRatio(hexA: string, hexB: string): number {
    const lA = relativeLuminance(hexA)
    const lB = relativeLuminance(hexB)
    const lighter = Math.max(lA, lB)
    const darker = Math.min(lA, lB)
    return (lighter + 0.05) / (darker + 0.05)
  }

  function extractDarkVar(styleCssSource: string, name: string): string {
    const [lightBlock, darkBlock] = styleCssSource.split('@media (prefers-color-scheme: dark)')
    const re = new RegExp(`--${name}\\s*:\\s*(#[0-9a-fA-F]{6})\\s*;`)
    // Токен, не переопределённый в тёмном блоке (например, `--color-accent`
    // — измеренное решение не переопределять его, см. doc `style.css`),
    // в тёмной теме продолжает действовать светлое значение по обычному
    // каскаду `:root` — поэтому ищем сначала в тёмном блоке, а если там
    // нет переопределения, честно берём светлое.
    const value = darkBlock?.match(re)?.[1] ?? lightBlock?.match(re)?.[1]
    if (!value) throw new Error(`--${name} не найден в src/style.css`)
    return value
  }

  it('обводка фокуса (--color-accent) держит ≥3:1 на фоне баннера (--color-accent-soft)', () => {
    // Ревьюер измерил 2.91:1 у прежнего значения `--color-accent-soft`
    // (`#17324d`) — ниже порога WCAG 1.4.11 (3:1 для нетекстовых
    // UI-элементов вроде кольца фокуса кнопки в QueueSection.vue:107 на
    // фоне баннера QueueSection.vue:192). Токен банера в тёмной теме
    // затемнён до `#0f2338`, что и проверяет этот тест — вместо ручного
    // пересчёта при каждой правке палитры.
    const styleCss = readFileSync(STYLE_CSS_PATH, 'utf-8')
    const accentDark = extractDarkVar(styleCss, 'color-accent') // не переопределён в тёмной теме
    const accentSoftDark = extractDarkVar(styleCss, 'color-accent-soft')

    const ratio = contrastRatio(accentDark, accentSoftDark)

    expect(ratio).toBeGreaterThanOrEqual(3)
  })
})
