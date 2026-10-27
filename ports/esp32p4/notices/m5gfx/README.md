# M5GFX (excerpt used by ports/esp32p4)

`ports/esp32p4/src/tab5_ili9881c.rs` holds the ILI9881C init commands of the
M5Stack Tab5 panel, extracted from `src/lgfx/v1/platforms/esp32p4/Panel_ILI9881C.hpp`
of M5GFX 0.2.32 (https://github.com/m5stack/M5GFX). The register values for the
Tab5's IO expanders in `ports/esp32p4/src/tab5.rs` come from `src/M5GFX.cpp`
of the same release.

- M5GFX is under the MIT license; its notice is `LICENSE` in this directory.
- `Panel_ILI9881C.hpp` carries the LovyanGFX header, which names the FreeBSD
  license (https://github.com/lovyan03/LovyanGFX/blob/master/license.txt). That
  notice is not copied here yet (docs/TODO.md §6-8).
