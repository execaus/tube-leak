import { invoke } from '@tauri-apps/api/core'
import { listen, type Event as TauriEvent, type UnlistenFn } from '@tauri-apps/api/event'
import { onUnmounted, ref, type Ref } from 'vue'

import type { YtDlpPrepareError, YtDlpPrepareEvent, YtDlpPrepared, YtDlpPrepareStage } from '@/types/ytdlp'

const PREPARE_YTDLP_COMMAND = 'prepare_ytdlp'

/**
 * Имя события хода подготовки, эмитится Rust-стороной под этим именем
 * (`crate::ytdlp::prepare::PREPARE_EVENT`, зеркалируется здесь, а не
 * импортируется — граница Rust↔TS не делится константами напрямую).
 */
const PREPARE_EVENT_NAME = 'ytdlp://prepare'

/**
 * Единственная точка входа фронтенда к Tauri-команде `prepare_ytdlp`
 * (TL-17). Фронтенд не запускает процессы и не трогает файловую систему
 * напрямую (CLAUDE.md, Ф-3) — только через `invoke`. Вызывать безопасно
 * всегда: обе двери внутрь подготовки (эта команда и автозапуск при
 * старте приложения) ведут в одну идемпотентную функцию под мьютексом на
 * Rust-стороне, поэтому повторный вызов дождётся уже идущей подготовки.
 */
export async function prepareYtDlp(): Promise<YtDlpPrepared> {
  return invoke<YtDlpPrepared>(PREPARE_YTDLP_COMMAND)
}

export interface UseYtDlpPrepareReturn {
  /** Последний этап из события `ytdlp://prepare`; `undefined` — событий ещё не было. */
  stage: Ref<YtDlpPrepareStage | undefined>
  /** Сквозной прогресс подготовки (0..100), из последнего полученного события. */
  percent: Ref<number>
  etaSecs: Ref<number | undefined>
  /** Итог успешной подготовки — заполняется по разрешению промиса команды. */
  result: Ref<YtDlpPrepared | undefined>
  /** Типизированная ошибка подготовки — заполняется по реджекту промиса команды. */
  error: Ref<YtDlpPrepareError | undefined>
  /** `true` пока промис текущего вызова `prepare()` не разрешился. */
  isPending: Ref<boolean>
  prepare: () => Promise<void>
}

/**
 * Composable подготовки yt-dlp (TL-17): подписывается на событие хода
 * `ytdlp://prepare` и оборачивает команду `prepare_ytdlp`.
 *
 * # Порядок вызовов — критично
 *
 * Подписка на событие устанавливается и дожидается **до** вызова
 * `prepare_ytdlp` — иначе на первом запуске можно пропустить ранние
 * события (распаковка занимает ~1,3 с, событие может прийти почти сразу
 * после вызова команды). Именно поэтому `prepare()` сам управляет
 * подпиской, а не полагается на внешний `onMounted` вызывающего
 * компонента: два независимых `onMounted` не дают гарантии порядка между
 * собой, а гонка здесь означает пропущенный прогресс, а не просто
 * визуальную мелочь.
 *
 * Подписка живёт от первого вызова `prepare()` до размонтирования
 * компонента, использующего composable (`onUnmounted` снимает слушатель) —
 * повторные вызовы (retry по кнопке после ошибки) переиспользуют её, а не
 * создают новую при каждой попытке.
 */
export function useYtDlpPrepare(): UseYtDlpPrepareReturn {
  const stage = ref<YtDlpPrepareStage>()
  const percent = ref(0)
  const etaSecs = ref<number>()
  const result = ref<YtDlpPrepared>()
  const error = ref<YtDlpPrepareError>()
  const isPending = ref(false)

  let unlisten: UnlistenFn | undefined
  let listening: Promise<void> | undefined

  function handleEvent(event: TauriEvent<YtDlpPrepareEvent>): void {
    stage.value = event.payload.stage
    percent.value = event.payload.percent
    etaSecs.value = event.payload.etaSecs
  }

  function ensureListening(): Promise<void> {
    if (!listening) {
      listening = listen<YtDlpPrepareEvent>(PREPARE_EVENT_NAME, handleEvent).then((fn) => {
        unlisten = fn
      })
    }
    return listening
  }

  async function prepare(): Promise<void> {
    isPending.value = true
    error.value = undefined
    stage.value = undefined
    percent.value = 0
    etaSecs.value = undefined

    // Ждём подтверждения подписки, прежде чем звать команду (см. doc выше) —
    // это и есть гарантия «check_sidecar не раньше разрешения prepare_ytdlp»
    // на уровне пропуска событий: без неё возможен пропуск unpacking на
    // самом первом запуске.
    await ensureListening()

    try {
      result.value = await prepareYtDlp()
    } catch (err) {
      error.value = err as YtDlpPrepareError
    } finally {
      isPending.value = false
    }
  }

  onUnmounted(() => {
    unlisten?.()
  })

  return { stage, percent, etaSecs, result, error, isPending, prepare }
}
