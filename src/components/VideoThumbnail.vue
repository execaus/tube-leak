<script setup lang="ts">
/**
 * Превью 16:9 карточки ролика (дизайн E2, раздел «Превью», решение
 * владельца Р-2). Только рендерится, если у карточки вообще есть
 * `thumbnailUrl` — родитель не монтирует этот компонент при его отсутствии
 * (то же правило, что и для имени канала: нет данных — элемент просто не
 * рисуется).
 *
 * «Грузится» и «не загрузилось» — сознательно один и тот же статичный
 * плейсхолдер без анимации (никакого спиннера): пользователю всё равно
 * нечего с этим сделать, а вечно крутящийся значок на неудачной загрузке
 * выглядел бы как ещё один «не отвечает» (урок E1). `<img>` рендерится
 * всегда (когда есть `src`), чтобы получать `load`/`error`, но визуально
 * скрыт до успешной загрузки — плейсхолдер лежит в той же области поверх
 * него, `object-fit: cover` не даёт сдвига layout при подстановке картинки.
 */
import { ref, watch } from 'vue'

const props = defineProps<{
  src: string
  alt: string
}>()

const loaded = ref(false)

// Смена ссылки (новая карточка) должна начинать состояние загрузки заново.
watch(
  () => props.src,
  () => {
    loaded.value = false
  },
)

function onLoad(): void {
  loaded.value = true
}

function onError(): void {
  loaded.value = false
}
</script>

<template>
  <div class="thumb">
    <img
      :key="props.src"
      :src="props.src"
      :alt="props.alt"
      class="thumb__img"
      :class="{ 'thumb__img--hidden': !loaded }"
      @load="onLoad"
      @error="onError"
    >
    <span
      v-show="!loaded"
      class="thumb__placeholder"
      aria-hidden="true"
    >🎬</span>
  </div>
</template>

<style scoped>
.thumb {
  position: relative;
  width: 160px;
  height: 90px;
  flex-shrink: 0;
  overflow: hidden;
  background: #eee;
  border: 1px solid #ccc;
}

.thumb__img {
  display: block;
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.thumb__img--hidden {
  position: absolute;
  top: 0;
  left: 0;
  opacity: 0;
}

.thumb__placeholder {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 2rem;
  color: #999;
}
</style>
