<script setup lang="ts">
/**
 * Карточка ролика — успешный результат разбора (дизайн E2, состояние 2).
 * Название рисуется как есть, без обрезания (К-1 требует посимвольного
 * совпадения с YouTube — `white-space: normal` вместо ellipsis).
 * Имя канала и превью не рисуются, если данных нет (Ф-5, Р-2).
 *
 * Превью — `alt=""` (декоративное): название уже показано текстом рядом,
 * иначе скринридер зачитал бы его дважды подряд.
 *
 * # Кнопка «Скачать» (эпик E3, дизайн «Кнопка Скачать»)
 *
 * Живёт здесь же, под лестницей качеств — несколько строк, физически
 * часть той же карточки, не отдельная ui-задача (декомпозиция E3, «Что
 * сознательно не резалось мельче»). Неактивна в двух независимых
 * случаях с разным текстом-подсказкой: ничего не выбрано (подсказки нет —
 * самообъясняющееся состояние лестницы) и уже идёт другая задача
 * (`downloadBlocked`, С-13 — независимо от того, к этому же ролику она
 * относится или к другому).
 *
 * Сама карточка не хранит и не запускает задачу скачивания — она лишь
 * эмитит снимок «что скачивать» в момент клика (заголовок, потоки,
 * подпись качества); что происходит с этим снимком дальше (стор
 * `useDownloadTaskStore`, независимая от карточки панель) её не касается —
 * это и есть требование С-13 «панель переживает замену карточки».
 */
import { computed, ref, useId } from 'vue'

import type { ProbeResult, QualityItem, QualitySize, QualityStreams } from '@/types/generated/probe'
import { formatDuration } from '@/utils/formatDuration'
import { qualityLabel } from '@/utils/qualityLabel'

import QualityLadder from './QualityLadder.vue'
import VideoThumbnail from './VideoThumbnail.vue'

const props = withDefaults(
  defineProps<{
    result: ProbeResult
    /** Уже идёт другая задача скачивания (нетерминальная фаза) — С-13. */
    downloadBlocked?: boolean
  }>(),
  {
    downloadBlocked: false,
  },
)

const emit = defineEmits<{
  download: [payload: { title: string; streams: QualityStreams; size: QualitySize; qualityLabel: string }]
}>()

const selected = ref<QualityItem>()

function onSelect(item: QualityItem | undefined): void {
  selected.value = item
}

const downloadDisabled = computed(() => selected.value === undefined || props.downloadBlocked)

/**
 * Подсказка под кнопкой — только для случая «занято другой задачей»
 * (дизайн: «ничего не выбрано» — без подсказки, самообъясняющееся
 * состояние лестницы).
 */
const downloadHint = computed(() =>
  props.downloadBlocked
    ? 'Уже идёт другая загрузка. Дождитесь её завершения или отмените её ниже, чтобы начать новую.'
    : undefined,
)

/**
 * Связывает подсказку с кнопкой для скринридера (`aria-describedby`) —
 * ревью TL-45, «Заметки»: заблокированная кнопка не получает фокус (это
 * обычное и ожидаемое поведение `disabled`), но клавиатурный пользователь,
 * дошедший до неё виртуальным курсором чтения, должен слышать не только
 * «Скачать, недоступно», а и причину — без явной связи подсказка была
 * соседним, никак не привязанным к кнопке абзацем.
 */
const downloadHintId = useId()

function onDownloadClick(): void {
  const item = selected.value
  if (!item) return
  emit('download', {
    title: props.result.title,
    streams: item.streams,
    // Оценка размера того же пункта — стартовый знаменатель агрегации
    // прогресса в ядре (TL-41); берётся отсюда же, где и `streams`, не
    // собирается отдельно (правило контракта `StartDownloadRequest.size`).
    size: item.size,
    qualityLabel: qualityLabel(item),
  })
}
</script>

<template>
  <article class="video-card">
    <div class="video-card__top">
      <VideoThumbnail
        v-if="result.thumbnailUrl"
        :src="result.thumbnailUrl"
        alt=""
      />
      <div class="video-card__info">
        <h2 class="video-card__title">
          {{ result.title }}
        </h2>
        <p
          v-if="result.channel"
          class="video-card__channel"
        >
          {{ result.channel }}
        </p>
        <p class="video-card__duration">
          {{ formatDuration(result.durationSecs) }}
        </p>
      </div>
    </div>

    <QualityLadder
      :items="result.qualities"
      @update:selected="onSelect"
    />

    <button
      type="button"
      class="tap-target video-card__download"
      :disabled="downloadDisabled"
      :aria-describedby="downloadHint ? downloadHintId : undefined"
      @click="onDownloadClick"
    >
      Скачать
    </button>
    <p
      v-if="downloadHint"
      :id="downloadHintId"
      class="video-card__download-hint"
    >
      {{ downloadHint }}
    </p>
  </article>
</template>

<style scoped>
.video-card {
  margin-top: 0.75rem;
}

.video-card__top {
  display: flex;
  flex-wrap: wrap;
  gap: 0.75rem;
  align-items: flex-start;
}

.video-card__info {
  min-width: 0;
  flex: 1;
}

.video-card__title {
  margin: 0 0 0.25rem;
  font-size: 1.05rem;
  font-weight: 600;
  line-height: 1.35;
  white-space: normal;
  overflow-wrap: break-word;
}

.video-card__channel {
  margin: 0 0 0.25rem;
  color: #555;
  font-size: 0.9rem;
}

.video-card__duration {
  margin: 0;
  color: #555;
}

.video-card__download {
  margin-top: 0.75rem;
}

.video-card__download-hint {
  margin: 0.35rem 0 0;
  font-size: 0.85rem;
  color: #777;
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
