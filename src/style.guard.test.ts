import { readFileSync, readdirSync } from 'node:fs'
import { dirname, join, relative, sep } from 'node:path'
import { fileURLToPath } from 'node:url'

import { describe, expect, it } from 'vitest'

/**
 * Сторож палитры (TL-22, issue execaus/tube-leak#23): вся палитра проекта
 * живёт в `src/style.css` как CSS-переменные (см. doc-комментарий того
 * файла). Новые компоненты не должны заводить собственный литерал цвета —
 * этот тест проходит по всем `.vue`/`.css`/`.ts` в `src/` и падает на
 * первом литерале (hex, `rgb()`/`rgba()`, `hsl()`/`hsla()`, именованный
 * цвет CSS) вне белого списка.
 *
 * # Белый список, не чёрный (обязательное требование задачи)
 *
 * Разрешён ровно один файл — `src/style.css`. Никакой список «плохих»
 * файлов не поддерживается: расширять список литералов новым компонентом
 * может кто угодно, а забыть добавить новый файл в чёрный список — типовая
 * ошибка сторожа этого проекта (см. историю в CLAUDE.md — уже было).
 *
 * # Почему по исходному тексту, а не по вычисленному стилю
 *
 * jsdom не применяет `scoped`-стили SFC (тот же приём и то же ограничение,
 * что в сторожах `YtDlpUpdateBlock.test.ts` и `DownloadPanel`/`SidecarStatusRow`
 * и т.п.) — единственный способ проверить исходники без сборки Vite.
 *
 * # Комментарии не считаются нарушением
 *
 * Историю решений (например, «явное значение `#555` — issue #61») проект
 * документирует прямо в коде, доказательно указывая литерал, который был
 * выбран. Это не живой стиль, а объяснение уже токенизированного значения
 * — блочные (`/* ... *\/`) и HTML/template (`<!-- ... -->`) комментарии
 * вырезаются перед поиском литералов, чтобы такая документация не
 * считалась нарушением.
 *
 * # Обход подкаталогов — урок проекта (см. CLAUDE.md)
 *
 * Сторож уже один раз был ослеплён собственной выборкой файлов в этом
 * проекте. Обход здесь рекурсивный вручную (`fs.readdirSync` с ручным
 * рекурсивным спуском, без опции `recursive`, чтобы не зависеть от версии
 * Node) и явно тестируется на подкаталоге `src/components/`.
 */

const SRC_DIR = join(dirname(fileURLToPath(import.meta.url)))
const SELF_PATH = fileURLToPath(import.meta.url)
// Белый список задачи — единственный файл палитры, а не список «плохих»
// файлов (см. doc-комментарий выше). Собственный исходник сторожа сюда не
// входит: он неизбежно содержит образцы цветовых литералов как текст
// регулярного выражения и как строки документации (например, «color:
// red;» строкой ниже) — это код проверки, а не палитра компонента, и
// исключён из сканирования отдельно, через `SELF_PATH`.
const ALLOWED_RELATIVE_PATHS = new Set(['style.css'])
const SCANNED_EXTENSIONS = ['.vue', '.css', '.ts']

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

const COLOR_LITERAL_PATTERN = new RegExp(
  [
    '#[0-9a-fA-F]{3,8}\\b', // hex
    'rgba?\\([^)]*\\)', // rgb()/rgba()
    'hsla?\\([^)]*\\)', // hsl()/hsla()
    // именованный цвет — только как значение CSS-декларации ("color: red;"),
    // а не любое вхождение слова в тексте.
    `:\\s*(?:${NAMED_COLORS.join('|')})\\s*(?:;|\\)|\\s|$)`,
  ].join('|'),
  'gi',
)

function stripComments(source: string): string {
  return source
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/<!--[\s\S]*?-->/g, '')
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
      const withoutComments = stripComments(source)
      const match = withoutComments.match(COLOR_LITERAL_PATTERN)

      expect(
        match,
        `найден литерал цвета "${String(match?.[0])}" — используй переменную из src/style.css`,
      ).toBeNull()
    },
  )
})
