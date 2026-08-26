<script setup lang="ts">
/**
 * Лестница качеств (дизайн E2, раздел «Лестница качеств», решение
 * владельца Р-1): `role="radiogroup"`, каждая строка — `<label>` с
 * настоящим `<input type="radio">`, чтобы клавиатура и скринридер
 * работали без дополнительной разметки.
 *
 * Ничего не выбрано по умолчанию (Р-1 — нет предвыбранного «лучшего
 * качества»). Выбор — только визуальная фиксация текущей карточки: в E2
 * действия «скачать» нет, поэтому наверх ничего не эмитится. Список не
 * сортируется — порядок задаёт ядро (см. doc `ProbeResult.qualities` в
 * `src/types/probe.ts`).
 *
 * Смена карточки (новый разбор или очистка поля) всегда пересоздаёт
 * список `items` новым массивом — выбор сбрасывается по смене ссылки на
 * объект, без сравнения по идентификатору строки.
 *
 * # Эмит выбора (эпик E3, TL-45)
 *
 * В E2 выбор был только визуальной фиксацией текущей карточки — действия
 * «скачать» не было, и наверх ничего не эмитилось. E3 добавляет кнопку
 * «Скачать» в `VideoCard`, которой нужен сам выбранный пункт — компонент
 * начинает сообщать его через `update:selected` (v-model), оставаясь тем
 * же компонентом: список, порядок, отсутствие предвыбранного пункта (Р-1)
 * не меняются ни строкой.
 */
import { ref, useId, watch } from 'vue'

import type { QualityItem } from '@/types/probe'
import { formatApproxSize } from '@/utils/formatApproxSize'
import { qualityLabel } from '@/utils/qualityLabel'

const props = defineProps<{
  items: QualityItem[]
}>()

const emit = defineEmits<{
  'update:selected': [item: QualityItem | undefined]
}>()

const selectedIndex = ref<number>()

/**
 * `name` радио-группы должен быть уникален на экране — общая строка
 * (`"quality"`) сегодня безобидна (в E2 лестница ровно одна), но E4
 * положит на экран несколько карточек с собственными лестницами, и общее
 * имя склеило бы их в одну группу выбора (замечание ревью TL-33).
 */
const groupName = `quality-${useId()}`

watch(
  () => props.items,
  () => {
    selectedIndex.value = undefined
    emit('update:selected', undefined)
  },
)

function select(index: number): void {
  selectedIndex.value = index
  emit('update:selected', props.items[index])
}
</script>

<template>
  <div
    role="radiogroup"
    aria-label="Качество скачивания"
    class="ladder"
  >
    <label
      v-for="(item, index) in items"
      :key="index"
      class="ladder__row tap-target"
    >
      <input
        type="radio"
        :name="groupName"
        class="ladder__radio"
        :checked="selectedIndex === index"
        @change="select(index)"
      >
      <span class="ladder__label">{{ qualityLabel(item) }}</span>
      <span class="ladder__size">{{ formatApproxSize(item.size) }}</span>
    </label>
  </div>
</template>

<style scoped>
.ladder {
  display: flex;
  flex-direction: column;
  margin-top: 0.75rem;
}

.ladder__row {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  min-height: 40px;
  padding: 0.25rem 0.5rem;
  box-sizing: border-box;
  cursor: pointer;
}

.ladder__row:focus-within {
  outline: 2px solid #1a73e8;
  outline-offset: 2px;
}

.ladder__radio {
  width: 1.1rem;
  height: 1.1rem;
  flex-shrink: 0;
}

.ladder__label {
  flex: 1;
}

.ladder__size {
  color: #555;
}

.tap-target {
  min-width: 40px;
  min-height: 40px;
}
</style>
