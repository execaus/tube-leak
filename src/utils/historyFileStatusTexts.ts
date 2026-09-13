import type { FolderDisplay } from '@/types/generated/download'
import type { HistoryFileStatus } from '@/types/generated/history'
import { getFolderDisplayText } from './downloadOutcomeTexts'

/**
 * Третья строка записи истории по статусу файла (Ф-5, дизайн E5, «Записи с
 * удалённым или перемещённым файлом (С-3)», таблица трёх случаев):
 * `undefined` для `present` — там строку по-прежнему несёт папка (третья
 * строка в разметке — обычный путь, не текст этой функции), непустая
 * строка для `missing`.
 *
 * `папка существует`/`тоже не существует` не повторяет путь дважды: у
 * первой строки (папка есть) дизайн его не называет вовсе — «Показать в
 * папке» и так ведёт куда нужно; у второй (папки тоже нет) название папки
 * нужно, чтобы объяснить, куда возврата нет — используется
 * {@link getFolderDisplayText} (тот же форматтер, что и у панели `Done` и
 * «присутствует» записи истории — не второй параллельный текст для того же
 * факта, TL-51 «два описания одного контракта»).
 *
 * Расхождение с дизайном (см. отчёт задачи TL-93): макет пишет
 * «папка «…» тоже не существует» с guillemets вокруг любого значения;
 * контракт отдаёт `FolderDisplay` — объединение, а не готовую строку
 * (Ф-15 задним числом сузила «уже готовую строку» дизайна до `{kind}`), и
 * `getFolderDisplayText` кладёт кавычки только вокруг системной «Загрузки»,
 * оставляя свой путь без них — тот же выбор, что уже сделан в
 * `DownloadPanel.vue` для терминальной панели `Done`.
 */
export function getHistoryFileStatusText(status: HistoryFileStatus, folderDisplay: FolderDisplay): string | undefined {
  if (status.kind === 'present') return undefined
  if (status.folderExists) return 'Файл сейчас не на месте — папка существует.'
  return `Файл сейчас не на месте, папка ${getFolderDisplayText(folderDisplay)} тоже не существует.`
}
