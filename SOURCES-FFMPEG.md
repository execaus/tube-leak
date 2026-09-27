# Исходный код ffmpeg, поставляемого с tube-leak

Этот файл — «clear directions next to the object code» в смысле §6d GNU
GPL v3: указание, где взять исходный код GPL-компонентов, которые мы
раздаём в составе установщика. Полные тексты лицензий (в том числе
GPL v3) — в `THIRD-PARTY-LICENSES.md`. Версии и адреса самих бинарников —
в `src-tauri/binaries.lock.json`.

Лицензионных выводов этот файл не делает: он ссылается на уже принятое —
решение владельца от 2026-09-18 в
https://github.com/execaus/tube-leak/issues/14 («остаёмся на GPLv3 и
выполняем требования»). Опиралось оно на внутреннее исследование проекта
(`specs/2026-09-17-ffmpeg-license-research.md` в репозитории
tube-leak-docs). **Этот документ публично недоступен и доступным не
станет: открывается только репозиторий кода, репозиторий документации
остаётся закрытым.** Указателем §6d он не является и не требуется для
получения исходников: всё, что нужно по §6d, — адреса в таблицах ниже.

**Сведения по macOS сняты 2026-09-19; по Windows и Linux — 2026-09-26
(TL-133). Адреса проверены 2026-09-26.**

Тогда же закрыт дефект самой проверки. До TL-133 обход ссылок этого файла
давал «42 из 44 отвечают 200, а два 404 — наш приватный репозиторий»
(состояние **на 2026-09-26**, когда репозиторий кода был ещё приватным), и
это было правдой — но ровно в тот же день **две из четырёх раздаваемых
сборок ffmpeg были недоступны** (#140). Обход их не видел: в таблице «Что
мы распространяем» стояли **имена файлов, а не адреса**, и в список
проверяемых ссылок они не попадали вовсе. Теперь там адреса, а сторожей
два:

- `npm test` — офлайн-сверка: каждый адрес сборки из
  `src-tauri/binaries.lock.json` обязан стоять и здесь, и в
  `THIRD-PARTY-LICENSES.md` **дословно**. Без неё вернуть имя файла
  вместо адреса можно было бы снова, и сетевая проверка снова ослепла бы.
- `npm run check-pins` — сетевая проверка: отвечают ли все адреса пина и
  все адреса этих двух документов. Отдельной командой, а не тестом:
  в тестах проекта сети нет. Запускать перед выпуском.

Те два адреса — `issues` репозитория и issue #14 — отвечали 404 только
потому, что репозиторий кода был приватным. **Репозиторий кода открыт, и
обе ссылки работают анонимно**, поэтому никаких поправок и исключений у
них больше нет: сторож проверяет их как любой другой адрес, и 404 по ним
теперь означает настоящую пропажу (удалённый issue), а не отсутствие
доступа.

Репозиторий документов `tube-leak-docs` остаётся закрытым, и анонимно его
адреса недоступны. Указатель §6d на них не ссылается: в этом файле и в
`THIRD-PARTY-LICENSES.md` нет ни одной ссылки на `tube-leak-docs` —
единственное упоминание (исследование лицензии выше) названо путём внутри
закрытого репозитория и прямо помечено как публично недоступное. Сторож
такие адреса пропускает с названной причиной; различает он два
репозитория по сегментам пути, потому что `tube-leak-docs` начинается
с `tube-leak` и проверка префиксом накрыла бы открытый репозиторий кода.

## Что мы распространяем

**ffmpeg 9.0.1**, статические сборки, по одной на таргет:

| Таргет | Сборщик | Адрес сборки |
|---|---|---|
| macOS aarch64 (Apple Silicon) | ffmpeg.martin-riedl.de | https://ffmpeg.martin-riedl.de/download/macos/arm64/1787073674_9.0.1/ffmpeg.zip |
| macOS x86_64 (Intel) | ffmpeg.martin-riedl.de | https://ffmpeg.martin-riedl.de/download/macos/amd64/1787081194_9.0.1/ffmpeg.zip |
| Windows x86_64 | GyanD/codexffmpeg (GitHub-зеркало gyan.dev) | https://github.com/GyanD/codexffmpeg/releases/download/9.0.1/ffmpeg-9.0.1-essentials_build.zip |
| Linux x86_64 | BtbN/FFmpeg-Builds | https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-08-31-13-27/ffmpeg-n9.0.1-11-ge47273f4d9-linux64-gpl-9.0.tar.xz |

Адреса Windows и Linux перепинованы 2026-09-26 (TL-133, #140): прежние
умерли. У gyan.dev архива версий на самом хосте нет — там держатся только
latest и предыдущий релиз, поэтому путь `packages/ffmpeg-9.0.1-…` исчез,
когда вышел 9.0.2; у BtbN пин стоял на **ежедневном** теге автосборки,
который живёт около двух недель. **Windows-сборка при этом не менялась:**
sha256 осталась прежней, сменился только адрес — на версионный тег
GitHub-зеркала того же сборщика. Linux-сборка — тот же релиз 9.0.1 с
более поздним патч-уровнем ветки release/9.0
(`-6-g9d4ca21220` → `-11-ge47273f4d9`).

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

Сценарии сборки Linux-таргета — https://github.com/BtbN/FFmpeg-Builds.
Здесь соответствие «архив ↔ коммит» **не выводится по датам, а читается**:
тег релиза `autobuild-2026-08-31-13-27` — настоящий git-тег на коммит
`8267213e26c1031621e6e1210fe3aa4867214f6a`, и ревизии библиотек сняты
именно с него (раздел «Linux: версии влинкованных библиотек»). Оговорка
про даты выше относится только к macOS.

По Windows сценариев нет: gyan.dev публикует страницу сборок
(https://www.gyan.dev/ffmpeg/builds/) и состав пакета в его `README.txt`,
но репозитория сборочных сценариев, сопоставимого с `scripts.d` у BtbN
или `version/*` у martin-riedl, не найдено (TL-133). Для §6 это значит:
версии по Windows у нас точные, а способ сборки описан только страницей
сборщика.

## Исходный код самого ffmpeg 9.0.1

- https://ffmpeg.org/releases/ffmpeg-9.0.1.tar.bz2 — ровно тот архив,
  который скачивает сценарий сборки (`script/build-ffmpeg.sh`);
- https://ffmpeg.org/releases/ffmpeg-9.0.1.tar.xz — тот же релиз в другом
  сжатии;
- https://ffmpeg.org/download.html — страница получения исходников
  проекта.

## Исходный код влинкованных библиотек (macOS)

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

## Windows: версии влинкованных библиотек

Таблица «Исходный код влинкованных библиотек» выше снята с **macOS-сборок**
(обеих: их `versions.txt` совпадают). Здесь — **Windows**; до TL-133 этого
раздела не было, и пробел был записан как #136.

Источник — сам поставляемый пакет: `README.txt` внутри архива
`ffmpeg-9.0.1-essentials_build.zip` содержит раздел «release-essentials
external libraries' versions». Читать его целиком не пришлось: у архива
запрошен HTTP-Range-ом хвост, по центральному каталогу zip найдено
смещение `README.txt` (111238173, 9135 сжатых байт), и вторым Range-запросом
скачаны только эти 9 КиБ — приём тот же, что в TL-108 для каталога deno.

Список флагов взят **из доставленного бинарника**, а не из README: строка
`configuration` вычитана из `binaries/ffmpeg-x86_64-pc-windows-msvc.exe`.
В ней **56** флагов `--enable-*`. Ниже — **42** строки: ровно столько
версий публикует README, и сошлись они **один в один**, без остатка с
обеих сторон.

Версии даны с точностью до `git describe`, то есть указывают конкретную
ревизию, — по этому таргету сведения **точнее**, чем по macOS, где x264
записан как «0.165.x» без ревизии.

| Флаг configure | Библиотека | Версия |
|---|---|---|
| `--enable-cairo` | cairo | 1.18.5 |
| `--enable-fontconfig` | fontconfig | 2.18.3 |
| `--enable-iconv` | libiconv | 1.19-1 |
| `--enable-gnutls` | gnutls | 3.8.13-1 |
| `--enable-libxml2` | libxml2 | v2.15.0-122-gddcb79dc |
| `--enable-gmp` | gmp | 6.3.0-2 |
| `--enable-libsrt` | srt | v1.5.6-2-gfcae571 |
| `--enable-libssh` | libssh | 0.12.0 |
| `--enable-libzmq` | zeromq | 4.3.5 |
| `--enable-avisynth` | avisynthplus | v3.7.5-362-gf4628d0a |
| `--enable-sdl2` | sdl | release-2.32.0-228-ga2e7c76bd |
| `--enable-libwebp` | libwebp | v1.6.0-199-g94d3c4a |
| `--enable-libx264` | x264 | v0.165.3223 |
| `--enable-libx265` | x265 | 4.3-6-g9ddc216 |
| `--enable-libxvid` | xvid | v1.3.7 |
| `--enable-libaom` | aom | v3.14.1-147-gec0dedc1a2 |
| `--enable-libopenjpeg` | openjpeg | 2.5.4 |
| `--enable-libvpx` | vpx | v1.16.0-184-g0cfc6da39 |
| `--enable-libass` | libass | 0.17.5-3-g89cc0f4 |
| `--enable-libfreetype` | freetype | VER-2-14-3 |
| `--enable-libfribidi` | fribidi | v1.0.16-5-g069a7e3 |
| `--enable-libharfbuzz` | harfbuzz | 14.3.0-10-g9f2f0317 |
| `--enable-libvidstab` | vidstab | v1.1.2-105-gc7a720a |
| `--enable-libvmaf` | vmaf | v3.2.0-9-g4991d2b5 |
| `--enable-libzimg` | zimg | release-3.0.6-252-gf6cc75a |
| `--enable-amf` | amf | v1.5.2-2-gc35f613 |
| `--enable-ffnvcodec` | ffnvcodec | n13.1.15.0-1-geddcea9 |
| `--enable-libvpl` | vpl | 2.17 |
| `--enable-vaapi` | vaapi | 2.25.0. |
| `--enable-openal` | openal | 1.25.2 |
| `--enable-libgme` | libgme | 0.6.6 |
| `--enable-libopenmpt` | openmpt | libopenmpt-0.6.28-40-gefc11a27 |
| `--enable-libopencore-amrwb` | libopencore-amrwb | 0.1.6 |
| `--enable-libmp3lame` | lame | 3.100 |
| `--enable-libtheora` | libtheora | v1.2.0 |
| `--enable-libvo-amrwbenc` | vo-amrwbenc | 0.1.3 |
| `--enable-libgsm` | gsm | 1.0.24 |
| `--enable-libopencore-amrnb` | libopencore-amrnb | 0.1.6 |
| `--enable-libopus` | opus | v1.6.1-50-g3da9f7a6 |
| `--enable-libspeex` | speex | Speex-1.2.1-51-g0589522 |
| `--enable-libvorbis` | vorbis | v1.3.7-37-g1b75110b |
| `--enable-librubberband` | rubberband | v4.0.0 |

**Чего в таблице нет и почему.** Остальные 14 флагов библиотек не
добавляют: `--enable-gpl`, `--enable-version3`, `--enable-static` —
лицензия и режим сборки; `--enable-mediafoundation`, `--enable-dxva2`,
`--enable-d3d11va`, `--enable-d3d12va`, `--enable-nvdec`,
`--enable-nvenc`, `--enable-cuvid`, `--enable-cuda-llvm` — аппаратные
бэкенды на системных API Windows и на заголовках, чьи версии уже названы
строками `amf` и `ffnvcodec`. Отдельный **named пробел**: три флага
системных компрессоров — `--enable-bzlib`, `--enable-lzma`,
`--enable-zlib` — в README версий не имеют, сборщик их не публикует.

Сценариев сборки gyan.dev не публикует (репозитория, сопоставимого с
`scripts.d` у BtbN, не найдено), поэтому по Windows у нас есть точные
версии, но не способ сборки; адрес исходников самого ffmpeg этой сборки
назван её же `README.txt`:
https://github.com/FFmpeg/FFmpeg/commit/bf1b838f2a

## Linux: версии влинкованных библиотек

Закрывает #136 по второму таргету. Здесь сведения **строго точнее**, чем
по macOS и Windows: у каждой библиотеки известна не версия, а **ревизия
исходников**.

Как снято. Тег `autobuild-2026-08-31-13-27` — настоящий git-тег на коммит
`8267213e26c1031621e6e1210fe3aa4867214f6a`; каждый файл `scripts.d/*.sh`
на этом коммите содержит `SCRIPT_REPO` (откуда берутся исходники) и
`SCRIPT_COMMIT` (какая именно ревизия). Список флагов — снова из
доставленного бинарника: строка `configuration` вычитана из
`binaries/ffmpeg-x86_64-unknown-linux-gnu`, в ней **75** флагов
`--enable-*`, и **72** из них отображены в сценарий сборщика; ни один
флаг не остался неразобранным. Сам ffmpeg собран как
`n9.0.1-11-ge47273f4d9-20260831`.

| Флаг configure | Библиотека | Исходники | Ревизия |
|---|---|---|---|
| `--enable-iconv` | libiconv | https://git.savannah.gnu.org/git/libiconv.git | `5e517e5bf0e1b4575ad431e81d7a4750fa2b284e` |
| `--enable-zlib` | zlib | https://github.com/madler/zlib.git | `e3dc0a85b7032e98380dec011bc8f2c2ee0d8fca` |
| `--enable-libxml2` | libxml2 | https://github.com/GNOME/libxml2.git | `c63248941708bc1d2e3a4292954593312212f6ca` |
| `--enable-libsoxr` | soxr | https://git.code.sf.net/p/soxr/code | `945b592b70470e29f917f4de89b4281fbbd540c0` |
| `--enable-openssl` | openssl | https://github.com/openssl/openssl.git | `openssl-3.6.3` |
| `--enable-libvmaf` | vmaf | https://github.com/Netflix/vmaf.git | `e80d6c593e6e2327687dccd00b7cc9c91036d79f` |
| `--enable-fontconfig` | fontconfig | https://gitlab.freedesktop.org/fontconfig/fontconfig.git | `d32a1911248576d01583557efcfe4b48fb2d0a5e` |
| `--enable-libharfbuzz` | harfbuzz | https://github.com/harfbuzz/harfbuzz.git | `d8dabe2594596c656b54c3b0072f3aa3093f30a9` |
| `--enable-libfreetype` | freetype | https://gitlab.freedesktop.org/freetype/freetype.git | `9e9d3b73f31367dbb4261f93c727a277f6632c77` |
| `--enable-libfribidi` | fribidi | https://github.com/fribidi/fribidi.git | `04a8cb7a3674717e509c79cd6ee3e127d0c75f4c` |
| `--enable-vulkan` | vulkan-headers | https://github.com/KhronosGroup/Vulkan-Headers.git | `v1.4.359` |
| `--enable-libvorbis` | libvorbis | https://github.com/xiph/vorbis.git | `1b75110b5a2754ba1931d82dd83cb822b266a21d` |
| `--enable-libxcb` | libxcb | https://gitlab.freedesktop.org/xorg/lib/libxcb.git | `4d6e1c8fffaf811cf4d0e68ff3bc6f50e62c32c7` |
| `--enable-xlib` | libx11 | https://gitlab.freedesktop.org/xorg/lib/libx11.git | `80dbb7d029b2bbf58b96421e59c83e34f763ab70` |
| `--enable-libpulse` | pulseaudio | https://gitlab.freedesktop.org/pulseaudio/pulseaudio.git | `0cc36279ec993680bccad6b907ba15de97b55c4d` |
| `--enable-gmp` | gmp | https://github.com/BtbN/gmplib.git | `9994908f090c694f8a152d660dc6852e0c48557a` |
| `--enable-lzma` | xz | https://github.com/tukaani-project/xz.git | `c8b8ab2ef1eb0a0217ad2027d7f5d242ceb944d3` |
| `--enable-liblcevc-dec` | lcevcdec | https://github.com/v-novaltd/LCEVCdec.git | `a254bd474649e5dcd8182689ac414420bfe8d8c3` |
| `--enable-opencl` | opencl | https://github.com/KhronosGroup/OpenCL-Headers.git | `c9c8ccfab584f9f7610057c4633dbd3df7e012cc` |
| `--enable-amf` | amf | https://github.com/GPUOpen-LibrariesAndSDKs/AMF.git | `c35f613aea2e5057a688c979e75b1cf24253297e` |
| `--enable-libaom` | aom | https://aomedia.googlesource.com/aom | `95f420511f698ea201e8928464704dc30da4b568` |
| `--enable-libaribb24` | libaribb24 | https://github.com/nkoriyama/aribb24.git | `5e9be272f96e00f15a2f3c5f8ba7e124862aec38` |
| `--enable-avisynth` | avisynth | https://github.com/AviSynth/AviSynthPlus.git | `de1d1fc0d69e6d308235491b24b8e9b26ac023ff` |
| `--enable-chromaprint` | chromaprint | https://github.com/acoustid/chromaprint.git | `aed8eba2202dd9d7b3b0a56c77904cc805490d72` |
| `--enable-libdav1d` | dav1d | https://code.videolan.org/videolan/dav1d.git | `54706fc6bc0cdecab7e9593974a4039cc038fca7` |
| `--enable-libdavs2` | davs2 | https://github.com/pkuvcl/davs2.git | `b41cf117452e2d73d827f02d3e30aa20f1c721ac` |
| `--enable-libdvdread` | libdvdread | https://code.videolan.org/videolan/libdvdread.git | `ce9bdb7775601f4e6b95136b875bc72a53c5ba7d` |
| `--enable-libdvdnav` | libdvdnav | https://code.videolan.org/videolan/libdvdnav.git | `2ffc50b5c37a6ddc086829203fc44e95588198dd` |
| `--enable-ffnvcodec` | ffnvcodec | https://github.com/FFmpeg/nv-codec-headers.git | `eddcea9e27f6b772057c9b3f87de2cc1737faffc` |
| `--enable-frei0r` | frei0r | https://github.com/dyne/frei0r.git | `253addfd4bea3c90b0bf765589ca28ea18f3ddc0` |
| `--enable-libgme` | gme | https://github.com/libgme/game-music-emu.git | `fe8da4b6d3876d7542c2fb69d94487e19836d678` |
| `--enable-libkvazaar` | kvazaar | https://github.com/ultravideo/kvazaar.git | `d6815293f34a094e26ba6c50b8644660ddc13e09` |
| `--enable-libaribcaption` | libaribcaption | https://github.com/xqq/libaribcaption.git | `c64c23b8905ba514b87c9789269e9f66f949ffe0` |
| `--enable-libass` | libass | https://github.com/libass/libass.git | `3087d2b2ffda76602a17f9b09d25cb8addc8d313` |
| `--enable-libbluray` | libbluray | https://code.videolan.org/videolan/libbluray.git | `065247e5ef40ccf39857db81e2c1368354a23ef8` |
| `--enable-libjxl` | libjxl | https://github.com/libjxl/libjxl.git | `d089091afeb7b00b3d0fec6f019d35eaa3b2b410` |
| `--enable-libmp3lame` | libmp3lame | https://svn.code.sf.net/p/lame/svn/trunk/lame | svn r6761 |
| `--enable-libopus` | libopus | https://github.com/xiph/opus.git | `3da9f7a6db1c05c3996cb363a9d1931a978bf1be` |
| `--enable-libplacebo` | libplacebo | https://code.videolan.org/videolan/libplacebo.git | `22ee762e8e0890fc54068beb670310f0edce7263` |
| `--enable-librist` | librist | https://code.videolan.org/rist/librist.git | `4f45ef8f78983892d52ccd52d9f675435b23738f` |
| `--enable-libssh` | libssh | https://gitlab.com/libssh/libssh-mirror.git | `1dc52926c54b59ea7a3350a59a476bb76da3ff33` |
| `--enable-libtheora` | libtheora | https://github.com/xiph/theora.git | `28fd5ec77f0ad0e07a371cef1047828116f6bd8a` |
| `--enable-libvpx` | libvpx | https://chromium.googlesource.com/webm/libvpx | `9cc8e1c18024d6b64422ecb7fdd7a43c8e873908` |
| `--enable-libwebp` | libwebp | https://chromium.googlesource.com/webm/libwebp | `ba8358578ecbc4c75f0723163275de00dec0c704` |
| `--enable-libzmq` | libzmq | https://github.com/zeromq/libzmq.git | `46493370217ac135246617fa2f6ac819d8b61bfc` |
| `--enable-lv2` | lv2 | https://github.com/lv2/lv2.git | `3c57dae600a5ad8d05acd53ee3490f7d91cb7be6` |
| `--enable-libvpl` | onevpl | https://github.com/intel/libvpl.git | `674d015bcb294bc39fa276e99a652ea045423e82` |
| `--enable-openal` | openal | https://github.com/kcat/openal-soft.git | `c157b87cb9eb58d747748e49f82ee4a64d1dcf8b` |
| `--enable-liboapv` | openapv | https://github.com/AcademySoftwareFoundation/openapv.git | `d625af974550427e638574db61c270fe7f8c5a73` |
| `--enable-libopencore-amrnb` | opencore-amr | https://git.code.sf.net/p/opencore-amr/code | `7dba8c32238418ce0b316a852b2224df586ca896` |
| `--enable-libopencore-amrwb` | opencore-amr | https://git.code.sf.net/p/opencore-amr/code | `7dba8c32238418ce0b316a852b2224df586ca896` |
| `--enable-libopenh264` | openh264 | https://github.com/cisco/openh264.git | `98bc7cbbeb7381c94ef8f9a5d158327abbf6b8b9` |
| `--enable-libopenjpeg` | openjpeg | https://github.com/uclouvain/openjpeg.git | `402ef5862195b177ea0a7788f2a6ef2804e62285` |
| `--enable-libopenmpt` | openmpt | https://github.com/OpenMPT/openmpt.git | `1b000d4bca071364157b5d35940efe4feecd2e0a` |
| `--enable-librav1e` | rav1e | https://github.com/xiph/rav1e.git | `564ae3b0007ae2b06893fd7166bf88c5a84c5b63` |
| `--enable-librubberband` | rubberband | https://github.com/breakfastquay/rubberband.git | `e4296ac80b1170018a110bc326fd0d45a0eb27d6` |
| `--enable-sdl2` | sdl | https://github.com/libsdl-org/SDL.git | `a2e7c76bda17c853ba93c7d2c9fdddf8a9d621d1` |
| `--enable-libsnappy` | snappy | https://github.com/google/snappy.git | `747488a9f3d0daf9b639b6704d7188fba48af179` |
| `--enable-libsrt` | srt | https://github.com/Haivision/srt.git | `fcae57145c000a9e7b72aa777adb8f85c2463242` |
| `--enable-libsvtav1` | svtav1 | https://gitlab.com/AOMediaCodec/SVT-AV1.git | `fb0ed7e5999caef1c4b51b7b4c8dc2c8c10f9291` |
| `--enable-libtwolame` | twolame | https://github.com/njh/twolame.git | `6fced852d4d5cfad58cf9dbe3ea619b08e87d398` |
| `--enable-libuavs3d` | uavs3d | https://github.com/uavs3/uavs3d.git | `0e20d2c291853f196c68922a264bcd8471d75b68` |
| `--enable-libdrm` | libdrm | https://gitlab.freedesktop.org/mesa/drm.git | `82f74e7a5a7403392e91352af00210ae26a81f19` |
| `--enable-vaapi` | libva | https://github.com/intel/libva.git | `6b07f7100512817f736967e899b8c26313c20623` |
| `--enable-libvidstab` | vidstab | https://github.com/georgmartius/vid.stab.git | `c9985e75303da2ba1a78b37083a2bec2ed5baebc` |
| `--enable-libvvenc` | vvenc | https://github.com/fraunhoferhhi/vvenc.git | `0f2e874451d6b194615e5dfefdc96796a7da00f4` |
| `--enable-libx264` | x264 | https://code.videolan.org/videolan/x264.git | `0480cb05fa188d37ae87e8f4fd8f1aea3711f7ee` |
| `--enable-libx265` | x265 | https://bitbucket.org/multicoreware/x265_git.git | `b81f650e21e8aacbe6a9ad04ce14aefc05b932c0` |
| `--enable-libxavs2` | xavs2 | https://github.com/pkuvcl/xavs2.git | `eae1e8b9d12468059bdd7dee893508e470fa83d8` |
| `--enable-libxvid` | xvid | https://svn.xvid.org/trunk/xvidcore | svn r2204 |
| `--enable-libzimg` | zimg | https://github.com/sekrit-twc/zimg.git | `f6cc75ad23db1bb9c53673c15523e6b6e960ffc6` |
| `--enable-libzvbi` | zvbi | https://github.com/zapping-vbi/zvbi | `d3a5ee9f2b047bf16cd1ee5ccf6ec05ee75409d0` |

Три оставшихся флага библиотек не добавляют: `--enable-gpl`,
`--enable-version3` (лицензия) и `--enable-cuda-llvm` (режим компиляции
поверх заголовков `ffnvcodec`, названных отдельной строкой).

**lame и xvid** пинятся не git-коммитом, а ревизией SVN (`SCRIPT_REV`):
сборщик выгружает их `svn checkout '<репозиторий>@<ревизия>'`. Ревизия
точная, поэтому пробелом это не является — в отличие от x264 на macOS,
где сборщик берёт снимок `master` и коммит нигде не публикует.

Сборка рассчитана на **glibc ≥ 2.28** (README BtbN: цель — RHEL/CentOS 8
и новее). Живьём не проверялась: машины нет (Р-6).

### Почему Linux берётся не у того же сборщика, что macOS

Соблазн «взять Linux у martin-riedl, раз оттуда обе macOS-сборки»
проверен и **отклонён измерением** (TL-133). Linux-сборки этого сервера
собраны с `--enable-nonfree` (из-за `--enable-decklink`): построчный
`diff` их `versions.txt` с нашей macOS-сборкой даёт ровно это отличие
плюс компилятор и префикс. Такую сборку **нельзя распространять вообще
ни под какой лицензией** — вывод раздела 1 исследования
`specs/2026-09-17-ffmpeg-license-research.md` (внутренний документ
проекта, публично недоступен — см. начало файла), а мы кладём ffmpeg в
установщик, то есть распространяем. Проверить вывод можно и без него: он
целиком следует из строки `configure` самой сборки, а она лежит в
`versions.txt` рядом с архивом у сборщика.

Наши **macOS-пины чисты**: у обеих сборок (`macos/arm64/1787073674_9.0.1`
и `macos/amd64/1787081194_9.0.1`) флага `--enable-nonfree` в строке
`configuration` нет — проверено 2026-09-26. Флаг есть у Linux-ветки
сборщика, а не у отдельной сборки, поэтому повторять попытку не нужно.
Windows этот сборщик не публикует вовсе.

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

Репозиторий кода открыт, поэтому **обе ссылки шаблона — и на файл, и на
issues — работают анонимно**, и §6d по ним выполняется для любого
получателя установщика, а не только для владельца: указание рядом с
объектным кодом ведёт на общедоступные адреса. Прежнее ограничение
(«раздавать по такому указателю установщик кому-то ещё нельзя») снято
именно открытием репозитория.

Оговорка, которая этим не закрывается: речь только о **работоспособности
указателя**, а не о полноте Corresponding Source. Пробелы комплекта —
ревизия x264 на macOS и версии вложенных библиотек для Windows и Linux —
остаются открытыми (#136), и по ним §6d ещё не выполнен. Порядок на
случай раздачи установщика (архив исходников ассетом рядом с
установщиком, экран «О программе») определён решением по
https://github.com/execaus/tube-leak/issues/14 и задачами #134/#135.

## Порядок при обновлении ffmpeg

Указатель обновляется **в той же задаче, что и пин**: меняя версию
ffmpeg в `src-tauri/binaries.lock.json`, заново снимите строку
`configuration` с нового бинарника и материалы сборщика (`versions.txt`
у martin-riedl, `README.txt` пакета у gyan, `scripts.d/*.sh` на коммите
тега у BtbN), обновите таблицы, адреса и дату снятия в этом файле — пин
без обновлённого указателя оставляет обязательство §6 незакрытым.

**Правило выбора источника (TL-133).** Адрес годится, только если он
**неподвижен по устройству**: версионный тег релиза GitHub либо
версионированный путь сборщика, который держит архив версий. Датированный
тег автосборки и любой «latest» непригодны, даже когда отвечают 200
сегодня: первый исчезнет по расписанию ротации, второй перезаписывается
при каждой сборке. После правки пина прогоните `npm run check-pins`.

**Известные сроки и долги.**

- Linux-адрес BtbN живёт примерно **до августа 2028**: последний билд
  месяца хранится два года. Дальше — перепиновка на свежий месячный тег.
- У martin-riedl с **января 2027** не будет новых release-сборок под
  macOS Intel (объявлено на его главной странице): при следующем бампе
  ffmpeg источник для `x86_64-apple-darwin` придётся искать заново.
