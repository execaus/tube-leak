import { spawn } from 'node:child_process'
import { join } from 'node:path'

import { describe, expect, it } from 'vitest'

import { loadPin } from '../fetch-binaries/pin.mjs'
import { KNOWN_TARGETS } from '../fetch-binaries/targets.mjs'
import { checkAll, checkUrl, hostCanary, judge, parseArgs, planAll, run } from './index.mjs'
import {
  canaryUrlFor,
  collectAllUrls,
  collectPinUrls,
  crossCheckDocs,
  DOC_FILES,
  DOCUMENTED_SECTION,
  extractMarkdownUrls,
  isPinAddress,
  ourTextOf,
  PIN_PATH,
  planProbe,
  readDocs,
  REPO_ROOT,
} from './sources.mjs'

// Сети здесь нет и быть не может (правило проекта): всё, что ходит
// наружу, живёт в `npm run check-pins`. В тестах — только разбор,
// сведение списков и офлайн-сверка пина с документами §6d; у checkUrl
// подменяются и fetch, и пауза между повторами.

const FIRST_LICENSE_HEADING = '### Полный текст GNU General Public License v3'

/**
 * Адресов в нашей части THIRD-PARTY-LICENSES.md. Число заморожено
 * НАМЕРЕННО: съехавшая граница втягивает чужие адреса из текста лицензии
 * (замерено — 14 превращались в 20), и проверка «наша часть короче
 * файла» такого не ловила.
 */
const OUR_LICENSE_TEXT_URLS = 14

const response = (status, headers = {}) => ({
  ok: status >= 200 && status < 300,
  status,
  headers: { get: (name) => headers[name.toLowerCase()] ?? null },
})
// Пауз в тестах нет: ждать по 10 секунд ради проверки логики повторов
// незачем, а настоящие задержки заданы константой рядом с ними.
const noSleep = async () => {}

const entry = (url, origins, where = ['DOC.md']) => ({ url, origins, where })

describe('extractMarkdownUrls', () => {
  it('ловит голые ссылки, ссылки в скобках и в обратных кавычках', () => {
    const text = [
      'Голая https://example.invalid/a рядом с текстом.',
      '[подпись](https://example.invalid/b) — ссылка markdown.',
      'В таблице: `https://example.invalid/c`',
      '| Linux | BtbN | https://example.invalid/d |',
    ].join('\n')

    expect(extractMarkdownUrls(text, 'DOC.md').map((item) => item.url)).toStrictEqual([
      'https://example.invalid/a',
      'https://example.invalid/b',
      'https://example.invalid/c',
      'https://example.invalid/d',
    ])
  })

  it('отрезает хвостовую пунктуацию, но не части адреса', () => {
    const text = 'См. https://example.invalid/x-1.tar.xz, а также https://example.invalid/y?v=1.'

    expect(extractMarkdownUrls(text, 'DOC.md').map((item) => item.url)).toStrictEqual([
      'https://example.invalid/x-1.tar.xz',
      'https://example.invalid/y?v=1',
    ])
  })

  it('не выдаёт имя файла за ссылку — ровно это и прятало дефект #140', () => {
    const text = '| Windows | gyan.dev | `packages/ffmpeg-9.0.1-essentials_build.zip` |'

    expect(extractMarkdownUrls(text, 'DOC.md')).toStrictEqual([])
  })

  it('помечает происхождение адресов документами', () => {
    expect(extractMarkdownUrls('https://example.invalid/a', 'DOC.md')[0].origin).toBe('docs')
  })
})

describe('ourTextOf: граница нашего текста', () => {
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

  it('на настоящем файле даёт РОВНО наш набор адресов', async () => {
    const docs = await readDocs()
    const ours = ourTextOf('THIRD-PARTY-LICENSES.md', docs['THIRD-PARTY-LICENSES.md'])

    expect(extractMarkdownUrls(ours, 'x')).toHaveLength(OUR_LICENSE_TEXT_URLS)

    const pin = await loadPin(PIN_PATH)
    for (const target of KNOWN_TARGETS) {
      expect(ours).toContain(pin[DOCUMENTED_SECTION].targets[target].url)
    }
  })

  it('пропажа ОДНОГО из трёх маркеров — отказ, а не тихий съезд границы', async () => {
    const docs = await readDocs()
    const full = docs['THIRD-PARTY-LICENSES.md']
    expect(full).toContain(FIRST_LICENSE_HEADING)

    const mutated = full.replace(FIRST_LICENSE_HEADING, '### Текст GNU GPL v3')

    expect(() => ourTextOf('THIRD-PARTY-LICENSES.md', mutated)).toThrow(/граница нашего текста съехала/)
    expect(() => ourTextOf('THIRD-PARTY-LICENSES.md', mutated)).toThrow(/TERMS AND CONDITIONS/)
  })
})

describe('planProbe: что проверяется, что заменяется, что пропускается', () => {
  it('проверяет .git-адреса, которые отвечают по HTTP — их большинство', () => {
    for (const url of [
      'https://github.com/google/snappy.git',
      'https://gitlab.com/AOMediaCodec/SVT-AV1.git',
      'https://code.videolan.org/videolan/x264.git',
      'https://git.savannah.gnu.org/git/libiconv.git',
      'https://svn.code.sf.net/p/lame/svn/trunk/lame',
    ]) {
      expect(planProbe(url)).toStrictEqual({ kind: 'check', probeUrl: url, why: null })
    }
  })

  it('не глотает заведомо мёртвый .git — воспроизведение находки ревью', () => {
    expect(planProbe('https://github.com/google/this-repo-does-not-exist-xyz123.git').kind).toBe('check')
  })

  it('git-эндпоинт SourceForge проверяет заменой — страницей того же репозитория', () => {
    expect(planProbe('https://git.code.sf.net/p/soxr/code')).toStrictEqual({
      kind: 'check',
      probeUrl: 'https://sourceforge.net/p/soxr/code/',
      why: expect.stringContaining('страница того же репозитория'),
    })
  })

  it('пропускает только измеренные отказы, называя причину', () => {
    expect(planProbe('https://bitbucket.org/multicoreware/x265_git.git').reason).toContain('Bitbucket')
    expect(planProbe('https://svn.xvid.org/trunk/xvidcore').reason).toContain('401')
  })

  it('пропускает issues нашего приватного репозитория, но не зеркало ассета', () => {
    expect(planProbe('https://github.com/execaus/tube-leak/issues/14').kind).toBe('skip')
    for (const url of [
      'https://github.com/execaus/tube-leak/releases/download/v0.1.1/ffmpeg.zip',
      'https://github.com/execaus/tube-leak-docs/blob/main/epics/E1.md',
    ]) {
      expect(planProbe(url).kind).toBe('check')
    }
  })
})

describe('происхождение адреса и строгость', () => {
  it('пин помечается происхождением pin, документы — docs', async () => {
    const pin = await loadPin(PIN_PATH)
    expect(collectPinUrls(pin)[0].origin).toBe('pin')

    const all = await collectAllUrls()
    const windows = all.find((item) => item.url === pin[DOCUMENTED_SECTION].targets['x86_64-pc-windows-msvc'].url)
    // Один и тот же адрес назван и пином, и документами — оба
    // происхождения сохраняются, строгость берётся по пину.
    expect(windows.origins).toStrictEqual(['docs', 'pin'])
    expect(isPinAddress(windows)).toBe(true)
  })

  it('«не подтверждён» у пина — отказ, у документов — терпимо', () => {
    const results = [
      { ...entry('https://a.invalid/pin', ['pin']), kind: 'warn' },
      { ...entry('https://a.invalid/doc', ['docs']), kind: 'warn' },
      { ...entry('https://a.invalid/live', ['pin']), kind: 'ok' },
      { ...entry('https://a.invalid/gone', ['docs']), kind: 'dead' },
    ]

    const { alive, fatal, tolerated } = judge(results)

    expect(alive.map((r) => r.url)).toStrictEqual(['https://a.invalid/live'])
    expect(fatal.map((r) => r.url)).toStrictEqual(['https://a.invalid/pin', 'https://a.invalid/gone'])
    expect(tolerated.map((r) => r.url)).toStrictEqual(['https://a.invalid/doc'])
  })

  it('класс, которого в приговоре ещё нет, зелёным не проходит нигде', () => {
    // Белый список зелёного, а не чёрный список бед: пропущенный адрес
    // пина не попадал ни в одну из трёх корзин и потому был невидим
    // (#142). Новый класс обязан краснеть, а не исчезать.
    const results = [
      { ...entry('https://a.invalid/pin', ['pin']), kind: 'новый-класс' },
      { ...entry('https://a.invalid/doc', ['docs']), kind: 'новый-класс' },
    ]

    const { alive, fatal, tolerated } = judge(results)

    expect(alive).toStrictEqual([])
    expect(tolerated).toStrictEqual([])
    expect(fatal).toHaveLength(2)
  })

  it('тот же всегда-500 у документов гейт не валит, но и живым не считается', async () => {
    // Зеркало предыдущего теста: разница строгости обязана быть видна на
    // одном и том же стенде, иначе разделение пин/документы формально.
    const entries = [
      entry('https://a.invalid/doc-1', ['docs']),
      entry('https://a.invalid/doc-2', ['docs']),
    ]
    const results = await checkAll(entries, {
      fetchImpl: async () => response(500),
      sleepImpl: noSleep,
    })

    const { alive, fatal, tolerated } = judge(results)

    expect(alive).toHaveLength(0)
    expect(fatal).toHaveLength(0)
    expect(tolerated).toHaveLength(entries.length)
    for (const result of tolerated) expect(result.detail).toContain('временно недоступен')
  })

  it('хост, отдающий 500 всем и всегда, НЕ пропускает пин в зелёный гейт', async () => {
    // Подстановка ревьюера: до правки такой стенд давал «мёртвых 0» и
    // EXIT=0, то есть весь пин исчезал из проверки.
    const pin = await loadPin(PIN_PATH)
    const entries = collectPinUrls(pin).map((item) => ({ ...item, origins: ['pin'], where: [item.where] }))
    const results = await checkAll(entries, { fetchImpl: async () => response(500), sleepImpl: noSleep })

    const { alive, fatal, tolerated } = judge(results)

    expect(alive).toHaveLength(0)
    expect(tolerated).toHaveLength(0)
    expect(fatal).toHaveLength(entries.length)
  })
})

describe('канарейка хоста', () => {
  it('хост, отвечающий 200 на несуществующий путь, не даёт «живых»', async () => {
    const results = await checkAll([entry('https://videolan.invalid/x264', ['docs'])], {
      fetchImpl: async () => response(200, { 'content-length': '0' }),
      sleepImpl: noSleep,
    })

    expect(results[0].kind).toBe('warn')
    expect(results[0].detail).toContain('несуществующий путь')
  })

  it('хост, честно отвечающий 404 на канарейку, проверяется как раньше', async () => {
    const fetchImpl = async (url) =>
      url.includes('canary-does-not-exist') ? response(404) : response(200, { 'content-length': '10' })

    const results = await checkAll([entry('https://github.invalid/asset.zip', ['pin'])], {
      fetchImpl,
      sleepImpl: noSleep,
    })

    expect(results[0].kind).toBe('ok')
    expect(judge(results).fatal).toHaveLength(0)
  })

  it('канарейка спрашивается один раз на хост, а не на адрес', async () => {
    let canaryCalls = 0
    const fetchImpl = async (url) => {
      if (url.includes('canary-does-not-exist')) {
        canaryCalls += 1
        return response(404)
      }
      return response(200)
    }

    await checkAll(
      [
        entry('https://same.invalid/a', ['docs']),
        entry('https://same.invalid/b', ['docs']),
        entry('https://same.invalid/c', ['docs']),
      ],
      { fetchImpl, sleepImpl: noSleep },
    )

    expect(canaryCalls).toBe(1)
  })

  it('упорный 5xx на канарейке не объявляет хост неразличающим', async () => {
    const fetchImpl = async (url) =>
      url.includes('canary-does-not-exist') ? response(503) : response(200)

    const canary = await hostCanary('https://flaky.invalid/asset', { fetchImpl, sleepImpl: noSleep })

    expect(canary.indistinguishable).toBe(false)
  })

  it('адрес канарейки строится на том же хосте', () => {
    expect(canaryUrlFor('https://code.videolan.org/videolan/x264')).toMatch(
      /^https:\/\/code\.videolan\.org\/.+/,
    )
  })
})

describe('collectAllUrls', () => {
  it('сводит пин и документы без повторов, сохраняя все места адреса', async () => {
    const all = await collectAllUrls()
    const urls = all.map((item) => item.url)

    expect(new Set(urls).size).toBe(urls.length)

    const pin = await loadPin(PIN_PATH)
    const linux = pin[DOCUMENTED_SECTION].targets['x86_64-unknown-linux-gnu'].url
    const found = all.find((item) => item.url === linux)
    expect(found?.where).toStrictEqual([
      'binaries.lock.json ffmpeg.x86_64-unknown-linux-gnu',
      ...DOC_FILES,
    ])
  })

  it('не тащит адреса из дословных текстов чужих лицензий', async () => {
    const all = await collectAllUrls()

    expect(all.some((item) => item.url.includes('chasen.aist-nara.ac.jp'))).toBe(false)
  })

  it('оставляет под охраной подавляющее большинство адресов', async () => {
    const { checked, skipped } = planAll(await collectAllUrls())

    expect(skipped.length).toBeLessThanOrEqual(5)
    expect(checked.length).toBeGreaterThan(100)
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
    docs['SOURCES-FFMPEG.md'] = docs['SOURCES-FFMPEG.md'].replaceAll(
      url,
      'packages/ffmpeg-9.0.1-essentials_build.zip',
    )

    const problems = crossCheckDocs(pin, docs)

    expect(problems).toHaveLength(1)
    expect(problems[0]).toContain('x86_64-pc-windows-msvc')
  })

  it('краснеет за каждый документ, потерявший адрес, и называет тройку', async () => {
    const pin = await loadPin(PIN_PATH)
    const docs = await readDocs()
    const url = pin[DOCUMENTED_SECTION].targets['aarch64-apple-darwin'].url
    for (const name of DOC_FILES) docs[name] = docs[name].replaceAll(url, 'см. пин')

    expect(crossCheckDocs(pin, docs)).toHaveLength(DOC_FILES.length)
  })

  it('перепиновка без правки документов — расхождение, а не тишина', async () => {
    const pin = await loadPin(PIN_PATH)
    const docs = await readDocs()
    pin[DOCUMENTED_SECTION].targets['x86_64-unknown-linux-gnu'].url =
      'https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2027-01-31-00-00/ffmpeg.tar.xz'

    expect(crossCheckDocs(pin, docs)).toHaveLength(DOC_FILES.length)
  })
})

/** Адрес, на который сегодня заведомо срабатывает правило пропуска. */
const SKIPPED_URL = 'https://svn.xvid.org/trunk/xvidcore'

describe('planAll', () => {
  it('делит адреса на проверяемые (в т.ч. по замене) и пропускаемые', () => {
    const { checked, skipped, refused } = planAll([
      entry('https://ffmpeg.org/releases/ffmpeg-9.0.1.tar.xz', ['docs']),
      entry('https://git.code.sf.net/p/soxr/code', ['docs']),
      entry(SKIPPED_URL, ['docs']),
    ])

    expect(checked.map((item) => item.probeUrl)).toStrictEqual([
      'https://ffmpeg.org/releases/ffmpeg-9.0.1.tar.xz',
      'https://sourceforge.net/p/soxr/code/',
    ])
    expect(skipped).toHaveLength(1)
    expect(skipped[0].reason).toContain('401')
    expect(refused).toStrictEqual([])
  })

  it('адрес ПИНА, подошедший под правило пропуска, — отказ, а не тихий зелёный (#142)', () => {
    const { checked, skipped, refused } = planAll([entry(SKIPPED_URL, ['pin'])])

    // Главное здесь — что его НЕТ в пропущенных: оттуда приговор его не
    // видит вовсе, и адрес пина оставался зелёным без единой проверки.
    expect(skipped).toStrictEqual([])
    expect(checked).toStrictEqual([])
    expect(refused).toHaveLength(1)
    expect(refused[0].detail).toContain('адрес ПИНА')
    expect(refused[0].detail).toContain('401')
    expect(judge(refused).fatal).toHaveLength(1)
  })

  it('тот же адрес, названный только документом, остаётся пропуском', () => {
    const { skipped, refused } = planAll([entry(SKIPPED_URL, ['docs'])])

    expect(refused).toStrictEqual([])
    expect(skipped).toHaveLength(1)
    expect(skipped[0].reason).toContain('401')
  })

  it('адрес, названный и пином, и документами, судится по пину', () => {
    const { skipped, refused } = planAll([entry(SKIPPED_URL, ['docs', 'pin'])])

    expect(skipped).toStrictEqual([])
    expect(refused).toHaveLength(1)
  })

  it('на настоящем списке адресов отказов плана сегодня нет', async () => {
    // Класс закрыт на будущее: хосты пина под действующие правила не
    // подходят, и гейт от этой правки не краснеет.
    expect(planAll(await collectAllUrls()).refused).toStrictEqual([])
  })
})

/**
 * Запускает саму команду. Сети не касается: разбор аргументов падает до
 * первого запроса.
 */
function runCli(args) {
  return new Promise((resolve, reject) => {
    const child = spawn(
      process.execPath,
      [join(REPO_ROOT, 'scripts', 'check-pins', 'index.mjs'), ...args],
      { stdio: ['ignore', 'ignore', 'pipe'] },
    )
    let stderr = ''
    child.stderr.on('data', (chunk) => {
      stderr += chunk
    })
    child.on('error', reject)
    child.on('close', (code) => resolve({ code, stderr }))
  })
}

describe('run: приговор решает код возврата', () => {
  const silent = () => {}
  // Если сторож полезет в сеть за адресом, который решено не проверять,
  // тест это увидит, а не примет молча.
  const noNetwork = () => {
    throw new Error('запрос к адресу, который проверять не собирались')
  }

  it('адрес ПИНА под правилом пропуска валит гейт, не сходив в сеть (EXIT=1)', async () => {
    await expect(
      run([entry(SKIPPED_URL, ['pin'])], { log: silent, fetchImpl: noNetwork, sleepImpl: noSleep }),
    ).rejects.toThrow(/из них адресов пина — 1/)
  })

  it('тот же адрес от документа гейт не валит (EXIT=0)', async () => {
    const verdict = await run([entry(SKIPPED_URL, ['docs'])], {
      log: silent,
      fetchImpl: noNetwork,
      sleepImpl: noSleep,
    })

    expect(verdict.fatal).toStrictEqual([])
    expect(verdict.skipped).toHaveLength(1)
  })

  it('отказ run — это EXIT=1 у самой команды, а не только исключение', async () => {
    // Связь «throw → код возврата» лежит в хвосте index.mjs, и без этого
    // теста «EXIT=1» выше был бы утверждением о непроверенном.
    const { code, stderr } = await runCli(['--no-such-flag'])

    expect(code).toBe(1)
    expect(stderr).toContain('unknown argument')
  })
})

describe('checkUrl', () => {
  it('считает живым ответ 200 и называет размер', async () => {
    const fetchImpl = async () => response(200, { 'content-length': '111253802' })

    await expect(checkUrl('https://example.invalid/a', { fetchImpl, sleepImpl: noSleep })).resolves.toStrictEqual({
      kind: 'ok',
      status: 200,
      detail: 'HTTP 200, 111253802 байт',
      attempts: 1,
    })
  })

  it('404 — отказ, и без повторов: так выглядела пропажа ассета в #140', async () => {
    let calls = 0
    const result = await checkUrl('https://example.invalid/gone', {
      fetchImpl: async () => {
        calls += 1
        return response(404)
      },
      sleepImpl: noSleep,
    })

    expect(result.kind).toBe('dead')
    expect(calls).toBe(1)
  })

  it('переживает мигающий 503 и зеленеет — гейт не краснеет на ровном месте', async () => {
    let calls = 0
    const result = await checkUrl('https://aomedia.invalid/aom', {
      fetchImpl: async () => {
        calls += 1
        return calls <= 3 ? response(503) : response(200)
      },
      sleepImpl: noSleep,
    })

    expect(result.kind).toBe('ok')
    expect(result.attempts).toBe(4)
  })

  it('упорный 5xx — «не подтверждён», повторы исчерпаны', async () => {
    let calls = 0
    const result = await checkUrl('https://example.invalid/down', {
      fetchImpl: async () => {
        calls += 1
        return response(503)
      },
      sleepImpl: noSleep,
    })

    expect(result.kind).toBe('warn')
    expect(calls).toBe(4)
  })

  it('401/403/406 — не отказ и без повторов: хост ответил про клиента', async () => {
    for (const status of [401, 403, 406]) {
      let calls = 0
      const result = await checkUrl('https://example.invalid/gitlab', {
        fetchImpl: async () => {
          calls += 1
          return response(status)
        },
        sleepImpl: noSleep,
      })

      expect(result.kind).toBe('warn')
      expect(calls).toBe(1)
    }
  })

  it('не принимает отказ метода HEAD за мёртвую ссылку', async () => {
    const seen = []
    const fetchImpl = async (url, init) => {
      seen.push(init.method)
      return init.method === 'HEAD' ? response(405) : response(206, { 'content-length': '1' })
    }

    const result = await checkUrl('https://example.invalid/head-less', { fetchImpl, sleepImpl: noSleep })

    expect(seen).toStrictEqual(['HEAD', 'GET'])
    expect(result.kind).toBe('ok')
  })

  it('сетевой сбой повторяется, и только потом становится отказом', async () => {
    let calls = 0
    const result = await checkUrl('https://example.invalid/dns', {
      fetchImpl: async () => {
        calls += 1
        throw new Error('getaddrinfo ENOTFOUND')
      },
      sleepImpl: noSleep,
    })

    expect(result.kind).toBe('dead')
    expect(calls).toBe(4)
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
