// Офлайн-сторож лицензионного комплекта (TL-136, #143).
//
// Сети и cargo здесь нет: измерение графа живёт в
// `npm run check-licenses`. Здесь — сверка снимка с тем, что лежит в
// репозитории (Cargo.lock, package-lock.json) и с разделами
// THIRD-PARTY-LICENSES.md.
//
// Почему этого достаточно, чтобы снимок не протух молча: лицензия пары
// «имя + версия» на crates.io неизменна, а новая зависимость меняет
// Cargo.lock. Пакет, не названный снимком ни одним ведром, красит
// прогон — то есть новая лицензия не может появиться в поставке,
// оставшись без раздела.

import { readFile } from 'node:fs/promises'
import { join } from 'node:path'

import { describe, expect, it } from 'vitest'

import {
  checkSnapshot,
  licensesRequiringSection,
  packagesNamedBy,
  parseCargoLock,
  parseNpmLock,
  readInputs,
  REPO_ROOT,
  SECTION_BY_LICENSE,
  TARGETS,
} from './licenses.mjs'
import { choose, effectiveLicenses, parseSpdx } from './spdx.mjs'

describe('разбор SPDX', () => {
  it('складывает конъюнкты и выбирает одну альтернативу из дизъюнкции', () => {
    // AND — обязательства складываются: ring требует и Apache-2.0, и ISC.
    expect(effectiveLicenses('Apache-2.0 AND ISC')).toStrictEqual(['Apache-2.0', 'ISC'])
    // OR — выбор наш, и он предсказуем.
    expect(effectiveLicenses('MIT OR Apache-2.0')).toStrictEqual(['MIT'])
    expect(effectiveLicenses('Unlicense OR MIT')).toStrictEqual(['MIT'])
    expect(effectiveLicenses('Apache-2.0 OR ISC OR MIT')).toStrictEqual(['MIT'])
  })

  it('не теряет конъюнкт за скобками', () => {
    // Единственное скобочное выражение графа. Разбор «по первому OR»
    // потерял бы Unicode-3.0 — то есть 18 крейтов ICU4X остались бы без
    // раздела ровно тем способом, которым #143 и возник.
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
    expect(effectiveLicenses('Apache-2.0 / MIT')).toStrictEqual(['MIT'])
    expect(effectiveLicenses('BSD-3-Clause/MIT')).toStrictEqual(['MIT'])
  })

  it('выбирает воспроизводимо, когда ни MIT, ни Apache-2.0 не предложены', () => {
    expect(choose(['Zlib', 'CDLA-Permissive-2.0'])).toBe('CDLA-Permissive-2.0')
    expect(effectiveLicenses('MPL-2.0')).toStrictEqual(['MPL-2.0'])
  })
})

describe('снимок лицензионного состава', () => {
  it('называет каждый пакет Cargo.lock ровно одним ведром', async () => {
    const { snapshot, cargoLock } = await readInputs()
    const named = packagesNamedBy(snapshot.rust)
    const all = parseCargoLock(cargoLock)

    expect(all.length).toBeGreaterThan(400)
    expect([...named].sort()).toStrictEqual(all)

    // Вёдра не пересекаются: пакет не может одновременно ехать в
    // поставку и не быть в релизном графе.
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
    // Наборы крейтов по тройкам разные (windows-*, gtk-*, objc2-*).
    // Мерить одну macOS — тот же класс слепоты, что спрятал дефект
    // `flate2`: на машине разработчика зелено, у получателя нет.
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

    // Позитивная сторона: именно те лицензии, ради которых заведён
    // #143, действительно измерены в графе, а не просто упомянуты в
    // таблице сторожа.
    const required = licensesRequiringSection(inputs.snapshot)
    for (const license of ['MPL-2.0', 'CDLA-Permissive-2.0', 'ISC', 'Unicode-3.0', 'Zlib', 'BSD-3-Clause']) {
      expect(required, `${license} обязана быть в измеренном графе`).toContain(license)
    }
  })

  it('краснеет на новой лицензии в графе, которой нет раздела (мутация)', async () => {
    const inputs = await readInputs()
    const mutated = structuredClone(inputs.snapshot)
    // Ровно тот случай, ради которого заведён сторож: в поставку въехал
    // крейт под лицензией, о которой в документе нет ни слова.
    mutated.rust.shipped['SSPL-1.0'] = ['evil-crate 1.0.0']

    const problems = checkSnapshot({ ...inputs, snapshot: mutated })
    expect(problems.join('\n')).toMatch(/SSPL-1\.0/)
    expect(problems.join('\n')).toMatch(/SECTION_BY_LICENSE/)
  })

  it('краснеет, когда раздел под известную лицензию пропал из документа (мутация)', async () => {
    const inputs = await readInputs()
    const withoutMpl = inputs.doc.replaceAll('## MPL-2.0', '## Некогда-MPL')

    const problems = checkSnapshot({ ...inputs, doc: withoutMpl })
    expect(problems.join('\n')).toMatch(/MPL-2\.0/)
  })

  it('краснеет на новой зависимости, не пересмотренной в снимке (мутация)', async () => {
    const inputs = await readInputs()
    const withNewCrate = `${inputs.cargoLock}\n[[package]]\nname = "evil-crate"\nversion = "1.0.0"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\n`

    const problems = checkSnapshot({ ...inputs, cargoLock: withNewCrate })
    expect(problems.join('\n')).toMatch(/evil-crate 1\.0\.0/)
    expect(problems.join('\n')).toMatch(/Cargo\.lock/)
  })

  it('краснеет на снимке, описывающем исчезнувший пакет (мутация)', async () => {
    const inputs = await readInputs()
    const mutated = structuredClone(inputs.snapshot)
    mutated.rust.notInReleaseGraph.push('ghost-crate 9.9.9')

    const problems = checkSnapshot({ ...inputs, snapshot: mutated })
    expect(problems.join('\n')).toMatch(/ghost-crate 9\.9\.9/)
  })
})

describe('таблица «лицензия → раздел»', () => {
  it('не содержит строк, под которые в документе нет заголовка', async () => {
    // Сторож отвечает и за собственную границу: устаревшая строка в
    // SECTION_BY_LICENSE — такая же ложь, как пропущенная лицензия.
    const doc = await readFile(join(REPO_ROOT, 'THIRD-PARTY-LICENSES.md'), 'utf8')
    for (const [license, heading] of Object.entries(SECTION_BY_LICENSE)) {
      expect(doc, `нет раздела «${heading}» под ${license}`).toContain(heading)
    }
  })

  it('разделы стоят там, где их видно: ниже дословных текстов чужих лицензий', async () => {
    // Граница нашего текста в этом файле проходит по первому заголовку
    // «Полный текст …» (scripts/check-pins/sources.mjs). Новые разделы
    // лежат НИЖЕ неё намеренно: они несут дословные чужие тексты, и
    // сторож адресов не вправе их править.
    const doc = await readFile(join(REPO_ROOT, 'THIRD-PARTY-LICENSES.md'), 'utf8')
    const boundary = doc.search(/^#{1,4} Полный текст/m)
    expect(boundary).toBeGreaterThan(-1)
    for (const heading of Object.values(SECTION_BY_LICENSE)) {
      expect(doc.indexOf(heading), `${heading} обязан лежать ниже границы нашего текста`).toBeGreaterThan(
        boundary,
      )
    }
  })
})
