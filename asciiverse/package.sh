#!/bin/sh
# Packs the game for av: dist/<id>-<target>.tar.gz (.zip for Windows),
# holding the binary, data/ with the freeware game data and Revolution's
# readme.txt (its licence asks that the readme travels with the data),
# NOTICE and LICENSE:
#
#   typosky             the game, stripped (typosky.exe on Windows)
#   data/sky.dsk        Beneath a Steel Sky CD v0.0372, unmodified
#   data/sky.dnr
#   data/sky.cpt
#   data/readme.txt
#   NOTICE              a modified version, credits, licences
#   LICENSE             GPL-3.0-or-later, for the engine
#
# The data is not in git. It is taken from reference/bass-cd-1.2/ when
# present, else downloaded from ScummVM's site; either way the zip's
# sha256 is checked first. Build the binary before packing:
#
#   cargo build --release --target x86_64-unknown-linux-musl
#   asciiverse/package.sh x86_64-unknown-linux-musl
set -eu
cd "$(dirname "$0")/.."
target="$1"
id="${ASCIIVERSE_ID:-beneath-a-typographic-sky}"
case "$target" in *windows*) exe=.exe ;; *) exe= ;; esac

zip_name="bass-cd-1.2.zip"
zip_url="https://downloads.scummvm.org/frs/extras/Beneath%20a%20Steel%20Sky/bass-cd-1.2.zip"
zip_sha256="53209b9400eab6fd7fa71518b2f357c8de75cfeaa5ba57024575ab79cc974593"

bin="target/$target/release/typosky$exe"
[ -f "$bin" ] || bin="target/release/typosky$exe"
[ -f "$bin" ] || { echo "package.sh: build typosky for $target first" >&2; exit 1; }

# the game data: the zip, checked, unpacked into a scratch folder
cache="stage/cache"
mkdir -p "$cache"
if [ -f "reference/$zip_name" ]; then
  cp "reference/$zip_name" "$cache/$zip_name"
elif [ ! -f "$cache/$zip_name" ]; then
  curl -sfL -o "$cache/$zip_name" "$zip_url"
fi
sum=$( (sha256sum "$cache/$zip_name" 2>/dev/null || shasum -a 256 "$cache/$zip_name") | cut -d' ' -f1)
[ "$sum" = "$zip_sha256" ] || { echo "package.sh: $zip_name has sha256 $sum, expected $zip_sha256" >&2; exit 1; }
rm -rf "$cache/unzipped" && mkdir -p "$cache/unzipped"
if command -v unzip >/dev/null 2>&1; then
  unzip -q "$cache/$zip_name" -d "$cache/unzipped"
else
  7z x -y -o"$cache/unzipped" "$cache/$zip_name" >/dev/null
fi
src="$cache/unzipped/bass-cd-1.2"

stage="stage/$id"
rm -rf "$stage" && mkdir -p "$stage/data"
cp "$bin" "$stage/"
# release archives carry a stripped binary (David, 2026-10-18); local
# builds keep their debug symbols. STRIP overrides the tool.
# MSVC builds keep their symbols in a separate .pdb, so there is nothing to
# strip; elsewhere the first strip tool found is used (a cross tool when
# building for another processor, the plain one on a native builder).
strip_tool="${STRIP:-}"
case "$target" in
  *windows-msvc*) strip_tool=none ;;
esac
if [ -z "$strip_tool" ]; then
  case "$target" in
    *windows-gnu*) tools="x86_64-w64-mingw32-strip llvm-strip" ;;
    aarch64*linux*) tools="aarch64-linux-gnu-strip strip llvm-strip" ;;
    *) tools="strip llvm-strip" ;;
  esac
  for t in $tools; do command -v "$t" >/dev/null 2>&1 && { strip_tool=$t; break; }; done
  [ -n "$strip_tool" ] || { echo "package.sh: no strip tool for $target (tried $tools)" >&2; exit 1; }
fi
if [ "$strip_tool" != none ]; then
  "$strip_tool" "$stage/typosky$exe" || { echo "package.sh: $strip_tool failed on typosky$exe" >&2; exit 1; }
fi
for f in sky.dsk sky.dnr sky.cpt readme.txt; do
  cp "$src/$f" "$stage/data/"
done
cp NOTICE LICENSE "$stage/"

mkdir -p dist
if [ -n "$exe" ]; then
  rm -f "dist/$id-$target.zip"
  # zip where it is installed, 7-Zip on Windows builders
  if command -v zip >/dev/null 2>&1; then
    (cd "$stage" && zip -qr "../../dist/$id-$target.zip" .)
  else
    (cd "$stage" && 7z a -tzip "../../dist/$id-$target.zip" . >/dev/null)
  fi
else
  tar -czf "dist/$id-$target.tar.gz" -C "$stage" .
fi
ls -l dist
