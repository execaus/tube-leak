import { listen, type Event as TauriEvent, type UnlistenFn } from '@tauri-apps/api/event'
import { onMounted, onUnmounted, watch, type Ref } from 'vue'

import type { YtDlpWarmupEvent } from '@/types/generated/ytdlp'
import { assertNever } from '@/utils/assertNever'

/**
 * Имя события конца фонового прогрева yt-dlp, эмитится Rust-стороной под
 * этим именем (`crate::ytdlp::prepare::WARMUP_EVENT`, зеркалируется здесь,
 * а не импортируется — граница Rust↔TS не делится константами напрямую,
 * тот же приём, что `PREPARE_EVENT_NAME`/`UPDATE_EVENT_NAME` в
 * `useYtDlpPrepare.ts`/`useYtDlpUpdate.ts`). Уже в белом списке каналов
 * `src-tauri/capabilities/main.json` (core-ветка `tl-64-80-65-ytdlp-contour`).
 */
const WARMUP_EVENT_NAME = 'ytdlp://warmup'

export interface UseYtDlpWarmupRecheckOptions {
  /**
   * `true`, пока уже идущая проверка `useSidecarCheck` не разрешилась —
   * это состояние **того же** composable, что и `check` ниже, не
   * собственная копия: у кнопки «Повторить проверку» и у этой подписки
   * должна быть одна защита от повторного вызова на двоих, а не две,
   * которые могут разойтись.
   */
  isLoading: Ref<boolean>
  /** `check()` того же `useSidecarCheck`, что показывает служебный экран. */
  check: () => Promise<void>
}

/**
 * Подписка на `ytdlp://warmup` (TL-118, долг #22): на медленной машине
 * фоновый прогрев yt-dlp (TL-21) уходит в фон уже после того, как
 * служебный экран отрисован — первая проверка видит холодное дерево,
 * строка yt-dlp показывает «не отвечает», поле ссылки заблокировано. Без
 * этой подписки экран сам не обновляется: пользователь застревает до
 * ручного клика «Повторить проверку».
 *
 * Единственное действие на любое событие — перепроверка тем же `check()`,
 * что и кнопка «Повторить проверку» (никаких таймеров/polling, CLAUDE.md):
 * composable не хранит и не интерпретирует `outcome` сам, `check()` решает
 * по свежему отчёту `check_sidecar`, что показать — версию (`warmed`) или
 * актуальный отказ с кнопкой повтора (`timedOut`/`failed`).
 *
 * # Перебор исходов — исчерпывающий
 *
 * {@link handleEvent} разбирает `outcome` через `switch` с `assertNever` в
 * `default`, хотя все три ветки делают одно и то же: если контракт
 * (`YtDlpWarmupOutcome`) добавит четвёртый исход, `npm run type-check`
 * остановится на этой строке, а не тихо продолжит перепроверять для трёх
 * старых и промолчать для нового — тот же приём, что `isNonTerminalStage`
 * в `useYtDlpPrepare.ts` и `toActiveQueueTaskPhase` в `App.vue`.
 *
 * # Защита от повторного вызова — общая с кнопкой, не собственная
 *
 * `isLoading` берётся у вызывающей стороны (тот же `useSidecarCheck()`,
 * что рисует служебный экран в `App.vue`), а не заводится здесь заново:
 * если бы у подписки была своя копия «идёт проверка», событие, пришедшее
 * ровно в момент клика по «Повторить проверку», могло бы запустить вторую
 * параллельную проверку — обе копии не видели бы состояние друг друга.
 *
 * # Событие во время идущей проверки — отложено, а не потеряно
 *
 * Возврат ведущего по первой версии: событие, пришедшее, пока
 * `isLoading.value === true` (проверка уже идёт — по кнопке или по
 * предыдущему событию), не имеет права просто пропадать. Сценарий
 * медленной машины: проверка стартует на t≈12 и идёт до таймаута 9 с;
 * фоновый прогрев заканчивается на t≈15, пока проверка ещё не закончилась;
 * старая версия выходила из `handleEvent` по защите и теряла событие;
 * проверка, начатая до конца прогрева, на t≈21 всё равно возвращает
 * `Timeout` — пользователь снова застревал до ручного повтора.
 *
 * Вместо немедленного вызова `handleEvent` в этом случае поднимает флаг
 * `pendingRecheck`. `watch(isLoading, …)` наблюдает конец **любой**
 * проверки (по кнопке или по предыдущему событию — источник triggера
 * этому watch неважен) и, если флаг поднят, снимает его и запускает
 * ровно одну перепроверку. Несколько событий подряд за время одной
 * проверки идемпотентно схлопываются в один и тот же флаг — не в
 * очередь и не в счётчик, поэтому после конца проверки происходит ровно
 * один `check()`, а не по одному на каждое отложенное событие.
 *
 * `watch` заводится в `setup()` вызывающего компонента (эта функция сама
 * вызывается из `<script setup>` `App.vue`) — Vue останавливает такие
 * watcher'ы автоматически при размонтировании владельца, отдельного
 * `onUnmounted` для него не нужно (в отличие от подписки `listen()` ниже,
 * которая живёт вне реактивности Vue и требует ручной отписки).
 *
 * # Отказ подписки — молчаливый, не полноэкранная ошибка
 *
 * В отличие от `useYtDlpPrepare` (где отказ подписки блокирует экран
 * первого запуска), здесь отказ `listen()` — тем же приёмом, что и
 * начальный снимок `useYtDlpUpdate`: без автоматической перепроверки
 * пользователь просто остаётся с рабочей ручной кнопкой «Повторить
 * проверку», а не видит отдельное уведомление (сама смена строки yt-dlp —
 * уже живая зона служебного экрана, отдельного оповещения не требуется).
 */
export function useYtDlpWarmupRecheck({ isLoading, check }: UseYtDlpWarmupRecheckOptions): void {
  let unlisten: UnlistenFn | undefined
  let listening: Promise<void> | undefined
  // Событие пришло, пока проверка уже шла — перепроверка нужна, но не
  // прямо сейчас (doc функции выше, «Событие во время идущей проверки»).
  let pendingRecheck = false

  function handleEvent(event: TauriEvent<YtDlpWarmupEvent>): void {
    switch (event.payload.outcome) {
      case 'warmed':
      case 'timedOut':
      case 'failed':
        break
      default:
        return assertNever(event.payload.outcome)
    }
    if (isLoading.value) {
      pendingRecheck = true
      return
    }
    void check()
  }

  // Конец любой проверки (по кнопке или по предыдущему событию) снимает
  // отложенный флаг ровно одной перепроверкой (doc функции выше).
  watch(isLoading, (loading) => {
    if (loading || !pendingRecheck) return
    pendingRecheck = false
    void check()
  })

  onMounted(() => {
    listening = listen<YtDlpWarmupEvent>(WARMUP_EVENT_NAME, handleEvent)
      .then((fn) => {
        unlisten = fn
      })
      .catch(() => {
        listening = undefined
      })
  })

  onUnmounted(() => {
    if (unlisten) {
      // Подписка уже подтверждена — снимаем сразу и синхронно.
      unlisten()
    } else if (listening) {
      // `listen()` мог ещё не успеть разрешиться (unmount раньше, чем
      // подтвердилась подписка) — цепляемся к тому же промису вместо
      // того, чтобы полагаться на уже присвоенное значение (тот же
      // приём, что `useYtDlpPrepare`/`useYtDlpUpdate`).
      void listening.then(() => unlisten?.())
    }
  })
}
