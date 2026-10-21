// Guest-side bindings of the Wasmicon HAL (C).
//
// generated.h is the output of wasmicon-gen; do not edit it by hand.
// This header adds thin helpers over the generated import declarations.
//
// C has no Drop, so release a handle by calling `wasmicon_*_drop` explicitly.
// (Handles still open when run returns are dropped by the host, docs/abi-spec.md §5.2.)
// The build flags live in build.sh (feature set and memory limit of docs/abi-spec.md §6.1).

#ifndef WASMICON_HAL_H
#define WASMICON_HAL_H

#include "generated.h"

#ifdef __cplusplus
extern "C" {
#endif

/**
 * Attribute for a guest export. The entry point is
 * `WASMICON_EXPORT("run") void run(void)` (docs/abi-spec.md §3.3).
 */
#define WASMICON_EXPORT(name) __attribute__((export_name(name)))

/**
 * Length of a NUL-terminated string. There is no strlen without libc (-nostdlib).
 * A literal folds to a constant at -O2.
 */
static inline uint32_t wasmicon_strlen(const char *s) {
    uint32_t n = 0;
    while (s[n] != '\0') {
        n++;
    }
    return n;
}

/** Logs a NUL-terminated string. The host appends the newline. */
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

/** Looks up the GPIO number of a NUL-terminated role name. Returns the status (0 = success). */
static inline uint32_t wasmicon_board_pin_by_role_str(const char *role, uint32_t *out) {
    return wasmicon_board_pin_by_role(role, wasmicon_strlen(role), out);
}

/**
 * Opens the asset with a NUL-terminated name (docs/app-workflow.md §3.10). Returns the
 * status (0 = success) and writes the handle to `out`. Release it with
 * `wasmicon_assets_blob_drop`.
 */
static inline uint32_t wasmicon_assets_blob_open_str(const char *name, uint32_t *out) {
    return wasmicon_assets_blob_open(name, wasmicon_strlen(name), out);
}

#ifdef __cplusplus
}
#endif

#endif // WASMICON_HAL_H
