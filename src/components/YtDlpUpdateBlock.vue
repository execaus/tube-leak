<script setup lang="ts">
/**
 * Блок «Обновление yt-dlp» служебного экрана (Ф-10, TL-59, дизайн E6):
 * статус-строка на все 14 состояний таблицы «Все состояния» + кнопка
 * «Проверить сейчас» + кнопка «Вернуться к …» (видна только когда есть
 * куда возвращаться).
 *
 * Чисто презентационный компонент — `invoke`/`listen` не вызывает сам,
 * это делает `useYtDlpUpdate` в родителе (тот же приём, что
 * `SidecarStatusRow`/`DownloadPanel`, doc-комментарии этих компонентов).
 *
 * # Нейтральный тон (С-4/С-5/С-11)
 *
 * Ни одно состояние не рисуется значком ошибки («✕» уже занят
 * `SidecarStatusRow` под «инструмент не работает» — здесь это неправда:
 * устаревший yt-dlp продолжает скачивать). Единственное визуальное
 * отличие пяти классов отказа (строки 8–12 таблицы) — приглушённый цвет
 * текста, класс `--muted`.
 *
 * **Про дословную ссылку дизайна и её решение.** Дизайн формулирует это
 * как «тот же цвет, что у пояснений `SidecarStatusRow` сейчас» — а там
 * `color` не объявлен вовсе ни на одном селекторе (глобальных стилей в
 * проекте тоже нет, наследовать неоткуда). Взятая буквально, эта ссылка
 * невыполнима: пустое правило `--muted` не просто ничего не красит, оно
 * вырезается сборщиком из итогового CSS (ревью TL-59, «Н-4», второй
 * заход — проверено сборкой), и строки 8–12 становятся пиксель-в-пиксель
 * равны строкам 1–7 — прямое нарушение дизайна, который требует именно
 * отличия. Решение владельца (issue execaus/tube-leak#61, 2026-08-29):
 * явное значение `#555` — тот же оттенок, что уже используется в
 * `DownloadPanel.__step--done`, то есть не новый цвет в проекте. Хардкод
 * остаётся осознанно и идёт в копилку #23 (нет темизации/тёмной темы) —
 * не чинится здесь в обход дизайна. Правку закрывает тест-сторож
 * (`YtDlpUpdateBlock.test.ts`), разбирающий блок `<style>` этого SFC и
 * проверяющий, что правило `--muted` не пусто: удаление декларации
 * `color` вырезает правило из собранного CSS целиком (проверено сборкой,
 * issue #61) и делает пять классов отказа пиксель-в-пиксель равными
 * обычным состояниям.
 *
 * #23 закрыт задачей TL-22: правило `--muted` теперь ссылается на
 * `var(--color-text-muted)` из `src/style.css`, а не на литерал. В
 * светлой теме значение то же самое (`#555`, решение Р-5 эпика E6 выше
 * не пересматривается), в тёмной токен подобран отдельно по контрасту.
 * Тест-сторож ниже проверяет только «декларация непустая» и не завязан
 * на конкретное значение, поэтому продолжает действовать без изменений.
 *
 * # Инлайн-подтверждение отката (TL-60, Р-3)
 *
 * Клик по «Вернуться к {версия}» не откатывает ничего сам — он раскрывает
 * на месте блока инлайн-подтверждение (текст с обеими версиями + кнопки
 * «Вернуться»/«Отмена»), тот же приём необязательного раскрытия, что
 * «Подробнее» у `SidecarStatusRow`: обычный текст с кнопками в
 * естественном порядке табуляции, `role="dialog"` не используется — на
 * экране это не модалка (design, «Ручной откат — полный путь»).
 * `ExitConfirmDialog` (TL-46) сюда не переиспользуется: он зарезервирован
 * за риском потери прогресса активной загрузки, откат версии инструмента
 * не тот случай.
 *
 * «Отмена» просто закрывает подтверждение — ни `emit`, ни изменения
 * статус-строки. «Вернуться» внутри подтверждения закрывает его и
 * эмитит `rollback` — сам вызов команды и применение её ответа делает
 * `rollback()` из `useYtDlpUpdate` в родителе (тот же контракт, что
 * `check`/`checkNow`): этот компонент не решает, применится ли откат
 * немедленно (строка 13) или встанет в ожидание границы задачи (строка
 * 14, Ф-7) — он только просит родителя выполнить действие и рисует то,
 * что вернул новый снимок.
 *
 * # Кнопки гаснут, пока ответ команды в пути (TL-120, issue #127)
 *
 * Регрессия TL-66: без активной загрузки `rollback()` теперь отвечает
 * только после переключения (на холодном дереве — до 24 с), и на это
 * время `props.snapshot?.busy` ещё несёт старое значение — само по себе
 * оно не гасит кнопки. `props.pending` (проекция `useYtDlpUpdate().pending`,
 * doc пропса ниже) — третье, отдельное от `snapshot`, условие в `busy`: без
 * него кнопки выглядели бы живыми, а ядро молча отклоняет повторное
 * нажатие как `busy`. Тот же `busy` теперь и на `aria-busy` секции — та же
 * пара «сам disabled + отражение в атрибуте», что `isLoading` у экрана
 * первого запуска (`App.vue`). Никакого локального прыжка в «применено»:
 * `pending` только держит кнопки неактивными, состояние по-прежнему
 * рисуется из `snapshot`.
 */
import { computed, ref, watch } from 'vue'

import type { YtDlpUpdateSnapshot } from '@/types/generated/update'
import { formatRelativeTime } from '@/utils/formatRelativeTime'
import {
  getYtDlpUpdateStatusText,
  isYtDlpUpdateStatusFailure,
  versionOrPlaceholder,
} from '@/utils/ytDlpUpdateStatusText'

const props = defineProps<{
  /**
   * Снимок контура — `undefined` до первого ответа `ytdlp_update_state`
   * (`useYtDlpUpdate`, композабл ещё не смонтировался/не ответил).
   * Не одно из 14 состояний дизайна — служебная предзагрузочная пауза,
   * тот же приём, что `result === undefined` у `SidecarStatusRow`.
   */
  snapshot?: YtDlpUpdateSnapshot
  /**
   * Активная версия yt-dlp — приходит из уже выполненного `check_sidecar`
   * (`SidecarCheckResult.version`), второй раз здесь не запрашивается
   * (дизайн E6, «Данные для UI»). Отсутствует, пока проверка sidecar не
   * завершилась статусом `ok`.
   */
  activeVersion?: string
  /**
   * «Команда в пути» (TL-120, issue #127) — проекция `useYtDlpUpdate().pending`
   * из родителя, doc там же: истинно с клика «Проверить сейчас»/«Вернуться»
   * и до разрешения/отказа промиса команды. После TL-66 ответ отката без
   * активной загрузки приходит не сразу (на холодном дереве — до 24 с), а
   * `snapshot.busy` до этого момента ещё несёт старое значение — без этого
   * пропса кнопки выглядели бы живыми, пока ядро уже отклоняет повторное
   * нажатие как `busy`. По умолчанию `false` — тот же нейтральный тон, что
   * у остальных необязательных пропсов блока.
   */
  pending?: boolean
}>()

const emit = defineEmits<{
  check: []
  rollback: []
}>()

/** Предзагрузочная пауза до первого снимка — см. doc пропса `snapshot` выше. */
const isBootstrapping = computed(() => props.snapshot === undefined)

const statusText = computed(() => {
  const snapshot = props.snapshot
  if (!snapshot) return 'Загружаем статус обновления…'
  return getYtDlpUpdateStatusText(snapshot, props.activeVersion, formatRelativeTime)
})

/** Приглушённый цвет — только для пяти классов `failed` (строки 8–12), см. doc компонента. */
const isMuted = computed(() => {
  const snapshot = props.snapshot
  return snapshot !== undefined && isYtDlpUpdateStatusFailure(snapshot)
})

/**
 * Обе кнопки блока неактивны, пока конвейер уже идёт — проекция
 * `YtDlpUpdateSnapshot.busy` (doc типа в контракте), не собственное
 * решение компонента. Пока снимка вовсе нет — тоже неактивны: нажатие
 * до первого известного состояния не на чем основывать. `props.pending`
 * (TL-120, doc пропса выше) добавляет третье условие: ответ ещё не
 * пришёл, `snapshot.busy` мог не успеть измениться.
 */
const busy = computed(() => isBootstrapping.value || (props.pending ?? false) || (props.snapshot?.busy ?? true))

/**
 * Кнопка «Вернуться к …» показана только когда на диске есть
 * известно-хорошая установка, отличная от активной (Ф-8, дизайн «Все
 * состояния»): ровно то, что несёт поле `rollbackTarget` снимка — здесь
 * не переоткрывается отдельным условием по статусу.
 */
const rollbackTarget = computed(() => props.snapshot?.rollbackTarget)

/**
 * Раскрыто ли инлайн-подтверждение отката (см. doc компонента, «Инлайн-
 * подтверждение отката») — свёрнуто по умолчанию, тот же приём, что
 * `detailsOpen` у `SidecarStatusRow`.
 */
const confirmingRollback = ref(false)

/**
 * Панель подтверждения рисуется только пока есть куда возвращаться —
 * `rollbackTarget` может пропасть из-под открытой панели, если снимок
 * сменился независимо от локального клика (например, следующим
 * `ytdlp://update`, пока пользователь ещё не решил); без этого условия
 * панель осталась бы висеть с версией, которой уже нет в снимке.
 */
const showRollbackConfirm = computed(() => confirmingRollback.value && rollbackTarget.value !== undefined)

// Тот же случай, что в doc `showRollbackConfirm` выше, но для локального
// флага: если цель отката исчезла, флаг сбрасывается, а не остаётся
// «раскрыт» на панели, которая больше не рисуется (иначе следующее
// появление `rollbackTarget`, скажем для другой пары версий, раскрыло бы
// подтверждение без клика пользователя).
watch(rollbackTarget, (target) => {
  if (target === undefined) {
    confirmingRollback.value = false
  }
})

const rollbackConfirmTitle = computed(() => {
  const target = rollbackTarget.value
  return target === undefined ? '' : `Вернуться на версию ${target}?`
})

const rollbackConfirmText = computed(() => {
  const target = rollbackTarget.value
  if (target === undefined) return ''
  const activeText = versionOrPlaceholder(props.activeVersion)
  return (
    `Сейчас активна ${activeText}. После возврата скачивание будет идти на ${target} — ` +
    `до тех пор, пока апстрим не выпустит более новый релиз, версия ${activeText} не будет ` +
    `предложена автоматически снова.`
  )
})

function onCheckClick(): void {
  emit('check')
}

/** Клик по «Вернуться к {версия}» в основной строке кнопок — только раскрывает подтверждение, ничего не эмитит. */
function onRollbackToggleClick(): void {
  confirmingRollback.value = !confirmingRollback.value
}

/** «Отмена» внутри подтверждения — просто закрывает панель, без `emit` (doc компонента выше). */
function onCancelRollbackClick(): void {
  confirmingRollback.value = false
}

/**
 * «Вернуться» внутри подтверждения — закрывает панель (пользователь уже
 * решил) и передаёт решение родителю. Закрытие панели — локальный UI-факт
 * («вопрос больше не задан»), а не предположение об исходе отката: строка
 * 13 или 14 появится из нового снимка, который принесёт `rollback()`
 * родителя, не отсюда.
 */
function onConfirmRollbackClick(): void {
  confirmingRollback.value = false
  emit('rollback')
}
</script>

<template>
  <section
    class="ytdlp-update-block"
    aria-live="polite"
    :aria-busy="busy"
  >
    <h2 class="ytdlp-update-block__title">
      Обновление yt-dlp
    </h2>
    <p
      class="ytdlp-update-block__status"
      :class="{ 'ytdlp-update-block__status--muted': isMuted }"
    >
      {{ statusText }}
    </p>
    <div class="ytdlp-update-block__actions">
      <button
        type="button"
        class="tap-target"
        :disabled="busy"
        @click="onCheckClick"
      >
        Проверить сейчас
      </button>
      <button
        v-if="rollbackTarget"
        type="button"
        class="tap-target"
        :disabled="busy"
        :aria-expanded="confirmingRollback"
        @click="onRollbackToggleClick"
      >
        Вернуться к {{ rollbackTarget }}
      </button>
    </div>

    <div
      v-if="showRollbackConfirm"
      class="ytdlp-update-block__rollback-confirm"
    >
      <p class="ytdlp-update-block__rollback-confirm-title">
        {{ rollbackConfirmTitle }}
      </p>
      <p class="ytdlp-update-block__rollback-confirm-text">
        {{ rollbackConfirmText }}
      </p>
      <div class="ytdlp-update-block__actions">
        <button
          type="button"
          class="tap-target"
          :disabled="busy"
          @click="onConfirmRollbackClick"
        >
          Вернуться
        </button>
        <button
          type="button"
          class="tap-target"
          @click="onCancelRollbackClick"
        >
          Отмена
        </button>
      </div>
    </div>
  </section>
</template>

<style scoped>
.ytdlp-update-block {
  padding: 0.5rem 0;
}

.ytdlp-update-block__title {
  margin: 0 0 0.25rem;
  font-size: 1rem;
  font-weight: 600;
}

.ytdlp-update-block__status {
  margin: 0;
  max-width: 40rem;
  line-height: 1.4;
}

/*
 * Было явным хардкодом, оставленным осознанно (ревью TL-59 «Н-4», решение
 * владельца issue execaus/tube-leak#61, 2026-08-29, см. doc компонента
 * выше) — тот же `#555`, что уже жил в `DownloadPanel.__step--done`, не
 * новый оттенок в проекте. TL-22 (issue #23) перевёл значение на токен
 * `var(--color-text-muted)` из `src/style.css`; в светлой теме значение
 * не изменилось (`#555`), само решение Р-5 E6 не пересматривается. Пустое
 * правило здесь однажды уже пропадало из собранного CSS целиком
 * (tree-shaking сборщика вырезает правило без деклараций) — доказано
 * сборкой, не рассуждением; отсюда тест-сторож ниже, который разбирает
 * исходный `<style>`-блок этого SFC (jsdom не применяет scoped-стили,
 * поэтому вычисленный цвет смонтированного компонента тут не проверить)
 * и требует непустой декларации `color` в правиле `--muted` — значение
 * декларации тест не проверяет, поэтому переход на переменную его не
 * задевает.
 */
.ytdlp-update-block__status--muted {
  color: var(--color-text-muted);
}

.ytdlp-update-block__actions {
  display: flex;
  gap: 0.5rem;
  margin-top: 0.5rem;
}

/*
 * Инлайн-подтверждение отката (TL-60) — та же секция, не модалка
 * (doc компонента, «Инлайн-подтверждение отката»): без своего фона/тени,
 * только небольшой отступ сверху, чтобы визуально отделить вопрос от
 * строки статуса выше.
 */
.ytdlp-update-block__rollback-confirm {
  margin-top: 0.5rem;
}

.ytdlp-update-block__rollback-confirm-title {
  margin: 0 0 0.25rem;
  font-weight: 600;
}

.ytdlp-update-block__rollback-confirm-text {
  margin: 0;
  max-width: 40rem;
  line-height: 1.4;
}

.tap-target {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 40px;
  min-height: 40px;
  padding: 0.5rem 0.75rem;
  box-sizing: border-box;
}

.tap-target:focus-visible {
  outline: 2px solid var(--color-accent);
  outline-offset: 2px;
}
</style>
