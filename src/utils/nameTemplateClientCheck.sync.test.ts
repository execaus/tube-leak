import { deepStrictEqual } from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

import { describe, expect, it } from 'vitest'

import type { TemplateProblem } from '@/types/generated/settings'

import { validateNameTemplateDraft } from './nameTemplateClientCheck'

/**
 * Сверка клиентского валидатора с ядром (TL-101, issue execaus/tube-leak#108,
 * из ревью TL-94 #101). `validateNameTemplateDraft` — продублированная в TS
 * копия правил `src-tauri/src/storage/settings.rs` (см. doc-комментарий
 * модуля): решает, доступна ли кнопка «Сохранить», ДО обращения к серверу, и
 * расхождение в строгую сторону (клиент строже ядра) блокирует валидный
 * шаблон навсегда — сервер такому вводу просто не даёт шанса возразить.
 * Разовая проверка на ревью такой дрейф не ловит; этот тест — постоянный
 * сторож.
 *
 * Источник истины — фикстура `src-tauri/tests/fixtures/name-template-verdicts.json`,
 * СГЕНЕРИРОВАННАЯ ядром (#107, `template_verdicts_tests.rs`, TL-100). Файл
 * читается напрямую, тем же приёмом, что и `YtDlpUpdateBlock.test.ts`/
 * `style.guard.test.ts` — путь строится через `node:path` от
 * `fileURLToPath(import.meta.url)`, а не через `new URL(..., import.meta.url)`
 * (тот паттерн Vite перехватывает как asset-импорт).
 *
 * Сравнение — `node:assert/strict` `deepStrictEqual`, а не `JSON.stringify`:
 * порядок ключей объекта не должен влиять на результат сверки, хотя на
 * практике клиент строит объекты в том же порядке, что и провод ядра.
 */

interface FixtureVerdict {
  readonly template: string
  readonly verdict: TemplateProblem | null
}

interface Fixture {
  readonly about: string
  readonly regenerate: string
  readonly count: number
  readonly verdicts: readonly FixtureVerdict[]
}

const FIXTURE_PATH = join(
  dirname(fileURLToPath(import.meta.url)),
  '..',
  '..',
  'src-tauri',
  'tests',
  'fixtures',
  'name-template-verdicts.json',
)

// Порог, ловящий пустой или обрезанный файл (TL-100 сгенерировала 2413
// записей на момент этой задачи) — не равен действующему числу нарочно,
// как и предел попыток в #51: сторож должен пережить рост фикстуры без
// правки.
const MIN_FIXTURE_RECORDS = 2000

function readFixture(): Fixture {
  const raw = readFileSync(FIXTURE_PATH, 'utf-8')
  return JSON.parse(raw) as Fixture
}

function verdictsMatch(client: TemplateProblem | null, core: TemplateProblem | null): boolean {
  try {
    deepStrictEqual(client, core)
    return true
  } catch {
    return false
  }
}

describe('validateNameTemplateDraft — сверка с ядром по фикстуре (#107/#108)', () => {
  const fixture = readFixture()

  it('фикстура не пуста и не обрезана: count совпадает с длиной verdicts и не меньше 2000', () => {
    expect(fixture.verdicts).toHaveLength(fixture.count)
    expect(fixture.verdicts.length).toBeGreaterThanOrEqual(MIN_FIXTURE_RECORDS)
  })

  it('вердикт клиента совпадает с вердиктом ядра на каждой записи фикстуры', () => {
    fixture.verdicts.forEach(({ template, verdict: coreVerdict }, index) => {
      const clientVerdict = validateNameTemplateDraft(template) ?? null

      if (!verdictsMatch(clientVerdict, coreVerdict)) {
        throw new Error(
          `Расхождение клиентского валидатора с ядром на записи #${index} ` +
            `из ${fixture.verdicts.length}.\n` +
            `  шаблон:         ${JSON.stringify(template)}\n` +
            `  вердикт ядра:   ${JSON.stringify(coreVerdict)}\n` +
            `  вердикт клиента: ${JSON.stringify(clientVerdict)}`,
        )
      }
    })
  })
})
