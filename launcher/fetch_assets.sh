#!/bin/zsh
# Copy the files the launcher embeds into assets/ (see assets/README.md), before a build.
# Usage: fetch_assets.sh <src> [--with-ips]   (or DBZ_ASSETS_SRC=<src> fetch_assets.sh [--with-ips])
#   <src>: a folder holding zh-tw.tsv, ui-zh-tw.tsv, zh-hk.tsv, ui-zh-hk.tsv (Taiwan's and Hong Kong's tables), fonts/, art-override/ (and its super-vegeta/ with its key-<key>.pose,
#   goku/, ending-goku/, variants/, ssj-lightning/, mode7/<content key>/) and
#   win/dbz-ssd-recompiled-windows-v2/ (layout in
#   assets/README.md). --with-ips only for a private `--features bundle-ips` build; without that feature
#   build.rs leaves an assets/dbz102.ips out of the exe anyway. <src>/voice/ssj-transform.wav (the voice of Goku's first
#   Super Saiyan transformation) is required, copied into assets/voice/.
set -eu
here=${0:A:h}
me=${0:t}
usage() { print -u2 "usage: $me <src> [--with-ips]   (or set DBZ_ASSETS_SRC=<src>)"; }
src=${DBZ_ASSETS_SRC:-}
with_ips=0
for arg in "$@"; do
  case $arg in
    --with-ips) with_ips=1 ;;
    -*) usage; exit 2 ;;
    *) src=$arg ;;
  esac
done
if [[ -z $src ]]; then usage; exit 2; fi
win="$src/win/dbz-ssd-recompiled-windows-v2"
a="$here/assets"
# super-vegeta/key-<key>.png (<key> 16 hexadecimal digits, ssd_hd::pack::KEYED_PICTURE) is drawn in a release only with
# its pose key-<key>.pose beside it (what the pose is without the original sprite's pixels; `cargo run -p xtask --
# keyed-poses <src>/art-override/super-vegeta` writes it from the pixels key-<key>.txt, which stay in <src> and are never
# copied), and the pose draws nothing without its picture: one without the other stops the fetch, before anything is copied
sv="$src/art-override/super-vegeta"
unpaired=()
for f in "$sv"/key-*.(png|pose)(N); do
  if [[ ${f:t} =~ '^key-[0-9a-fA-F]{16}\.(png|pose)$' ]]; then
    if [[ ${f:e} == png ]]; then other="${f:r}.pose"; else other="${f:r}.png"; fi
    [[ -f $other ]] || unpaired+=( "${f:t} without ${other:t}" )
  fi
done
if (( ${#unpaired} )); then print -u2 "$me: $sv: ${(j:, :)unpaired}"; exit 1; fi
mkdir -p "$a/fonts" "$a/licenses"
cp "$win/ssd-recompiled.exe" "$win/symbols.toml" "$win/LICENSE.txt" "$win/NOTICE.md" "$win/PATCH_NOTES.txt" "$a/"
cp "$win"/licenses/* "$a/licenses/"
cp "$src/zh-tw.tsv" "$src/ui-zh-tw.tsv" "$src/zh-hk.tsv" "$src/ui-zh-hk.tsv" "$a/"
# The font is LXGW WenKai TC; an older Noto Sans TC left in assets/ would be embedded too (build.rs takes every file)
rm -f "$a/fonts/NotoSansTC-Regular.otf"
cp "$src/fonts/LXGWWenKaiTC-Regular.ttf" "$src/fonts/OFL.txt" "$a/fonts/"
# Super Vegeta without his shoulder pads (the game draws <data dir>\art-override\<art id>.png in place of the HD pack's
# picture, patch 0047): every PNG of <src>/art-override/, the old ones in assets/ replaced
art=( "$src"/art-override/*.png(N) )
if (( ${#art} == 0 )); then print -u2 "$me: no $src/art-override/*.png"; exit 1; fi
mkdir -p "$a/art-override"
rm -f "$a"/art-override/*.png(N)
cp "${art[@]}" "$a/art-override/"
# and the PNGs of its folders super-vegeta/ (patch 0049, Super Vegeta's own pictures of Vegeta's poses), goku/ (patch 0051,
# the normal Goku's of Super Saiyan poses), ending-goku/ (Goku's pictures for the ending's flight with Gohan only),
# variants/ (pose variants: the pack's picture grown by 76 px on each side) and ssj-lightning/ (frame-1.png to frame-4.png,
# the lightning drawn over the picture while Goku first becomes a Super Saiyan, the game's ssd_hd::lightning::DIR),
# each replaced the same way; a folder <src> lacks is left empty
for sub in super-vegeta goku ending-goku variants ssj-lightning; do
  mkdir -p "$a/art-override/$sub"
  rm -f "$a/art-override/$sub"/*.png(N)
  pics=( "$src/art-override/$sub"/*.png(N) )
  if (( ${#pics} )); then cp "${pics[@]}" "$a/art-override/$sub/"; fi
done
# and of super-vegeta/ the poses key-<key>.pose (<key> 16 hexadecimal digits) the game reads beside key-<key>.png (a pose
# the pack has no art for, ssd_hd::pack::KEYED_PICTURE), replaced the same way; never the pixels key-<key>.txt (the original
# sprite's: an older fetch's are removed, and build.rs refuses any left in assets/)
rm -f "$a/art-override/super-vegeta"/key-*.(txt|pose)(N)
for f in "$src/art-override/super-vegeta"/key-*.pose(N.); do
  if [[ ${f:t} =~ '^key-[0-9a-fA-F]{16}\.pose$' ]]; then cp "$f" "$a/art-override/super-vegeta/"; fi
done
# and the Mode 7 planes of mode7/ (a folder <content key>/ for each plane by what it shows, the game's
# ssd_hd::pack::MODE7_DIR), two levels deep: of each <key>/ its PNG tiles, palette.txt and the empty file replace (names
# exactly so: the game reads only those), the old ones in assets/ replaced; a mode7/ <src> lacks is left empty
mkdir -p "$a/art-override/mode7"
rm -f "$a/art-override/mode7"/*/(*.png|palette.txt|replace)(N)
for plane in "$src/art-override/mode7"/*(N/); do
  files=( "$plane"/(*.png|palette.txt|replace)(N.) )
  if (( ${#files} )); then
    mkdir -p "$a/art-override/mode7/${plane:t}"
    cp "${files[@]}" "$a/art-override/mode7/${plane:t}/"
  fi
done
if (( with_ips )); then cp "$src/dbz102.ips" "$a/"; fi
[[ -s "$src/voice/ssj-transform.wav" ]] || { print -u2 "$me: no $src/voice/ssj-transform.wav"; exit 1; }
mkdir -p "$a/voice"
cp "$src/voice/ssj-transform.wav" "$a/voice/"
(cd "$a" && find . -type f ! -name README.md ! -name .gitignore | sort | xargs shasum -a 256)
