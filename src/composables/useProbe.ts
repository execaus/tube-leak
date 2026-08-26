import { invoke } from '@tauri-apps/api/core'
import { onUnmounted, ref, watch, type Ref } from 'vue'

import type { ProbeError, ProbeErrorKind, ProbeResult } from '@/types/probe'
import { looksLikeUrl } from '@/utils/looksLikeUrl'

const PROBE_URL_COMMAND = 'probe_url'
const CANCEL_PROBE_COMMAND = 'cancel_probe'

/**
 * Единственная точка входа фронтенда к команде `probe_url` (эпик E2,
 * TL-32 — команда ещё не существует на момент TL-33, экран строится
 * против контракта TL-29, см. бриф TL-33). Фронтенд не запускает процессы
 * и не трогает файловую систему напрямую (CLAUDE.md) — только `invoke`.
 */
export async function probeUrl(url: string): Promise<ProbeResult> {
  return invoke<ProbeResult>(PROBE_URL_COMMAND, { url })
}

/**
 * Отмена текущего разбора без старта нового (дизайн E2, «Управляющие
 * вызовы») — нужна отдельно от {@link probeUrl}, потому что очистка поля
 * не сопровождается новой ссылкой, которую можно было бы передать вместо
 * неё.
 */
export async function cancelProbe(): Promise<void> {
  return invoke<void>(CANCEL_PROBE_COMMAND)
}

const KNOWN_ERROR_KINDS: readonly ProbeErrorKind[] = [
  'notAUrl',
  'videoUnavailable',
  'signInRequired',
  'regionBlocked',
  'networkUnavailable',
  'playlistUnsupported',
  'liveUnsupported',
  'ytDlpFailure',
  'timeout',
]

/**
 * То, что реально может оказаться отказом `probe_url`. Контрактный путь
 * честный ({@link ProbeError}), но неконтрактный отказ существует (паника
 * команды, отказ самого IPC-вызова) и должен быть учтён без каста вслепую
 * — тот же приём, что `PrepareFailure` в `useYtDlpPrepare.ts` (E1).
 */
export type ProbeFailure = ProbeError | { kind?: undefined; message: string }

function isProbeError(value: unknown): value is ProbeError {
  if (typeof value !== 'object' || value === null) return false
  const candidate = value as Record<string, unknown>
  return (
    typeof candidate.kind === 'string' &&
    (KNOWN_ERROR_KINDS as readonly string[]).includes(candidate.kind) &&
    typeof candidate.message === 'string'
  )
}

function toProbeFailure(err: unknown): ProbeFailure {
  if (isProbeError(err)) return err
  if (err instanceof Error) return { message: err.message }
  if (typeof err === 'string' && err.length > 0) return { message: err }
  return { message: 'Разбор ролика завершился нераспознанной ошибкой.' }
}

/**
 * Состояние карточной области экрана (дизайн E2, раздел «Состояния») —
 * ровно одно одновременно с полем ссылки.
 */
export type LinkProbeState =
  | { kind: 'empty' }
  | { kind: 'notAUrl' }
  | { kind: 'loading'; slow: boolean }
  | { kind: 'success'; result: ProbeResult }
  | { kind: 'error'; error: ProbeFailure }

/** Задержка тишины ввода перед стартом разбора (дизайн E2 — стартовая точка, подлежит калибровке на bundle). */
const DEBOUNCE_MS = 400
/** Порог, после которого «Получаем данные…» обзаводится второй строкой пояснения. */
const SLOW_HINT_MS = 6000

export interface UseLinkProbeReturn {
  url: Ref<string>
  state: Ref<LinkProbeState>
  /** Повторяет разбор для текущего содержимого поля немедленно (без ожидания тишины ввода). */
  retry: () => void
}

/**
 * Composable экрана разбора ссылки (TL-33, эпик E2).
 *
 * # Одно общее правило отмены (дизайн E2, «Триггер разбора и отмена»)
 *
 * {@link evaluate} — единственное место, решающее судьбу поля по его
 * текущему содержимому: пусто → пустое состояние; не похоже на ссылку →
 * мгновенная ошибка без запуска процесса; похоже на ссылку → карточная
 * область немедленно переходит в «получаем данные…» (старая
 * карточка/ошибка не остаётся видна ни на долю секунды — «замена
 * карточки, а не наложение поверх старой»), а сам вызов `probeUrl`
 * откладывается на ~400 мс тишины ввода (кроме retry — тот стартует
 * немедленно). Очистка поля, вторая ссылка и правка уже вставленной — не
 * три ветки, а один и тот же путь с разным содержимым на входе.
 *
 * # Сторож по поколениям (обязателен, TL-27 review, К-4)
 *
 * Наивная защита «есть ли активный запрос» не закрывает случай, когда
 * разбор A успевает завершиться в ядре **раньше**, чем вызов по ссылке B
 * вообще доходит до ядра: ответ на A приходит совершенно легитимным
 * успехом уже после того, как поле показывает B. Сравнение по URL тоже не
 * годится — одна и та же ссылка, вставленная дважды подряд, и расхождения
 * нормализации ломают его.
 *
 * Поэтому здесь — монотонный счётчик `generation`, увеличиваемый в
 * {@link evaluate} на **каждое** решение по содержимому поля (на каждое
 * изменение — evaluate вызывается и для мгновенных веток, и для той, что
 * готовит debounce). Каждый вызов {@link dispatchProbe} запоминает
 * поколение, с которым он был запущен, и при разрешении/отклонении
 * промиса `probeUrl` сверяет его с текущим — несовпадение отбрасывает
 * результат целиком, **и resolve, и reject**, не трогая `state`. Ключ —
 * именно счётчик, не URL: одна и та же ссылка, вставленная дважды, и
 * расхождения нормализации не ломают сравнение.
 */
export function useLinkProbe(): UseLinkProbeReturn {
  const url = ref('')
  const state = ref<LinkProbeState>({ kind: 'empty' })

  let generation = 0
  let inFlight = false
  let debounceTimer: ReturnType<typeof setTimeout> | undefined
  let slowTimer: ReturnType<typeof setTimeout> | undefined

  function clearDebounceTimer(): void {
    if (debounceTimer !== undefined) {
      clearTimeout(debounceTimer)
      debounceTimer = undefined
    }
  }

  function clearSlowTimer(): void {
    if (slowTimer !== undefined) {
      clearTimeout(slowTimer)
      slowTimer = undefined
    }
  }

  /** Отменяет фактически запущенный разбор, если он есть — не более того («Экономия вызовов»). */
  function cancelIfInFlight(): void {
    if (inFlight) {
      inFlight = false
      void cancelProbe().catch(() => {})
    }
  }

  /** Собственно вызов `probeUrl` — то, что задерживается debounce'ом (не сам переход в «получаем данные…»). */
  function dispatchProbe(myGeneration: number, value: string): void {
    inFlight = true

    slowTimer = setTimeout(() => {
      // Поколение могло уже смениться, пока не сработал этот таймер.
      if (myGeneration !== generation) return
      state.value = { kind: 'loading', slow: true }
    }, SLOW_HINT_MS)

    probeUrl(value)
      .then((result) => {
        // Сторож по поколениям: отбрасываем resolve не текущего поколения
        // (см. doc {@link useLinkProbe}) — легитимный успех для ссылки,
        // которой в поле уже нет, не должен попасть на экран.
        if (myGeneration !== generation) return
        inFlight = false
        clearSlowTimer()
        state.value = { kind: 'success', result }
      })
      .catch((err: unknown) => {
        // И reject тоже отбрасывается тем же способом.
        if (myGeneration !== generation) return
        inFlight = false
        clearSlowTimer()
        state.value = { kind: 'error', error: toProbeFailure(err) }
      })
  }

  function evaluate(value: string, opts: { immediate: boolean }): void {
    clearDebounceTimer()
    clearSlowTimer()
    generation += 1
    const myGeneration = generation

    const trimmed = value.trim()

    if (trimmed.length === 0) {
      cancelIfInFlight()
      state.value = { kind: 'empty' }
      return
    }

    if (!looksLikeUrl(trimmed)) {
      cancelIfInFlight()
      state.value = { kind: 'notAUrl' }
      return
    }

    // Похоже на ссылку — карточная область переходит в «получаем данные…»
    // немедленно, а не после тишины ввода: «карточная область немедленно
    // показывает» (дизайн) относится к решению по содержимому поля в
    // целом, а старая карточка/ошибка не обязана оставаться видна ни на
    // долю секунды дольше необходимого («замена карточки, а не наложение
    // поверх старой»). Задерживается debounce'ом только сам вызов
    // `probeUrl` — чтобы не запускать yt-dlp на каждое нажатие клавиши.
    state.value = { kind: 'loading', slow: false }

    if (opts.immediate) {
      dispatchProbe(myGeneration, trimmed)
    } else {
      debounceTimer = setTimeout(() => {
        dispatchProbe(myGeneration, trimmed)
      }, DEBOUNCE_MS)
    }
  }

  watch(url, (value) => {
    evaluate(value, { immediate: false })
  })

  /** Повтор по кнопке «Повторить» — сознательно без ожидания тишины ввода. */
  function retry(): void {
    evaluate(url.value, { immediate: true })
  }

  onUnmounted(() => {
    clearDebounceTimer()
    clearSlowTimer()
  })

  return { url, state, retry }
}
