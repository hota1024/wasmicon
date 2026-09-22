/* Raspberry Pi Pico 2 / Pico 2 W (RP2350A)。4 MB のフラッシュと 520 KB の SRAM。
 *
 * セクションの構成は rp235x-hal-examples/memory.x（rp235x-hal 0.4.0）と同じ。
 * RP2040 と違って二段目のブートローダは要らず、代わりに Boot ROM が読む
 * IMAGE_DEF ブロック (.start_block) をフラッシュの先頭 4 KB に置く。 */
MEMORY {
    FLASH : ORIGIN = 0x10000000, LENGTH = 4096K
    /* SRAM0-7。ストライプ配置なのでバンクへの負荷が散る。 */
    RAM   : ORIGIN = 0x20000000, LENGTH = 512K
    /* バンク 8/9 はダイレクト配置。用途を固定したいときに使う（今は使わない）。 */
    SRAM8 : ORIGIN = 0x20080000, LENGTH = 4K
    SRAM9 : ORIGIN = 0x20081000, LENGTH = 4K
}

SECTIONS {
    /* Boot ROM と picotool が読むブロック。フラッシュの先頭 4 KB に収める
     * ため .vector_table の直後に置く。 */
    .start_block : ALIGN(4)
    {
        __start_block_addr = .;
        KEEP(*(.start_block));
        KEEP(*(.boot_info));
    } > FLASH

} INSERT AFTER .vector_table;

/* .text はブロックの後ろから始める。
 * 上流の memory.x は ALIGN を掛けていないが、それだと .text の先頭が
 * 4 バイト境界にしか乗らず、rust-lld が「.text の配置が alignment (8) の
 * 倍数でない」と警告する。8 に切り上げておく。 */
_stext = ALIGN(ADDR(.start_block) + SIZEOF(.start_block), 8);

SECTIONS {
    /* picotool が読むメタデータ。今は空。 */
    .bi_entries : ALIGN(4)
    {
        __bi_entries_start = .;
        KEEP(*(.bi_entries));
        . = ALIGN(4);
        __bi_entries_end = .;
    } > FLASH
} INSERT AFTER .text;

SECTIONS {
    /* 署名が入りうるので、プログラム全体の後ろに置く。 */
    .end_block : ALIGN(4)
    {
        __end_block_addr = .;
        KEEP(*(.end_block));
        __flash_binary_end = .;
    } > FLASH

} INSERT AFTER .uninit;

PROVIDE(start_to_end = __end_block_addr - __start_block_addr);
PROVIDE(end_to_start = __start_block_addr - __end_block_addr);
