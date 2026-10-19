// Wasmicon HAL のゲスト向けバインディング（C）。
//
// generated.h は wasmicon-gen の出力。手で編集しない。
// ここは生成された import 宣言に薄いヘルパをかぶせたもの。
//
// C には Drop が無いので、ハンドルの解放は `wasmicon_*_drop` を明示的に呼ぶ
// （呼ばずに run から戻っても、ホストが残りを drop する。abi-spec §5.2）。
// ビルドフラグは build.sh が持つ（abi-spec §6.1 の機能セットとメモリ上限）。

#ifndef WASMICON_HAL_H
#define WASMICON_HAL_H

#include "generated.h"

#ifdef __cplusplus
extern "C" {
#endif

/** ゲストの export に付ける属性。エントリポイントは `WASMICON_EXPORT("run") void run(void)`（abi-spec §3.3）。 */
#define WASMICON_EXPORT(name) __attribute__((export_name(name)))

/**
 * NUL 終端の文字列の長さ。libc を使わない（-nostdlib）ので strlen は無い。
 * リテラルなら -O2 で定数に畳まれる。
 */
static inline uint32_t wasmicon_strlen(const char *s) {
    uint32_t n = 0;
    while (s[n] != '\0') {
        n++;
    }
    return n;
}

/** NUL 終端の文字列をログに出す。改行はホストが付加する。 */
static inline void wasmicon_log_write(wasmicon_log_level_t level, const char *message) {
    wasmicon_log_log((uint32_t)level, message, wasmicon_strlen(message));
}

static inline void wasmicon_log_error(const char *message) {
    wasmicon_log_write(WASMICON_LOG_LEVEL_ERROR, message);
}

static inline void wasmicon_log_warn(const char *message) {
    wasmicon_log_write(WASMICON_LOG_LEVEL_WARN, message);
}

static inline void wasmicon_log_info(const char *message) {
    wasmicon_log_write(WASMICON_LOG_LEVEL_INFO, message);
}

static inline void wasmicon_log_debug(const char *message) {
    wasmicon_log_write(WASMICON_LOG_LEVEL_DEBUG, message);
}

/** NUL 終端の役割名から GPIO 番号を引く。戻り値はステータス（0 = 成功）。 */
static inline uint32_t wasmicon_board_pin_by_role_str(const char *role, uint32_t *out) {
    return wasmicon_board_pin_by_role(role, wasmicon_strlen(role), out);
}

#ifdef __cplusplus
}
#endif

#endif // WASMICON_HAL_H
