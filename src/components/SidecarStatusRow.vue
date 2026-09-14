<script setup lang="ts">
/**
 * Одна строка служебного экрана проверки sidecar (Ф-9, TL-8): значок,
 * подпись, статус и — для состояний ошибки — пояснительный текст плюс
 * свёрнутый по умолчанию блок технических подробностей.
 *
 * Компонент только отображает то, что ему передали, — сам `invoke` не
 * вызывает (это делает `useSidecarCheck` в родителе, App.vue).
 */
import { computed, ref } from 'vue'

import type { LaunchFailedReason, SidecarCheckResult } from '@/types/generated/sidecar'
import { assertNever } from '@/utils/assertNever'

const props = defineProps<{
  /**
   * Подпись строки, пока результат ещё не пришёл (состояние «Проверяем…»):
   * данные о `name` в этот момент ещё не заполнены сервером. Как только
   * `result` появляется, подпись берётся из `result.name`, а не отсюда —
   * это и есть данные, «а не сортировка/хардкод на фронте».
   */
  fallbackName: string
  result?: SidecarCheckResult
}>()

/** Раскрыт ли блок «Подробнее» (свёрнут по умолчанию для любой ошибки). */
const detailsOpen = ref(false)

const displayName = computed(() => props.result?.name ?? props.fallbackName)

/** `undefined` result => состояние «Проверяем…» (ответ команды ещё не пришёл). */
const isChecking = computed(() => props.result === undefined)
const isOk = computed(() => props.result?.status === 'ok')
const isError = computed(() => props.result !== undefined && props.result.status !== 'ok')

const icon = computed(() => {
  if (isChecking.value) return '○'
  return isOk.value ? '✓' : '✕'
})

const iconLabel = computed(() => {
  if (isChecking.value) return 'проверка идёт'
  return isOk.value ? 'в порядке' : 'ошибка'
})

const statusText = computed(() => {
  const result = props.result
  if (!result) return 'Проверяем…'
  switch (result.status) {
    case 'ok':
      return result.version ?? ''
    case 'notFound':
      return 'не найден'
    case 'launchFailed':
      // `unrecognizedOutput` (TL-113): бинарник запустился и завершился
      // успешно — «не удалось запустить» здесь было бы неправдой, короткий
      // статус называет то, что реально произошло.
      return result.reason === 'unrecognizedOutput' ? 'неожиданный ответ' : 'не удалось запустить'
    case 'nonZeroExit':
      return 'не удалось запустить'
    case 'timeout':
      return 'не отвечает'
    default:
      // Пять перечисленных веток покрывают весь `SidecarStatus` — эта
      // ветка недостижима сегодня и остаётся сторожем: новый вариант
      // объединения не сузится до `never`, и `npm run type-check`
      // откажется собирать вызов (TL-52 ревью, дефект класса TL-18).
      return assertNever(result.status)
  }
})

function capitalize(value: string): string {
  return value.length === 0 ? value : value.charAt(0).toUpperCase() + value.slice(1)
}

/**
 * Тексты `launchFailed` по под-причине (Ф-9): раньше это был `if/else` по
 * `permissionDenied`, где ветка `else` молча приписывала «похоже, он
 * повреждён» и варианту `other`, и отсутствующему `reason` — то есть
 * пользователю называли причину, которую ядро не утверждало (ревью
 * TL-52). `other` получает отдельный нейтральный текст, который не
 * называет причину: формулировки `permissionDenied`/`corrupted` не
 * менялись ни на символ.
 *
 * `Record<LaunchFailedReason, ...>` — тот же приём, что `KNOWN_ERROR_KINDS`
 * в `useProbe.ts`/`useYtDlpPrepare.ts`: пропущенный вариант ловится
 * `npm run type-check`, а не тихой веткой `else`.
 */
const LAUNCH_FAILED_EXPLANATIONS: Record<LaunchFailedReason, (name: string, path: string) => string> = {
  permissionDenied: (name, path) =>
    `У файла ${name} нет прав на выполнение. Такое случается, если архив с приложением ` +
    `распаковывали вручную сторонним инструментом. Решение: выполните в терминале ` +
    `chmod +x «${path}» либо переустановите tube-leak обычным способом.`,
  corrupted: (name) =>
    `Файл ${name} найден, но не запустился. Похоже, он повреждён — например, был усечён ` +
    `при скачивании или заблокирован антивирусом на лету. Попробуйте переустановить tube-leak.`,
  // Нейтральный текст: причина не установлена, и здесь она не
  // додумывается за ядро — в отличие от `corrupted`, ничего не
  // утверждается про повреждение файла.
  other: (name) =>
    `Файл ${name} найден, но не запустился, а точную причину определить не удалось. ` +
    `Попробуйте переустановить tube-leak; если не поможет — посмотрите код ошибки ОС в «Подробнее» ниже.`,
  // `unrecognizedOutput` (TL-113, ядро TL-109): в отличие от всех
  // остальных причин, бинарник здесь именно запустился и завершился без
  // ошибки — «не запустился»/«код ошибки ОС» были бы неправдой (ядро их и
  // не присылает: `osErrorCode` для этой причины пуст). Правдивая версия:
  // ответ есть, но в нём нет ожидаемой строки версии — типичный симптом
  // подмены файла под тем же именем.
  unrecognizedOutput: (name) =>
    `Файл ${name} запустился и завершился без ошибок, но ответил не так, как мы ожидали: ` +
    `версии в выводе нет. Похоже, под этим именем лежит не тот файл — он подменён или ` +
    `повреждён. Попробуйте переустановить tube-leak; вывод процесса — в «Подробнее» ниже.`,
}

const explanation = computed(() => {
  const result = props.result
  if (!result) return undefined
  const name = result.name

  switch (result.status) {
    // Проверка `ok` не имеет пояснения — успех ничего не объясняет.
    case 'ok':
      return undefined
    case 'notFound':
      return (
        `Не нашли файл ${name} по ожидаемому пути. Возможно, антивирус удалил его в карантин, ` +
        `либо он был случайно удалён вместе с частью установки. Попробуйте переустановить tube-leak.`
      )
    case 'launchFailed':
      // `reason` объявлен опциональным во всём контракте (заполняется
      // только при `status === 'launchFailed'`) — на случай его
      // фактического отсутствия здесь используется тот же нейтральный
      // текст, что и для `other`, а не молчаливое приписывание
      // повреждения.
      return LAUNCH_FAILED_EXPLANATIONS[result.reason ?? 'other'](name, result.path)
    case 'nonZeroExit':
      return `${capitalize(name)} запустился, но завершился с ошибкой (код выхода: ${result.exitCode ?? '—'}).`
    case 'timeout': {
      const seconds = result.timeoutMs !== undefined ? Math.round(result.timeoutMs / 1000) : '—'
      return (
        `Проверка ${name} не завершилась за отведённое время (${seconds} с). Чаще всего это значит, ` +
        `что антивирус (Windows) или Gatekeeper (macOS) блокирует запуск файла в фоне. ` +
        `В документации проекта описано, как это обойти.`
      )
    }
    default:
      // См. doc `statusText` выше — тот же сторож `assertNever`.
      return assertNever(result.status)
  }
})

interface DetailEntry {
  label: string
  value: string
}

/**
 * Технические подробности для свёрнутого блока «Подробнее» — только те
 * поля, что реально заполнены для данного состояния (Ф-8/С-6: технические
 * коды остаются здесь, а не в основном тексте).
 */
const details = computed<DetailEntry[]>(() => {
  const result = props.result
  if (!result || result.status === 'ok') return []

  const entries: DetailEntry[] = [{ label: 'Путь', value: result.path }]

  if (result.osErrorCode !== undefined) {
    entries.push({ label: 'Код ошибки ОС', value: result.osErrorCode })
  }
  if (result.exitCode !== undefined) {
    entries.push({ label: 'Код выхода', value: String(result.exitCode) })
  }
  if (result.timeoutMs !== undefined) {
    entries.push({ label: 'Таймаут', value: `${result.timeoutMs} мс` })
  }
  if (result.stderrTail) {
    // `unrecognizedOutput` (TL-113): это не stderr процесса, упавшего с
    // ошибкой, а полный вывод (stdout, затем stderr) процесса, который
    // отработал успешно и просто ответил не то, что ожидалось —
    // подпись «stderr» здесь была бы неправдой.
    const isUnrecognizedOutput =
      result.status === 'launchFailed' && result.reason === 'unrecognizedOutput'
    entries.push({ label: isUnrecognizedOutput ? 'Вывод' : 'stderr', value: result.stderrTail })
  }

  return entries
})

const isTimeout = computed(() => props.result?.status === 'timeout')

// Раздел README про обход блокировок антивирусом (Windows) / Gatekeeper
// (macOS) при первом запуске sidecar-процесса. Якорь — явный HTML-анкор
// в README.md (не автосгенерированный GitHub-слаг заголовка), чтобы
// ссылка не ломалась при правках текста заголовка.
const instructionsUrl =
  'https://github.com/execaus/tube-leak#sidecar-blocked-by-av-gatekeeper'

function toggleDetails(): void {
  detailsOpen.value = !detailsOpen.value
}
</script>

<template>
  <div class="sidecar-row">
    <div class="sidecar-row__summary">
      <span
        class="sidecar-row__icon"
        :class="{ 'sidecar-row__icon--spin': isChecking }"
        aria-hidden="true"
      >{{
        icon
      }}</span>
      <span class="sidecar-row__name">{{ displayName }}</span>
      <span class="sidecar-row__status">
        {{ statusText }}
        <span class="visually-hidden">({{ iconLabel }})</span>
      </span>
    </div>

    <p
      v-if="isError && explanation"
      class="sidecar-row__explanation"
    >
      {{ explanation }}
    </p>

    <div
      v-if="isError"
      class="sidecar-row__actions"
    >
      <button
        type="button"
        class="tap-target"
        :aria-expanded="detailsOpen"
        :aria-label="`Подробнее о ${displayName}`"
        @click="toggleDetails"
      >
        Подробнее {{ detailsOpen ? '▴' : '▾' }}
      </button>
      <a
        v-if="isTimeout"
        class="tap-target"
        :href="instructionsUrl"
        target="_blank"
        rel="noopener noreferrer"
      >
        Инструкция →
      </a>
    </div>

    <dl
      v-if="isError && detailsOpen"
      class="sidecar-row__details"
    >
      <template
        v-for="entry in details"
        :key="entry.label"
      >
        <dt>{{ entry.label }}</dt>
        <dd>{{ entry.value }}</dd>
      </template>
    </dl>
  </div>
</template>

<style scoped>
.sidecar-row {
  padding: 0.5rem 0;
}

.sidecar-row__summary {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.sidecar-row__icon {
  display: inline-block;
  width: 1.25em;
  text-align: center;
}

.sidecar-row__icon--spin {
  animation: sidecar-row-spin 1.2s linear infinite;
}

@keyframes sidecar-row-spin {
  from {
    transform: rotate(0deg);
  }
  to {
    transform: rotate(360deg);
  }
}

.sidecar-row__name {
  font-weight: 600;
  min-width: 5rem;
}

.sidecar-row__explanation {
  margin: 0.5rem 0 0;
  max-width: 40rem;
  font-size: 1rem;
  line-height: 1.4;
}

.sidecar-row__actions {
  display: flex;
  gap: 0.5rem;
  margin-top: 0.25rem;
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

.sidecar-row__details {
  margin: 0.5rem 0 0;
  padding: 0.5rem;
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 0.8rem;
  background: var(--color-surface-subtle);
  white-space: pre-wrap;
  word-break: break-word;
}

.sidecar-row__details dt {
  font-weight: 600;
}

.sidecar-row__details dd {
  margin: 0 0 0.5rem;
}

.visually-hidden {
  position: absolute;
  width: 1px;
  height: 1px;
  padding: 0;
  margin: -1px;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
  white-space: nowrap;
  border: 0;
}
</style>
