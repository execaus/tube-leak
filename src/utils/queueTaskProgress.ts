import type { DownloadProgress } from '@/types/generated/download'
import type { QueueTask } from '@/types/generated/queue'

/**
 * Проекция задачи снимка очереди в форму {@link DownloadProgress}, которую
 * рисует `DownloadPanel` (дизайн E3, компонент не изменён ни строкой).
 * `QueueTask` несёт те же тегированные по `phase` поля, что и
 * `DownloadProgress` (doc `QueueTask` в `src/types/generated/queue.ts`:
 * «форма выбрана так, чтобы совпадать с `DownloadProgressEvent`»), плюс
 * `taskId`/`title`/`quality`/`plan` — этот util отбрасывает именно их.
 *
 * Общий util, а не приватная функция стора или `QueueSection.vue`:
 * используется обеими сторонами (обратная совместимость
 * `useDownloadTaskStore.progress` и рендер каждой активной/терминальной
 * строки списка) — дублировать одну и ту же проекцию было бы вторым
 * местом, которому расходиться с контрактом.
 */
export function toDownloadProgress(task: QueueTask): DownloadProgress {
  const { taskId, title, quality, plan, ...progress } = task
  void taskId
  void title
  void quality
  void plan
  return progress
}
