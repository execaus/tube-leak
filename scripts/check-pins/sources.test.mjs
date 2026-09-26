import { describe, expect, it } from 'vitest'

import { loadPin } from '../fetch-binaries/pin.mjs'
import { KNOWN_TARGETS } from '../fetch-binaries/targets.mjs'
import { checkUrl, parseArgs, partitionBySkip } from './index.mjs'
import {
  collectAllUrls,
  collectPinUrls,
  crossCheckDocs,
  DOC_FILES,
  DOCUMENTED_SECTION,
  extractMarkdownUrls,
  ourTextOf,
  PIN_PATH,
  readDocs,
  skipReason,
} from './sources.mjs'

// Сети здесь нет и быть не может (правило проекта): всё, что ходит
// наружу, живёт в `npm run check-pins`. В тестах — только разбор,
// сведение списков и офлайн-сверка пина с документами §6d.

describe('extractMarkdownUrls', () => {
  it('ловит голые ссылки, ссылки в скобках и в обратных кавычках', () => {
    const text = [
      'Голая https://example.invalid/a рядом с текстом.',
      '[подпись](https://example.invalid/b) — ссылка markdown.',
      'В таблице: `https://example.invalid/c`',
      '| Linux | BtbN | https://example.invalid/d |',
    ].join('\n')

    expect(extractMarkdownUrls(text, 'DOC.md').map((entry) => entry.url)).toStrictEqual([
      'https://example.invalid/a',
      'https://example.invalid/b',
      'https://example.invalid/c',
      'https://example.invalid/d',
    ])
  })

  it('отрезает хвостовую пунктуацию, но не части адреса', () => {
    const text = 'См. https://example.invalid/x-1.tar.xz, а также https://example.invalid/y?v=1.'

    expect(extractMarkdownUrls(text, 'DOC.md').map((entry) => entry.url)).toStrictEqual([
      'https://example.invalid/x-1.tar.xz',
      'https://example.invalid/y?v=1',
    ])
  })

  it('не выдаёт имя файла за ссылку — ровно это и прятало дефект #140', () => {
    const text = '| Windows | gyan.dev | `packages/ffmpeg-9.0.1-essentials_build.zip` |'

    expect(extractMarkdownUrls(text, 'DOC.md')).toStrictEqual([])
  })
})

describe('ourTextOf', () => {
  it('оставляет SOURCES-FFMPEG.md целиком — файл наш от начала до конца', () => {
    const text = '# Заголовок\n## Полный текст чего-то\nhttps://example.invalid/a'

    expect(ourTextOf('SOURCES-FFMPEG.md', text)).toBe(text)
  })

  it('обрезает THIRD-PARTY-LICENSES.md на первом дословном тексте лицензии', () => {
    const text = [
      '# Лицензии',
      'Наш адрес: https://example.invalid/ours',
      '### Полный текст GNU General Public License v3',
      'Чужой адрес из текста лицензии: https://example.invalid/theirs',
    ].join('\n')

    const ours = ourTextOf('THIRD-PARTY-LICENSES.md', text)

    expect(ours).toContain('https://example.invalid/ours')
    expect(ours).not.toContain('https://example.invalid/theirs')
  })

  it('без маркера границы отказывает, а не проверяет чужой текст молча', () => {
    expect(() => ourTextOf('THIRD-PARTY-LICENSES.md', '# Лицензии\nбез маркера')).toThrow(
      /граница нашего текста/,
    )
  })

  it('на настоящем файле граница проходит до текстов лицензий', async () => {
    const docs = await readDocs()
    const ours = ourTextOf('THIRD-PARTY-LICENSES.md', docs['THIRD-PARTY-LICENSES.md'])

    expect(ours.length).toBeLessThan(docs['THIRD-PARTY-LICENSES.md'].length)
    // Адреса сборок ffmpeg — наши обещания и обязаны остаться внутри границы.
    const pin = await loadPin(PIN_PATH)
    for (const target of KNOWN_TARGETS) {
      expect(ours).toContain(pin[DOCUMENTED_SECTION].targets[target].url)
    }
  })
})

describe('skipReason', () => {
  it('пропускает VCS-эндпоинты: они отвечают на clone, а не на HTTP', () => {
    for (const url of [
      'https://bitbucket.org/multicoreware/x265_git.git',
      'https://git.code.sf.net/p/soxr/code',
      'https://svn.xvid.org/trunk/xvidcore',
      'https://git.savannah.gnu.org/git/libiconv.git',
    ]) {
      expect(skipReason(url)).toMatch(/VCS-эндпоинт/)
    }
  })

  it('пропускает наш приватный репозиторий с названной причиной', () => {
    expect(skipReason('https://github.com/execaus/tube-leak/issues/14')).toMatch(/приватный/)
  })

  it('не пропускает обычные адреса — в том числе релизные ассеты GitHub', () => {
    for (const url of [
      'https://github.com/GyanD/codexffmpeg/releases/download/9.0.1/ffmpeg-9.0.1-essentials_build.zip',
      'https://ffmpeg.org/releases/ffmpeg-9.0.1.tar.xz',
      'https://github.com/BtbN/FFmpeg-Builds',
    ]) {
      expect(skipReason(url)).toBeNull()
    }
  })
})

describe('collectPinUrls', () => {
  it('берёт по адресу на каждую пару (sidecar, тройка)', async () => {
    const pin = await loadPin(PIN_PATH)
    const urls = collectPinUrls(pin)

    expect(urls).toHaveLength(3 * KNOWN_TARGETS.length)
    for (const entry of urls) {
      expect(entry.url).toMatch(/^https:\/\//)
    }
  })
})

describe('collectAllUrls', () => {
  it('сводит пин и документы без повторов, сохраняя все места адреса', async () => {
    const all = await collectAllUrls()
    const urls = all.map((entry) => entry.url)

    expect(new Set(urls).size).toBe(urls.length)

    const pin = await loadPin(PIN_PATH)
    const linux = pin[DOCUMENTED_SECTION].targets['x86_64-unknown-linux-gnu'].url
    const found = all.find((entry) => entry.url === linux)
    // Адрес сборки Linux обязан быть назван и пином, и обоими документами:
    // это и есть свидетельство, что дыра #140 закрыта с обеих сторон.
    expect(found?.where).toStrictEqual([
      'binaries.lock.json ffmpeg.x86_64-unknown-linux-gnu',
      ...DOC_FILES,
    ])
  })

  it('не тащит адреса из дословных текстов чужих лицензий', async () => {
    const all = await collectAllUrls()

    // Этот адрес есть в THIRD-PARTY-LICENSES.md, но только внутри
    // скопированного уведомления ICU — он не наше обещание и мёртв не по
    // нашей вине. Сторож, красневший от него, читать перестали бы.
    expect(all.some((entry) => entry.url.includes('chasen.aist-nara.ac.jp'))).toBe(false)
  })
})

describe('crossCheckDocs', () => {
  it('молчит, когда документы называют все четыре сборки адресами', async () => {
    const pin = await loadPin(PIN_PATH)

    expect(crossCheckDocs(pin, await readDocs())).toStrictEqual([])
  })

  it('краснеет, если сборку записали именем файла вместо адреса', async () => {
    const pin = await loadPin(PIN_PATH)
    const docs = await readDocs()
    const url = pin[DOCUMENTED_SECTION].targets['x86_64-pc-windows-msvc'].url
    // Ровно та подмена, что была в файле до TL-133: URL → имя файла.
    docs['SOURCES-FFMPEG.md'] = docs['SOURCES-FFMPEG.md'].replaceAll(
      url,
      'packages/ffmpeg-9.0.1-essentials_build.zip',
    )

    const problems = crossCheckDocs(pin, docs)

    expect(problems).toHaveLength(1)
    expect(problems[0]).toContain('SOURCES-FFMPEG.md')
    expect(problems[0]).toContain('x86_64-pc-windows-msvc')
  })

  it('краснеет за каждый документ, потерявший адрес, и называет тройку', async () => {
    const pin = await loadPin(PIN_PATH)
    const docs = await readDocs()
    const url = pin[DOCUMENTED_SECTION].targets['aarch64-apple-darwin'].url
    for (const name of DOC_FILES) docs[name] = docs[name].replaceAll(url, 'см. пин')

    const problems = crossCheckDocs(pin, docs)

    expect(problems).toHaveLength(DOC_FILES.length)
    for (const name of DOC_FILES) {
      expect(problems.some((problem) => problem.startsWith(`${name}:`))).toBe(true)
    }
  })

  it('перепиновка без правки документов — расхождение, а не тишина', async () => {
    const pin = await loadPin(PIN_PATH)
    const docs = await readDocs()
    pin[DOCUMENTED_SECTION].targets['x86_64-unknown-linux-gnu'].url =
      'https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2027-01-31-00-00/ffmpeg.tar.xz'

    expect(crossCheckDocs(pin, docs)).toHaveLength(DOC_FILES.length)
  })
})

describe('partitionBySkip', () => {
  it('делит адреса на проверяемые и пропускаемые с причиной', () => {
    const { checked, skipped } = partitionBySkip([
      { url: 'https://ffmpeg.org/releases/ffmpeg-9.0.1.tar.xz', where: ['DOC.md'] },
      { url: 'https://git.code.sf.net/p/soxr/code', where: ['DOC.md'] },
    ])

    expect(checked.map((entry) => entry.url)).toStrictEqual([
      'https://ffmpeg.org/releases/ffmpeg-9.0.1.tar.xz',
    ])
    expect(skipped[0].reason).toMatch(/VCS-эндпоинт/)
  })
})

describe('checkUrl', () => {
  const response = (status, headers = {}) => ({
    ok: status >= 200 && status < 300,
    status,
    headers: { get: (name) => headers[name.toLowerCase()] ?? null },
  })

  it('считает живым ответ 200 и называет размер', async () => {
    const fetchImpl = async () => response(200, { 'content-length': '111253802' })

    await expect(checkUrl('https://example.invalid/a', { fetchImpl })).resolves.toStrictEqual({
      kind: 'ok',
      status: 200,
      detail: 'HTTP 200, 111253802 байт',
    })
  })

  it('404 — отказ: ровно так выглядела пропажа ассета в #140', async () => {
    const fetchImpl = async () => response(404)

    const result = await checkUrl('https://example.invalid/gone', { fetchImpl })

    expect(result.kind).toBe('dead')
    expect(result.detail).toBe('HTTP 404')
  })

  it('401/403/406 — не отказ: хост ответил про клиента, а не про ресурс', async () => {
    for (const status of [401, 403, 406]) {
      const result = await checkUrl('https://example.invalid/gitlab', {
        fetchImpl: async () => response(status),
      })

      expect(result.kind).toBe('warn')
      expect(result.detail).toContain('не отдаёт')
    }
  })

  it('не принимает отказ метода HEAD за мёртвую ссылку', async () => {
    const seen = []
    const fetchImpl = async (url, init) => {
      seen.push(init.method)
      return init.method === 'HEAD' ? response(405) : response(206, { 'content-length': '1' })
    }

    const result = await checkUrl('https://example.invalid/head-less', { fetchImpl })

    expect(seen).toStrictEqual(['HEAD', 'GET'])
    expect(result.kind).toBe('ok')
  })

  it('сетевой отказ — это отказ проверки, а не исключение наружу', async () => {
    const fetchImpl = async () => {
      throw new Error('getaddrinfo ENOTFOUND')
    }

    const result = await checkUrl('https://example.invalid/dns', { fetchImpl })

    expect(result).toStrictEqual({
      kind: 'dead',
      status: null,
      detail: 'запрос не удался: getaddrinfo ENOTFOUND',
    })
  })
})

describe('parseArgs', () => {
  it('по умолчанию проверяет и пин, и документы', () => {
    expect(parseArgs([])).toStrictEqual({ pinOnly: false })
  })

  it('--pin сужает проверку до адресов пина', () => {
    expect(parseArgs(['--pin'])).toStrictEqual({ pinOnly: true })
  })

  it('отказывает на незнакомом флаге вместо молчаливого пропуска', () => {
    expect(() => parseArgs(['--all'])).toThrow(/unknown argument: --all/)
  })
})
