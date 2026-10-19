// Blink (C). Produces the same host-call sequence as blink-rs and blink-as.
//
// The LED pin number comes from board.pin-by-role, so the same .wasm runs
// on both ESP32-S3 and Pico 2 W (docs/abi-spec.md §8).

#include "wasmicon_hal.h"

/** Number of blinks. */
#define BLINKS 3

/** On and off interval in milliseconds. */
#define INTERVAL_MS 1

/** Entry point. The host calls it once, right after instantiation (docs/abi-spec.md §3.3). */
WASMICON_EXPORT("run") void run(void) {
    wasmicon_log_info("blink start");

    // The pin number differs per board, so look it up by role name.
    uint32_t index;
    if (wasmicon_board_pin_by_role_str("led", &index) != WASMICON_OK) {
        wasmicon_log_error("no pin assigned to role led");
        return;
    }
    uint32_t pin;
    if (wasmicon_gpio_pin_open(index, WASMICON_GPIO_PIN_MODE_OUTPUT, &pin) != WASMICON_OK) {
        wasmicon_log_error("cannot open led pin");
        return;
    }

    for (int i = 0; i < BLINKS; i++) {
        if (wasmicon_gpio_pin_write(pin, WASMICON_GPIO_LEVEL_HIGH) != WASMICON_OK) {
            wasmicon_log_error("write failed");
            return;
        }
        wasmicon_time_sleep_ms(INTERVAL_MS);
        if (wasmicon_gpio_pin_toggle(pin) != WASMICON_OK) {
            wasmicon_log_error("toggle failed");
            return;
        }
        wasmicon_time_sleep_ms(INTERVAL_MS);
    }
    wasmicon_gpio_pin_drop(pin);
}
