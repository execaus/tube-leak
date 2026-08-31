import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { defineStore } from 'pinia'
import { computed, onScopeDispose, ref } from 'vue'

import type {
  DownloadCommandError,
  DownloadPhase,
  DownloadPlan,
  DownloadProgress,
  DownloadProgressEvent,
  DownloadStarted,
  StartDownloadRequest,
} from '@/types/generated/download'
import type { QueuePauseReason, QueueSnapshot, QueueTask } from '@/types/generated/queue'
import { knownKindsOf } from '@/utils/knownKinds'
import { toDownloadProgress } from '@/utils/queueTaskProgress'
import { formatTaskDisplayTitle } from '@/utils/queueTaskTitle'

const START_DOWNLOAD_COMMAND = 'start_download'
const CANCEL_DOWNLOAD_COMMAND = 'cancel_download'
const RETRY_DOWNLOAD_COMMAND = 'retry_download'
const RESUME_QUEUE_COMMAND = 'resume_queue'
const DISMISS_QUEUE_TASK_COMMAND = 'dismiss_queue_task'
const QUEUE_STATE_COMMAND = 'queue_state'

/** Имя события прогресса, эмитится Rust-стороной (Ф-2, TL-38/TL-44). По id задачи — без изменений в эпике E4 (Ф-6). */
const PROGRESS_EVENT_NAME = 'download://progress'

/**
 * Имя события состава очереди (контракт TL-70, дизайн E4 «Данные для
 * API»): полный снимок на любое структурное изменение (постановка,
 * старт, терминальный исход, скрытие, продолжение после паузы, начало/
 * конец паузы на обновление) — не дельта.
 */
const QUEUE_CHANGED_EVENT = 'queue://changed'

/** Порог локального косметического индикатора зависания (дизайн E3, «Числа»). */
const SOFT_STALL_THRESHOLD_MS = 5_000
const SOFT_STALL_TICK_MS = 1_000

/** То, что панель держит про текущую задачу помимо серверного `DownloadProgress`. */
export interface DownloadTask {
  taskId: string
  plan: DownloadPlan
  /**
   * Заголовок панели — ««Название» — качество» ({@link formatTaskDisplayTitle}).
   * Для активной задачи очереди строится из тех же двух полей
   * ({@link QueueTask.title}/{@link QueueTask.quality}), что переживают
   * перезапуск приложения (Ф-9 E4) — тем самым заголовок остаётся верным
   * и после восстановления по снимку, а не только сразу после клика
   * «Скачать» (требование С-13/TL-45, п.6, унаследованное TL-75).
   */
  displayTitle: string
}

/**
 * Белый список семи классов `DownloadCommandError['kind']`, выведенный из
 * сгенерированного типа (TL-52, см. doc `@/utils/knownKinds`) — тот же
 * приём, что и `KNOWN_ERROR_KINDS` в `useProbe.ts`/`useYtDlpPrepare.ts`.
 *
 * С TL-70 `DownloadCommandErrorKind` стал размеченным объединением
 * объектов (несёт `existing` у `duplicateTask`), поэтому список строится
 * по `DownloadCommandError['kind']` — строковому union тегов, извлечённому
 * индексированным доступом, а не по имени типа целиком (правило CLAUDE.md
 * о `satisfies` при разметке объединений).
 */
const KNOWN_COMMAND_ERROR_KINDS = knownKindsOf({
  unknownTask: true,
  notFailed: true,
  notRetryable: true,
  noStreamsSelected: true,
  invalidUrl: true,
  duplicateTask: true,
  taskNotFinished: true,
} satisfies Record<DownloadCommandError['kind'], true>)

function isDownloadCommandError(value: unknown): value is DownloadCommandError {
  if (typeof value !== 'object' || value === null) return false
  const candidate = value as Record<string, unknown>
  return (
    typeof candidate.kind === 'string' &&
    (KNOWN_COMMAND_ERROR_KINDS as readonly string[]).includes(candidate.kind) &&
    typeof candidate.message === 'string'
  )
}

/**
 * То, что реально может оказаться отказом одной из пяти команд управления
 * загрузкой/очередью (`start_download`/`cancel_download`/`retry_download`/
 * `resume_queue`/`dismiss_queue_task`). Контрактный путь честный
 * ({@link DownloadCommandError}), но неконтрактный отказ существует
 * (паника команды, отказ самого IPC-вызова) и должен быть учтён без каста
 * вслепую — тот же приём, что `ProbeFailure` в `useProbe.ts` (эпик E2) и
 * `PrepareFailure` в `useYtDlpPrepare.ts` (эпик E1).
 */
export type DownloadCommandFailure = DownloadCommandError | { kind?: undefined; message: string }

function toDownloadCommandFailure(err: unknown): DownloadCommandFailure {
  if (isDownloadCommandError(err)) return err
  if (err instanceof Error) return { message: err.message }
  if (typeof err === 'string' && err.length > 0) return { message: err }
  return { message: 'Команда управления загрузкой отклонена по нераспознанной причине.' }
}

function isTerminalPhase(phase: DownloadPhase): boolean {
  return phase === 'done' || phase === 'failed' || phase === 'cancelled'
}

/**
 * Начальное представление фазы задачи сразу после `start_download` —
 * строится по фазе из ответа команды, а не константой (ревью TL-45):
 * doc-комментарий `DownloadStarted.phase` в
 * `src/types/generated/download.ts` прямо требует рисовать по присланному
 * полю, потому что с эпика очереди (E4) задача может реально задержаться
 * в `queued`, ожидая слот, — сегодня это и есть штатный путь (Ф-2), не
 * оборона на будущее.
 *
 * Форма результата — ровно {@link DownloadProgress} (не вычисленный
 * `Omit<QueueTask, ...>`): TS расщепляет `spread` размеченного
 * объединения в новом литерале, теряя связь тега с полями конкретного
 * варианта (`phase: 'cancelled'` перестаёт требовать `partialData` рядом)
 * — аннотация целым готовым union-типом эту проблему обходит, `Omit` над
 * пересечением с тем же типом — нет (проверено мутацией при разработке:
 * `Omit`-вариант не собирался на ветке `downloading`).
 */
function initialQueueTaskPhaseFields(phase: DownloadPhase): DownloadProgress {
  switch (phase) {
    case 'queued':
      return { phase: 'queued' }
    case 'fetching':
      return { phase: 'fetching' }
    case 'downloading':
      return { phase: 'downloading', state: 'running' }
    case 'merging':
      return { phase: 'merging' }
    // Ветки для терминальных фаз — оборона на случай будущего расширения
    // контракта, а не ожидаемый путь (задача только что создана): ближайшая
    // честная трактовка — «ещё не начиналась».
    case 'done':
    case 'failed':
    case 'cancelled':
      return { phase: 'queued' }
  }
}

/**
 * Домен «очередь загрузок» (эпик E3 → E4, TL-45 → TL-75): один
 * Pinia-стор на домен (CLAUDE.md). До TL-75 держал ровно одну задачу; с
 * эпиком очереди источник истины — упорядоченный список `tasks`,
 * зеркалящий снимок ядра (`QueueSnapshot`, контракт TL-70) — стор здесь
 * снова проекция, а не хозяин данных (инвариант CLAUDE.md).
 *
 * # Два разных способа узнать о задаче — и почему оба нужны
 *
 * 1. **Оптимистичная вставка в `start()`.** Ответ `start_download`
 *    (`DownloadStarted`) достаточен, чтобы собрать новую запись списка
 *    сразу — ждать отдельного `queue://changed` ради задачи, которую сам
 *    же только что поставил, было бы лишним кругом и лишней гонкой
 *    (тот же приём, что уже был в E3 до появления снимка очереди).
 * 2. **Полный снимок по `queue_state`/`queue://changed`.** Всё остальное
 *    (продвижение по фазам ожидающих задач, скрытие, продолжение после
 *    паузы, пауза на обновление yt-dlp) видно **только** отсюда — Ф-6
 *    прямо ограничивает `download://progress` активной задачей
 *    («ожидающие молчат»), а список — не то, что можно накопить из потока
 *    прогресса (дизайн E4, «Данные для API»: «не по накопленным событиям
 *    прогресса», С-9). `initialize()` — разовый запрос на маунте плюс
 *    подписка на событие, тот же приём, что `check_sidecar` (Ф-9 E1) и
 *    `useYtDlpUpdate` (эпик E6): не polling.
 *
 * # Обратная совместимость: `task`/`progress`/`isActive`
 *
 * Три производных геттера сохранены буквально ради существующих
 * потребителей вне этого файла (`useExitConfirmation.ts`/TL-46) —
 * TL-76 переведёт диалог выхода на срез всей очереди отдельной задачей;
 * до тех пор они остаются проекцией **активной** задачи списка (ровно
 * одной, Р-1: `phase` не `queued` и не терминальна) — то же самое
 * значение, которое эти поля несли до TL-75, когда задача была ровно
 * одна.
 *
 * # Окно двойного клика — на весь стор, не на задачу
 *
 * `commandInFlight` — общий флаг на пять команд (`start`/`cancel`/
 * `retry`/`resume`/`hide`), как и в E3 (тогда команд было три). Точность
 * «блокировать только повтор той же самой команды над той же задачей»
 * потребовала бы состояния на ключ задачи ради сценария, которого нет в
 * критериях приёмки (К-1 — единицы задач в очереди, двойной клик по двум
 * разным строкам одновременно не описан ни одним требованием); простой
 * общий флаг предсказуем и достаточен.
 */
export const useDownloadTaskStore = defineStore('downloadTask', () => {
  const tasks = ref<QueueTask[]>([])
  const commandError = ref<DownloadCommandFailure>()

  /**
   * Очередь восстановлена после перезапуска приостановленной (Р-3) и ещё
   * ни разу не запускалась в этом сеансе — зеркало
   * `QueueSnapshot.awaitingContinue`, обновляется только снимком/событием
   * (см. doc класса выше, «Полный снимок»).
   */
  const awaitingContinue = ref(false)

  /** Планировщик держит паузу между задачами (Р-7) — зеркало `QueueSnapshot.pauseReason`. */
  const pauseReason = ref<QueuePauseReason>()

  /**
   * Секунды без события — только пока активная задача в
   * `downloading`/`running` (единственное состояние, где есть скорость,
   * которую нечестно было бы продолжать показывать замороженной, дизайн
   * E3 «Три разных нет движения», пункт 1). `undefined` — индикатор не
   * показан.
   */
  const softStallSeconds = ref<number>()

  let unlistenProgress: UnlistenFn | undefined
  let listeningProgress: Promise<void> | undefined
  let unlistenQueue: UnlistenFn | undefined
  let listeningQueue: Promise<void> | undefined
  let lastEventAt = 0
  let stallTimer: ReturnType<typeof setInterval> | undefined
  // Окно двойного клика (doc класса выше, «Окно двойного клика»).
  let commandInFlight = false

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
      const current = firstTask.value
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
    const { taskId, ...rest } = event.payload
    const index = tasks.value.findIndex((t) => t.taskId === taskId)
    // Ключ задачи — `taskId`; сверяем его, а не считаем любое пришедшее
    // событие «своим» (doc `DownloadProgressEvent`). Не найдена — либо
    // ещё не отражена в списке (гонка с очень ранним прогрессом,
    // недостижимо по контракту: `start_download` резолвится раньше
    // первого прогресса), либо уже скрыта.
    if (index === -1) return

    const current = tasks.value[index]!
    // `as QueueTask` — тот же приём и то же основание, что в `start()`
    // (doc `initialQueueTaskPhaseFields` выше): `rest` уже сузился до
    // конкретного варианта `DownloadProgress` пришедшим событием, спред
    // лишь теряет это в выводе типов литерала, не в рантайме.
    const updated = {
      taskId: current.taskId,
      title: current.title,
      quality: current.quality,
      plan: current.plan,
      ...rest,
    } as QueueTask
    tasks.value = tasks.value.map((t, i) => (i === index ? updated : t))

    if (rest.phase === 'downloading' && rest.state === 'running') {
      noteActivity()
      ensureStallTimer()
    } else {
      clearStallTimer()
    }
  }

  function ensureProgressListening(): Promise<void> {
    if (!listeningProgress) {
      listeningProgress = listen<DownloadProgressEvent>(PROGRESS_EVENT_NAME, handleProgressEvent)
        .then((fn) => {
          unlistenProgress = fn
        })
        .catch((err: unknown) => {
          // Сбрасываем memoization — следующий `start()` должен снова
          // попытаться подписаться, а не унаследовать отклонённый промис
          // навсегда (тот же приём, что `useYtDlpPrepare`, эпик E1).
          listeningProgress = undefined
          throw err
        })
    }
    return listeningProgress
  }

  function applySnapshot(snapshot: QueueSnapshot): void {
    tasks.value = snapshot.tasks
    awaitingContinue.value = snapshot.awaitingContinue
    pauseReason.value = snapshot.pauseReason
  }

  function ensureQueueListening(): Promise<void> {
    if (!listeningQueue) {
      listeningQueue = listen<QueueSnapshot>(QUEUE_CHANGED_EVENT, (event) => applySnapshot(event.payload))
        .then((fn) => {
          unlistenQueue = fn
        })
        .catch((err: unknown) => {
          listeningQueue = undefined
          throw err
        })
    }
    return listeningQueue
  }

  /**
   * Разовый снимок при маунте (С-9) плюс подписка на `queue://changed` —
   * вызывается явно из `App.vue` (`onMounted`), тем же приёмом, что
   * `runPrepareAndCheckSidecar`/`useSidecarCheck.check()`, а не изнутри
   * стора самостоятельно: подписка на `queue_state` не нужна каждому
   * потребителю стора (например, `useExitConfirmation.ts` его не читает
   * до TL-76), только экрану очереди.
   */
  async function initialize(): Promise<void> {
    await ensureQueueListening()
    try {
      const snapshot = await invoke<QueueSnapshot>(QUEUE_STATE_COMMAND)
      applySnapshot(snapshot)
    } catch (err) {
      // Отказ снимка здесь не превращается в `commandError` — это не
      // отказ одной из пяти команд контракта {@link DownloadCommandError},
      // а сбой самого запроса чтения; молчаливый откат к пустому списку
      // такой же нейтральный тон, что и у `useYtDlpUpdate.loadInitialSnapshot`.
      console.error('queue_state rejected', err)
    }
  }

  /**
   * Первая задача списка — источник для трёх геттеров обратной
   * совместимости ниже (`task`/`progress`/`isActive`). До TL-75 в сторе
   * было ровно одно место для одной задачи независимо от её фазы —
   * терминальной в том числе (панель E3 рисует Done/Failed/Cancelled тем
   * же `progress`, что и нетерминальные фазы). `tasks.value[0]` — тот же
   * снимок для однозадачного случая (единственный, который проверяют
   * `useExitConfirmation.ts`/его тесты сегодня); для по-настоящему
   * многозадачной очереди это временный, заведомо неполный выбор
   * («первая добавленная», не «единственная, которую стоит спросить при
   * выходе») — TL-76 (issue #83) заменит его срезом всей очереди.
   * Компонент, который различает `queued` и реально идущую фазу для
   * рендера списка (`DownloadPanel` vs `QueueWaitingRow`), —
   * `QueueSection.vue`, у него собственный критерий (`isPanelPhase`),
   * независимый от этого геттера.
   */
  const firstTask = computed(() => tasks.value[0])

  /** Обратная совместимость — см. doc класса выше, «Обратная совместимость». */
  const task = computed<DownloadTask | undefined>(() => {
    const current = firstTask.value
    if (!current) return undefined
    return {
      taskId: current.taskId,
      plan: current.plan,
      displayTitle: formatTaskDisplayTitle(current.title, current.quality),
    }
  })

  /** Обратная совместимость — см. doc класса выше, «Обратная совместимость». */
  const progress = computed<DownloadProgress | undefined>(() => {
    const current = firstTask.value
    return current ? toDownloadProgress(current) : undefined
  })

  /**
   * Обратная совместимость — см. doc класса выше, «Обратная
   * совместимость». Ровно та же проверка, что была в E3 до TL-75:
   * задача существует и не терминальна (`queued` в их числе — задел
   * под очередь появился в контракте, а не в этом геттере).
   */
  const isActive = computed(() => firstTask.value !== undefined && !isTerminalPhase(firstTask.value.phase))

  /** Скрывает баннер отказа команды («Скрыть» на нём же). */
  function dismissCommandError(): void {
    commandError.value = undefined
  }

  /**
   * Ставит новую задачу в хвост очереди (Ф-2 E4: занятый слот больше не
   * повод для мгновенного отказа — единственный оставшийся повод отказа
   * это дубль, Ф-8). Вставка в `tasks` — оптимистичная, по ответу самой
   * команды (см. doc класса выше, «Два разных способа узнать о задаче»).
   */
  async function start(request: StartDownloadRequest): Promise<void> {
    if (commandInFlight) return
    commandInFlight = true
    commandError.value = undefined
    try {
      await ensureProgressListening()
      const started = await invoke<DownloadStarted>(START_DOWNLOAD_COMMAND, { request })
      // `as QueueTask`: TS расщепляет spread размеченного объединения
      // (`initialQueueTaskPhaseFields`) в новом литерале, теряя связь
      // тега `phase` с полями своего варианта (см. doc функции выше) —
      // сборка корректна по построению (варианты `DownloadProgress` и
      // фазовый хвост `QueueTask` совпадают дословно, doc `QueueTask` в
      // `src/types/generated/queue.ts`), кастуем явно вместо борьбы с
      // выводом типов на спреде.
      const newTask = {
        taskId: started.taskId,
        title: request.title,
        quality: request.quality,
        plan: started.plan,
        ...initialQueueTaskPhaseFields(started.phase),
      } as QueueTask
      tasks.value = [...tasks.value, newTask]
      clearStallTimer()
    } catch (err) {
      console.error('start_download rejected', err)
      commandError.value = toDownloadCommandFailure(err)
    } finally {
      commandInFlight = false
    }
  }

  /** Отмена — доступна в любой нетерминальной фазе (Ф-4), в том числе ожидающей (новое в E4). */
  async function cancel(taskId: string): Promise<void> {
    if (commandInFlight) return
    commandInFlight = true
    try {
      await invoke<void>(CANCEL_DOWNLOAD_COMMAND, { taskId })
      commandError.value = undefined
    } catch (err) {
      console.error('cancel_download rejected', err)
      commandError.value = toDownloadCommandFailure(err)
    } finally {
      commandInFlight = false
    }
  }

  /**
   * Повтор — продолжение той же задачи (тот же `taskId`), не новая
   * сущность (дизайн E3, «Управляющие вызовы», п.3); с E4 задача
   * встаёт в хвост текущего порядка (дизайн E4, «Действия: отмена и
   * повтор» — решение дизайна, не решает фронтенд). Поток событий
   * возобновляется сам, без какого-либо сброса состояния здесь.
   */
  async function retry(taskId: string): Promise<void> {
    if (commandInFlight) return
    commandInFlight = true
    try {
      await invoke<void>(RETRY_DOWNLOAD_COMMAND, { taskId })
      commandError.value = undefined
    } catch (err) {
      console.error('retry_download rejected', err)
      commandError.value = toDownloadCommandFailure(err)
    } finally {
      commandInFlight = false
    }
  }

  /**
   * «Скрыть» (дизайн E4, «Данные для API», п.5) — команда ядра
   * (`dismiss_queue_task`), не локальное состояние фронтенда, как было в
   * E3: С-9 требует, чтобы перезагрузка webview восстанавливала список по
   * снимку ядра, а не по тому, что фронтенд стёр у себя локально.
   * Отклоняется типизированно (`taskNotFinished`), если задача не
   * терминальна — эта проверка не дублируется здесь (см. doc
   * `downloadCommandErrorTexts.ts`).
   */
  async function hide(taskId: string): Promise<void> {
    if (commandInFlight) return
    commandInFlight = true
    try {
      await invoke<void>(DISMISS_QUEUE_TASK_COMMAND, { taskId })
      commandError.value = undefined
    } catch (err) {
      console.error('dismiss_queue_task rejected', err)
      commandError.value = toDownloadCommandFailure(err)
    } finally {
      commandInFlight = false
    }
  }

  /** «Скрыть завершённые» (дизайн E4, «Пять состояний», «Список из нескольких завершённых») — то же «Скрыть» для каждой терминальной задачи разом. */
  async function hideAllTerminal(): Promise<void> {
    const ids = tasks.value.filter((t) => isTerminalPhase(t.phase)).map((t) => t.taskId)
    for (const id of ids) {
      // Последовательно: `commandInFlight` — общий флаг (doc класса выше),
      // параллельные вызовы просто отбросили бы друг друга.
      await hide(id)
    }
  }

  /**
   * «Продолжить очередь» (Р-3, дизайн «Продолжение после перезапуска») —
   * `resume_queue`, без параметров. Локально ничего не переключает
   * оптимистично: `awaitingContinue` — часть снимка, и её новое значение
   * (`false`) приходит тем же `queue://changed`, что и старт первой
   * задачи (доверие единственному источнику истины, doc класса выше).
   */
  async function resume(): Promise<void> {
    if (commandInFlight) return
    commandInFlight = true
    try {
      await invoke<void>(RESUME_QUEUE_COMMAND)
      commandError.value = undefined
    } catch (err) {
      console.error('resume_queue rejected', err)
      commandError.value = toDownloadCommandFailure(err)
    } finally {
      commandInFlight = false
    }
  }

  onScopeDispose(() => {
    clearStallTimer()
    if (unlistenProgress) unlistenProgress()
    if (unlistenQueue) unlistenQueue()
  })

  return {
    tasks,
    awaitingContinue,
    pauseReason,
    task,
    progress,
    softStallSeconds,
    commandError,
    isActive,
    initialize,
    start,
    cancel,
    retry,
    hide,
    hideAllTerminal,
    resume,
    dismissCommandError,
  }
})
