<script setup lang="ts">
import { onUnmounted, ref, shallowRef } from 'vue'
import { check, type Update } from '@tauri-apps/plugin-updater'

const props = defineProps<{ currentVersion: string; queueBusy: boolean }>()

const available = shallowRef<Update | null>(null)
const checking = ref(false)
const installing = ref(false)
const installed = ref(false)
const checked = ref(false)
const error = ref('')
const downloaded = ref(0)
const total = ref<number | undefined>()

async function releaseUpdate(): Promise<void> {
  const old = available.value
  available.value = null
  if (old) await old.close()
}

onUnmounted(() => { void releaseUpdate() })

async function checkForUpdate(): Promise<void> {
  if (checking.value || installing.value) return
  checking.value = true
  checked.value = false
  installed.value = false
  error.value = ''
  try {
    await releaseUpdate()
    available.value = await check()
    checked.value = true
  } catch (cause) {
    error.value = `Не удалось проверить обновления: ${String(cause)}`
  } finally {
    checking.value = false
  }
}

async function installUpdate(): Promise<void> {
  const update = available.value
  if (!update || installing.value || props.queueBusy) return
  installing.value = true
  error.value = ''
  downloaded.value = 0
  total.value = undefined
  try {
    await update.downloadAndInstall((event) => {
      if (event.event === 'Started') total.value = event.data.contentLength
      if (event.event === 'Progress') downloaded.value += event.data.chunkLength
    })
    installed.value = true
    await releaseUpdate()
  } catch (cause) {
    error.value = `Не удалось установить обновление: ${String(cause)}`
  } finally {
    installing.value = false
  }
}
</script>

<template>
  <section class="settings-screen__field app-update">
    <h3>Обновление приложения</h3>
    <p>Установлена версия {{ currentVersion }}. Проверка выполняется только по нажатию кнопки.</p>
    <button
      type="button"
      class="tap-target"
      :disabled="checking || installing"
      @click="checkForUpdate"
    >
      {{ checking ? 'Проверяем…' : 'Проверить обновления' }}
    </button>
    <p
      v-if="checked && !available && !installed"
      role="status"
    >
      У вас последняя версия.
    </p>
    <div
      v-if="available"
      role="status"
    >
      <p>Доступна версия {{ available.version }}.</p>
      <p
        v-if="available.body"
        class="app-update__notes"
      >
        {{ available.body }}
      </p>
      <button
        type="button"
        class="tap-target"
        :disabled="installing || queueBusy"
        @click="installUpdate"
      >
        {{ installing ? 'Устанавливаем…' : 'Скачать и установить' }}
      </button>
      <p v-if="queueBusy">
        Дождитесь завершения очереди загрузок перед установкой.
      </p>
      <p v-if="installing && total">
        Загружено {{ Math.min(100, Math.round(downloaded / total * 100)) }} %.
      </p>
    </div>
    <p
      v-if="installed"
      role="status"
    >
      Обновление установлено. Закройте и снова откройте приложение.
    </p>
    <p
      v-if="error"
      class="settings-screen__error"
      role="alert"
    >
      {{ error }}
    </p>
  </section>
</template>

<style scoped>
.app-update { margin: 1.5rem 0; padding-bottom: 1rem; border-bottom: 1px solid var(--color-border-subtle); }
.app-update__notes { white-space: pre-wrap; }
.tap-target { min-height: 40px; padding: 0.5rem 0.75rem; }
</style>
