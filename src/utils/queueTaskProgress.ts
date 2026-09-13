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
 * Общий util, а не приватная функция одного места: используется сразу
 * несколькими сторонами (`useExitConfirmation.ts` — прогресс активной
 * задачи диалога выхода; `QueueSection.vue`/`App.vue` — рендер каждой
 * активной/терминальной строки списка) — дублировать одну и ту же
 * проекцию в каждой было бы вторым местом, которому расходиться с
 * контрактом. До TL-82 (issue 89) третьей стороной была ещё и временная
 * обратная совместимость `useDownloadTaskStore.progress` — она удалена.
 */
export function toDownloadProgress(task: QueueTask): DownloadProgress {
  const { taskId, title, quality, plan, ...progress } = task
  void taskId
  void title
  void quality
  void plan
  return progress
}
