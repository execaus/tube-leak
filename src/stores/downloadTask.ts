import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { defineStore } from 'pinia'
import { computed, onScopeDispose, ref } from 'vue'

import type {
  DownloadPlan,
  DownloadProgress,
  DownloadProgressEvent,
  DownloadStarted,
  StartDownloadRequest,
} from '@/types/download'

const START_DOWNLOAD_COMMAND = 'start_download'
const CANCEL_DOWNLOAD_COMMAND = 'cancel_download'
const RETRY_DOWNLOAD_COMMAND = 'retry_download'

/** Имя события прогресса, эмитится Rust-стороной (Ф-2, TL-38/TL-44). */
const PROGRESS_EVENT_NAME = 'download://progress'

/** Порог локального косметического индикатора зависания (дизайн E3, «Числа»). */
const SOFT_STALL_THRESHOLD_MS = 5_000
const SOFT_STALL_TICK_MS = 1_000

/** То, что панель держит про текущую задачу помимо серверного `DownloadProgress`. */
export interface DownloadTask {
  taskId: string
  plan: DownloadPlan
  /**
   * Заголовок панели, снятый с карточки ролика **в момент клика** «Скачать»
   * (требование С-13/TL-45, п.6) — не читается из карточки повторно, поэтому
   * переживает замену карточки на новую ссылку.
   */
  displayTitle: string
}

function isTerminalPhase(progress: DownloadProgress): boolean {
  return progress.phase === 'done' || progress.phase === 'failed' || progress.phase === 'cancelled'
}

/**
 * Домен «текущая загрузка» (эпик E3, TL-45): один Pinia-стор на домен
 * (CLAUDE.md), переживающий замену `VideoCard` и используемый и панелью, и
 * (с TL-46) диалогом подтверждения выхода — общее состояние, а не удобство
 * изложения.
 *
 * # Почему это стор, а не composable, локальный компоненту
 *
 * Задача должна пережить размонтирование карточки, которая её запустила
 * (С-13), и её должен видеть код вне панели (кнопка «Скачать» на
 * возможно уже другой карточке, будущий диалог выхода TL-46) — состояние
 * не принадлежит времени жизни одного компонента.
 *
 * # Поток событий после отказа (требование TL-45, п.3)
 *
 * Подписка на `download://progress` живёт всё время жизни стора (эффективно
 * — всё приложение) и никогда не отписывается и не фильтруется по фазе:
 * `failed` не глушит подписку, `retry()` лишь просит ядро продолжить ту же
 * задачу, а новые события того же `taskId` продолжают приходить в тот же
 * `progress` без каких-либо дополнительных действий на этой стороне (doc
 * `DownloadProgressEvent` в `src/types/download.ts`).
 */
export const useDownloadTaskStore = defineStore('downloadTask', () => {
  const task = ref<DownloadTask>()
  const progress = ref<DownloadProgress>()

  /**
   * Секунды без события — только пока идёт `downloading`/`running`
   * (единственное состояние, где есть скорость, которую нечестно было бы
   * продолжать показывать замороженной, дизайн E3 «Три разных нет
   * движения», пункт 1). `undefined` — индикатор не показан.
   */
  const softStallSeconds = ref<number>()

  let unlisten: UnlistenFn | undefined
  let listening: Promise<void> | undefined
  let lastEventAt = 0
  let stallTimer: ReturnType<typeof setInterval> | undefined

  function clearStallTimer(): void {
    if (stallTimer !== undefined) {
      clearInterval(stallTimer)
      stallTimer = undefined
    }
    softStallSeconds.value = undefined
  }

  function noteActivity(): void {
    lastEventAt = Date.now()
    softStallSeconds.value = undefined
  }

  function ensureStallTimer(): void {
    if (stallTimer !== undefined) return
    stallTimer = setInterval(() => {
      const current = progress.value
      if (!current || current.phase !== 'downloading' || current.state !== 'running') {
        clearStallTimer()
        return
      }
      const elapsedMs = Date.now() - lastEventAt
      softStallSeconds.value =
        elapsedMs >= SOFT_STALL_THRESHOLD_MS ? Math.floor(elapsedMs / 1000) : undefined
    }, SOFT_STALL_TICK_MS)
  }

  function handleProgressEvent(event: { payload: DownloadProgressEvent }): void {
    const current = task.value
    // Ключ задачи — `taskId`; сверяем его, а не считаем любое пришедшее
    // событие «своим» (doc `DownloadProgressEvent`).
    if (!current || event.payload.taskId !== current.taskId) return

    // `taskId` — только ключ маршрутизации события, в состояние панели не
    // входит (панель уже знает id из своей задачи); явно отделяем его от
    // остальных полей, а не оставляем висеть лишним свойством на объекте.
    const { taskId, ...rest } = event.payload
    void taskId
    progress.value = rest

    if (rest.phase === 'downloading' && rest.state === 'running') {
      noteActivity()
      ensureStallTimer()
    } else {
      clearStallTimer()
    }
  }

  function ensureListening(): Promise<void> {
    if (!listening) {
      listening = listen<DownloadProgressEvent>(PROGRESS_EVENT_NAME, handleProgressEvent)
        .then((fn) => {
          unlisten = fn
        })
        .catch((err: unknown) => {
          // Сбрасываем memoization — следующий `start()` должен снова
          // попытаться подписаться, а не унаследовать отклонённый промис
          // навсегда (тот же приём, что `useYtDlpPrepare`, эпик E1).
          listening = undefined
          throw err
        })
    }
    return listening
  }

  /**
   * Слот занят, пока задача существует и не в терминальной фазе — по этому
   * полю кнопка «Скачать» на карточке решает, показывать ли подсказку С-13.
   * Мгновенно после `start()` `progress` уже заполнен (`queued`), поэтому
   * «задача создана, но фаза ещё не пришла» здесь не бывает.
   */
  const isActive = computed(() => {
    const current = progress.value
    return task.value !== undefined && current !== undefined && !isTerminalPhase(current)
  })

  /**
   * Запускает новую задачу. Слот уже должен быть свободен (кнопка на
   * карточке это гарантирует) — ядро всё равно проверяет это само
   * (`alreadyActive`) и является единственным источником истины (дизайн
   * E3, «Кнопка Скачать»): отказ здесь просто логируется, не выбрасывается
   * наверх — исправный фронтенд такую кнопку не показывает.
   */
  async function start(request: StartDownloadRequest, displayTitle: string): Promise<void> {
    try {
      await ensureListening()
      const started = await invoke<DownloadStarted>(START_DOWNLOAD_COMMAND, { request })
      task.value = { taskId: started.taskId, plan: started.plan, displayTitle }
      // `DownloadStarted.phase` — всегда `'queued'` в E3 (doc-комментарий
      // контракта): единственный вариант объединения, конструируемый без
      // дополнительных полей, поэтому литерал, а не `{ phase: started.phase }`
      // (тот не сузился бы до конкретного варианта union).
      progress.value = { phase: 'queued' }
      clearStallTimer()
    } catch (err) {
      console.error('start_download rejected', err)
    }
  }

  /** Отмена — доступна в любой нетерминальной фазе (Ф-4). */
  async function cancel(): Promise<void> {
    const current = task.value
    if (!current) return
    try {
      await invoke<void>(CANCEL_DOWNLOAD_COMMAND, { taskId: current.taskId })
    } catch (err) {
      console.error('cancel_download rejected', err)
    }
  }

  /**
   * Повтор — продолжение той же задачи (тот же `taskId`), не новая
   * сущность (дизайн E3, «Управляющие вызовы», п.3). Поток событий
   * возобновляется сам, без какого-либо сброса состояния здесь.
   */
  async function retry(): Promise<void> {
    const current = task.value
    if (!current) return
    try {
      await invoke<void>(RETRY_DOWNLOAD_COMMAND, { taskId: current.taskId })
    } catch (err) {
      console.error('retry_download rejected', err)
    }
  }

  /** «Скрыть» — освобождает слот панели; допустимо только для терминальной задачи. */
  function hide(): void {
    const current = progress.value
    if (current && isTerminalPhase(current)) {
      task.value = undefined
      progress.value = undefined
      clearStallTimer()
    }
  }

  onScopeDispose(() => {
    clearStallTimer()
    if (unlisten) unlisten()
  })

  return { task, progress, softStallSeconds, isActive, start, cancel, retry, hide }
})
