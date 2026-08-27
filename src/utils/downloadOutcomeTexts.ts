import type { PartialData } from '@/types/generated/download'

/**
 * Тексты о судьбе частично скачанных данных (Ф-8), выведенные строго из
 * {@link PartialData} — не из класса ошибки: таблица «класс → что на
 * диске» дизайна E3 не постоянна (`destinationUnavailable` зависит от
 * того, доступна ли ещё папка), поэтому решает ядро, а UI лишь озвучивает
 * присланное значение (doc-комментарий `DownloadError.partialData` в
 * `src/types/generated/download.ts`).
 */

/** Дополнительное предложение после пояснения ошибки в терминальной панели Failed. */
export function getFailedPartialDataNote(partialData: PartialData): string {
  switch (partialData) {
    case 'kept':
      return 'Уже скачанное осталось на диске.'
    case 'removed':
      return 'Скачанные данные удалены — на диске ничего не осталось.'
    case 'nothingCreated':
      // Не встречается по контракту (отказ всегда после запуска процесса),
      // но обрабатывается на случай будущего расширения набора значений.
      return 'Скачать ничего не успело — на диске ничего не осталось.'
  }
}

/** Основной текст терминальной панели Cancelled (Ф-4: подчистка всегда полная). */
export function getCancelledText(partialData: PartialData): string {
  switch (partialData) {
    case 'nothingCreated':
      return 'Загрузка отменена до начала скачивания.'
    case 'removed':
      return 'Загрузка отменена. Скачанные данные удалены — на диске ничего не осталось.'
    case 'kept':
      // Не встречается по контракту (отмена подчищает всегда и без
      // исключений), но обрабатывается на случай будущего расширения.
      return 'Загрузка отменена.'
  }
}
