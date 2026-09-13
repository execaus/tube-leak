// Файл СГЕНЕРИРОВАН из src-tauri/src/types.rs. Руками не править: правка
// живёт в Rust, сюда она приезжает перегенерацией.
//
//   перегенерация:  cd src-tauri && TUBE_LEAK_UPDATE_TS_BINDINGS=1 cargo test --locked
//   сторож:         cargo test падает, если этот файл разошёлся с types.rs (TL-51)
//
// Комментарии ниже — те же doc-комментарии, что стоят у типов в types.rs;
// расходиться с ними этот файл не может по построению.
import type { FolderDisplay } from "./download";
import type { SelectedQuality } from "./queue";

/**
 * Отказ команды над записями истории в сериализуемом виде.
 *
 * `message` — диагностика для лога; решение принимается по `kind`.
 */
export type HistoryCommandError = { message: string, } & ({ "kind": "unknownRecord" } | { "kind": "writeFailed" } | { "kind": "unavailable", reason: HistoryUnavailableReason, });

/**
 * Почему отклонена команда над записями истории (`delete_history_record`,
 * `clear_history`).
 */
export type HistoryCommandErrorKind = { "kind": "unknownRecord" } | { "kind": "writeFailed" } | { "kind": "unavailable", reason: HistoryUnavailableReason, };

/**
 * Курсор постраничного чтения: последняя запись, которую уже показали.
 *
 * Пара (время завершения, id), а не смещение: список растёт сверху, и
 * смещение пропускало бы или повторяло строку, если между двумя «Показать
 * ещё» завершилась загрузка (дизайн E5, пункт 2). `id` разводит записи
 * одной секунды.
 *
 * Фронтенд значение не собирает: берёт [`HistoryPage::next_cursor`] и
 * отдаёт как есть. Отсюда `Deserialize` — тип ходит в обе стороны.
 */
export type HistoryCursor = { finishedAtUnixSecs: number, id: string, };

/**
 * Одна запись истории — то, что видит одна строка списка.
 *
 * Состав — минимум из данных `Done` (Р-2): длительности, канала и превью
 * здесь нет и не будет без отдельного решения. **Сырого пути папки нет**:
 * «Показать в папке» принимает `id`, и путь ядро строит из собственной
 * копии записи (Ф-8). Фронтенду путь нужен только как текст, и этот текст —
 * [`Self::folder_display`].
 */
export type HistoryEntry = { 
/**
 * Идентификатор записи. Для фронтенда непрозрачен: уходит обратно в
 * `delete_history_record` и `show_in_folder`, но не разбирается.
 */
id: string, 
/**
 * Канонический id ролика (TL-72) — то же значение, по которому очередь
 * сравнивает дубли.
 */
videoId: string, 
/**
 * Ссылка в том виде, в каком её вставили. Показывается текстом, а не
 * активной ссылкой (граница продукта).
 */
url: string, 
/**
 * Название ролика, как его показала карточка E2.
 */
title: string, 
/**
 * Выбранный пункт лестницы — вторая половина заголовка
 * ««Название» — качество».
 */
quality: SelectedQuality, 
/**
 * Имя готового файла с расширением, без пути.
 */
fileName: string, 
/**
 * Папка на момент завершения — по тому же правилу, что у `Done`.
 */
folderDisplay: FolderDisplay, 
/**
 * Размер файла в байтах, снятый с диска в момент записи (Ф-2).
 */
sizeBytes: number, 
/**
 * Время завершения, Unix-секунды UTC. Относительное время
 * («3 часа назад») строит интерфейс.
 */
finishedAtUnixSecs: number, 
/**
 * Файл на месте или нет — на момент этого запроса.
 */
fileStatus: HistoryFileStatus, };

/**
 * Есть ли на диске файл записи истории (Ф-5).
 *
 * Вычисляется при каждом запросе страницы и в базе не хранится:
 * единственный источник — диск в момент запроса.
 */
export type HistoryFileStatus = { "kind": "present" } | { "kind": "missing", folderExists: boolean, };

/**
 * Однократная пометка на экране истории: случилось то, о чём
 * пользователь должен узнать (дизайн E5, пункт 2).
 *
 * **«Выдано один раз» — свойство реализации, а не контракта.** Для
 * контракта это просто необязательное поле ответа [`HistoryPage::notice`].
 * Обещание ядра: пометка приходит в первом ответе `history_page` после
 * события и больше не повторяется, показал её фронтенд или нет. Как ядро
 * это держит, решает TL-90 вместе с хранилищем (TL-85).
 */
export type HistoryNotice = { "kind": "baseRecreated" } | { "kind": "lastWriteFailed", cause: HistoryWriteFailure, };

/**
 * Ответ `history_page`: одна порция истории, новые записи сверху.
 */
export type HistoryPage = { 
/**
 * Не больше [`HISTORY_PAGE_SIZE`] записей. Порядок задаёт ядро: время
 * завершения по убыванию, при равенстве — по `id`. UI не сортирует.
 */
entries: Array<HistoryEntry>, 
/**
 * Курсор следующей порции. Отсутствует, когда записей старше этих нет;
 * по нему интерфейс и прячет «Показать ещё».
 */
nextCursor?: HistoryCursor, 
/**
 * Однократная пометка ([`HistoryNotice`]). В подавляющем большинстве
 * ответов отсутствует.
 */
notice?: HistoryNotice, };

/**
 * Отказ `history_page`: истории в этом сеансе нет.
 *
 * `message` — диагностика для лога, как у [`DownloadCommandError`]:
 * экран рисуется по `reason`.
 */
export type HistoryUnavailableError = { reason: HistoryUnavailableReason, message: string, };

/**
 * Почему история недоступна в этом сеансе (Ф-1 б, в, д).
 *
 * `corrupted` сюда не входит: порча базы не лишает сеанс истории, ядро
 * заводит новую (см. [`HistoryNotice::BaseRecreated`]).
 */
export type HistoryUnavailableReason = "newerVersion" | "noAccess" | "migrationFailed";

/**
 * Почему последняя запись в историю не сохранилась (Ф-3).
 *
 * Перечисление, а не строка: см. пункт 3 шапки секции. Набор — кандидаты
 * дизайна за вычетом «база недоступна»: при недоступной базе
 * `history_page` отвечает отказом [`HistoryUnavailableError`], и страница
 * с этой пометкой не приходит вовсе.
 */
export type HistoryWriteFailure = "diskFull" | "noAccess" | "storageFailed";

/**
 * Подробности отказа системного файлового менеджера — для свёрнутого
 * блока «Подробнее» (Ф-17).
 *
 * Оба поля необязательны. Программа, которая не запустилась вовсе
 * (`xdg-open` не установлен), не даёт ни кода, ни stderr; её причина — в
 * `message`.
 */
export type LauncherFailureDetails = { 
/**
 * Код завершения программы показа, если она успела завершиться.
 */
exitCode?: number, 
/**
 * Хвост stderr программы показа, обрезанный по длине.
 */
stderrTail?: string, };

/**
 * Отказ `show_in_folder` в сериализуемом виде.
 *
 * `message` — диагностика для лога; решение принимается по `kind`.
 */
export type ShowInFolderError = { message: string, } & ({ "kind": "fileMissing" } | { "kind": "folderMissing" } | { "kind": "launcherFailed", details: LauncherFailureDetails, } | { "kind": "unknownRecord" } | { "kind": "unavailable", reason: HistoryUnavailableReason, });

/**
 * Почему «Показать в папке» не выделило файл (Ф-8).
 *
 * **У `fileMissing` есть побочный эффект.** Если папка на месте, ядро
 * открывает её и **при этом** возвращает этот отказ: так интерфейс может
 * объяснить, почему файл не выделен (дизайн E5, пункт 2, таблица трёх
 * случаев). Отказ здесь означает «не выделено», а не «ничего не
 * произошло».
 */
export type ShowInFolderErrorKind = { "kind": "fileMissing" } | { "kind": "folderMissing" } | { "kind": "launcherFailed", details: LauncherFailureDetails, } | { "kind": "unknownRecord" } | { "kind": "unavailable", reason: HistoryUnavailableReason, };
