# Roadmap

Beneath a Typographic Sky is at 0.9.0 (2026-10-28): the whole game plays,
from the intro to the end credits. 1.0.0 is when the items below are done.

## 0.9.0: built

- [x] ScummVM's sky engine ported to Rust, running the original CD data
- [x] Truecolour Braille, half blocks or quadrants (F2 cycles); game text as
  real type over the picture; the mouse through the terminal
- [x] Speech, sound effects, AdLib music, the CD intro
- [x] A headless walkthrough that plays the whole game, with saves at 23
  story points
- [x] Packaging, the preview cast, the catalog entry

## 0.9.1: built

- [x] Release workflow: Linux (glibc, x86_64 and ARM64), macOS (Intel and
  Apple Silicon) and Windows (x64 and ARM64), each packed by
  asciiverse/package.sh with the game data
- [x] package.sh runs on GitHub's builders: 7-Zip where zip and unzip are
  missing (Windows), no strip for MSVC builds, a native strip on ARM Linux

## 1.0.0

- [ ] The port checked against ScummVM frame by frame on a few scenes
- [ ] macOS and Windows builds
- [ ] Played through and tuned
