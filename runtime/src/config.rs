//! ポートが決める上限。arena と同じくコアは自分で決めない。

/// ランタイムの上限設定。
#[derive(Clone, Copy)]
pub struct Config {
    /// 検証時の値スタックの最大段数。
    pub max_value_stack: usize,
    /// 制御構造のネスト上限。
    pub max_control_depth: usize,
    /// 1 関数のローカル（引数を含む）の上限。
    pub max_locals: usize,
    /// 線形メモリの最大ページ数（1 ページ = 64 KiB）。
    pub max_memory_pages: u32,
    /// テーブルの最大要素数。
    pub max_table_elems: u32,
    /// 実行時の呼び出し深さ上限。超えたら `Exhausted`。
    pub max_call_depth: usize,
    /// 実行時のオペランドスタックのスロット数。
    pub operand_stack_slots: usize,
}

impl Config {
    /// ホスト PC 向けの既定値。マイコンのポートは小さくする。
    ///
    /// `Default` から分けて `const` にしてあるのは、ボードプロファイル
    /// （`ports/common` の `profile`）が const 文脈でこれを使うため。
    /// 二重に書くと必ず食い違う。
    pub const DEFAULT: Config = Config {
        max_value_stack: 8192,
        max_control_depth: 2048,
        max_locals: 8192,
        max_memory_pages: 65536,
        max_table_elems: 0x0010_0000,
        max_call_depth: 512,
        operand_stack_slots: 16384,
    };
}

impl Default for Config {
    fn default() -> Self {
        Config::DEFAULT
    }
}
