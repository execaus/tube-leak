import { describe, expect, it } from 'vitest'

import { loadPin } from '../fetch-binaries/pin.mjs'
import { KNOWN_TARGETS } from '../fetch-binaries/targets.mjs'
import { checkUrl, parseArgs, planAll } from './index.mjs'
import {
  collectAllUrls,
  collectPinUrls,
  crossCheckDocs,
  DOC_FILES,
  DOCUMENTED_SECTION,
  extractMarkdownUrls,
  ourTextOf,
  PIN_PATH,
  planProbe,
  readDocs,
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
 * файла» такого не ловила. Меняется вместе с нашим текстом — осознанно.
 */
const OUR_LICENSE_TEXT_URLS = 14

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

    // Точное число, а не «меньше файла»: прежняя проверка была верна и
    // при съехавшей границе (замечание Б2 ревью TL-133).
    expect(extractMarkdownUrls(ours, 'x')).toHaveLength(OUR_LICENSE_TEXT_URLS)

    // Адреса сборок ffmpeg — наши обещания и обязаны остаться внутри.
    const pin = await loadPin(PIN_PATH)
    for (const target of KNOWN_TARGETS) {
      expect(ours).toContain(pin[DOCUMENTED_SECTION].targets[target].url)
    }
  })

  it('пропажа ОДНОГО из трёх маркеров — отказ, а не тихой съезд границы', async () => {
    const docs = await readDocs()
    const full = docs['THIRD-PARTY-LICENSES.md']
    expect(full).toContain(FIRST_LICENSE_HEADING)

    // Мутация ревьюера: убираем только ПЕРВЫЙ заголовок. Маркеры ещё
    // есть, поэтому проверка «маркер найден» молчит — ловить обязаны
    // отпечатки дословного текста лицензии.
    const mutated = full.replace(FIRST_LICENSE_HEADING, '### Текст GNU GPL v3')

    expect(() => ourTextOf('THIRD-PARTY-LICENSES.md', mutated)).toThrow(/граница нашего текста съехала/)
    expect(() => ourTextOf('THIRD-PARTY-LICENSES.md', mutated)).toThrow(/TERMS AND CONDITIONS/)
  })
})

describe('planProbe: что проверяется, что заменяется, что пропускается', () => {
  it('проверяет .git-адреса, которые отвечают по HTTP — их большинство', () => {
    // Все пять измерены 2026-09-26 и отвечают 200. Прежнее правило
    // «любой .git — пропустить» глотало их все (замечание Б1).
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
    const url = 'https://github.com/google/this-repo-does-not-exist-xyz123.git'

    expect(planProbe(url).kind).toBe('check')
  })

  it('git-эндпоинт SourceForge проверяет заменой — страницей того же репозитория', () => {
    expect(planProbe('https://git.code.sf.net/p/soxr/code')).toStrictEqual({
      kind: 'check',
      probeUrl: 'https://sourceforge.net/p/soxr/code/',
      why: expect.stringContaining('страница того же репозитория'),
    })
    expect(planProbe('https://git.code.sf.net/p/opencore-amr/code').probeUrl).toBe(
      'https://sourceforge.net/p/opencore-amr/code/',
    )
  })

  it('пропускает только измеренные отказы, называя причину', () => {
    expect(planProbe('https://bitbucket.org/multicoreware/x265_git.git')).toStrictEqual({
      kind: 'skip',
      reason: expect.stringContaining('Bitbucket'),
    })
    expect(planProbe('https://svn.xvid.org/trunk/xvidcore')).toStrictEqual({
      kind: 'skip',
      reason: expect.stringContaining('401'),
    })
  })

  it('пропускает issues нашего приватного репозитория', () => {
    for (const url of [
      'https://github.com/execaus/tube-leak/issues',
      'https://github.com/execaus/tube-leak/issues/14',
    ]) {
      expect(planProbe(url).kind).toBe('skip')
    }
  })

  it('НЕ пропускает будущее зеркало ассета и соседний репозиторий', () => {
    // Оба прошли бы как «наш приватный» при правиле startsWith (Н1):
    // в день собственного зеркала пин-адрес ffmpeg перестал бы
    // охраняться молча.
    for (const url of [
      'https://github.com/execaus/tube-leak/releases/download/v0.1.1/ffmpeg-x86_64-pc-windows-msvc.zip',
      'https://github.com/execaus/tube-leak-docs/blob/main/epics/E1.md',
      'https://github.com/execaus/tube-leak',
    ]) {
      expect(planProbe(url).kind).toBe('check')
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

  it('оставляет под охраной подавляющее большинство адресов', async () => {
    const { checked, skipped } = planAll(await collectAllUrls())

    // До сужения правила пропускался 71 адрес из 129 — то есть строки
    // Linux-таблицы, которыми закрыт #136, почти не охранялись.
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

describe('planAll', () => {
  it('делит адреса на проверяемые (в т.ч. по замене) и пропускаемые', () => {
    const { checked, skipped } = planAll([
      { url: 'https://ffmpeg.org/releases/ffmpeg-9.0.1.tar.xz', where: ['DOC.md'] },
      { url: 'https://git.code.sf.net/p/soxr/code', where: ['DOC.md'] },
      { url: 'https://svn.xvid.org/trunk/xvidcore', where: ['DOC.md'] },
    ])

    expect(checked.map((entry) => entry.probeUrl)).toStrictEqual([
      'https://ffmpeg.org/releases/ffmpeg-9.0.1.tar.xz',
      'https://sourceforge.net/p/soxr/code/',
    ])
    expect(skipped).toHaveLength(1)
    expect(skipped[0].reason).toContain('401')
  })
})

describe('checkUrl', () => {
  const response = (status, headers = {}) => ({
    ok: status >= 200 && status < 300,
    status,
    headers: { get: (name) => headers[name.toLowerCase()] ?? null },
  })
  // Пауз в тестах нет: ждать по 10 секунд ради проверки логики повторов
  // незачем, а настоящие задержки заданы константой рядом с ними.
  const noSleep = async () => {}

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
    const fetchImpl = async () => {
      calls += 1
      return response(404)
    }

    const result = await checkUrl('https://example.invalid/gone', { fetchImpl, sleepImpl: noSleep })

    expect(result.kind).toBe('dead')
    expect(result.detail).toBe('HTTP 404')
    expect(calls).toBe(1)
  })

  it('переживает мигающий 503 и зеленеет — гейт не краснеет на ровном месте', async () => {
    let calls = 0
    const fetchImpl = async () => {
      calls += 1
      return calls <= 3 ? response(503) : response(200)
    }

    const result = await checkUrl('https://aomedia.invalid/aom', { fetchImpl, sleepImpl: noSleep })

    expect(result.kind).toBe('ok')
    expect(result.attempts).toBe(4)
    expect(result.detail).toContain('с попытки 4')
  })

  it('упорный 5xx — не отказ, а «временно недоступен», но повторы исчерпаны', async () => {
    let calls = 0
    const fetchImpl = async () => {
      calls += 1
      return response(503)
    }

    const result = await checkUrl('https://example.invalid/down', { fetchImpl, sleepImpl: noSleep })

    expect(result.kind).toBe('warn')
    expect(calls).toBe(4)
    expect(result.detail).toContain('временно недоступен')
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
      expect(result.detail).toContain('не отдаёт')
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
    const fetchImpl = async () => {
      calls += 1
      throw new Error('getaddrinfo ENOTFOUND')
    }

    const result = await checkUrl('https://example.invalid/dns', { fetchImpl, sleepImpl: noSleep })

    expect(result.kind).toBe('dead')
    expect(calls).toBe(4)
    expect(result.detail).toContain('getaddrinfo ENOTFOUND')
  })

  it('сетевой сбой, прошедший со второй попытки, — живой адрес', async () => {
    let calls = 0
    const fetchImpl = async () => {
      calls += 1
      if (calls === 1) throw new Error('socket hang up')
      return response(200)
    }

    const result = await checkUrl('https://example.invalid/flaky', { fetchImpl, sleepImpl: noSleep })

    expect(result.kind).toBe('ok')
    expect(result.attempts).toBe(2)
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
