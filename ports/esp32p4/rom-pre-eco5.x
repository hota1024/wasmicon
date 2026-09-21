/* ESP32-P4 の ROM 関数アドレスを **ECO5 より前**のレイアウトへ戻す。
 *
 * esp-rom-sys 0.1.5 の ld/esp32p4/rom-functions.x は ECO5 の表を
 * ハードコードしている（選択肢が無い）:
 *
 *     INCLUDE "rom/esp32p4.rom.eco5.ld"
 *     INCLUDE "rom/esp32p4.rom.eco5.libgcc.ld"
 *     INCLUDE "rom/esp32p4.rom.eco5.rvfp.ld"
 *
 * ところが M5Stack Tab5 の ROM は **esp32p4-eco2-20240710**（起動バナー）で、
 * ECO5 ではない。表がずれているので ROM 関数の呼び出しが別の関数に当たる。
 *
 * 実害の実例（2026-09-21 に実機で特定）:
 *   __ashldi3  ECO5 = 0x4fc00744 / 非 ECO5 = 0x4fc00750  … 12 バイトの差
 * 64bit の可変シフトはこの関数へのライブラリ呼び出しになるため、
 * `u64_leb` が 1 ではなく 0x0000_0001_0000_0001 を返し、デコードが
 * 「memory size must be at most 65536 pages」で失敗していた。
 * **クラッシュせずもっともらしい誤値を返す**ので発見が難しい。
 *
 * 非 ECO5 の表は esp-rom-sys が同じディレクトリに同梱しているので、
 * linkall.x の後でこれを読ませ、シンボル代入の後勝ちで上書きする。
 * INCLUDE のパス解決には esp-rom-sys が出す link-search をそのまま使う。
 *
 * **上流が ROM リビジョンを選べるようになったら消すこと。**
 * 経緯と削除条件は docs/TODO.md §1.1.5。
 */

INCLUDE "rom/esp32p4.rom.ld"
INCLUDE "rom/esp32p4.rom.libgcc.ld"
INCLUDE "rom/esp32p4.rom.rvfp.ld"

/* memcpy / memset / memmove / memcmp は **非 ECO5 の正しいアドレスが
 * 同梱されていない**（additional.ld の ECO5 由来の定義しか無い）ので、
 * ROM を使わず自前実装 (src/mem.rs) へ向ける。
 *
 * これを入れる前は、`decode` が完全に成功しているのに `instantiate` を
 * 通ると `Module` のスライス長が部分的に 0 へ潰れていた（2026-09-21 実機）。
 */
memcpy  = wasmicon_memcpy;
memmove = wasmicon_memmove;
memset  = wasmicon_memset;
memcmp  = wasmicon_memcmp;

/* スタックの先頭を**実在する RAM の内側**へ下げる。
 *
 * esp-hal は L2MEM を 768KB (0x4FF00000..0x4FFC0000) とみなし
 * `_stack_start = 0x4FFADFC0` を置く。しかし実機で測ると
 * **0x4FF9E000 は生きていて 0x4FFA0000 は死んでいる**。つまり使えるのは
 * 0x4FF00000..0x4FFA0000 の 640KB だけで、上位 128KB は L2 キャッシュに
 * 割り当てられていて RAM として存在しない（128KB は P4 の L2 キャッシュ容量）。
 *
 * 実在しない領域に置かれたスタックは**キャッシュに載っている間だけ正しく
 * 見える**。arena に 48KB 以上書いてキャッシュラインが追い出された瞬間に
 * 内容が失われ、`instantiate` が線形メモリを 64KB ゼロ埋めした直後に
 * `Instance` が壊れる、という症状になっていた（2026-09-21 に実機で特定）。
 * クラッシュせず静かに壊すので、これも発見が難しい類い。
 *
 * **上流が L2 キャッシュ設定を見て RAM 長を決めるようになったら消すこと。**
 * 経緯と測定値は docs/TODO.md §1.1.5。
 */
_stack_start = 0x4FFA0000;
_stack_start_cpu0 = 0x4FFA0000;
