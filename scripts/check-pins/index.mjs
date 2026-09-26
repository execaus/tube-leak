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
// Что ловит: исчезновение чужого ассета (у сборщиков ffmpeg теги
// ротируются, см. _note записей пина) и протухание указателя §6d GPL v3.
// Чего НЕ ловит: подмену файла под тем же адресом — это работа sha256 в
// пине, её проверяет доставка (scripts/fetch-binaries).
//
// Офлайн-часть сторожа — в sources.mjs (crossCheckDocs) и идёт в обычном
// `npm test`: она требует, чтобы документы называли сборки АДРЕСАМИ, а не
// именами файлов. Без неё эта команда была бы слепа ровно там, где #140.

import { fileURLToPath } from 'node:url'

import { collectAllUrls, collectPinUrls, mergeByUrl, PIN_PATH, planProbe } from './sources.mjs'
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
 * паузы растут. Без повторов релизный гейт краснел на ровном месте
 * (замечание Б3 ревью TL-133), а сторож, красный от случайностей,
 * перестают читать — это принцип из шапки этого же файла.
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
 * они стабильны, поэтому попытка одна.
 *
 * Границу это не размывает: пропажа ассета у GitHub — 404, и она
 * остаётся отказом. Именно так выглядел #140.
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
          detail: `HTTP ${status} — хост ответил, но этому клиенту не отдаёт (ресурс не опровергнут)`,
          attempts,
        }
      }
      if (status >= 500) {
        last = {
          kind: 'warn',
          status,
          detail: `HTTP ${status} — хост временно недоступен, ${attempts} попыток подряд (ресурс не опровергнут)`,
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
 * Делит адреса на проверяемые (возможно, по адресу-замене) и
 * пропускаемые с названной причиной — см. planProbe.
 *
 * @param {Array<{ url: string; where: string[] }>} entries
 */
export function planAll(entries) {
  const checked = []
  const skipped = []
  for (const entry of entries) {
    const plan = planProbe(entry.url)
    if (plan.kind === 'skip') skipped.push({ ...entry, reason: plan.reason })
    else checked.push({ ...entry, probeUrl: plan.probeUrl, why: plan.why })
  }
  return { checked, skipped }
}

/**
 * @param {Array<{ url: string; probeUrl: string; where: string[] }>} entries
 * @param {{ fetchImpl?: typeof fetch; sleepImpl?: (ms: number) => Promise<void> }} [deps]
 */
export async function checkAll(entries, deps = {}) {
  const results = new Array(entries.length)
  let next = 0

  const worker = async () => {
    for (;;) {
      const index = next
      next += 1
      if (index >= entries.length) return
      const entry = entries[index]
      const { kind, detail } = await checkUrl(entry.probeUrl ?? entry.url, deps)
      results[index] = { ...entry, kind, detail }
    }
  }

  await Promise.all(Array.from({ length: Math.min(CONCURRENCY, entries.length) }, worker))
  return results
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

async function main() {
  const { pinOnly } = parseArgs(process.argv.slice(2))
  const entries = pinOnly ? mergeByUrl(collectPinUrls(await loadPin(PIN_PATH))) : await collectAllUrls()
  const { checked, skipped } = planAll(entries)

  console.log(
    `check-pins: ${entries.length} адресов${pinOnly ? ' (только пин)' : ''}, ` +
      `проверяем ${checked.length}, пропускаем ${skipped.length}\n`,
  )

  const results = await checkAll(checked)
  const mark = { ok: 'OK   ', warn: 'WARN ', dead: 'DEAD ' }

  for (const result of results) {
    console.log(`${mark[result.kind]} ${result.detail}`)
    console.log(`      ${result.url}`)
    if (result.why) console.log(`      проверено заменой: ${result.probeUrl} — ${result.why}`)
    if (result.kind !== 'ok') console.log(`      назван в: ${result.where.join(', ')}`)
  }

  // Пропуски печатаются всегда: молча выкинутый адрес ничем не отличается
  // от непроверенного, а сторож обязан отчитываться о собственных границах.
  for (const entry of skipped) {
    console.log(`SKIP  ${entry.reason}`)
    console.log(`      ${entry.url}`)
  }

  const dead = results.filter((result) => result.kind === 'dead')
  const warned = results.filter((result) => result.kind === 'warn')
  console.log(
    `\nИтог: живых ${results.length - dead.length - warned.length}, ` +
      `не подтверждённых ${warned.length}, мёртвых ${dead.length}, пропущено ${skipped.length}.`,
  )

  if (dead.length > 0) {
    throw new Error(
      `${dead.length} адресов недоступны — перепиновать источник и обновить указатель §6d ` +
        '(SOURCES-FFMPEG.md, THIRD-PARTY-LICENSES.md) той же задачей',
    )
  }
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
