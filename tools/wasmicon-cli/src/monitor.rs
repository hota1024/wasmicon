//! `wasmicon monitor` — シリアルを開いてトレースを取る（`docs/app-workflow.md` §4.2）。
//!
//! **手順に罠が 3 つある。** 実機で全部踏んだ（`docs/verification-report.md` §9）
//! ので、ここに閉じ込める。
//!
//! 1. **`stty` は開いたまま当てる。** macOS の `stty -f` は開いて設定して
//!    閉じるので、保持されない（次に開くと 9600 に戻る）。一度、全部ノイズの
//!    ログを取った。**先にポートを開き、その handle を持ったまま当てる。**
//! 2. **流れ始める前に当てる。** データが流れている最中に当てると行が混ざって
//!    バイトが落ちる。だから `monitor` は開いて設定してから、呼び出し側が
//!    ボードをリセットする（`deploy --monitor` がその順でやる）。
//! 3. **バイト列として扱う。** リセットの瞬間に NUL やライン・ノイズが入る。
//!    `String` に落とすと UTF-8 でないバイトが潰れるので、**行は `Vec<u8>`**
//!    で持つ（`trace` と同じ理由）。
//!
//! 止めどきは**無音**で決める。アプリは `run` を抜けるとファームが idle に
//! 入って何も出なくなるので、「N 秒無音なら終わり」が自然な区切りになる。
//! Ctrl-C を待つだけだと自動化に使えない。
//!
//! identity（§3.8）の刻印は `info` が入ってから（2 段）。今はファームの
//! バナー（`wasmicon rp2350`）とスロットの行（`slot <len> B crc32=<..>`）が
//! シリアルに出るので、それが由来の記録になっている。

use anyhow::{Context, Result, bail};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// 既定のボーレート（`ports/*` の UART0 と揃える）。
pub const DEFAULT_BAUD: u32 = 115_200;

/// 既定の無音の長さ。これだけ何も来なければ終わり。
pub const DEFAULT_IDLE: Duration = Duration::from_secs(3);

pub struct Options {
    /// `--port`。`None` なら `/dev/cu.usb*` から選ぶ。
    pub port: Option<PathBuf>,
    pub baud: u32,
    /// `-o`。標準出力に出すのとは別に、ファイルにも書く。
    pub out: Option<PathBuf>,
    /// 無音で終わる長さ。`None` なら終わらない（Ctrl-C で止める）。
    pub idle: Option<Duration>,
    /// 全体の上限。`None` なら無し。
    pub timeout: Option<Duration>,
}

/// 取り込みの結果。
pub struct Stats {
    pub bytes: usize,
    /// `>` / `<` で始まる行（abi-spec §9 のトレース行）。
    pub trace_lines: usize,
    /// 止まった理由。
    pub stopped: Stopped,
}

#[derive(PartialEq, Eq)]
pub enum Stopped {
    /// 無音が続いた（アプリが終わってファームが idle に入った形）。
    Idle,
    /// 上限に達した。
    Timeout,
    /// 相手が閉じた（USB が抜けた）。
    Closed,
}

impl Stopped {
    #[must_use]
    pub fn reason(&self) -> &'static str {
        match self {
            Stopped::Idle => "無音になった",
            Stopped::Timeout => "上限に達した",
            Stopped::Closed => "ポートが閉じた（USB が抜けた）",
        }
    }
}

/// 開いたシリアル。**`stty` を当てたあとの状態**。
pub struct Session {
    port: PathBuf,
    file: std::fs::File,
}

impl Session {
    #[must_use]
    pub fn port(&self) -> &Path {
        &self.port
    }
}

/// 候補になるシリアルの口を挙げる。
///
/// **`usbserial` を決め打ちしない。** ブリッジの型番で名前が変わる
/// （手元の ESP32-S3 は CH343 で `usbmodem` になる。`docs/TODO.md` §1.1）。
#[must_use]
pub fn ports() -> Vec<PathBuf> {
    let Ok(dir) = std::fs::read_dir("/dev") else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = dir
        .filter_map(|e| {
            let name = e.ok()?.file_name().to_string_lossy().into_owned();
            name.starts_with("cu.usb")
                .then(|| PathBuf::from("/dev").join(name))
        })
        .collect();
    found.sort();
    found
}

/// どの口を使うか決める。
///
/// 候補が 1 つならそれ。複数あって指定が無ければ**選ばない**
/// （間違った口を黙って開くと「何も出ない」になって配線を疑い始める）。
///
/// # Errors
/// 候補が無い、または複数あって指定が無いとき。
pub fn choose(candidates: &[PathBuf], explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(p.to_path_buf());
    }
    match candidates {
        [] => bail!(
            "シリアルの口が見つからない（/dev/cu.usb* が無い）。\n  \
             USB-シリアル変換を挿して、ボードの TX を繋ぐこと"
        ),
        [one] => Ok(one.clone()),
        many => {
            let list: Vec<String> = many.iter().map(|p| p.display().to_string()).collect();
            bail!(
                "シリアルの口が {} 個ある。--port で選ぶこと:\n  {}",
                many.len(),
                list.join("\n  ")
            )
        }
    }
}

/// 開いてボーレートを当てる。
///
/// **順序が肝。** 先に開き、その handle を持ったまま `stty` を当てる
/// （閉じると設定が戻る。このモジュールの罠 1）。
///
/// # Errors
/// 開けない、`stty` が無い・失敗したとき。
pub fn open(port: &Path, baud: u32) -> Result<Session> {
    let file =
        std::fs::File::open(port).with_context(|| format!("{} を開けない", port.display()))?;

    // ここで handle は開いたまま。`stty` は同じデバイスに当てる。
    let flag = if cfg!(target_os = "linux") {
        "-F"
    } else {
        "-f"
    };
    let status = Command::new("stty")
        .args([
            flag,
            &port.to_string_lossy(),
            &baud.to_string(),
            "raw",
            "-echo",
        ])
        .status()
        .context("stty を起動できない")?;
    if !status.success() {
        bail!("{} に {baud} baud を設定できなかった", port.display());
    }
    Ok(Session {
        port: port.to_path_buf(),
        file,
    })
}

/// 読み続けて `sink` に流す。
///
/// 行は**バイト列のまま**扱う（罠 3）。`>` / `<` で始まる行だけ数える。
///
/// # Errors
/// 書き出しに失敗したとき。
pub fn stream<R: Read + Send + 'static>(
    src: R,
    sink: &mut dyn Write,
    idle: Option<Duration>,
    timeout: Option<Duration>,
) -> Result<Stats> {
    // 読みに時限を付けるため、別スレッドで読んでチャネルで受ける
    // （シリアルの read はブロックするので、タイマだけでは抜けられない）。
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut src = src;
        let mut buf = [0u8; 4096];
        loop {
            match src.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let started = Instant::now();
    let mut bytes = 0usize;
    let mut trace_lines = 0usize;
    let mut partial: Vec<u8> = Vec::new();
    let stopped;

    loop {
        if let Some(limit) = timeout
            && started.elapsed() >= limit
        {
            stopped = Stopped::Timeout;
            break;
        }
        // 無音の判定に使う待ち時間。上限があるなら残りで切る。
        let wait = idle.unwrap_or(Duration::from_secs(3600));
        let wait = match timeout {
            Some(limit) => wait.min(limit.saturating_sub(started.elapsed())),
            None => wait,
        };
        match rx.recv_timeout(wait) {
            Ok(chunk) => {
                bytes += chunk.len();
                sink.write_all(&chunk).context("書き出せない")?;
                sink.flush().context("書き出せない")?;
                trace_lines += count_trace_lines(&chunk, &mut partial);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if idle.is_some() {
                    stopped = Stopped::Idle;
                    break;
                }
                // 無音で終わらない設定。上限の判定に戻る。
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                stopped = Stopped::Closed;
                break;
            }
        }
    }

    Ok(Stats {
        bytes,
        trace_lines,
        stopped,
    })
}

/// チャンクをまたぐ行を `partial` で繋ぎながら、トレース行を数える。
fn count_trace_lines(chunk: &[u8], partial: &mut Vec<u8>) -> usize {
    let mut n = 0;
    for b in chunk {
        if *b == b'\n' {
            // CR と NUL は落とす（`trace::normalize` と同じ規則）。
            let line: Vec<u8> = partial
                .iter()
                .copied()
                .filter(|c| *c != b'\r' && *c != 0)
                .collect();
            if matches!(line.first(), Some(b'>' | b'<')) {
                n += 1;
            }
            partial.clear();
        } else {
            partial.push(*b);
        }
    }
    n
}

/// 取り込む。
///
/// # Errors
/// 口が決まらない、開けない、書き出せないとき。
pub fn run(opts: &Options) -> Result<bool> {
    let port = choose(&ports(), opts.port.as_deref())?;
    let session = open(&port, opts.baud)?;
    eprintln!(
        "{} を {} baud で開いた。ボードをリセットすると出力が始まる",
        session.port().display(),
        opts.baud
    );
    capture(session, opts)
}

/// 開いてある口から取り込む。
///
/// `deploy --monitor` は**リセットの前に開く**必要があるので、開く処理と
/// 分けてある（罠 2）。
///
/// # Errors
/// 書き出せないとき。
pub fn capture(session: Session, opts: &Options) -> Result<bool> {
    if let Some(idle) = opts.idle {
        eprintln!("（{} 秒無音になったら終わる）", idle.as_secs());
    }

    let mut sink = Sink::new(opts.out.as_deref())?;
    let stats = stream(session.file, &mut sink, opts.idle, opts.timeout)?;
    sink.finish()?;

    eprintln!(
        "{}: {} B / トレース {} 行",
        stats.stopped.reason(),
        stats.bytes,
        stats.trace_lines
    );
    if stats.trace_lines == 0 {
        // 「何も取れていない」を黙って成功にしない（§9 で踏んだ形）。
        eprintln!(
            "  トレース行が 1 行も無い。ボーレートが違う（全部ノイズになる）、\n  \
             口が違う、ボードをリセットしていない、trace feature を切って\n  \
             焼いた、のどれかを疑うこと"
        );
        return Ok(false);
    }
    Ok(true)
}

/// 標準出力と、指定があればファイルの両方に書く。
struct Sink {
    file: Option<std::fs::File>,
}

impl Sink {
    fn new(path: Option<&Path>) -> Result<Self> {
        let file = match path {
            None => None,
            Some(p) => Some(
                std::fs::File::create(p).with_context(|| format!("{} を書けない", p.display()))?,
            ),
        };
        Ok(Sink { file })
    }

    fn finish(&mut self) -> Result<()> {
        if let Some(f) = &mut self.file {
            f.flush().context("書き出せない")?;
        }
        Ok(())
    }
}

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        std::io::stdout().write_all(buf)?;
        if let Some(f) = &mut self.file {
            f.write_all(buf)?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        std::io::stdout().flush()?;
        if let Some(f) = &mut self.file {
            f.flush()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_candidate_needs_no_choice() {
        let c = [PathBuf::from("/dev/cu.usbserial-1")];
        assert_eq!(choose(&c, None).expect("選べる"), c[0]);
    }

    #[test]
    fn several_candidates_are_not_guessed() {
        // 間違った口を黙って開くと「何も出ない」になって配線を疑い始める。
        let c = [
            PathBuf::from("/dev/cu.usbserial-1"),
            PathBuf::from("/dev/cu.usbmodem2"),
        ];
        let e = choose(&c, None).expect_err("選ばない");
        let msg = format!("{e:#}");
        assert!(msg.contains("--port"), "{msg}");
        assert!(msg.contains("usbmodem2"), "候補を並べる: {msg}");
    }

    #[test]
    fn an_explicit_port_wins() {
        let c = [PathBuf::from("/dev/cu.usbserial-1")];
        let want = PathBuf::from("/dev/cu.usbmodem9");
        assert_eq!(choose(&c, Some(&want)).expect("選べる"), want);
    }

    #[test]
    fn no_candidate_says_what_to_do() {
        let e = choose(&[], None).expect_err("無い");
        assert!(format!("{e:#}").contains("USB-シリアル変換"), "{e:#}");
    }

    #[test]
    fn trace_lines_are_counted_across_chunks() {
        // チャンクの切れ目が行をまたいでも数えられること。
        let mut partial = Vec::new();
        let mut n = count_trace_lines(b"wasmicon rp2350\n> a/b(1)\n< 0 ", &mut partial);
        assert_eq!(n, 1, "バナーは数えない");
        n += count_trace_lines(b"[1]\n", &mut partial);
        assert_eq!(n, 2, "チャンクをまたいだ行も数える");
    }

    #[test]
    fn noise_does_not_break_the_count() {
        // リセットの瞬間の NUL と CR を落として判定する（§9 で踏んだ形）。
        let mut partial = Vec::new();
        let n = count_trace_lines(b"\0\0> a/b(1)\r\n\0< 0\r\n", &mut partial);
        assert_eq!(n, 2);
    }

    #[test]
    fn silence_ends_the_capture() {
        // 無音で終わる（アプリが終わってファームが idle に入った形）。
        let (_tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        // 何も来ないパイプの代わりに、空のまま閉じない Read を作る。
        struct Quiet(std::sync::mpsc::Receiver<Vec<u8>>);
        impl Read for Quiet {
            fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
                // 送られてこないので永遠にブロックする（シリアルと同じ）。
                match self.0.recv() {
                    Ok(_) | Err(_) => Ok(0),
                }
            }
        }
        let mut out = Vec::new();
        let stats = stream(
            Quiet(rx),
            &mut out,
            Some(Duration::from_millis(50)),
            Some(Duration::from_secs(5)),
        )
        .expect("読める");
        assert!(stats.stopped == Stopped::Idle, "{}", stats.stopped.reason());
        assert_eq!(stats.bytes, 0);
    }

    #[test]
    fn what_comes_in_is_written_out() {
        let data = b"wasmicon rp2350\n> a/b(1)\n< 0 [1]\n".to_vec();
        let mut out = Vec::new();
        let stats = stream(
            std::io::Cursor::new(data.clone()),
            &mut out,
            Some(Duration::from_millis(50)),
            None,
        )
        .expect("読める");
        assert_eq!(out, data, "バイト列をそのまま流す");
        assert_eq!(stats.trace_lines, 2);
        assert_eq!(stats.bytes, data.len());
    }
}
