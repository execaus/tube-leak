<script setup lang="ts">
/**
 * Карточка ролика — успешный результат разбора (дизайн E2, состояние 2).
 * Название рисуется как есть, без обрезания (К-1 требует посимвольного
 * совпадения с YouTube — `white-space: normal` вместо ellipsis).
 * Имя канала и превью не рисуются, если данных нет (Ф-5, Р-2).
 *
 * Превью — `alt=""` (декоративное): название уже показано текстом рядом,
 * иначе скринридер зачитал бы его дважды подряд.
 */
import type { ProbeResult } from '@/types/probe'
import { formatDuration } from '@/utils/formatDuration'

import QualityLadder from './QualityLadder.vue'
import VideoThumbnail from './VideoThumbnail.vue'

defineProps<{
  result: ProbeResult
}>()
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

    <QualityLadder :items="result.qualities" />
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
</style>
