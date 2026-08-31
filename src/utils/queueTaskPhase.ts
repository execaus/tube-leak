import type { DownloadPhase } from '@/types/generated/download'

/**
 * Терминальность фазы задачи очереди — общее место для трёх независимых
 * потребителей (`useDownloadTaskStore`, `QueueSection.vue`,
 * `useExitConfirmation.ts`/TL-76), которые до TL-76 держали каждый свою
 * копию одного и того же списка из трёх фаз. Три копии одного и того же
 * белого списка — ровно тот класс дефекта, о котором предупреждает
 * CLAUDE.md («белый список, разошедшийся с Rust»): разойдись они хоть в
 * одной фазе, дефект был бы не виден ни одному тесту, кроме теста именно
 * разошедшейся копии. Единая функция — единая точка, которой достаточно
 * сверяться с {@link DownloadPhase} (контракт TL-51/TL-70) один раз.
 */
export function isTerminalQueuePhase(phase: DownloadPhase): boolean {
  return phase === 'done' || phase === 'failed' || phase === 'cancelled'
}
