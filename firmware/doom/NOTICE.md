# NOTICE — GPL-walled demo payload (NOT part of the MIT package)

The `doom` demo in this repo is a GPL derivative and stays OUT of the npm
tarball (`site/firmware.js` packed manifest + `site/vendor/` + engine JS are
MIT; see the root LICENSE).

## What is GPL here, and why

- `firmware/doom/engine/` — 177 files from the Chocolate-Doom / doomgeneric
  lineage (id Software 1993–1996, Simon Howard 2005–2014, Ozkan Sezgin).
  Each carries the GPL-2.0-or-later header
  (e.g. `firmware/doom/engine/d_main.c:1-8`). Upstream:
  https://github.com/ozkl/doomgeneric (GPL-2.0).
- `firmware/doom/doom.bin` + the `doom` entry in `site/firmware.js`
  (full demo bundle only) — a linked binary of the above, hence GPL-derived.
- `site/doom1.wad` — DOOM 1 shareware game data (id Software, commercial
  shareware terms, NOT open source). Served to the demo page only; never in
  npm `files`.

## What is original (MIT, yours)

- `firmware/doom/f407/` (8 files: `main_f407.c`, `startup.c`, `platform.c`,
  `i_system_f407.c`, `i_sound_f407.c`, `i_joystick_f407.c`, `w_file_mem.c`,
  `doomplatform.h`) — clean F407 glue with no GPL headers. If you re-port
  this glue to a non-GPL engine, that port is yours under MIT.
- The emulator engine itself (`stm32-periph-wasm/src`, `site/emulator.js`,
  `site/netsim.js`, drivers, tools) contains zero GPL text — verified by
  grep (only `firmware/doom/engine` + `firmware/lwip_demo/lwip` +
  root `LICENSE` matched before the MIT cutover).

## LwIP note (BSD, fine)

`firmware/lwip_demo/lwip/` is the SICS BSD stack
(`Redistribution and use...` headers) + clean glue — it ships in the npm
bundle alongside the other MIT/BSD demos.

## Packaging rule

`tools/make_firmware.mjs` defaults to the slim manifest (`NPM_SLIM=1`):
`doom` excluded. The full demo bundle (doom blob for `site/doom.html`) is
regenerated with `NPM_SLIM=0 node tools/make_firmware.mjs`. The `prepack`
hook uses the slim default, so `npm pack` can never smuggle the GPL blob.
