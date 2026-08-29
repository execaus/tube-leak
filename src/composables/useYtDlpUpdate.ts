import { invoke } from '@tauri-apps/api/core'
import { listen, type Event as TauriEvent, type UnlistenFn } from '@tauri-apps/api/event'
import { onMounted, onUnmounted, ref, type Ref } from 'vue'

import type { YtDlpUpdateSnapshot } from '@/types/generated/update'

const UPDATE_STATE_COMMAND = 'ytdlp_update_state'
const CHECK_UPDATE_COMMAND = 'check_ytdlp_update'
const ROLL_BACK_COMMAND = 'roll_back_ytdlp'

/**
 * Имя события хода контура обновления (контракт TL-53). Отдельный канал от
 * `ytdlp://prepare` — тот зарезервирован за блокирующей подготовкой
 * первого запуска и не должен путаться с фоновым обновлением (дизайн E6,
 * «Данные для UI», «Насколько тихо — конкретно»).
 */
const UPDATE_EVENT_NAME = 'ytdlp://update'

/**
 * Разовый снимок состояния контура обновления (без polling) — тот же
 * приём, что `check_sidecar` (Ф-9 E1): снимок на маунте, дальше — только
 * события.
 */
export async function fetchYtDlpUpdateState(): Promise<YtDlpUpdateSnapshot> {
  return invoke<YtDlpUpdateSnapshot>(UPDATE_STATE_COMMAND)
}

/**
 * «Проверить сейчас» (С-12) — запускает тот же конвейер, что плановая
 * проверка. Возвращается быстро (сам запуск конвейера), терминальный
 * исход приходит тем же событием `ytdlp://update`, что и у фоновой
 * проверки — отдельного «висящего» до конца конвейера промиса нет
 * (дизайн E6, «Команды», п.1).
 */
export async function checkYtDlpUpdate(): Promise<YtDlpUpdateSnapshot> {
  return invoke<YtDlpUpdateSnapshot>(CHECK_UPDATE_COMMAND)
}

/**
 * «Вернуться» (Р-3, TL-60) — вызывает `roll_back_ytdlp` контракта TL-53.
 * **Без параметра-версии**: цель ровно одна по построению (Ф-8 держит на
 * диске не более двух установок), контракт `types.rs` объявляет команду
 * так же, без аргументов — принимать версию от фронтенда значило бы
 * принимать выбор, которого он не делает. Терминальный исход (сразу или
 * после паузы между задачами — Ф-7) тот же снимок, что и у остальных
 * команд контура (см. doc функции ниже, «Окно Б»).
 */
export async function rollBackYtDlp(): Promise<YtDlpUpdateSnapshot> {
  return invoke<YtDlpUpdateSnapshot>(ROLL_BACK_COMMAND)
}

export interface UseYtDlpUpdateReturn {
  /** Последний известный снимок контура — `undefined` до первого ответа `ytdlp_update_state`. */
  snapshot: Ref<YtDlpUpdateSnapshot | undefined>
  /**
   * «Проверить сейчас» — вызывающая сторона обязана не давать нажать её,
   * пока `snapshot.value?.busy` истинно (тот же контракт, что у кнопок
   * `YtDlpUpdateSnapshot.busy` — doc в `src/types/generated/update.ts`).
   */
  checkNow: () => Promise<void>
  /**
   * «Вернуться» (Р-3, TL-60) — тот же контракт неактивности кнопки, что и
   * `checkNow`: вызывающая сторона не даёт нажать её, пока
   * `snapshot.value?.busy` истинно. Инлайн-подтверждение и решение
   * пользователя «Вернуться»/«Отмена» — в `YtDlpUpdateBlock`; здесь только
   * сам вызов команды и применение её ответа.
   */
  rollback: () => Promise<void>
}

/**
 * Composable блока «Обновление yt-dlp» (TL-59/TL-60, дизайн E6): разовый
 * снимок при маунте + подписка на `ytdlp://update` — тот же приём, что у
 * `useSidecarCheck` (снимок) и `useYtDlpPrepare` (событие). Ядро — источник
 * истины состояния контура (CLAUDE.md), этот composable — проекция, не
 * второй источник: пишет только то, что получил снимком, событием или
 * ответом одной из двух команд (`checkNow`/`rollback`).
 *
 * # Порядок «подписка раньше снимка» — что он закрывает, а что нет
 *
 * Подписка устанавливается и дожидается **до** запроса начального снимка
 * (тот же порядок, что у `useYtDlpPrepare`). Это закрывает только
 * **пропуск** события, пришедшего в узком окне между разрешением снимка и
 * подтверждением подписки — не более того. Он **не** закрывает обратную
 * гонку — **затирание** уже пришедшего события более поздним ответом
 * команды (ревью TL-59, «Н-1»): промис `fetchYtDlpUpdateState()`/
 * `checkYtDlpUpdate()` и подписка на `ytdlp://update` летят независимо, и
 * порядок их разрешения не гарантирован — тот же класс гонки, что
 * `useYtDlpPrepare` описывает в doc «Почему терминальные `stage`
 * игнорируются» (`ytdlp://prepare` vs ответ `prepare_ytdlp`). Там урок —
 * не доверять `stage` из события, здесь симметричный: не давать более
 * старому ответу команды переписать более свежее событие. Оба случая
 * реальны:
 *
 * - **Окно А (маунт).** Снимок мог начать запрашиваться раньше, чем
 *   пришло первое `ytdlp://update` (например, конвейер уже шёл до
 *   открытия экрана и как раз в этот момент прислал событие) — если бы
 *   `snapshot.value` присваивался ответу команды безусловно, только что
 *   пришедшее свежее событие откатилось бы назад.
 * - **Окно Б (клик «Проверить сейчас» или «Вернуться»).** Ответ
 *   `check_ytdlp_update`/`roll_back_ytdlp` обычно несёт промежуточный
 *   статус (`checking`/`rollbackWaiting`), но конвейер может успеть дойти
 *   до терминального исхода и прислать событие раньше, чем разрешится сам
 *   вызов команды — тогда безусловное присваивание затёрло бы честный
 *   терминальный текст (и `busy: true` из ответа команды) обратно в
 *   вечный спиннер до следующего события по расписанию (прямая
 *   регрессия С-12, и её же зеркало для отката — Р-3).
 *
 * Лечится счётчиком поколений `eventGeneration`: каждое событие его
 * увеличивает, и ответ команды применяется, только если поколение не
 * успело измениться за время ожидания — иначе он заведомо старше того,
 * что уже показано, и отбрасывается молча (то же «источник истины —
 * событие», что и подход `useYtDlpPrepare` к `stage`, только в обратную
 * сторону: там не доверяют событию, здесь — устаревшему ответу команды).
 */
export function useYtDlpUpdate(): UseYtDlpUpdateReturn {
  const snapshot = ref<YtDlpUpdateSnapshot>()

  let unlisten: UnlistenFn | undefined
  let listening: Promise<void> | undefined

  /**
   * Счётчик поколений событий (doc функции выше, «Окно А»/«Окно Б») —
   * растёт на каждое `ytdlp://update`, не на каждый рендер: ответ команды,
   * захвативший поколение до старта своего `await`, применяется только
   * если оно не изменилось к моменту разрешения, иначе событие пришло
   * позже и уже успело стать более свежей правдой.
   */
  let eventGeneration = 0

  function handleEvent(event: TauriEvent<YtDlpUpdateSnapshot>): void {
    eventGeneration += 1
    snapshot.value = event.payload
  }

  function ensureListening(): Promise<void> {
    if (!listening) {
      listening = listen<YtDlpUpdateSnapshot>(UPDATE_EVENT_NAME, handleEvent)
        .then((fn) => {
          unlisten = fn
        })
        .catch((err: unknown) => {
          // Сбрасываем memoization, чтобы следующая попытка (следующий
          // маунт в тестах, либо — раз composable общий для блока TL-59 и
          // отката TL-60 — повторный вызов checkNow()/rollback()) не
          // унаследовала уже отклонённый промис навсегда (тот же приём,
          // что `useYtDlpPrepare`).
          listening = undefined
          throw err
        })
    }
    return listening
  }

  async function loadInitialSnapshot(): Promise<void> {
    try {
      await ensureListening()
      const generationBeforeFetch = eventGeneration
      const fetched = await fetchYtDlpUpdateState()
      // Окно А (doc функции выше): применяем ответ, только если за время
      // ожидания не пришло ни одного события — иначе оно свежее.
      if (eventGeneration === generationBeforeFetch) {
        snapshot.value = fetched
      }
    } catch {
      // Отказ снимка или самой подписки здесь не показывается отдельной
      // ошибкой (в отличие от `useYtDlpPrepare`, где отказ блокирует
      // полноэкранный экран первого запуска): блок «Обновление yt-dlp» —
      // необязательная информация на служебном экране (дизайн E6, «Что
      // проектируем»), молчаливый откат к состоянию «снимка ещё нет» и
      // есть нейтральный тон, который дизайн требует от всего блока.
    }
  }

  /**
   * Общий хвост «Окна Б» для обеих команд, ждущих терминального исхода
   * событием (`checkNow`/`rollback`): вызывает команду и применяет её
   * ответ, только если за время ожидания не пришло более свежее событие
   * (doc функции выше). Один код на обе команды — не удобство изложения:
   * разные копии одной и той же гонки расходятся так же надёжно, как
   * разные копии одного правила (тот же довод, что у `YtDlpUpdateSnapshot`
   * — «одно значение и на снимок, и на событие» в контракте).
   */
  async function callCommandAndApply(command: () => Promise<YtDlpUpdateSnapshot>): Promise<void> {
    const generationBeforeCall = eventGeneration
    try {
      const result = await command()
      if (eventGeneration === generationBeforeCall) {
        snapshot.value = result
      }
    } catch {
      // `YtDlpUpdateCommandError` ("busy"/"nothingToRollBackTo") — отказ, до
      // которого исправный UI не доводит: обе кнопки неактивны, пока
      // `snapshot.value.busy` истинно, а «Вернуться» вдобавок не рисуется
      // вовсе без `rollbackTarget` (doc типов в контракте). Проглатывается
      // по той же причине, что и в `loadInitialSnapshot`.
    }
  }

  async function checkNow(): Promise<void> {
    await callCommandAndApply(checkYtDlpUpdate)
  }

  /**
   * «Вернуться» (Р-3, TL-60) — вызывается только по подтверждению из
   * инлайн-диалога `YtDlpUpdateBlock` («Вернуться»/«Отмена»); сам выбор
   * версии не передаётся (см. doc `rollBackYtDlp` выше). Composable не
   * предсказывает исход локально: применится ли откат сразу (строка 13)
   * или встанет в ожидание границы задачи (строка 14, Ф-7) — решает
   * ответ команды, а не клик сам по себе.
   */
  async function rollback(): Promise<void> {
    await callCommandAndApply(rollBackYtDlp)
  }

  onMounted(() => {
    void loadInitialSnapshot()
  })

  onUnmounted(() => {
    if (unlisten) {
      unlisten()
    } else if (listening) {
      // `listen()` мог не успеть разрешиться до unmount — цепляемся к
      // тому же промису вместо того, чтобы полагаться на уже присвоенное
      // значение (тот же приём, что `useYtDlpPrepare`).
      void listening.then(() => unlisten?.()).catch(() => {})
    }
  })

  return { snapshot, checkNow, rollback }
}
