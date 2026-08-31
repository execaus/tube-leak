import type { SelectedQuality } from '@/types/generated/queue'
import { qualityLabel } from './qualityLabel'

/**
 * Заголовок задачи очереди — ««Название» — качество» (эпик E4, дизайн
 * «Пять состояний одной задачи»: «Заголовок — тот же формат, что уже
 * строит `App.vue` для `displayTitle` панели»).
 *
 * Единая точка форматирования, используемая и в момент постановки задачи
 * (сразу после клика «Скачать», пока карточка ролика ещё жива), и при
 * восстановлении списка по снимку очереди (`QueueTask.quality`, ни
 * карточки, ни лестницы к этому моменту уже нет — контракт TL-70 поэтому
 * и несёт `quality` в каждой задаче снимка), и в тексте отказа по дублю
 * (`QueueTaskRef.quality`, Р-5) — три источника одних и тех же двух полей,
 * не три копии форматирования.
 */
export function formatTaskDisplayTitle(title: string, quality: SelectedQuality): string {
  return `«${title}» — ${qualityLabel(quality)}`
}
