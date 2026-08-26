/**
 * Узкий адаптер над оконным событием закрытия (Р-2, эпик E3, TL-46).
 *
 * # Почему это отдельный файл с ровно двумя операциями
 *
 * Перехват попытки закрыть окно в Tauri 2 требует `@tauri-apps/api/window`
 * (`getCurrentWindow().onCloseRequested(...)`, а при подтверждённом выходе —
 * `.destroy()`). Разбор реализации показал (см. отчёт TL-46, задача ядра
 * #49): подписка идёт тем же IPC, что уже разрешён (`plugin:event|listen`/
 * `unlisten`, покрыто `core:event:allow-listen`/`allow-unlisten`), а вот
 * фактическое закрытие окна (`.destroy()`) требует `plugin:window|destroy`,
 * то есть разрешения `core:window:allow-destroy` — его нет в
 * `src-tauri/capabilities/main.json`, и добавить его может только ядро
 * (правка `src-tauri/` — не область `ui`). Сторож `src-tauri/tests/
 * frontend_acl.rs` вдобавок не знает модуль `window` вовсе (`ipc_commands_of`
 * перечисляет только `core`/`event`) — реальный импорт `getCurrentWindow`
 * уронит его тестом «неизвестная привязка», пока ядро не допишет туда
 * сопоставление.
 *
 * Поэтому всё, что реально трогает `@tauri-apps/api/window`, обязано жить
 * только здесь — за интерфейсом {@link WindowExitPort}. Остальной код
 * (`useExitConfirmation`, диалог) работает против интерфейса, а не против
 * Tauri API напрямую: когда задача #49 выдаст разрешение и запись в
 * стороже, реализацию этого файла достаточно заменить на настоящую
 * (`getCurrentWindow().onCloseRequested`/`.destroy()`) — не трогая ничего
 * снаружи.
 *
 * # Что нужно от #49 (передать исполнителю дословно)
 *
 * 1. `src-tauri/capabilities/main.json` — добавить `core:window:allow-destroy`
 *    (не `allow-close`: `close()` заново входит в тот же цикл
 *    `closeRequested`, а `destroy()` — ровно тот путь, которым сама
 *    библиотека Tauri продолжает закрытие после неотменённого
 *    `onCloseRequested`, без повторного цикла).
 * 2. `src-tauri/tests/frontend_acl.rs` — добавить в `ipc_commands_of`
 *    сопоставление для `("window", "getCurrentWindow")`, покрывающее
 *    команды, которые реально понадобятся: `plugin:event|listen`,
 *    `plugin:event|unlisten`, `plugin:window|destroy`.
 *
 * Ни строки в `src-tauri/` в рамках TL-46 не менялось — обе правки числятся
 * за #49.
 */

/** Что нужно диалогу выхода от оконного слоя — и ничего больше. */
export interface WindowExitPort {
  /**
   * Подписывается на каждую попытку пользователя закрыть окно (крестик,
   * системное завершение, Cmd+Q/Alt+F4). Возвращает функцию отписки.
   */
  onCloseAttempt(handler: () => void): () => void
  /**
   * Завершает окно/приложение — вызывается либо сразу (задачи нет или она
   * терминальна — диалог не нужен), либо после «Всё равно выйти».
   */
  finishWindow(): Promise<void>
}

const NOT_WIRED_SUBSCRIBE_WARNING =
  '[TL-46] Оконное событие закрытия ещё не подключено: ждём разрешение ' +
  'core:window:allow-destroy и запись в src-tauri/tests/frontend_acl.rs ' +
  '(задача ядра #49). Диалог подтверждения выхода не увидит настоящую ' +
  'попытку закрыть приложение, пока эта заглушка не заменена.'

const NOT_WIRED_FINISH_WARNING =
  '[TL-46] finishWindow() вызван, а оконный адаптер ещё не подключён (#49) ' +
  '— этот вызов не закрывает окно.'

/**
 * Заглушка на время #49. Умышленно не молчит: предупреждает в консоль на
 * каждый вызов, а не только один раз, — прецедент TL-24/TL-46 (эпик E1)
 * состоял именно в том, что незамеченная тихая заглушка/отказ дожили до
 * собранного приложения через две сборки и два ревью. Здесь конкретно
 * `onCloseAttempt` не перехватывает ничего реального: обработчик никогда
 * не будет вызван этой реализацией, поэтому диалог в собранном приложении
 * до #49 не появится вовсе — окно закрывается системой как обычно.
 */
export function createUnwiredWindowExitPort(): WindowExitPort {
  return {
    onCloseAttempt() {
      console.warn(NOT_WIRED_SUBSCRIBE_WARNING)
      return () => {}
    },
    async finishWindow() {
      console.warn(NOT_WIRED_FINISH_WARNING)
    },
  }
}

/**
 * Единственная продакшен-точка сборки адаптера — один и тот же экземпляр на
 * приложение (окно одно, состояние подписки не должно дублироваться между
 * вызовами). `useExitConfirmation` берёт его по умолчанию; тесты подставляют
 * свой фейк вместо него явным аргументом.
 */
export const windowExitPort: WindowExitPort = createUnwiredWindowExitPort()
