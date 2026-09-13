import { open } from '@tauri-apps/plugin-dialog'

/**
 * Единственная точка входа фронтенда к диалогу выбора папки (эпик E5,
 * TL-94; Ф-11, дизайн E5 «Данные для API» — «выбор папки диалогом — не
 * отдельная команда ядра: фронтенд вызывает `tauri-plugin-dialog` напрямую»,
 * Р-3).
 *
 * Обёрнуто в собственную функцию, а не вызывается из компонента напрямую,
 * чтобы в тестах `SettingsScreen.vue`/`settings.ts` мокался один модуль
 * (`vi.mock('@/composables/usePickFolder', ...)`), а не пакет плагина —
 * тот же приём, что `probeUrl`/`cancelProbe` в `useProbe.ts` для команд
 * ядра.
 *
 * Возвращает абсолютный путь либо `null`, если пользователь отменил диалог
 * (`open()` в этом случае и так возвращает `null` — здесь только сужение
 * типа `string | string[] | null` до `string | null`, ожидаемого вызывающим
 * кодом: без `multiple` результат никогда не бывает массивом при
 * `directory: true`, но сам тип `OpenDialogReturn` пакета выводит это
 * условно через дженерик, а не как отдельный простой тип).
 */
export async function pickDestinationFolder(): Promise<string | null> {
  const result = await open({ directory: true })
  return typeof result === 'string' ? result : null
}
