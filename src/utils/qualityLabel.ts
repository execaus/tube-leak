import type { QualityItem } from '@/types/probe'

/**
 * Подпись строки лестницы качеств (дизайн E2, раздел «Лестница качеств»,
 * решение владельца Р-1). Вид пункта (`kind`) приходит явно с бэкенда —
 * UI не выводит его сам из `heightPx`, иначе правило «2160p» против
 * «максимальное доступное (NNNp)» жило бы двумя копиями (см. doc-комментарий
 * `QualityKind` в `src/types/probe.ts`).
 */
export function qualityLabel(item: QualityItem): string {
  switch (item.kind) {
    case 'standard':
      return `${item.heightPx}p`
    case 'maxAvailable':
      return `Максимальное доступное (${item.heightPx}p)`
    case 'audioOnly':
      return 'Только аудио'
  }
}
