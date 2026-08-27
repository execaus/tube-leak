import { storeToRefs } from 'pinia'
import { onMounted, onScopeDispose, ref, type Ref } from 'vue'

import { useDownloadTaskStore, type DownloadTask } from '@/stores/downloadTask'
import type { DownloadProgress } from '@/types/generated/download'
import { windowExitPort as defaultWindowExitPort, type WindowExitPort } from './windowExitPort'

/**
 * Диалог подтверждения выхода при активной загрузке (Р-2, эпик E3, TL-46).
 *
 * Разграничение «выйти» / «отменить» — ключевое требование Р-2: при любом
 * ответе уже скачанное остаётся на диске (правила подчистки живут в ядре,
 * этот композабл их не трогает), выбор влияет только на факт закрытия окна.
 * Поэтому «Всё равно выйти» здесь **не** зовёт `downloadTaskStore.cancel()`
 * — только {@link WindowExitPort.finishWindow}.
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
  task: Ref<DownloadTask | undefined>
  progress: Ref<DownloadProgress | undefined>
  stay: () => void
  exitAnyway: () => void
} {
  const store = useDownloadTaskStore()
  // `storeToRefs`, не `store.task` напрямую: последнее вернуло бы
  // разово развёрнутое значение, а не живую ссылку — тот же приём,
  // что уже применён в `App.vue` для панели (ревью TL-45).
  const { task, progress } = storeToRefs(store)
  const visible = ref(false)

  /**
   * Показывается тогда и только тогда, когда задача существует и её фаза
   * нетерминальна — буквально `store.isActive`, то же поле, которым уже
   * решает кнопка «Скачать» (С-13, дизайн). Для отсутствующей или уже
   * терминальной (даже не скрытой) задачи выход не перехватывается: диалог
   * был бы лишним трением, данные уже не в опасности.
   */
  function handleCloseAttempt(): void {
    if (store.isActive) {
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
    task,
    progress,
    stay,
    exitAnyway,
  }
}
