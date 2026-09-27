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
 * # Файлы — «рядом с приложением», не «в репозитории» (правки ревью, Б1)
 *
 * `THIRD-PARTY-LICENSES.md` и `SOURCES-FFMPEG.md` устанавливаются вместе
 * с самим приложением как ресурсы бандла (`src-tauri/tauri.conf.json`,
 * `bundle.resources`) — получатель установщика видит их на диске
 * независимо от доступа к репозиторию, который у него как раз обычно нет
 * (репозиторий приватный). Текст ниже поэтому отсылает к файлам «рядом с
 * приложением», а доступ к репозиторию называет отдельно и только как
 * «для тех, у кого он есть» — так текст остаётся правдивым и до, и после
 * того, как `SOURCES-FFMPEG.md` попадёт в `bundle.resources` (задача идёт
 * в core-ветке `tl-135-134-pin-guards` параллельно этой).
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
 *
 * # Список компонентов — не выдаётся за полный, и лицензии остального —
 * # не обобщаются (правки ревью, Б2, затем Б3)
 *
 * `THIRD-PARTY-LICENSES.md` документирует физически попадающими в
 * бинарник ещё несколько зависимостей (`tauri-plugin-dialog`, `rfd`,
 * `tauri-plugin-fs`, под Windows — `windows-sys`/`windows-targets`, плюс
 * раздел «Прочие зависимости» — `tauri`/`serde`/`tokio`/`vue`/`pinia` и их
 * транзитивные зависимости). Вписывать их все в UI незачем — этот экран
 * называет только компоненты, для которых лицензия требует прямого
 * указания в about-box (ffmpeg — GPL v3, здесь единственный настоящий
 * повод для всего экрана) или которые пользователь может счесть частью
 * «движка» приложения (yt-dlp, deno и то, что внутри него).
 *
 * Первая версия этого абзаца (Б2) сопровождала список фразой «остальное —
 * пермиссивные MIT/Apache-2.0» — и это оказалось неправдой (ревью Б3):
 * измерение `Cargo.lock` (474 пакета релизного графа) нашло **MPL-2.0**
 * (слабый copyleft, не пермиссивная) у `cssparser`/`selectors`/
 * `dtoa-short`/`option-ext`, приходящих транзитивно через `tauri`/`wry`, а
 * ещё `Apache-2.0 AND ISC` (`ring`), `CDLA-Permissive-2.0`
 * (`webpki-roots`), `Unicode-3.0` (18 крейтов ICU/zerovec) и другие —
 * список лицензий на этом экране был бы юридическим утверждением сверх
 * того, что документирует наш собственный `THIRD-PARTY-LICENSES.md` (там
 * честнее: «**преимущественно** MIT и/или Apache-2.0»). Решение ведущего:
 * на этом экране лицензии вспомогательных библиотек не называются и не
 * обобщаются вовсе — только отсылка к файлу за полным перечнем.
 *
 * # Ссылка на исходники ffmpeg — только для macOS-сборки (правки ревью, M4)
 *
 * Прямая ссылка на `ffmpeg-9.0.1.tar.bz2` — Corresponding Source ровно
 * той сборки, которую использует macOS (`SOURCES-FFMPEG.md`, «Исходный
 * код самого ffmpeg 9.0.1»: это буквально тот архив, который скачивает
 * сценарий сборки martin-riedl). Для Windows и Linux фактическая сборка
 * взята не строго из этого архива тега — README.txt сборки для Windows
 * называет отдельный коммит, а Linux-сборка стоит на теге автосборки на
 * 11 коммитов позже релиза 9.0.1 (`SOURCES-FFMPEG.md`, разделы «Windows»/
 * «Linux»). Текст ниже поэтому не утверждает, что одна ссылка — точный
 * источник для всех платформ: она помечена как источник macOS-сборки, а
 * за точным источником для конкретной платформы отсылает в
 * `SOURCES-FFMPEG.md` — неточность документа («один источник на все
 * платформы») сюда сознательно не воспроизводится.
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
            <li>ICU (внутри deno) — Unicode License v3</li>
            <li>TypeScript (внутри deno) — Apache License 2.0</li>
          </ul>
        </li>
        <li>
          <strong>SQLite</strong> — Public Domain
        </li>
      </ul>
      <p class="about-screen__hint">
        Кроме перечисленного, приложение включает библиотеки с открытым исходным кодом; их полный перечень и полные
        тексты лицензий — в файле <code>THIRD-PARTY-LICENSES.md</code>, который ставится вместе с этим приложением;
        у кого есть доступ к репозиторию проекта, тот же файл лежит там же.
      </p>
    </section>

    <section class="about-screen__section">
      <h3>Исходный код ffmpeg</h3>
      <p>
        tube-leak поставляется вместе со статической сборкой ffmpeg 9.0.1 под GNU General Public License версии 3
        (GPL v3). Прямая ссылка ниже ведёт на исходный код этой версии — точный источник macOS-сборки:
      </p>
      <p class="about-screen__code">
        {{ FFMPEG_SOURCES_URL }}
      </p>
      <p>
        Для Windows и Linux фактическая сборка взята не строго из этого архива релиза (иной коммит и, для Linux,
        более поздний патч-уровень) — точный источник для сборки под каждую платформу и полный перечень
        влинкованных библиотек с версиями — в файле <code>SOURCES-FFMPEG.md</code>, который ставится вместе с этим
        приложением; у кого есть доступ к репозиторию проекта, тот же файл лежит там же.
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
