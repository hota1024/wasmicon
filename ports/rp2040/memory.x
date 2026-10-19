/* Raspberry Pi Pico / Pico W(H): 2 MB of flash and 264 KB of SRAM. */
MEMORY {
    BOOT2 : ORIGIN = 0x10000000, LENGTH = 0x100
    FLASH : ORIGIN = 0x10000100, LENGTH = 2048K - 0x100
    RAM   : ORIGIN = 0x20000000, LENGTH = 256K
}

EXTERN(BOOT2_FIRMWARE)

SECTIONS {
    /* Second-stage bootloader. Must occupy the first 256 bytes. */
    .boot2 ORIGIN(BOOT2) :
    {
        KEEP(*(.boot2));
    } > BOOT2
} INSERT BEFORE .text;

SECTIONS {
    /* End of the firmware image. Needed at boot to check that the app slot
     * (profile::RP2040.slot in ports/common) does not overlap the firmware.
     * Same symbol name as the one the rp2350 memory.x places inside
     * .end_block. */
    .wasmicon_fw_end : ALIGN(4)
    {
        __flash_binary_end = .;
    } > FLASH
} INSERT AFTER .uninit;
