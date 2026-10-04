//! `wasmicon monitor` の**止めどきの判定**を固定する。
//!
//! シリアルそのものは実機が要るので、`stream` に好きな `Read` を渡して
//! 「いつ、どの理由で止まったか」だけを見る。
//!
//! **止まった理由は報告に出る。** ここを間違えると、途中で切れたログが
//! 「無音になった」と出て**きれいに終わったように見える** —— トレースの
//! 突き合わせでは行数が合わないところで初めて気付く形になる。

use std::io::Read;
use std::time::Duration;

use wasmicon_cli::monitor::{self, Stopped};

/// 1 回だけ返して、あとは黙るシリアル。
struct OnceThenSilent {
    sent: bool,
}

impl Read for OnceThenSilent {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if !self.sent {
            self.sent = true;
            let line = b"> wasmicon:hal/log@0.1.0/log(2, \"x\")\n";
            buf[..line.len()].copy_from_slice(line);
            return Ok(line.len());
        }
        // **閉じない**（`Ok(0)` を返さない）。実機は黙るだけで、口は開いたまま。
        std::thread::sleep(Duration::from_secs(3600));
        Ok(0)
    }
}

/// すぐ閉じるシリアル（USB が抜けた形）。
struct Closed;

impl Read for Closed {
    fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
        Ok(0)
    }
}

#[test]
fn hitting_the_overall_limit_is_not_reported_as_silence() {
    // **`--idle` と `--timeout` が両方あるとき**、無音の待ち時間は上限の
    // 残りで詰められる。だから `recv_timeout` の時間切れだけでは
    // どちらで切れたのか分からない。上限を先に見ないと、
    // **流れている最中に打ち切ったログが「無音になった」になり**、
    // `Stopped::Timeout` が（`--idle` は既定で入っているので）事実上
    // 出なくなる。
    let mut sink: Vec<u8> = Vec::new();
    let stats = monitor::stream(
        OnceThenSilent { sent: false },
        &mut sink,
        Some(Duration::from_secs(5)),
        Some(Duration::from_millis(600)),
    )
    .expect("書き出せる");

    assert!(
        stats.stopped == Stopped::Timeout,
        "上限で切れたのに「{}」と言っている",
        stats.stopped.reason()
    );
    assert_eq!(stats.trace_lines, 1);
}

#[test]
fn silence_without_a_limit_is_reported_as_silence() {
    // 上限が無ければ、無音はそのまま無音（アプリが終わってファームが
    // idle に入った形）。
    let mut sink: Vec<u8> = Vec::new();
    let stats = monitor::stream(
        OnceThenSilent { sent: false },
        &mut sink,
        Some(Duration::from_millis(200)),
        None,
    )
    .expect("書き出せる");

    assert!(stats.stopped == Stopped::Idle, "{}", stats.stopped.reason());
    assert_eq!(stats.trace_lines, 1);
    assert!(
        String::from_utf8_lossy(&sink).contains("log(2"),
        "受けたバイトを書き出していない"
    );
}

#[test]
fn a_closed_port_is_distinguished_from_silence() {
    // USB が抜けたのを「無音」と言うと、アプリが終わったように見える。
    let mut sink: Vec<u8> = Vec::new();
    let stats =
        monitor::stream(Closed, &mut sink, Some(Duration::from_secs(5)), None).expect("書き出せる");

    assert!(
        stats.stopped == Stopped::Closed,
        "{}",
        stats.stopped.reason()
    );
    assert_eq!(stats.bytes, 0);
}
