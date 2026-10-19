// Lチカ（C）。blink-rs / blink-as と同じ host call 列を出す。
//
// LED のピン番号は board.pin-by-role で引くので、同一の .wasm が
// ESP32-S3 と Pico 2 W の両方で動く（abi-spec §8）。

#include "wasmicon_hal.h"

/** 点滅回数。 */
#define BLINKS 3

/** 点灯・消灯の間隔（ミリ秒）。 */
#define INTERVAL_MS 1

/** エントリポイント。ホストがインスタンス化直後に 1 回呼ぶ（abi-spec §3.3）。 */
WASMICON_EXPORT("run") void run(void) {
    wasmicon_log_info("blink start");

    // ピン番号はボードごとに違うので、役割名で引く。
    uint32_t index;
    if (wasmicon_board_pin_by_role_str("led", &index) != WASMICON_OK) {
        wasmicon_log_error("led の割り当てが無い");
        return;
    }
    uint32_t pin;
    if (wasmicon_gpio_pin_open(index, WASMICON_GPIO_PIN_MODE_OUTPUT, &pin) != WASMICON_OK) {
        wasmicon_log_error("led のピンを開けない");
        return;
    }

    for (int i = 0; i < BLINKS; i++) {
        if (wasmicon_gpio_pin_write(pin, WASMICON_GPIO_LEVEL_HIGH) != WASMICON_OK) {
            wasmicon_log_error("write に失敗");
            return;
        }
        wasmicon_time_sleep_ms(INTERVAL_MS);
        if (wasmicon_gpio_pin_toggle(pin) != WASMICON_OK) {
            wasmicon_log_error("toggle に失敗");
            return;
        }
        wasmicon_time_sleep_ms(INTERVAL_MS);
    }
    wasmicon_gpio_pin_drop(pin);
}
