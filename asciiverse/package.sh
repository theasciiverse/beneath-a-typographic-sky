#!/bin/sh
# Packs the game for av: dist/<id>-<target>.tar.gz (.zip for Windows),
# holding the binary, data/ with the freeware game data and Revolution's
# readme.txt (its licence asks that the readme travels with the data),
# NOTICE and LICENSE:
#
#   typosky             the game (typosky.exe on Windows)
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
unzip -q "$cache/$zip_name" -d "$cache/unzipped"
src="$cache/unzipped/bass-cd-1.2"

stage="stage/$id"
rm -rf "$stage" && mkdir -p "$stage/data"
cp "$bin" "$stage/"
for f in sky.dsk sky.dnr sky.cpt readme.txt; do
  cp "$src/$f" "$stage/data/"
done
cp NOTICE LICENSE "$stage/"

mkdir -p dist
if [ -n "$exe" ]; then
  rm -f "dist/$id-$target.zip"
  (cd "$stage" && zip -qr "../../dist/$id-$target.zip" .)
else
  tar -czf "dist/$id-$target.tar.gz" -C "$stage" .
fi
ls -l dist
