import { invoke } from '@tauri-apps/api/core'
import { listen, type Event as TauriEvent, type UnlistenFn } from '@tauri-apps/api/event'
import { onMounted, onUnmounted, ref, type Ref } from 'vue'

import type { YtDlpUpdateSnapshot } from '@/types/generated/update'

const UPDATE_STATE_COMMAND = 'ytdlp_update_state'
const CHECK_UPDATE_COMMAND = 'check_ytdlp_update'

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

export interface UseYtDlpUpdateReturn {
  /** Последний известный снимок контура — `undefined` до первого ответа `ytdlp_update_state`. */
  snapshot: Ref<YtDlpUpdateSnapshot | undefined>
  /**
   * «Проверить сейчас» — вызывающая сторона обязана не давать нажать её,
   * пока `snapshot.value?.busy` истинно (тот же контракт, что у кнопок
   * `YtDlpUpdateSnapshot.busy` — doc в `src/types/generated/update.ts`).
   */
  checkNow: () => Promise<void>
}

/**
 * Composable блока «Обновление yt-dlp» (TL-59, дизайн E6): разовый снимок
 * при маунте + подписка на `ytdlp://update` — тот же приём, что у
 * `useSidecarCheck` (снимок) и `useYtDlpPrepare` (событие). Ядро — источник
 * истины состояния контура (CLAUDE.md), этот composable — проекция, не
 * второй источник: пишет только то, что получил снимком или событием.
 *
 * Подписка устанавливается и дожидается **до** запроса начального снимка
 * (тот же порядок и то же обоснование, что у `useYtDlpPrepare`): иначе
 * возможно пропустить событие, пришедшее в узком окне между разрешением
 * снимка и подтверждением подписки. В отличие от `useYtDlpPrepare`, здесь
 * это не критично для полноэкранного блокирующего экрана (события контура
 * обновления эмитятся независимо от того, открыт ли служебный экран —
 * дизайн, «Насколько тихо — конкретно») — тем не менее тот же порядок
 * сохранён, чтобы не полагаться на то, что «следующее событие всё равно
 * придёт» там, где можно просто не потерять текущее.
 */
export function useYtDlpUpdate(): UseYtDlpUpdateReturn {
  const snapshot = ref<YtDlpUpdateSnapshot>()

  let unlisten: UnlistenFn | undefined
  let listening: Promise<void> | undefined

  function handleEvent(event: TauriEvent<YtDlpUpdateSnapshot>): void {
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
          // маунт в тестах, либо — раз composable общий для TL-59/TL-60 —
          // повторный вызов checkNow()) не унаследовала уже отклонённый
          // промис навсегда (тот же приём, что `useYtDlpPrepare`).
          listening = undefined
          throw err
        })
    }
    return listening
  }

  async function loadInitialSnapshot(): Promise<void> {
    try {
      await ensureListening()
      snapshot.value = await fetchYtDlpUpdateState()
    } catch {
      // Отказ снимка или самой подписки здесь не показывается отдельной
      // ошибкой (в отличие от `useYtDlpPrepare`, где отказ блокирует
      // полноэкранный экран первого запуска): блок «Обновление yt-dlp» —
      // необязательная информация на служебном экране (дизайн E6, «Что
      // проектируем»), молчаливый откат к состоянию «снимка ещё нет» и
      // есть нейтральный тон, который дизайн требует от всего блока.
    }
  }

  async function checkNow(): Promise<void> {
    try {
      snapshot.value = await checkYtDlpUpdate()
    } catch {
      // `YtDlpUpdateCommandError` ("busy"/"nothingToRollBackTo") — отказ, до
      // которого исправный UI не доводит: кнопка неактивна, пока
      // `snapshot.value.busy` истинно (doc типа в контракте). Проглатывается
      // по той же причине, что и в `loadInitialSnapshot`.
    }
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

  return { snapshot, checkNow }
}
