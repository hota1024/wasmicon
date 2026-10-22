// Assets (C). Produces the same host-call sequence as assets-rs and assets-as.
//
// hello.txt is read whole and logged as text. pattern.bin is read in chunks
// and checked against the formula that generated it, so the trace and the
// log together show that every byte arrives intact (docs/app-workflow.md §3.10).

#include "wasmicon_hal.h"

/** Bytes of pattern.bin read per host call. Smaller than the asset, so the
 *  reads cross a 4 KiB boundary of the slot. */
#define CHUNK 1024u

/** pattern.bin is PATTERN_LEN bytes of (i * 7) as a byte (apps/assets-rs/assets/). */
#define PATTERN_LEN 4100u

/** Longest hello.txt the text buffer holds (the same bound as assets-rs). */
#define TEXT_CAP 64u

/** Entry point. The host calls it once, right after instantiation (docs/abi-spec.md §3.3). */
WASMICON_EXPORT("run") void run(void) {
    wasmicon_log_info("assets start");

    uint32_t hello;
    if (wasmicon_assets_blob_open_str("hello.txt", &hello) != WASMICON_OK) {
        wasmicon_log_error("no asset hello.txt");
        return;
    }
    uint32_t len;
    if (wasmicon_assets_blob_len(hello, &len) != WASMICON_OK) {
        wasmicon_log_error("len failed");
        return;
    }
    uint8_t text[TEXT_CAP];
    uint32_t got;
    if (len > TEXT_CAP ||
        wasmicon_assets_blob_read(hello, 0, len, text, len, &got) != WASMICON_OK) {
        wasmicon_log_error("cannot read hello.txt");
        return;
    }
    // Exactly the bytes read, so no terminator is needed (and assets-rs logs
    // the same bytes).
    wasmicon_log_log((uint32_t)WASMICON_LOG_LEVEL_INFO, (const char *)text, len);
    // Released here, so pattern.bin gets the same handle.
    wasmicon_assets_blob_drop(hello);

    uint32_t pattern;
    if (wasmicon_assets_blob_open_str("pattern.bin", &pattern) != WASMICON_OK) {
        wasmicon_log_error("no asset pattern.bin");
        return;
    }
    if (wasmicon_assets_blob_len(pattern, &len) != WASMICON_OK || len != PATTERN_LEN) {
        wasmicon_log_error("pattern.bin has the wrong length");
        return;
    }
    uint8_t buf[CHUNK];
    uint32_t at = 0;
    int ok = 1;
    while (at < PATTERN_LEN) {
        uint32_t n = PATTERN_LEN - at < CHUNK ? PATTERN_LEN - at : CHUNK;
        if (wasmicon_assets_blob_read(pattern, at, n, buf, n, &got) != WASMICON_OK) {
            wasmicon_log_error("cannot read pattern.bin");
            return;
        }
        for (uint32_t i = 0; i < n; i++) {
            if (buf[i] != (uint8_t)((at + i) * 7u)) {
                ok = 0;
            }
        }
        at += n;
    }
    wasmicon_log_info(ok ? "pattern ok" : "pattern mismatch");
    wasmicon_assets_blob_drop(pattern);
}
