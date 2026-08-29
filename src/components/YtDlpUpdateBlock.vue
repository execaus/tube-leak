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
 * текста, тот же, что у пояснений `SidecarStatusRow` (дизайн, «Где живёт
 * блок»).
 *
 * # Откат — не отсюда (TL-60)
 *
 * Кнопка «Вернуться к …» здесь — только видимость и надпись по правилам
 * таблицы; инлайн-подтверждение и вызов команды отката (Р-3) — отдельная
 * задача TL-60, которая достраивает обработчик `rollback` этого же
 * компонента, не переписывая его.
 */
import { computed } from 'vue'

import type { YtDlpUpdateSnapshot } from '@/types/generated/update'
import { formatRelativeTime } from '@/utils/formatRelativeTime'
import { getYtDlpUpdateStatusText, isYtDlpUpdateStatusFailure } from '@/utils/ytDlpUpdateStatusText'

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
 * до первого известного состояния не на чем основывать.
 */
const busy = computed(() => isBootstrapping.value || (props.snapshot?.busy ?? true))

/**
 * Кнопка «Вернуться к …» показана только когда на диске есть
 * известно-хорошая установка, отличная от активной (Ф-8, дизайн «Все
 * состояния»): ровно то, что несёт поле `rollbackTarget` снимка — здесь
 * не переоткрывается отдельным условием по статусу.
 */
const rollbackTarget = computed(() => props.snapshot?.rollbackTarget)

function onCheckClick(): void {
  emit('check')
}

function onRollbackClick(): void {
  emit('rollback')
}
</script>

<template>
  <section
    class="ytdlp-update-block"
    aria-live="polite"
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
        @click="onRollbackClick"
      >
        Вернуться к {{ rollbackTarget }}
      </button>
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

.ytdlp-update-block__status--muted {
  color: #555;
}

.ytdlp-update-block__actions {
  display: flex;
  gap: 0.5rem;
  margin-top: 0.5rem;
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
  outline: 2px solid #1a73e8;
  outline-offset: 2px;
}
</style>
