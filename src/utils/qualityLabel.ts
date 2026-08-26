import type { QualityItem } from '@/types/probe'

/**
 * Подпись строки лестницы качеств (дизайн E2, раздел «Лестница качеств»,
 * решение владельца Р-1). Вид пункта (`kind`) приходит явно с бэкенда —
 * UI не выводит его сам из `heightPx`, иначе правило «2160p» против
 * «максимальное доступное (NNNp)» жило бы двумя копиями (см. doc-комментарий
 * `QualityKind` в `src/types/probe.ts`).
 *
 * `heightPx` в контракте опционален и для `standard`/`maxAvailable`
 * заполнен только по договорённости с core, а не по типу — печатать
 * `undefined` в подписи («undefinedp») тестами не поймать, а на экране
 * видно, поэтому у обоих есть честный текстовый фолбэк на случай
 * расхождения контракта.
 */
export function qualityLabel(item: QualityItem): string {
  switch (item.kind) {
    case 'standard':
      return item.heightPx !== undefined ? `${item.heightPx}p` : 'Видео'
    case 'maxAvailable':
      return item.heightPx !== undefined
        ? `Максимальное доступное (${item.heightPx}p)`
        : 'Максимальное доступное качество'
    case 'audioOnly':
      return 'Только аудио'
  }
}
