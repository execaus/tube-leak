// Сторож лицензионного комплекта (TL-136, #143) — прогон.
//
// Две части, и обе здесь:
//
// 1. Офлайн: снимок против Cargo.lock, package-lock.json и разделов
//    документов. Быстро, cargo не нужен.
// 2. ИЗМЕРЕНИЕ графа (`measure()`): заново строит релизный граф по
//    четырём тройкам и сверяет со снимком целиком. Требует cargo.
//
// Почему измерение попало в `npm test`, хотя раньше жило отдельной
// командой. Ревью показало мутацией, что офлайн-часть слепа ровно на
// границе «не едет» / «едет, но раздела нет»: перенеси четыре
// MPL-крейта из `shipped` в `notInReleaseGraph`, убери строку из
// SECTION_BY_LICENSE и вырежи раздел — офлайн-часть возвращает ноль
// проблем. Ловило это только `npm run check-licenses`, а он не
// вызывался ни из `npm test`, ни из workflow, то есть не запускался
// автоматически никогда. Сторож, который никогда не запускается,
// охраняет ровно ничего.
//
// Цена названа: `npm test` теперь требует cargo в PATH. Для этого
// репозитория это не новое требование (`cargo test` и так в обычном
// прогоне), а отказ будет назван причиной, а не пустой ошибкой.

import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { beforeAll, describe, expect, it } from 'vitest'

import { cargoEnv, cargoFailure, measure } from './index.mjs'
import {
  checkNotices,
  checkSnapshot,
  checkSourceUrls,
  hasSection,
  headingPattern,
  licensesRequiringSection,
  packagesNamedBy,
  parseCargoLock,
  parseNpmLock,
  readInputs,
  REPO_ROOT,
  SECTION_BY_LICENSE,
  SECTION_LEVEL,
  shippedPackages,
  TARGETS,
} from './licenses.mjs'
import {
  copyrightLinesOf,
  crateLicenseTexts,
  noticeFrom,
  noticeHeadingPattern,
  npmLicenseTexts,
  parseNotices,
} from './notices.mjs'
import { collectLicenseSourceUrls } from '../check-pins/sources.mjs'
import { choose, effectiveLicenses, parseSpdx } from './spdx.mjs'

const MEASURE_TIMEOUT = 300_000

describe('разбор SPDX', () => {
  it('складывает конъюнкты и выбирает одну альтернативу из дизъюнкции', () => {
    expect(effectiveLicenses('Apache-2.0 AND ISC')).toStrictEqual(['Apache-2.0', 'ISC'])
    expect(effectiveLicenses('MIT OR Apache-2.0')).toStrictEqual(['MIT'])
    expect(effectiveLicenses('Unlicense OR MIT')).toStrictEqual(['MIT'])
    expect(effectiveLicenses('Apache-2.0 OR ISC OR MIT')).toStrictEqual(['MIT'])
  })

  it('не теряет конъюнкт за скобками', () => {
    expect(effectiveLicenses('(MIT OR Apache-2.0) AND Unicode-3.0')).toStrictEqual([
      'MIT',
      'Unicode-3.0',
    ])
    expect(parseSpdx('(MIT OR Apache-2.0) AND Unicode-3.0')).toStrictEqual([
      ['MIT', 'Apache-2.0'],
      ['Unicode-3.0'],
    ])
  })

  it('понимает исторический разделитель «/» как OR', () => {
    expect(effectiveLicenses('MIT/Apache-2.0')).toStrictEqual(['MIT'])
    expect(effectiveLicenses('BSD-3-Clause/MIT')).toStrictEqual(['MIT'])
  })

  it('выбирает воспроизводимо, когда ни MIT, ни Apache-2.0 не предложены', () => {
    expect(choose(['Zlib', 'CDLA-Permissive-2.0'])).toBe('CDLA-Permissive-2.0')
    expect(effectiveLicenses('MPL-2.0')).toStrictEqual(['MPL-2.0'])
  })
})

describe('заголовок раздела, а не подстрока', () => {
  it('понижённый уровень заголовка разделом не считается', () => {
    // Замечание ревью: проверка `doc.includes('## BSD-3-Clause')`
    // истинна и для `#### BSD-3-Clause`, то есть раздел можно спрятать
    // на четвёртый уровень, и сторож этого не заметит.
    const lowered = '#### BSD-3-Clause\n\nтекст\n'
    expect(lowered.includes(`${'#'.repeat(SECTION_LEVEL)} BSD-3-Clause`)).toBe(true)
    expect(hasSection(lowered, 'BSD-3-Clause')).toBe(false)
    expect(hasSection('## BSD-3-Clause\n', 'BSD-3-Clause')).toBe(true)
  })

  it('не считает разделом ни повышенный уровень, ни совпадение внутри строки', () => {
    expect(hasSection('# BSD-3-Clause\n', 'BSD-3-Clause')).toBe(false)
    expect(hasSection('см. раздел ## MIT ниже\n', 'MIT')).toBe(false)
    // Название раздела — целая строка, а не её начало: иначе «## MIT»
    // удовлетворялся бы заголовком «## MIT-0».
    expect(hasSection('## MIT-0\n', 'MIT')).toBe(false)
  })

  it('экранирует спецсимволы названия', () => {
    expect(headingPattern('CDLA-Permissive-2.0').test('## CDLA-Permissive-2X0\n')).toBe(false)
    expect(hasSection('## CDLA-Permissive-2.0\n', 'CDLA-Permissive-2.0')).toBe(true)
  })
})

describe('снимок лицензионного состава', () => {
  it('называет каждый пакет Cargo.lock ровно одним ведром', async () => {
    const { snapshot, cargoLock } = await readInputs()
    const named = packagesNamedBy(snapshot.rust)
    const all = parseCargoLock(cargoLock)

    expect(all.length).toBeGreaterThan(400)
    expect([...named].sort()).toStrictEqual(all)

    const shipped = new Set(Object.values(snapshot.rust.shipped).flat())
    for (const entry of snapshot.rust.notInReleaseGraph) {
      expect(shipped.has(entry), `${entry} и в поставке, и вне релизного графа`).toBe(false)
    }
    for (const entry of snapshot.rust.procMacro) {
      expect(shipped.has(entry), `${entry} и в поставке, и proc-macro`).toBe(false)
    }
  })

  it('называет каждый production-пакет package-lock.json', async () => {
    const { snapshot, npmLock } = await readInputs()
    expect([...packagesNamedBy(snapshot.npm)].sort()).toStrictEqual(parseNpmLock(npmLock))
  })

  it('снят по всем четырём тройкам, а не по одной', async () => {
    const { snapshot } = await readInputs()
    expect(snapshot.rust.targets).toStrictEqual([...TARGETS])

    const shipped = new Set(Object.values(snapshot.rust.shipped).flat())
    expect([...shipped].some((entry) => entry.startsWith('windows-sys '))).toBe(true)
    expect([...shipped].some((entry) => entry.startsWith('gtk '))).toBe(true)
    expect([...shipped].some((entry) => entry.startsWith('objc2 '))).toBe(true)
  })

  it('у каждой лицензии поставки есть раздел в THIRD-PARTY-LICENSES.md', async () => {
    const inputs = await readInputs()
    expect(checkSnapshot(inputs)).toStrictEqual([])

    const required = licensesRequiringSection(inputs.snapshot)
    for (const license of ['MPL-2.0', 'CDLA-Permissive-2.0', 'ISC', 'Unicode-3.0', 'Zlib', 'BSD-3-Clause']) {
      expect(required, `${license} обязана быть в измеренном графе`).toContain(license)
    }
  })

  it('краснеет на понижённом заголовке раздела (мутация)', async () => {
    const inputs = await readInputs()
    const lowered = inputs.doc.replace(/^## MPL-2\.0$/m, '#### MPL-2.0')
    expect(lowered).not.toBe(inputs.doc)

    const problems = checkSnapshot({ ...inputs, doc: lowered })
    expect(problems.join('\n')).toMatch(/MPL-2\.0/)
    expect(problems.join('\n')).toMatch(/уровня/)
  })

  it('краснеет на новой лицензии в графе, которой нет раздела (мутация)', async () => {
    const inputs = await readInputs()
    const mutated = structuredClone(inputs.snapshot)
    mutated.rust.shipped['SSPL-1.0'] = ['evil-crate 1.0.0']

    const problems = checkSnapshot({ ...inputs, snapshot: mutated })
    expect(problems.join('\n')).toMatch(/SSPL-1\.0/)
    expect(problems.join('\n')).toMatch(/SECTION_BY_LICENSE/)
  })

  it('краснеет на новой зависимости, не пересмотренной в снимке (мутация)', async () => {
    const inputs = await readInputs()
    const withNewCrate = `${inputs.cargoLock}\n[[package]]\nname = "evil-crate"\nversion = "1.0.0"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\n`

    const problems = checkSnapshot({ ...inputs, cargoLock: withNewCrate })
    expect(problems.join('\n')).toMatch(/evil-crate 1\.0\.0/)
  })

  it('краснеет на снимке, описывающем исчезнувший пакет (мутация)', async () => {
    const inputs = await readInputs()
    const mutated = structuredClone(inputs.snapshot)
    mutated.rust.notInReleaseGraph.push('ghost-crate 9.9.9')

    expect(checkSnapshot({ ...inputs, snapshot: mutated }).join('\n')).toMatch(/ghost-crate 9\.9\.9/)
  })
})

describe('уведомления об авторских правах (Б1)', () => {
  it('у каждого пакета поставки есть запись в NOTICES.md', async () => {
    const { snapshot, notices } = await readInputs()
    const shipped = shippedPackages(snapshot)
    expect(shipped.length).toBeGreaterThan(300)
    expect(checkNotices({ snapshot, notices })).toStrictEqual([])
  })

  it('не молчит, когда NOTICES.md не передан на проверку (М3)', async () => {
    // Прежняя версия возвращала пустой список, и checkSnapshot без
    // этого поля про NOTICES.md не говорил ни слова: проверка
    // отключалась забытым аргументом.
    const { snapshot } = await readInputs()
    expect(checkNotices({ snapshot, notices: undefined })).not.toStrictEqual([])
    expect(checkNotices({ snapshot, notices: undefined }).join('\n')).toMatch(/не передан/)
  })

  it('краснеет, когда у пакета поставки записи нет (мутация)', async () => {
    const { snapshot, notices } = await readInputs()
    const victim = shippedPackages(snapshot)[0]
    const without = notices.replace(noticeHeadingPattern(victim), '### вырезано')
    expect(without).not.toBe(notices)

    const problems = checkNotices({ snapshot, notices: without })
    expect(problems.join('\n')).toMatch(/NOTICES\.md/)
    expect(problems.join('\n')).toMatch(/условие гранта/)
  })

  it('уведомление берётся из файла, из метаданных или честно не берётся', () => {
    expect(
      noticeFrom({ licenseTexts: ['Copyright (c) 2024 Someone\nблаблабла'], authors: [] }),
    ).toStrictEqual({ source: 'file', lines: ['Copyright (c) 2024 Someone'], hadFiles: true })
    // У 19 пакетов ведра shipped файла лицензии нет вовсе — для них
    // источник метаданные, и запись обязана это называть.
    expect(noticeFrom({ licenseTexts: [], authors: ['Кто-то <a@b.c>'] })).toStrictEqual({
      source: 'metadata',
      lines: ['Кто-то <a@b.c>'],
      hadFiles: false,
    })
    // Ни файла, ни авторов — выдумывать правообладателя нельзя.
    expect(noticeFrom({ licenseTexts: [], authors: [] })).toStrictEqual({
      source: 'none',
      lines: [],
      hadFiles: false,
    })
  })

  it('различает «файла нет» и «файл есть, копирайта в нём нет»', () => {
    // `winnow`, `pin-project-lite`, `cargo-platform`, `webpki-roots`
    // файл лицензии ПОСТАВЛЯЮТ — он просто начинается сразу с условий.
    // Сказать про них «апстрим не поставляет файла лицензии» значило бы
    // соврать получателю в том самом файле, который заведён ради
    // правдивости.
    const noNotice = noticeFrom({
      licenseTexts: ['Permission is hereby granted, free of charge, to any person obtaining'],
      authors: [],
    })
    expect(noNotice).toStrictEqual({ source: 'none', lines: [], hadFiles: true })
    expect(noticeFrom({ licenseTexts: [], authors: [] }).hadFiles).toBe(false)
  })

  it('собирает уведомления из всех файлов пакета, а не только из первого', () => {
    // Случай `ring`: в LICENSE — только объяснение, копирайты лежат в
    // соседних файлах, и правообладателей несколько. Брать первый файл
    // значило бы записать пакет в «недоступно», хотя уведомление есть;
    // брать только первый НЕПУСТОЙ — потерять часть правообладателей.
    expect(
      noticeFrom({
        licenseTexts: [
          '*ring* uses an "ISC" license. See LICENSE-other-bits.',
          'Copyright (c) 2009 The Go Authors. All rights reserved.',
          'Copyright 2015-2025 Brian Smith.',
        ],
        authors: [],
      }),
    ).toStrictEqual({
      source: 'file',
      lines: ['Copyright (c) 2009 The Go Authors. All rights reserved.', 'Copyright 2015-2025 Brian Smith.'],
      hadFiles: true,
    })
  })

  it('не принимает прозу лицензии за уведомление', () => {
    // Дефект, пойманный на `ring`: половина его записи состояла из
    // перенесённых строк текста Apache-2.0, начинающихся со слова
    // copyright. Это условия лицензии, а не имя правообладателя.
    const prose = [
      'copyright notice that is included in or attached to the work',
      'copyright license to reproduce, prepare Derivative Works of,',
      'copyright notice, this list of conditions and the following disclaimer',
    ].join('\n')
    expect(copyrightLinesOf(prose)).toStrictEqual([])
    expect(copyrightLinesOf('Copyright (c) Felix Böhm')).toStrictEqual(['Copyright (c) Felix Böhm'])
    expect(copyrightLinesOf('Copyright 2015-2025 Brian Smith.')).toStrictEqual([
      'Copyright 2015-2025 Brian Smith.',
    ])

    // Уведомление без года — настоящее. Первая попытка чинить прозу
    // требованием года отсекала такие строки, и 35 пакетов уехали в
    // «восстановлено из метаданных», хотя уведомление у них есть.
    expect(copyrightLinesOf('Copyright The Rust Project Developers')).toStrictEqual([
      'Copyright The Rust Project Developers',
    ])
    // А `Copyright notice…` с заглавной — всё ещё проза.
    expect(copyrightLinesOf('Copyright notice shall be included in all copies')).toStrictEqual([])
    // Заголовок раздела лицензии Unicode — не уведомление.
    expect(copyrightLinesOf('COPYRIGHT AND PERMISSION NOTICE')).toStrictEqual([])
  })

  it('не тащит прозу лицензии и заготовку Apache вместо уведомления', () => {
    // Широкий /copyright/i вытащил бы строку про «copyright notice
    // shall be included», и уведомление утонуло бы в тексте лицензии.
    const mit = 'Copyright (c) 2020 A\n\nThe above copyright notice shall be included in all copies.'
    expect(copyrightLinesOf(mit)).toStrictEqual(['Copyright (c) 2020 A'])
    // Приложение Apache-2.0 — заготовка, а не имя правообладателя.
    expect(copyrightLinesOf('   Copyright [yyyy] [name of copyright owner]')).toStrictEqual([])
  })
})

/**
 * Строки копирайта, взятые ПРЯМО ИЗ ФАЙЛОВ пакета — независимо от того,
 * что записано в NOTICES.md.
 *
 * @param {string} entry «имя версия»
 * @param {boolean} isNpm
 * @returns {string[]}
 */
function noticeLinesFromPackage(entry, isNpm) {
  const texts = isNpm
    ? npmLicenseTexts(join(REPO_ROOT, 'node_modules', entry.slice(0, entry.lastIndexOf(' '))))
    : crateLicenseTexts(...entry.split(' '))
  if (texts === null) {
    throw new Error(`${entry}: исходников нет в кэше — сверять запись не с чем (cargo fetch / npm ci)`)
  }
  const lines = []
  for (const text of texts) {
    for (const line of copyrightLinesOf(text)) if (!lines.includes(line)) lines.push(line)
  }
  return lines
}

describe('содержимое записей сверяется с файлами пакетов (Б1, М2)', () => {
  it('каждая запись NOTICES.md совпадает с тем, что лежит в самом пакете', async () => {
    // Охраняется СОДЕРЖИМОЕ, а не заголовок. До этого сторожа удаление
    // строки копирайта внутри записи (мутация ревьюера на `ring`)
    // проходило `npm test` зелёным: заголовок-то на месте.
    const { snapshot, notices } = await readInputs()
    const records = parseNotices(notices)
    const npmShipped = new Set(Object.values(snapshot.npm.shipped).flat())

    const problems = []
    for (const entry of shippedPackages(snapshot)) {
      const record = records.get(entry)
      if (record === undefined) {
        problems.push(`${entry}: записи нет вовсе`)
        continue
      }
      const fromPackage = noticeLinesFromPackage(entry, npmShipped.has(entry))
      const restored = /восстановлено из метаданных|Уведомление недоступно/.test(record.body)

      if (restored) {
        // Уйти к метаданным можно ТОЛЬКО когда в файлах пакета
        // уведомления действительно нет. Иначе это потерянный
        // правообладатель, выданный за отсутствующего, — ровно дефект Б1.
        if (fromPackage.length > 0) {
          problems.push(`${entry}: в файлах пакета есть ${fromPackage.length} строк(и), а запись их не несёт`)
        }
        continue
      }
      if (record.lines.join('\n') !== fromPackage.join('\n')) {
        problems.push(`${entry}: запись разошлась с файлами пакета`)
      }
    }
    expect(problems).toStrictEqual([])
  })

  it('узнаёт уведомление, названное не первым словом строки (Б1)', () => {
    // Два случая, на которых правило «строка начинается словом
    // Copyright» теряло правообладателей у 22 пакетов.
    expect(
      copyrightLinesOf('PackageCopyrightText: 2019-2022, The Tauri Programme in the Commons Conservancy'),
    ).toStrictEqual(['PackageCopyrightText: 2019-2022, The Tauri Programme in the Commons Conservancy'])

    // IBM — не то же лицо, что Unicode, и его уведомление обязано
    // сохраняться наравне.
    const icu = [
      'Copyright © 2020-2024 Unicode, Inc.',
      'ICU 1.8.1 to ICU 57.1 © 1995-2016 International Business Machines Corporation and others.',
    ].join('\n')
    expect(copyrightLinesOf(icu)).toStrictEqual([
      'Copyright © 2020-2024 Unicode, Inc.',
      'ICU 1.8.1 to ICU 57.1 © 1995-2016 International Business Machines Corporation and others.',
    ])

    // А поле SPDX без правообладателя — не уведомление.
    expect(copyrightLinesOf('PackageCopyrightText: NOASSERTION')).toStrictEqual([])
  })

  it('промах кэша не выдаёт себя за «у пакета нет файла лицензии» (М1)', () => {
    // Иначе при пустом кэше все 295 крейтов получили бы пометку «файла
    // лицензии нет вовсе», и --write записал бы её получателю как факт.
    expect(crateLicenseTexts('заведомо-нет-такого-крейта', '9.9.9')).toBeNull()
    expect(npmLicenseTexts(join(REPO_ROOT, 'node_modules', 'заведомо-нет-такого-пакета'))).toBeNull()
  })
})

describe('адреса исходников по §3.2 MPL (Н6)', () => {
  it('снимок называет адрес на каждый MPL-крейт, и все они есть в документе', async () => {
    const { snapshot, doc } = await readInputs()
    const urls = snapshot.rust.sourceUrls ?? []
    expect(urls.map(({ package: pkg }) => pkg).sort()).toStrictEqual(
      [...snapshot.rust.shipped['MPL-2.0']].sort(),
    )
    expect(checkSourceUrls({ snapshot, doc })).toStrictEqual([])
  })

  it('краснеет, когда документ и проверяемый список разошлись (мутация)', async () => {
    const { snapshot, doc } = await readInputs()
    const url = snapshot.rust.sourceUrls[0].url
    const without = doc.replaceAll(url, 'https://example.invalid/подменено')

    expect(checkSourceUrls({ snapshot, doc: without }).join('\n')).toMatch(/§3\.2/)
  })

  it('не исчезает молча от переименованного ключа или порчи снимка (М4)', async () => {
    // Измерено на прежней версии: переименование ключа превращало
    // четыре адреса в ноль без жалобы и с нулевым кодом возврата.
    const dir = await mkdtemp(join(tmpdir(), 'tube-leak-licenses-'))
    const snapshot = { rust: { shipped: { 'MPL-2.0': ['a 1.0.0'] }, sourceUrls: [{ package: 'a 1.0.0', url: 'https://example.invalid/a' }] } }

    await writeFile(join(dir, 'licenses.lock.json'), JSON.stringify(snapshot), 'utf8')
    await expect(collectLicenseSourceUrls(dir)).resolves.toHaveLength(1)

    const renamed = structuredClone(snapshot)
    renamed.rust.sourceURLs = renamed.rust.sourceUrls
    delete renamed.rust.sourceUrls
    await writeFile(join(dir, 'licenses.lock.json'), JSON.stringify(renamed), 'utf8')
    await expect(collectLicenseSourceUrls(dir)).rejects.toThrow(/rust\.sourceUrls/)

    await writeFile(join(dir, 'licenses.lock.json'), '{ это не json', 'utf8')
    await expect(collectLicenseSourceUrls(dir)).rejects.toThrow(/JSON/)

    await rm(join(dir, 'licenses.lock.json'))
    await expect(collectLicenseSourceUrls(dir)).rejects.toThrow(/не прочитан/)
    await rm(dir, { recursive: true, force: true })
  })
})

describe('измерение графа (закрывает слепоту офлайн-части, Н4)', () => {
  /** @type {object} */
  let measured
  /** @type {object} */
  let stored

  beforeAll(async () => {
    measured = await measure()
    stored = (await readInputs()).snapshot
  }, MEASURE_TIMEOUT)

  it('снимок совпадает с заново измеренным графом', () => {
    expect(measured.rust).toStrictEqual(stored.rust)
    expect(measured.npm).toStrictEqual(stored.npm)
  })

  it('ловит мутацию ревьюера: перенос пакетов между вёдрами (дословно)', () => {
    // Мутация ревью целиком: четыре MPL-крейта уезжают из `shipped` в
    // `notInReleaseGraph`. Офлайн-часть этого не видит — обе стороны
    // сверки меняются согласованно. Измерение видит сразу.
    const mutated = structuredClone(stored)
    const mpl = mutated.rust.shipped['MPL-2.0']
    delete mutated.rust.shipped['MPL-2.0']
    mutated.rust.notInReleaseGraph.push(...mpl)
    mutated.rust.notInReleaseGraph.sort()
    delete mutated.rust.sourceUrls

    expect(mutated.rust).not.toStrictEqual(measured.rust)
    expect(Object.keys(measured.rust.shipped)).toContain('MPL-2.0')
  })
})

describe('отказ измерения', () => {
  it('называет причину, когда cargo нет в PATH', () => {
    const failure = cargoFailure(Object.assign(new Error('spawn cargo ENOENT'), { code: 'ENOENT' }))
    expect(failure.message).toMatch(/cargo не найден в PATH/)
    expect(failure.message).toMatch(/rustup/)
  })

  it('чужую ошибку не подменяет своей', () => {
    const original = Object.assign(new Error('bang'), { code: 'EACCES' })
    expect(cargoFailure(original)).toBe(original)
  })

  it('ищет cargo там, где он лежит в этом окружении, прежде чем отказать', () => {
    // `npm test` теперь измеряет граф, а cargo здесь вне PATH по
    // умолчанию (CLAUDE.md). Без поиска прогон краснел бы от переменной
    // окружения, а не от дефекта. Пропускать измерение нельзя — это
    // вернуло бы слепое пятно Н4, — поэтому сначала ищем.
    const augmented = cargoEnv({ PATH: '/usr/bin' })
    expect(augmented.PATH).toContain('/opt/homebrew/opt/rustup/bin')
    expect(augmented.PATH).toContain('/usr/bin')

    // Уже есть в PATH — окружение не трогаем.
    const already = { PATH: `/usr/bin:/opt/homebrew/opt/rustup/bin:${join(process.env.HOME ?? '', '.cargo', 'bin')}` }
    expect(cargoEnv(already)).toBe(already)
  })
})

describe('таблица «лицензия → раздел»', () => {
  it('не содержит строк, под которые в документе нет заголовка нужного уровня', async () => {
    const doc = await readFile(join(REPO_ROOT, 'THIRD-PARTY-LICENSES.md'), 'utf8')
    for (const [license, title] of Object.entries(SECTION_BY_LICENSE)) {
      expect(hasSection(doc, title), `нет раздела «${title}» под ${license}`).toBe(true)
    }
  })

  it('разделы стоят ниже дословных текстов чужих лицензий', async () => {
    const doc = await readFile(join(REPO_ROOT, 'THIRD-PARTY-LICENSES.md'), 'utf8')
    const boundary = doc.search(/^#{1,4} Полный текст/m)
    expect(boundary).toBeGreaterThan(-1)
    for (const title of Object.values(SECTION_BY_LICENSE)) {
      expect(doc.search(headingPattern(title)), `${title} обязан лежать ниже границы`).toBeGreaterThan(
        boundary,
      )
    }
  })
})
