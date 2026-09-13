<script setup lang="ts">
/**
 * Экран истории (эпик E5, TL-93; Ф-4…Ф-6, Ф-8, С-1…С-4, С-10; дизайн E5,
 * раздел «2. Экран истории»). Заголовок «История» (фокус-цель переключения
 * вкладок, К-14) остаётся в `App.vue` — этот компонент рисует всё, что
 * дизайн кладёт под ним, тем же разделением ответственности, что
 * `ProbeSection`/`QueueSection` под `<h1>` «Главного».
 *
 * Состояние и все вызовы `invoke` живут в `useHistoryStore` (один Pinia-стор
 * на домен, CLAUDE.md) — компонент только читает срез стора и эмитит клики
 * в его действия, тот же приём, что `QueueSection`/`downloadTaskStore`.
 */
import { computed, onMounted, ref } from 'vue'

import { useHistoryStore } from '@/stores/history'
import type { HistoryEntry } from '@/types/generated/history'
import { getFolderDisplayText } from '@/utils/downloadOutcomeTexts'
import { fileContainer } from '@/utils/fileContainer'
import { formatApproxSize } from '@/utils/formatApproxSize'
import { formatRelativeTime } from '@/utils/formatRelativeTime'
import {
  getHistoryCommandErrorText,
  NON_CONTRACTUAL_HISTORY_COMMAND_ERROR_TEXT,
} from '@/utils/historyCommandErrorTexts'
import { getHistoryFileStatusText } from '@/utils/historyFileStatusTexts'
import { getHistoryNoticeText } from '@/utils/historyNoticeTexts'
import { getHistoryUnavailableText } from '@/utils/historyUnavailableTexts'
import {
  getLauncherFailureDetails,
  getShowInFolderErrorText,
  NON_CONTRACTUAL_SHOW_IN_FOLDER_ERROR_TEXT,
} from '@/utils/historyShowInFolderTexts'
import { formatTaskDisplayTitle } from '@/utils/queueTaskTitle'
import { unixSecsToIso } from '@/utils/unixSecsToIso'

import ClearHistoryConfirmDialog from './ClearHistoryConfirmDialog.vue'

const store = useHistoryStore()

onMounted(() => {
  void store.initialize()
})

function entryMetaLine(entry: HistoryEntry): string {
  const parts = [
    fileContainer(entry.fileName),
    formatApproxSize({ kind: 'known', bytes: entry.sizeBytes }),
    formatRelativeTime(unixSecsToIso(entry.finishedAtUnixSecs)),
  ].filter((part) => part.length > 0)
  return parts.join(' · ')
}

/** Третья строка записи: путь папки для `present`, статус файла для `missing` (Ф-5, таблица трёх случаев). */
function entryThirdLine(entry: HistoryEntry): string {
  return getHistoryFileStatusText(entry.fileStatus, entry.folderDisplay) ?? getFolderDisplayText(entry.folderDisplay)
}

/** «Показать в папке» — отсутствует ровно для «нет файла, нет папки» (Ф-8, таблица трёх случаев, строка 3). */
function showButtonVisible(entry: HistoryEntry): boolean {
  return entry.fileStatus.kind === 'present' || entry.fileStatus.folderExists
}

function deleteAriaLabel(entry: HistoryEntry): string {
  return `Удалить запись ${formatTaskDisplayTitle(entry.title, entry.quality)} из истории`
}

function showInFolderErrorText(entry: HistoryEntry) {
  const failure = store.showInFolderErrors[entry.id]
  if (!failure) return undefined
  return failure.kind === undefined ? NON_CONTRACTUAL_SHOW_IN_FOLDER_ERROR_TEXT : getShowInFolderErrorText(failure)
}

function showInFolderErrorDetails(entry: HistoryEntry) {
  const failure = store.showInFolderErrors[entry.id]
  if (!failure || failure.kind === undefined) return undefined
  return getLauncherFailureDetails(failure)
}

const commandErrorText = computed(() => {
  const failure = store.commandError
  if (!failure) return undefined
  return failure.kind === undefined ? NON_CONTRACTUAL_HISTORY_COMMAND_ERROR_TEXT : getHistoryCommandErrorText(failure)
})

/**
 * Число для тела диалога очистки (пункт 2 дизайна, «Очистить —
 * подтверждение»): только когда список точно долистан до конца
 * (`nextCursor` отсутствует) — иначе оно не гарантированно полное.
 */
const knownRecordCountForClear = computed(() => (store.nextCursor ? undefined : store.entries.length))

const showClearConfirm = ref(false)

function onClearConfirmed(): void {
  showClearConfirm.value = false
  void store.clearHistory()
}
</script>

<template>
  <div class="history-screen">
    <template v-if="store.availability">
      <p class="history-screen__unavailable">
        {{ getHistoryUnavailableText(store.availability.reason) }}
      </p>
    </template>

    <template v-else>
      <ul
        v-if="store.notices.length > 0"
        class="history-screen__notices"
      >
        <li
          v-for="notice in store.notices"
          :key="notice.kind"
          class="history-screen__banner"
          role="status"
        >
          <p class="history-screen__banner-text">
            ⚠ {{ getHistoryNoticeText(notice) }}
          </p>
          <button
            type="button"
            class="tap-target"
            @click="store.dismissNotice(notice.kind)"
          >
            Скрыть
          </button>
        </li>
      </ul>

      <p
        v-if="commandErrorText"
        class="history-screen__command-error"
        role="alert"
      >
        {{ commandErrorText.title }}: {{ commandErrorText.explanation }}
        <button
          type="button"
          class="tap-target"
          @click="store.dismissCommandError"
        >
          Скрыть
        </button>
      </p>

      <template v-if="store.loaded">
        <p
          v-if="store.entries.length === 0"
          class="history-screen__empty"
        >
          История пуста. Здесь появятся ролики после первой завершённой загрузки.
        </p>

        <template v-else>
          <div class="history-screen__toolbar">
            <button
              type="button"
              class="tap-target"
              @click="showClearConfirm = true"
            >
              Очистить
            </button>
          </div>

          <ul class="history-screen__list">
            <li
              v-for="entry in store.entries"
              :key="entry.id"
              class="history-screen__entry"
            >
              <p class="history-screen__entry-title">
                {{ formatTaskDisplayTitle(entry.title, entry.quality) }}
              </p>
              <p class="history-screen__entry-meta">
                {{ entryMetaLine(entry) }}
              </p>
              <p class="history-screen__entry-line">
                {{ entryThirdLine(entry) }}
              </p>

              <p
                v-if="showInFolderErrorText(entry)"
                class="history-screen__entry-error"
                role="alert"
              >
                {{ showInFolderErrorText(entry)!.title }}: {{ showInFolderErrorText(entry)!.explanation }}
              </p>
              <details v-if="showInFolderErrorDetails(entry)">
                <summary>Подробнее</summary>
                <dl class="history-screen__entry-error-details">
                  <template v-if="showInFolderErrorDetails(entry)?.exitCode !== undefined">
                    <dt>Код выхода</dt>
                    <dd>{{ showInFolderErrorDetails(entry)?.exitCode }}</dd>
                  </template>
                  <template v-if="showInFolderErrorDetails(entry)?.stderrTail">
                    <dt>stderr</dt>
                    <dd>{{ showInFolderErrorDetails(entry)?.stderrTail }}</dd>
                  </template>
                </dl>
              </details>

              <div class="history-screen__entry-actions">
                <button
                  v-if="showButtonVisible(entry)"
                  type="button"
                  class="tap-target"
                  @click="store.showInFolder(entry)"
                >
                  Показать в папке
                </button>
                <button
                  type="button"
                  class="tap-target"
                  :aria-label="deleteAriaLabel(entry)"
                  @click="store.deleteRecord(entry.id)"
                >
                  Удалить
                </button>
              </div>
            </li>
          </ul>

          <div
            v-if="store.nextCursor"
            class="history-screen__load-more"
          >
            <button
              type="button"
              class="tap-target"
              :disabled="store.isLoadingMore"
              @click="store.loadMore"
            >
              {{ store.isLoadingMore ? 'Загружаем…' : 'Показать ещё' }}
            </button>
          </div>
        </template>
      </template>
      <p
        v-else
        class="history-screen__loading"
        aria-live="polite"
      >
        Загружаем историю…
      </p>
    </template>

    <ClearHistoryConfirmDialog
      v-if="showClearConfirm"
      :known-record-count="knownRecordCountForClear"
      @cancel="showClearConfirm = false"
      @confirm="onClearConfirmed"
    />
  </div>
</template>

<style scoped>
.history-screen__unavailable {
  max-width: 40rem;
  line-height: 1.4;
}

.history-screen__notices {
  list-style: none;
  margin: 0 0 0.75rem;
  padding: 0;
}

.history-screen__banner {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 0.75rem;
  padding: 0.75rem;
  margin-bottom: 0.5rem;
  background: var(--color-accent-soft);
  border-radius: 0.3rem;
  box-shadow: inset 0 0 0 1px var(--color-banner-border);
}

.history-screen__banner-text {
  margin: 0;
  color: var(--color-text-secondary);
}

.history-screen__command-error {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  margin: 0 0 0.75rem;
  color: var(--color-error);
}

.history-screen__empty,
.history-screen__loading {
  color: var(--color-text-muted);
}

.history-screen__toolbar {
  display: flex;
  justify-content: flex-end;
  margin-bottom: 0.5rem;
}

.history-screen__list {
  list-style: none;
  margin: 0;
  padding: 0;
}

.history-screen__entry {
  padding: 0.75rem 0;
  border-bottom: 1px solid var(--color-border-subtle);
}

.history-screen__entry-title {
  margin: 0 0 0.25rem;
  font-weight: 600;
}

.history-screen__entry-meta {
  margin: 0 0 0.25rem;
  color: var(--color-text-muted);
}

.history-screen__entry-line {
  margin: 0 0 0.5rem;
  color: var(--color-text-secondary);
  word-break: break-word;
}

.history-screen__entry-error {
  margin: 0 0 0.5rem;
  color: var(--color-error);
}

.history-screen__entry-actions {
  display: flex;
  gap: 0.5rem;
}

.history-screen__load-more {
  margin-top: 0.75rem;
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
