/**
 * Форма «Заголовок: пояснение» экрана истории (С-4, правки ревью TL-93,
 * второй раунд, дизайн — «Не удалось открыть проводник: …», «История
 * недоступна: …»). Общая точка сборки для баннера отказа команды
 * (`HistoryScreen.vue`, `.history-screen__command-error`) и построчной
 * ошибки «Показать в папке» (`.history-screen__entry-error`) — раньше
 * каждое место соединяло `title`/`explanation` через `: ` прямо в
 * шаблоне, и пустое `explanation` (нейтральные сообщения вроде «Этой
 * записи больше нет в истории») давало висящее двоеточие без ничего
 * после него.
 */
export function formatHistoryMessage(title: string, explanation: string): string {
  return explanation.length > 0 ? `${title}: ${explanation}` : title
}
