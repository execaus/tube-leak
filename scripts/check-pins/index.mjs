#!/usr/bin/env node
// Сторож доступности адресов, которые мы обещаем (TL-133, #140):
// ссылки пина src-tauri/binaries.lock.json и ссылки указателя исходников
// SOURCES-FFMPEG.md / THIRD-PARTY-LICENSES.md.
//
//   npm run check-pins            # все адреса пина и документов
//   npm run check-pins -- --pin   # только адреса пина (быстро)
//
// Отдельная команда, а не тест: в тестах проекта сети нет, и `cargo test`
// с `npm test` ходить наружу не должны. Запускать перед выпуском.
//
// Строгость разная у пина и у документов (решение ведущего, Б4 ревью):
//
// - АДРЕС ПИНА обязан быть подтверждён живым. Это то, что доставка
//   СКАЧИВАЕТ на сборке: неподтверждённый адрес там — несобираемый
//   установщик, то есть ровно #140. Любой исход, кроме «жив», валит гейт:
//   и 404, и упорный 5xx, и «клиент не пропущен», и хост-канарейка.
// - АДРЕС ДОКУМЕНТА — указатель §6d. Мёртвый валит гейт, «не
//   подтверждён» — нет, но печатается отдельным списком с причиной.
//
// Чего сторож НЕ ловит: подмену файла под тем же адресом — это работа
// sha256 в пине, её проверяет доставка (scripts/fetch-binaries).
//
// Офлайн-часть сторожа — в sources.mjs (crossCheckDocs) и идёт в обычном
// `npm test`: она требует, чтобы документы называли сборки АДРЕСАМИ, а не
// именами файлов. Без неё эта команда была бы слепа ровно там, где #140.

import { fileURLToPath } from 'node:url'

import {
  canaryUrlFor,
  collectAllUrls,
  collectPinUrls,
  isPinAddress,
  mergeByUrl,
  PIN_PATH,
  planProbe,
} from './sources.mjs'
import { loadPin } from '../fetch-binaries/pin.mjs'

/** Сколько адресов проверяется одновременно. */
const CONCURRENCY = 6
/** Ожидание ответа на один запрос. */
const TIMEOUT_MS = 30_000

/**
 * Паузы перед повторами, то есть до четырёх попыток на адрес.
 *
 * Не перестраховка, а измерение (2026-09-26): шесть HEAD подряд к
 * `aomedia.googlesource.com/aom` дали `200 200 503 503 503 200`, GET —
 * `200 503 200`. Три отказа подряд возможны, поэтому и попыток четыре, и
 * паузы растут.
 */
const RETRY_DELAYS_MS = Object.freeze([1_000, 3_000, 6_000])

/**
 * Коды, которыми хост отвечает ПРО КЛИЕНТА, а не про наличие ресурса.
 *
 * Измерено (2026-09-26): `code.videolan.org` и `gitlab.com` отдают архивы
 * исходников по `curl -I` с кодом 200, а точно тем же запросом из node
 * `fetch` — 406, и это не лечится ни `User-Agent`, ни `Accept`, ни
 * `Accept-Encoding` (проверены все семь комбинаций, HEAD и GET с Range).
 * Отличается не ресурс, а клиент. Повторы таким ответам не помогают —
 * они стабильны, поэтому попытка одна. Для адреса ПИНА этот исход всё
 * равно валит гейт: «не опровергнут» — не то же, что «подтверждён».
 */
const CLIENT_REFUSED = new Set([401, 403, 406])

/** Коды, при которых имеет смысл повторить запрос методом GET. */
const HEAD_UNSUPPORTED = new Set([405, 501])

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

/**
 * Проверяет один адрес. Сначала HEAD (тело не качается), при отказе
 * метода — GET с Range на один байт. 5xx и сетевые сбои повторяются,
 * 404 и 401/403/406 — нет: они стабильны, и повтор только тянул бы время.
 *
 * @param {string} url
 * @param {{ fetchImpl?: typeof fetch; sleepImpl?: (ms: number) => Promise<void> }} [deps]
 * @returns {Promise<{ kind: 'ok' | 'warn' | 'dead'; status: number | null; detail: string; attempts: number }>}
 */
export async function checkUrl(url, { fetchImpl = fetch, sleepImpl = sleep } = {}) {
  const attempt = (init) =>
    fetchImpl(url, { redirect: 'follow', signal: AbortSignal.timeout(TIMEOUT_MS), ...init })

  let attempts = 0
  let last = null

  for (let round = 0; round <= RETRY_DELAYS_MS.length; round += 1) {
    if (round > 0) await sleepImpl(RETRY_DELAYS_MS[round - 1])
    attempts += 1

    try {
      let response = await attempt({ method: 'HEAD' })
      if (HEAD_UNSUPPORTED.has(response.status)) {
        response = await attempt({ method: 'GET', headers: { Range: 'bytes=0-0' } })
      }

      const { status } = response
      if (response.ok) {
        const size = response.headers?.get?.('content-length')
        return {
          kind: 'ok',
          status,
          detail: `HTTP ${status}${size ? `, ${size} байт` : ''}${attempts > 1 ? `, с попытки ${attempts}` : ''}`,
          attempts,
        }
      }
      if (CLIENT_REFUSED.has(status)) {
        return {
          kind: 'warn',
          status,
          detail: `HTTP ${status} — хост ответил, но этому клиенту не отдаёт (существование не подтверждено)`,
          attempts,
        }
      }
      if (status >= 500) {
        last = {
          kind: 'warn',
          status,
          detail: `HTTP ${status} — хост временно недоступен, ${attempts} попыток подряд (существование не подтверждено)`,
        }
        continue
      }
      return { kind: 'dead', status, detail: `HTTP ${status}`, attempts }
    } catch (err) {
      last = { kind: 'dead', status: null, detail: `запрос не удался (${attempts} попыток): ${err.message}` }
    }
  }

  return { ...last, attempts }
}

/**
 * Спрашивает у хоста заведомо несуществующий путь. Если хост отвечает на
 * него «жив», значит его 200 ничего не доказывает — ни один его адрес
 * нельзя считать подтверждённым.
 *
 * Механизм общий, а не список хостов: список устарел бы молча. Измерено
 * 2026-09-26 — так отвечают `code.videolan.org` (200, длина 0) и
 * `gitlab.freedesktop.org` (200), причём второго не было ни в одном
 * списке, пока канарейка его не нашла.
 *
 * Результат кэшируется по хосту: один лишний запрос на хост, а не на
 * адрес.
 *
 * @param {string} url любой адрес нужного хоста
 * @param {{ fetchImpl?: typeof fetch; sleepImpl?: (ms: number) => Promise<void>; canaryCache?: Map<string, object> }} [deps]
 * @returns {Promise<{ url: string; status: number | null; indistinguishable: boolean }>}
 */
export function hostCanary(url, deps = {}) {
  const cache = deps.canaryCache ?? new Map()
  const { host } = new URL(url)
  const cached = cache.get(host)
  if (cached) return cached

  // Кэшируется ОБЕЩАНИЕ, а не готовый ответ. Проверки идут параллельно
  // (CONCURRENCY), и при кэше по результату несколько адресов одного
  // хоста успевают промахнуться мимо пустого кэша раньше, чем первый из
  // них допросит канарейку: замерено — три адреса давали три запроса
  // вместо одного. С обещанием все ждут первый запрос.
  const canaryUrl = canaryUrlFor(url)
  const pending = checkUrl(canaryUrl, deps).then((verdict) => ({
    url: canaryUrl,
    status: verdict.status,
    // Только «жив» означает, что хост не различает существование. 404,
    // 403, 401 и даже упорный 5xx — не ложное подтверждение.
    indistinguishable: verdict.kind === 'ok',
  }))
  cache.set(host, pending)
  return pending
}

/**
 * Делит адреса на три списка: проверяемые (возможно, по адресу-замене),
 * пропускаемые с названной причиной (см. planProbe) и ОТКАЗЫ — адреса
 * пина, на которые сработало правило пропуска.
 *
 * Третий список — не формальность (#142). Пропущенный адрес до judge() не
 * доходит вовсе: судятся только проверенные. Значит правило «у пина нет
 * права на непроверенность» имело дыру ровно в одном классе — адрес ПИНА,
 * подошедший под правило пропуска, молча оставался зелёным. Сегодня хосты
 * пина под действующие правила не подходят, но правила будут меняться: в
 * день, когда ассет ffmpeg переедет на наше зеркало (§2.4 исследования),
 * пропуск накрыл бы главный охраняемый адрес — и сторож промолчал бы.
 *
 * Пропуск — это частный случай «не подтверждён», и прав на него у пина
 * нет: отказ приходит сюда тем же классом `warn`, каким приходит упорный
 * 5xx, и становится фатальным тем же правилом judge(), а не вторым.
 *
 * @param {Array<{ url: string; where: string[]; origins: string[] }>} entries
 */
export function planAll(entries) {
  const checked = []
  const skipped = []
  const refused = []
  for (const entry of entries) {
    const plan = planProbe(entry.url)
    if (plan.kind !== 'skip') {
      checked.push({
        ...entry,
        probeUrl: plan.probeUrl,
        why: plan.why,
        notFoundMeans: plan.notFoundMeans,
      })
      continue
    }
    if (isPinAddress(entry)) {
      refused.push({
        ...entry,
        kind: 'warn',
        detail:
          `не проверялся: сработало правило пропуска (${plan.reason}) — ` +
          'но это адрес ПИНА, который доставка скачивает на сборке, ' +
          'и «не проверен» для него то же самое, что «не подтверждён». ' +
          'Сузить правило пропуска или перепиновать источник на адрес, отвечающий по HTTP',
      })
      continue
    }
    skipped.push({ ...entry, reason: plan.reason })
  }
  return { checked, skipped, refused }
}

/**
 * @param {Array<{ url: string; probeUrl?: string; where: string[]; origins: string[] }>} entries
 * @param {{ fetchImpl?: typeof fetch; sleepImpl?: (ms: number) => Promise<void>; canaryCache?: Map<string, object> }} [deps]
 */
export async function checkAll(entries, deps = {}) {
  const canaryCache = deps.canaryCache ?? new Map()
  const withCache = { ...deps, canaryCache }
  const results = new Array(entries.length)
  let next = 0

  const worker = async () => {
    for (;;) {
      const index = next
      next += 1
      if (index >= entries.length) return
      const entry = entries[index]
      const probeUrl = entry.probeUrl ?? entry.url
      const verdict = await checkUrl(probeUrl, withCache)
      let { kind, detail } = verdict

      // Оговорка про 404 (см. planProbe): есть адреса, по которым 404
      // отсутствия не доказывает — наш приватный репозиторий отвечает им
      // и на существующий issue. Тогда это «не подтверждён», а не
      // «мёртв». Пину оговорка не помогает: у него «не подтверждён» —
      // тоже отказ, и это правильно.
      if (kind === 'dead' && verdict.status === 404 && entry.notFoundMeans) {
        kind = 'warn'
        detail = `${detail} — ${entry.notFoundMeans}`
      }

      // «Жив» засчитывается только у хоста, который умеет отвечать
      // отказом. Иначе 200 не отличает существующее от выдуманного.
      if (kind === 'ok') {
        const canary = await hostCanary(probeUrl, withCache)
        if (canary.indistinguishable) {
          kind = 'warn'
          detail =
            `${detail}, но хост отвечает так же на несуществующий путь ` +
            `(канарейка ${canary.url} → HTTP ${canary.status}): существование не подтверждено`
        }
      }

      results[index] = { ...entry, kind, detail }
    }
  }

  await Promise.all(Array.from({ length: Math.min(CONCURRENCY, entries.length) }, worker))
  return results
}

/**
 * Приговор по строгости происхождения: у пина «не подтверждён» —
 * это отказ, у документов — терпимая оговорка (Б4 ревью).
 *
 * Зелёное перечислено белым списком, отказ — всё остальное. Чёрный
 * список («отказ — это dead или warn у пина») давал классу, которого в
 * нём нет, тихий зелёный: ровно так адрес пина, не дошедший до проверки,
 * не попадал ни в один список (#142).
 *
 * Класс, которого здесь ещё нет, становится ОТКАЗОМ и у пина, и у
 * документа — терпимость заслуживает только явно названный «не
 * подтверждён». Это намеренно строже нужного: новый класс у документа
 * покраснеет и потребует решения, а не промолчит. Молчание и было
 * дефектом.
 *
 * @param {Array<{ kind: string; origins: string[] }>} results
 */
export function judge(results) {
  const alive = results.filter((result) => result.kind === 'ok')
  const tolerated = results.filter(
    (result) => result.kind === 'warn' && !isPinAddress(result),
  )
  const green = new Set([...alive, ...tolerated])
  const fatal = results.filter((result) => !green.has(result))
  return { alive, fatal, tolerated }
}

/**
 * @param {string[]} argv
 */
export function parseArgs(argv) {
  let pinOnly = false
  for (const arg of argv) {
    if (arg === '--pin') {
      pinOnly = true
      continue
    }
    throw new Error(`unknown argument: ${arg} (known: --pin)`)
  }
  return { pinOnly }
}

function report(log, title, results) {
  log(`\n── ${title} ──`)
  if (results.length === 0) {
    log('  (пусто)')
    return
  }
  const mark = { ok: 'OK   ', warn: 'WARN ', dead: 'DEAD ' }
  for (const result of results) {
    log(`${mark[result.kind] ?? `${result.kind}?`} ${result.detail}`)
    log(`      ${result.url}`)
    if (result.why) log(`      проверено заменой: ${result.probeUrl} — ${result.why}`)
    if (result.kind !== 'ok') log(`      назван в: ${result.where.join(', ')}`)
  }
}

/**
 * Весь прогон над готовым списком адресов: план, проверка, печать,
 * приговор. Вынесен из `main` отдельной функцией затем, что строгость
 * решает судьбу кода возврата, а проверять её надо ровно там, где она
 * решает: `throw` отсюда — это EXIT=1 (обработчик в хвосте файла), а
 * `main` только собирает список адресов.
 *
 * @param {Array<{ url: string; where: string[]; origins: string[] }>} entries
 * @param {{ log?: (line: string) => void; label?: string; fetchImpl?: typeof fetch; sleepImpl?: (ms: number) => Promise<void>; canaryCache?: Map<string, object> }} [options]
 */
export async function run(entries, { log = console.log, label = '', ...deps } = {}) {
  const { checked, skipped, refused } = planAll(entries)

  log(
    `check-pins: ${entries.length} адресов${label}, ` +
      `проверяем ${checked.length}, пропускаем ${skipped.length}` +
      (refused.length > 0
        ? `, отказано без проверки ${refused.length} (адреса пина под правилом пропуска)`
        : ''),
  )

  // Отказы плана идут в ОБЩИЙ список результатов, а не мимо него: они
  // печатаются и судятся наравне с проверенными. Мимо списка они и были
  // невидимы (#142).
  const results = [...(await checkAll(checked, deps)), ...refused]

  // Пин и документы печатаются врозь: у них разная строгость, и
  // сваливать их в один список — значит прятать, что именно упало.
  report(log, 'адреса пина (обязаны быть подтверждены живыми)', results.filter(isPinAddress))
  report(log, 'адреса документов §6d', results.filter((result) => !isPinAddress(result)))

  if (skipped.length > 0) {
    log('\n── пропущено (причина названа, адрес не проверялся) ──')
    for (const entry of skipped) {
      log(`SKIP  ${entry.reason}`)
      log(`      ${entry.url}`)
    }
  }

  const { alive, fatal, tolerated } = judge(results)
  log(
    `\nИтог: подтверждено живыми ${alive.length}, ` +
      `терпимо не подтверждено ${tolerated.length} (только документы), ` +
      `отказов ${fatal.length}, пропущено ${skipped.length}.`,
  )

  if (fatal.length > 0) {
    const pinFatal = fatal.filter(isPinAddress).length
    throw new Error(
      `${fatal.length} адресов не прошли проверку (из них адресов пина — ${pinFatal}). ` +
        'Перепиновать источник и обновить указатель §6d (SOURCES-FFMPEG.md, ' +
        'THIRD-PARTY-LICENSES.md) той же задачей',
    )
  }

  return { alive, fatal, tolerated, skipped, refused }
}

async function main() {
  const { pinOnly } = parseArgs(process.argv.slice(2))
  const entries = pinOnly ? mergeByUrl(collectPinUrls(await loadPin(PIN_PATH))) : await collectAllUrls()

  await run(entries, { label: pinOnly ? ' (только пин)' : '' })
}

const isMainModule = process.argv[1] === fileURLToPath(import.meta.url)
if (isMainModule) {
  try {
    await main()
  } catch (err) {
    console.error(`check-pins: ${err.message}`)
    process.exitCode = 1
  }
}
