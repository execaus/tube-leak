<script setup lang="ts">
/**
 * Экран «О программе» (TL-128, issue execaus/tube-leak#135, решение
 * владельца по #14 от 2026-09-18) — обязателен до первой раздачи
 * установщика кому-либо, кроме владельца: `ffmpeg.org/legal.html` требует
 * указывать использование FFmpeg и его лицензию в самом приложении, а до
 * этой задачи тексты лицензий были видны только в файлах репозитория
 * (`THIRD-PARTY-LICENSES.md`, `SOURCES-FFMPEG.md`), которые у получателя
 * собранного приложения не открываются — репозиторий приватный.
 *
 * # Почему четвёртая вкладка, а не раскрывающийся блок
 *
 * Навигация «Главный/История/Настройки» (Ф-16, TL-92, дизайн E5
 * «Навигация») уже несёт весь механизм, который нужен отдельному экрану:
 * доступный `tablist` с клавиатурой (стрелки/Home/End), управление
 * фокусом при переключении и `v-show`, не размонтирующий состояние
 * (К-14). Экран «О программе» статичен и не хранит собственного
 * состояния между переключениями — ему не нужно ничего из этого сверх
 * того, что уже даёт готовая вкладка. Раскрывающийся блок на одном из
 * существующих экранов потребовал бы завести новый accessible-паттерн
 * (`aria-expanded`/фокус на раскрытии) ради контента, который не связан
 * по смыслу ни с «Главным» (E1–E4, разбор и загрузка конкретного ролика),
 * ни с «Историей»/«Настройками» — четвёртая вкладка дешевле, не плодит
 * новый паттерн и одинаково доступна из любого состояния очереди.
 *
 * # Компонент только отображает то, что ему передали
 *
 * Тот же приём, что `SidecarStatusRow`/`YtDlpUpdateBlock` — `invoke`/
 * `listen` не вызывает сам, версию приложения и отчёт проверки sidecar
 * получает пропсами от `App.vue`, который их уже держит для служебного
 * экрана Ф-9.
 *
 * # Два разных источника версии ffmpeg на этом экране — осознанно
 *
 * Список лицензий ниже называет версию ffmpeg **9.0.1** текстом,
 * буквально совпадающим с `SOURCES-FFMPEG.md`/`THIRD-PARTY-LICENSES.md`
 * (документ и лицензионный вывод про GPL v3 привязаны именно к этой
 * версии; при следующем обновлении пина оба файла правятся в одной
 * задаче, док-комментарий `SOURCES-FFMPEG.md`, «Порядок при обновлении
 * ffmpeg»). Раздел «Что установлено сейчас» — про другое: живая версия
 * бинарника на этой машине из `report` (`SidecarCheckReport`), она может
 * временно разойтись с пином (сборка ещё не обновлена и т.п.) — эти два
 * числа не должны молча схлопываться в одно.
 */
import { computed } from 'vue'

import type { SidecarCheckReport, SidecarCheckResult } from '@/types/generated/sidecar'

const props = defineProps<{
  appVersion: string
  report?: SidecarCheckReport
}>()

/**
 * Текст версии одной строки «Что установлено сейчас» — читает только
 * `report`, полученный пропом (никакого собственного `invoke`, доля
 * «Экономия вызовов»/CLAUDE.md — это тот же отчёт, что уже показывает
 * служебный экран Ф-9, второй раз не запрашивается).
 */
function installedVersionText(result: SidecarCheckResult | undefined): string {
  if (!result) return 'проверяется…'
  if (result.status === 'ok') return result.version ?? '—'
  return 'не удалось определить — статус см. на главном экране'
}

const ytDlpInstalledVersion = computed(() => installedVersionText(props.report?.ytDlp))
const ffmpegInstalledVersion = computed(() => installedVersionText(props.report?.ffmpeg))
const denoInstalledVersion = computed(() => installedVersionText(props.report?.deno))

const FFMPEG_SOURCES_URL = 'https://ffmpeg.org/releases/ffmpeg-9.0.1.tar.bz2'
</script>

<template>
  <div class="about-screen">
    <section class="about-screen__section">
      <h3>Версия</h3>
      <p>tube-leak {{ appVersion }}</p>
    </section>

    <section class="about-screen__section">
      <h3>Что установлено сейчас</h3>
      <p class="about-screen__hint">
        Версии внешних инструментов, с которыми приложение работает на этой машине сейчас (тот же отчёт, что и на
        главном экране).
      </p>
      <dl class="about-screen__versions">
        <dt>yt-dlp</dt>
        <dd>{{ ytDlpInstalledVersion }}</dd>
        <dt>ffmpeg</dt>
        <dd>{{ ffmpegInstalledVersion }}</dd>
        <dt>deno</dt>
        <dd>{{ denoInstalledVersion }}</dd>
      </dl>
    </section>

    <section class="about-screen__section">
      <h3>Компоненты и лицензии</h3>
      <p class="about-screen__hint">
        tube-leak — GUI-обёртка над внешними инструментами, которые поставляются вместе с приложением.
      </p>
      <ul class="about-screen__licenses">
        <li>
          <strong>ffmpeg 9.0.1</strong> — GNU General Public License версии 3 (GPL v3)
        </li>
        <li>
          <strong>yt-dlp</strong> — Unlicense
        </li>
        <li>
          <strong>deno</strong> — MIT License
          <ul class="about-screen__licenses about-screen__licenses--nested">
            <li>V8 (JavaScript-движок внутри deno) — BSD 3-Clause License</li>
            <li>ICU (внутри deno) — Unicode License</li>
            <li>TypeScript (внутри deno) — Apache License 2.0</li>
          </ul>
        </li>
        <li>
          <strong>SQLite</strong> — Public Domain
        </li>
      </ul>
    </section>

    <section class="about-screen__section">
      <h3>Исходный код ffmpeg</h3>
      <p>
        tube-leak поставляется вместе со статической сборкой ffmpeg под GNU General Public License версии 3
        (GPL v3). Исходный код этой версии ffmpeg:
      </p>
      <p class="about-screen__code">
        {{ FFMPEG_SOURCES_URL }}
      </p>
      <p>
        Полный перечень влинкованных в эту сборку библиотек с версиями и ссылками на их исходники — в файле
        <code>SOURCES-FFMPEG.md</code> репозитория проекта. Полные тексты лицензий всех компонентов, перечисленных
        выше, — в файле <code>THIRD-PARTY-LICENSES.md</code> там же.
      </p>
    </section>
  </div>
</template>

<style scoped>
.about-screen {
  max-width: 40rem;
}

.about-screen__section {
  margin: 0 0 1.5rem;
}

.about-screen__section h3 {
  margin: 0 0 0.5rem;
}

.about-screen__section p {
  margin: 0 0 0.5rem;
  line-height: 1.4;
}

.about-screen__hint {
  color: var(--color-text-muted);
}

.about-screen__versions {
  display: grid;
  grid-template-columns: auto 1fr;
  gap: 0.25rem 0.75rem;
  margin: 0;
}

.about-screen__versions dt {
  font-weight: 600;
}

.about-screen__versions dd {
  margin: 0;
}

.about-screen__licenses {
  margin: 0;
  padding-left: 1.25rem;
}

.about-screen__licenses--nested {
  margin-top: 0.35rem;
}

.about-screen__code {
  padding: 0.5rem;
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 0.85rem;
  background: var(--color-surface-subtle);
  overflow-wrap: anywhere;
}
</style>
