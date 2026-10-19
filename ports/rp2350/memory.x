/* Raspberry Pi Pico 2 / Pico 2 W (RP2350A): 4 MB of flash and 520 KB of SRAM.
 *
 * The section layout is that of rp235x-hal-examples/memory.x (rp235x-hal
 * 0.4.0). Unlike the RP2040 there is no second-stage bootloader; instead the
 * IMAGE_DEF block (.start_block) the boot ROM reads goes in the first 4 KB of
 * flash. */
MEMORY {
    FLASH : ORIGIN = 0x10000000, LENGTH = 4096K
    /* SRAM0-7. Striped, so the load spreads across the banks. */
    RAM   : ORIGIN = 0x20000000, LENGTH = 512K
    /* Banks 8/9 are direct-mapped, for data with a fixed purpose (unused). */
    SRAM8 : ORIGIN = 0x20080000, LENGTH = 4K
    SRAM9 : ORIGIN = 0x20081000, LENGTH = 4K
}

SECTIONS {
    /* The block the boot ROM and picotool read. Placed right after
     * .vector_table so that it lands in the first 4 KB of flash. */
    .start_block : ALIGN(4)
    {
        __start_block_addr = .;
        KEEP(*(.start_block));
        KEEP(*(.boot_info));
    } > FLASH

} INSERT AFTER .vector_table;

/* .text starts after the block.
 * The upstream memory.x has no ALIGN here, which puts .text on a 4-byte
 * boundary only, and rust-lld warns that the placement of .text is not a
 * multiple of its alignment (8). Round up to 8. */
_stext = ALIGN(ADDR(.start_block) + SIZEOF(.start_block), 8);

SECTIONS {
    /* Metadata that picotool reads. Empty for now. */
    .bi_entries : ALIGN(4)
    {
        __bi_entries_start = .;
        KEEP(*(.bi_entries));
        . = ALIGN(4);
        __bi_entries_end = .;
    } > FLASH
} INSERT AFTER .text;

SECTIONS {
    /* May carry a signature, so it goes after the whole program. */
    .end_block : ALIGN(4)
    {
        __end_block_addr = .;
        KEEP(*(.end_block));
        __flash_binary_end = .;
    } > FLASH

} INSERT AFTER .uninit;

PROVIDE(start_to_end = __end_block_addr - __start_block_addr);
PROVIDE(end_to_start = __start_block_addr - __end_block_addr);
