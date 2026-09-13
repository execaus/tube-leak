import { storeToRefs } from 'pinia'
import { computed, onMounted, onScopeDispose, ref, type Ref } from 'vue'

import { useDownloadTaskStore } from '@/stores/downloadTask'
import type { QueuePauseReason } from '@/types/generated/queue'
import type { ExitDialogActiveTask } from '@/utils/exitDialogTexts'
import { isTerminalQueuePhase } from '@/utils/queueTaskPhase'
import { toDownloadProgress } from '@/utils/queueTaskProgress'
import { formatTaskDisplayTitle } from '@/utils/queueTaskTitle'
import { windowExitPort as defaultWindowExitPort, type WindowExitPort } from './windowExitPort'

/**
 * Диалог подтверждения выхода при непустой очереди (Р-2, эпик E3, TL-46;
 * срез всей очереди вместо одной задачи — эпик E4, TL-76, С-6, Р-8).
 *
 * Разграничение «выйти» / «отменить» — ключевое требование Р-2: при любом
 * ответе уже скачанное остаётся на диске (правила подчистки живут в ядре,
 * этот композабл их не трогает), выбор влияет только на факт закрытия окна.
 * Поэтому «Всё равно выйти» здесь **не** зовёт `downloadTaskStore.cancel()`
 * — только {@link WindowExitPort.finishWindow}.
 *
 * # Полный срез очереди, а не одна задача (TL-76)
 *
 * До TL-76 композабл читал `store.task`/`store.progress`/`store.isActive`
 * — временную проекцию первой задачи списка, которую TL-75 оставил
 * намеренно; TL-82 (issue 89) убрал эти геттеры из стора совсем, когда
 * подтвердилось, что читать их больше некому (doc `useDownloadTaskStore`
 * в `src/stores/downloadTask.ts`, «Чего здесь больше нет»). Теперь вход
 * этого композабла — сам список `tasks` плюс
 * `awaitingContinue`/`pauseReason` уровня очереди, из которых считаются:
 * - `activeTask` — задача, реально выполняющаяся прямо сейчас (голова
 *   нетерминального подсписка, Р-4: активная задача всегда первая среди
 *   нетерминальных); отсутствует во время паузы между задачами на
 *   обновление yt-dlp (Р-7) — в этот момент предыдущая задача уже
 *   терминальна, а следующая ещё не стартовала, назвать эту задачу
 *   активной значило бы соврать про фазу, которой у неё ещё нет;
 * - `waitingCount` — число нетерминальных задач сверх названной
 *   `activeTask` (во время паузы, когда `activeTask` не назван никто, —
 *   это все нетерминальные задачи целиком).
 *
 * # Условие показа — С-6, уточнённое Р-8
 *
 * Буквально С-6 требует диалог при хоть одной нетерминальной задаче. Р-8
 * сужает это условие: диалог **не** показывается, если очередь
 * восстановлена после перезапуска и пользователь ещё ни разу не нажал
 * «Продолжить» (`awaitingContinue: true`) — в этом состоянии закрытие
 * ничего не теряет: снимок уже лежит на диске, сетевой активности нет,
 * частичные файлы целы. Диалог, предупреждающий о том, чего не
 * происходит, приучает закрывать его не читая (обоснование — дизайн E4,
 * раздел «Р-8»).
 *
 * # Оконное событие — за портом, не напрямую
 *
 * `port` по умолчанию — настоящая привязка к Tauri
 * (`src/composables/windowExitPort.ts`, TL-47/#49); вся логика ниже
 * написана и протестирована против интерфейса порта
 * (`useExitConfirmation.test.ts`, фейковый порт), а не против конкретной
 * реализации — привязка к настоящему оконному событию проверяется
 * отдельно, на уровне `App.download.test.ts` (мок оконного модуля Tauri).
 */
export function useExitConfirmation(port: WindowExitPort = defaultWindowExitPort): {
  visible: Ref<boolean>
  activeTask: Ref<ExitDialogActiveTask | undefined>
  pauseReason: Ref<QueuePauseReason | undefined>
  waitingCount: Ref<number>
  stay: () => void
  exitAnyway: () => void
} {
  const store = useDownloadTaskStore()
  // `storeToRefs`, не `store.tasks` напрямую: последнее вернуло бы
  // разово развёрнутое значение, а не живую ссылку — тот же приём,
  // что уже применён в `App.vue` для панели (ревью TL-45).
  const { tasks, awaitingContinue, pauseReason } = storeToRefs(store)
  const visible = ref(false)

  const nonTerminalTasks = computed(() => tasks.value.filter((t) => !isTerminalQueuePhase(t.phase)))

  /** Планировщик держит паузу между задачами на обновление yt-dlp (Р-7) — единственный случай, когда `activeTask` не назван никто. */
  const isPaused = computed(() => pauseReason.value === 'ytDlpUpdate')

  const activeTask = computed<ExitDialogActiveTask | undefined>(() => {
    if (isPaused.value) return undefined
    const head = nonTerminalTasks.value[0]
    if (!head) return undefined
    return {
      displayTitle: formatTaskDisplayTitle(head.title, head.quality),
      progress: toDownloadProgress(head),
    }
  })

  const waitingCount = computed(() => {
    const total = nonTerminalTasks.value.length
    return isPaused.value ? total : Math.max(total - 1, 0)
  })

  /**
   * Показывается тогда и только тогда, когда есть хоть одна нетерминальная
   * задача (С-6) и очередь не лежит приостановленной после перезапуска,
   * ещё не продолженной пользователем (Р-8, см. doc функции выше). Для
   * пустой очереди или для приостановленной, ещё не продолженной, выход
   * не перехватывается: диалог был бы лишним трением, данные уже не в
   * опасности.
   */
  function handleCloseAttempt(): void {
    if (nonTerminalTasks.value.length > 0 && !awaitingContinue.value) {
      visible.value = true
    } else {
      void port.finishWindow()
    }
  }

  let unsubscribe: (() => void) | undefined
  onMounted(() => {
    unsubscribe = port.onCloseAttempt(handleCloseAttempt)
  })
  onScopeDispose(() => {
    unsubscribe?.()
  })

  /** «Остаться» — кнопка по умолчанию и Esc: просто закрывает диалог, ничего больше. */
  function stay(): void {
    visible.value = false
  }

  /**
   * «Всё равно выйти» — продолжает штатное закрытие. Не отмена: подчистка
   * частичных файлов сюда не входит и не должна — это работа ядра при
   * реальном завершении процессов (TL-10), а не композабла диалога.
   */
  function exitAnyway(): void {
    visible.value = false
    void port.finishWindow()
  }

  return {
    visible,
    activeTask,
    pauseReason,
    waitingCount,
    stay,
    exitAnyway,
  }
}
