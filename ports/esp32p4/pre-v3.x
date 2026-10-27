/* Makes the esp-hal link fit ESP32-P4 silicon before v3.0 (the M5Stack Tab5
 * has v1.0, ROM "esp32p4-eco2-20240710").
 *
 * Read after linkall.x (.cargo/config.toml), so each assignment here replaces
 * esp-hal's. All four fix the same kind of mismatch: esp-hal 1.2 and
 * esp-rom-sys 0.1.5 describe v3.x (ECO5) silicon only. None of them crashes
 * when wrong; each returns plausible garbage, which is why they are worth the
 * comments. Delete them once upstream lets the link pick the silicon
 * revision (docs/TODO.md §1.3.2). The image header already keeps a v3.x chip
 * from booting this build (espflash writes a maximum revision of v1.99), and
 * `main` checks again.
 */

/* 1. ROM function addresses.
 *
 * esp-rom-sys's ld/esp32p4/rom-functions.x includes only the ECO5 tables.
 * Older ROMs have the same functions at other addresses: __ashldi3 is
 * 0x4fc00744 in ECO5 and 0x4fc00750 here, so every 64-bit variable shift
 * called a different function, LEB128 decoding returned 0x1_0000_0001 for 1,
 * and `decode` rejected valid modules. The pre-ECO5 tables ship in the same
 * directory, found through the link path esp-rom-sys sets.
 *
 * A few names are defined only by the ECO5 tables or rom/additional.ld and
 * keep their ECO5 values. Of those, the firmware uses only
 * `syscall_table_ptr`, set in 4 below.
 */
INCLUDE "rom/esp32p4.rom.ld"
INCLUDE "rom/esp32p4.rom.libgcc.ld"
INCLUDE "rom/esp32p4.rom.rvfp.ld"

/* 2. memcpy and friends.
 *
 * esp-rom-sys points these at ECO5 ROM addresses (rom/additional.ld) and
 * ships no pre-ECO5 ones, so they go to the port's own versions in
 * src/mem.rs instead of the ROM.
 */
memcpy = wasmicon_memcpy;
memmove = wasmicon_memmove;
memset = wasmicon_memset;
memcmp = wasmicon_memcmp;

/* 3. The top of the stack.
 *
 * esp-hal puts the stack top at the end of its RAM region, 0x4FFADFC0. On
 * the v1.0 chip the L2 memory from 0x4FFA0000 up is not RAM (the second-stage
 * bootloader gives it to the L2 cache), so a stack there only looks right
 * while its lines stay in the cache: once `instantiate` zeroed linear memory
 * and evicted them, the stack contents were lost (docs/TODO.md §1.3.2).
 * esp-riscv-rt loads `sp` from `_stack_start`; `_stack_start_cpu0` is the
 * bound esp-hal checks `sp` against, so both move together.
 */
_stack_start = 0x4FFA0000;
_stack_start_cpu0 = 0x4FFA0000;

/* 4. The ROM C library's data.
 *
 * `esp_hal::init` stores the address of its syscall table in
 * `syscall_table_ptr` (esp-rom-sys `init_syscall_table`). rom/additional.ld
 * puts that variable at the ECO5 address 0x4FFBFFE4, inside the L2 cache on
 * the v1.0 chip, so the store would land in a cache line. The pre-ECO5
 * addresses are those of ESP-IDF v5.5.3,
 * components/esp_rom/esp32p4/ld/esp32p4.rom.libc.ld (the ECO5 file of the
 * same release differs from it in these two lines only).
 */
syscall_table_ptr = 0x4ff3ffe4;
_global_impure_ptr = 0x4ff3ffe0;
