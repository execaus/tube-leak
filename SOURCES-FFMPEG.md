# Исходный код ffmpeg, поставляемого с tube-leak

Этот файл — «clear directions next to the object code» в смысле §6d GNU
GPL v3: указание, где взять исходный код GPL-компонентов, которые мы
раздаём в составе установщика. Полные тексты лицензий (в том числе
GPL v3) — в `THIRD-PARTY-LICENSES.md`. Версии и адреса самих бинарников —
в `src-tauri/binaries.lock.json`.

Лицензионных выводов этот файл не делает: он ссылается на уже принятые —
исследование `specs/2026-09-17-ffmpeg-license-research.md` в репозитории
tube-leak-docs и решение владельца от 2026-09-18 в
https://github.com/execaus/tube-leak/issues/14 («остаёмся на GPLv3 и
выполняем требования»).

**Сведения сняты 2026-09-19. Ссылки проверены в тот же день запросом
(`curl`, с учётом перенаправлений): из 44 адресов 42 отвечают HTTP 200
анонимно.** Оставшиеся два (`issues` нашего репозитория и issue #14)
анонимно отвечают **404, потому что репозиторий приватный**; их
существование подтверждено авторизованным запросом `gh api`
(issue #14 — открыт). Как только репозиторий станет публичным, они
станут доступны без авторизации.

## Что мы распространяем

**ffmpeg 9.0.1**, статические сборки, по одной на таргет:

| Таргет | Сборщик | Архив |
|---|---|---|
| macOS aarch64 (Apple Silicon) | ffmpeg.martin-riedl.de | `download/macos/arm64/1787073674_9.0.1/ffmpeg.zip` |
| macOS x86_64 (Intel) | ffmpeg.martin-riedl.de | `download/macos/amd64/1787081194_9.0.1/ffmpeg.zip` |
| Windows x86_64 | gyan.dev | `packages/ffmpeg-9.0.1-essentials_build.zip` |
| Linux x86_64 | BtbN/FFmpeg-Builds | `ffmpeg-n9.0.1-6-g9d4ca21220-linux64-gpl-9.0.tar.xz` |

Лицензия сборок — **GNU General Public License v3**: они собраны с
`--enable-gpl` и `--enable-version3` и содержат x264/x265. Это не наш
вывод из флагов, а утверждение самой сборки: `ffmpeg -L` на нашем
бинарнике печатает GPL «version 3 of the License, or (at your option) any
later version». Разбор флагов — в `THIRD-PARTY-LICENSES.md`, обоснование —
в исследовании.

Собственный код tube-leak под GPL не подпадает и в Corresponding Source
не входит: ffmpeg запускается отдельным процессом. Основание — раздел 3
исследования; здесь этот вывод только цитируется.

## Как снят состав сборки

Перечень ниже получен из самого поставляемого бинарника и из материалов
сборщика, а не из общего знания:

1. `src-tauri/binaries/ffmpeg-aarch64-apple-darwin -version` и
   `-buildconf` — строка `configuration` с 28 флагами `--enable-lib*`
   (плюс `--enable-openssl` и `--enable-fontconfig`, у которых нет
   префикса `lib`). Это список того, **что** влинковано.
2. `versions.txt`, который сборщик публикует рядом с тем же архивом, —
   31 строка с версиями:
   https://ffmpeg.martin-riedl.de/download/macos/arm64/1787073674_9.0.1/versions.txt
   Это список того, **каких версий**.
3. Файлы `version/*` в репозитории сценариев сборки. Сверено построчно:
   из 31 строки `versions.txt` **30 совпали с пином в репозитории
   сборщика дословно**, расхождений нет; 31-я — x264, у которого пина
   нет вовсе (см. ниже).
4. `versions.txt` обеих macOS-сборок (arm64 и Intel) сверены `diff` —
   блоки версий **совпадают полностью**, поэтому таблица ниже относится
   к обоим macOS-таргетам.

Что дополнительно подтверждено содержимым бинарника:

- zlib: внутри лежат баннеры `deflate 1.3.2` и `inflate 1.3.2` — версия
  из `versions.txt` подтверждена самим файлом;
- SVT-AV1: внутри сохранились пути каталога сборки вида
  `/Volumes/ffmpeg_arm64/source/svt-av1/SVT-AV1-v3.1.2/...` — то же для
  `srt-1.5.6` и `vvenc-1.14.0`;
- x264 присутствует (сотни символов `x264_*`, в том числе 10-битных),
  x265 — символ `x265_encoder_open_216`.

**Чему здесь нельзя верить.** Проверка «есть ли символы библиотеки в
бинарнике» как способ доказать **отсутствие** компонента не работает:
для SVT-AV1 совпадающих символов не нашлось ни одного, хотя пути её
исходников лежат в том же файле, а `--enable-libsvtav1` стоит в
configure. То же для libklvanc и libzvbi. Поэтому ни одна строка ниже не
помечена как «не влинкована» на основании символов — источник истины
здесь `-buildconf` и `versions.txt`.

## Сценарии сборки

Сценарии, которыми собраны оба macOS-бинарника, открыты:

- https://git.martin-riedl.de/ffmpeg/build-script — репозиторий сценариев
  (`build.sh` + `script/build-*.sh` + пины в `version/*`). Сам сценарий
  распространяется под Apache-2.0 — это лицензия сценария, не результата
  сборки.
- https://git.martin-riedl.de/ffmpeg/build-script/commit/f63b8aab8f5ce1a067da86ba69e34a36a7e217e5
  — коммит «chore: ffmpeg update (version 9.0.1)» от 2026-08-17, самый
  поздний на ветке `main` к моменту нашей сборки (архив собран
  2026-08-18 17:21 UTC). **Оговорка:** сервер сборки не публикует, каким
  именно коммитом собран конкретный архив, поэтому соответствие
  «архив ↔ коммит» здесь выведено по датам и по совпадению всех 30
  сравнимых версий, а не прочитано из метаданных сборки.

Сценарии сборки Windows- и Linux-таргетов: https://www.gyan.dev/ffmpeg/builds/
и https://github.com/BtbN/FFmpeg-Builds.

## Исходный код самого ffmpeg 9.0.1

- https://ffmpeg.org/releases/ffmpeg-9.0.1.tar.bz2 — ровно тот архив,
  который скачивает сценарий сборки (`script/build-ffmpeg.sh`);
- https://ffmpeg.org/releases/ffmpeg-9.0.1.tar.xz — тот же релиз в другом
  сжатии;
- https://ffmpeg.org/download.html — страница получения исходников
  проекта.

## Исходный код влинкованных библиотек

По одной строке на каждый флаг `--enable-*` из `-buildconf` нашего
бинарника. Версия — из `versions.txt` этой сборки. Ссылка ведёт на тот
же адрес, с которого исходники берёт сценарий сборки.

| Флаг configure | Библиотека | Версия | Исходники |
|---|---|---|---|
| `--enable-libaom` | aom | 3.14.1 | https://storage.googleapis.com/aom-releases/libaom-3.14.1.tar.gz |
| `--enable-libass` | libass | 0.17.5 | https://github.com/libass/libass/releases/download/0.17.5/libass-0.17.5.tar.gz |
| `--enable-libbluray` | libbluray | 1.5.0 | https://download.videolan.org/pub/videolan/libbluray/1.5.0/libbluray-1.5.0.tar.xz |
| `--enable-libdav1d` | dav1d | 1.5.4 | https://code.videolan.org/videolan/dav1d/-/archive/1.5.4/dav1d-1.5.4.tar.gz |
| `--enable-fontconfig` | fontconfig | 2.17.1 | https://gitlab.freedesktop.org/api/v4/projects/890/packages/generic/fontconfig/2.17.1/fontconfig-2.17.1.tar.xz |
| `--enable-libfreetype` | freetype | 2.13.0 | https://download.savannah.gnu.org/releases/freetype/freetype-2.13.0.tar.gz |
| `--enable-libharfbuzz` | harfbuzz | 14.3.0 | https://github.com/harfbuzz/harfbuzz/releases/download/14.3.0/harfbuzz-14.3.0.tar.xz |
| `--enable-libklvanc` | libklvanc | 1.6.0 | https://github.com/stoth68000/libklvanc/archive/refs/tags/vid.obe.1.6.0.tar.gz |
| `--enable-libmp3lame` | LAME | 3.100 | https://unlimited.dl.sourceforge.net/project/lame/lame/3.100/lame-3.100.tar.gz |
| `--enable-libopenh264` | openh264 | 2.6.0 | https://github.com/cisco/openh264/archive/v2.6.0.tar.gz |
| `--enable-libopenjpeg` | OpenJPEG | 2.5.4 | https://github.com/uclouvain/openjpeg/archive/refs/tags/v2.5.4.tar.gz |
| `--enable-openssl` | OpenSSL | 3.6.1 | https://github.com/openssl/openssl/releases/download/openssl-3.6.1/openssl-3.6.1.tar.gz |
| `--enable-libopus` | opus | 1.6.1 | https://downloads.xiph.org/releases/opus/opus-1.6.1.tar.gz |
| `--enable-librav1e` | rav1e | 0.8.1 | https://github.com/xiph/rav1e/archive/refs/tags/v0.8.1.tar.gz |
| `--enable-libsnappy` | snappy | 1.2.2 | https://github.com/google/snappy/archive/refs/tags/1.2.2.tar.gz |
| `--enable-libsrt` | srt | 1.5.6 | https://github.com/Haivision/srt/archive/refs/tags/v1.5.6.tar.gz |
| `--enable-libsvtav1` | SVT-AV1 | 3.1.2 | https://gitlab.com/AOMediaCodec/SVT-AV1/-/archive/v3.1.2/SVT-AV1-v3.1.2.tar.gz |
| `--enable-libtheora` | libtheora | 1.2.0 | https://downloads.xiph.org/releases/theora/libtheora-1.2.0.tar.gz |
| `--enable-libvmaf` | libvmaf | 3.2.0 | https://github.com/Netflix/vmaf/archive/refs/tags/v3.2.0.tar.gz |
| `--enable-libvorbis` | libvorbis | 1.3.7 | https://ftp.osuosl.org/pub/xiph/releases/vorbis/libvorbis-1.3.7.tar.gz |
| `--enable-libvpx` | libvpx | 1.16.0 | https://github.com/webmproject/libvpx/archive/v1.16.0.tar.gz |
| `--enable-libvvenc` | vvenc | 1.14.0 | https://github.com/fraunhoferhhi/vvenc/archive/refs/tags/v1.14.0.tar.gz |
| `--enable-libwebp` | libwebp | 1.6.0 | https://github.com/webmproject/libwebp/archive/refs/tags/v1.6.0.tar.gz |
| `--enable-libx264` | x264 | **0.165.x — точную ревизию установить не удалось** | https://code.videolan.org/videolan/x264 |
| `--enable-libx265` | x265 | 4.2 | https://bitbucket.org/multicoreware/x265_git/get/4.2.tar.gz |
| `--enable-libxml2` | libxml2 | 2.15.3 | https://download.gnome.org/sources/libxml2/2.15/libxml2-2.15.3.tar.xz |
| `--enable-libzimg` | zimg | 3.0.6 | https://github.com/sekrit-twc/zimg/archive/refs/tags/release-3.0.6.tar.gz |
| `--enable-libzvbi` | zvbi | 0.2.35 | https://sourceforge.net/projects/zapping/files/zvbi/0.2.35/zvbi-0.2.35.tar.bz2/download |

Остальные флаги из `configuration` (`--prefix`, `--pkg-config-flags`,
`--extra-version`, `--enable-gray`, `--enable-gpl`, `--enable-version3`)
библиотек не добавляют.

### x264: чего установить не удалось

`versions.txt` называет версию **0.165.x** — без точной ревизии.
Причина видна в сценарии сборки `script/build-x264.sh`: он скачивает
`https://code.videolan.org/videolan/x264/-/archive/master/x264-master.tar.gz`,
то есть **снимок ветки `master` на момент сборки**, а не тег. Коммит
сборщик нигде не публикует. Поэтому:

- **точную ревизию исходников x264, попавшую в наш бинарник, установить
  не удалось**; известно только, что это `master` по состоянию на
  2026-08-18 и что x264 сообщает себя как 0.165.x;
- ссылка выше ведёт на репозиторий x264 целиком (история включает нужный
  коммит). Архив `master` по тому же адресу сегодня даст **другой**
  снимок, поэтому он как «исходники этой версии» не указан.

Это известный пробел в комплекте Corresponding Source; закрывается либо
получением коммита от сборщика, либо собственной сборкой с пином x264.

### Сопутствующие компоненты

Эти четыре есть в `versions.txt` и/или в пинах сборщика, но отдельного
флага `--enable-*` у них нет — они приходят как зависимости других
библиотек или используются другими программами того же пайплайна
(ffplay). Что именно из них попало в наш `ffmpeg`, здесь **не
устанавливалось** (почему — см. «Чему здесь нельзя верить»), поэтому
ссылки даны на всякий случай.

| Компонент | Версия | Откуда | Исходники |
|---|---|---|---|
| fribidi | 1.0.16 | `versions.txt`; зависимость libass | https://github.com/fribidi/fribidi/releases/download/v1.0.16/fribidi-1.0.16.tar.xz |
| zlib | 1.3.2 | `versions.txt`; баннер найден внутри нашего бинарника | https://www.zlib.net/fossils/zlib-1.3.2.tar.gz |
| SDL2 | 2.32.10 | `versions.txt`; в пайплайне нужен ffplay | https://www.libsdl.org/release/SDL2-2.32.10.tar.gz |
| libogg | 1.3.6 | **в `versions.txt` отсутствует**; пин сборщика `version/libogg`, зависимость libvorbis и libtheora по `DEPENDENCY.md` | https://ftp.osuosl.org/pub/xiph/releases/ogg/libogg-1.3.6.tar.gz |
| libiconv | 1.17 | **в `versions.txt` отсутствует**; пин сборщика `version/libiconv` | https://ftp.gnu.org/pub/gnu/libiconv/libiconv-1.17.tar.gz |

### Windows и Linux

Таблица выше снята с **macOS-сборок** (обеих: их `versions.txt`
совпадают). Для Windows (gyan.dev) и Linux (BtbN) **версии вложенных
библиотек в этом файле не приведены — установить их не удалось из того,
что опубликовано рядом со сборкой**: gyan.dev публикует только строку
`configuration` без версий, у BtbN версии восстанавливаются из
сценариев соответствующего тега autobuild, и эта работа не выполнялась.
Ссылки на сценарии обоих сборщиков — в разделе «Сценарии сборки».
Пробел зафиксирован в https://github.com/execaus/tube-leak/issues/14.

## Шаблон строки для описания релиза

Копировать в release notes каждого выпуска, в котором раздаётся
установщик (это и есть требуемое §6d указание рядом с объектным кодом):

> **Исходный код поставляемых GPL-компонентов.** В состав установщика
> входит ffmpeg 9.0.1 — статическая сборка под GNU General Public
> License v3. Исходный код ffmpeg этой версии:
> https://ffmpeg.org/releases/ffmpeg-9.0.1.tar.bz2. Полный перечень
> влинкованных библиотек с версиями, ссылки на их исходники и на
> сценарии сборки — в файле `SOURCES-FFMPEG.md` репозитория проекта
> (тег этого релиза). Полные тексты лицензий — `THIRD-PARTY-LICENSES.md`
> там же. Запрос исходников также можно направить через issues
> репозитория: https://github.com/execaus/tube-leak/issues.

Пока репозиторий приватный, обе ссылки шаблона — и на файл, и на issues —
работают только у тех, кому выдан доступ; анонимно они дают 404. Для
нынешнего режима («релиз адресован владельцу») этого достаточно, но
раздавать по такому указателю установщик кому-то ещё нельзя: §6d требует
работающего доступа к исходникам у того, кто получил бинарник.
Порядок на случай раздачи установщика кому-либо кроме
владельца (архив исходников ассетом рядом с установщиком и экран
«О программе») определён решением по
https://github.com/execaus/tube-leak/issues/14 и задачами #134/#135; этот
файл сам по себе такую раздачу не покрывает.

## Порядок при обновлении ffmpeg

Указатель обновляется **в той же задаче, что и пин**: меняя версию
ffmpeg в `src-tauri/binaries.lock.json`, заново снимите `-buildconf` с
нового бинарника и `versions.txt` новой сборки, обновите таблицы,
ссылки и дату снятия в этом файле — пин без обновлённого указателя
оставляет обязательство §6 незакрытым.
