//! 盤面と世代の計算。描画から切り離して、ホストで単体テストできるようにしてある。

/// 盤面の幅（セル）。320 px / 8 px。
pub const COLS: usize = 40;
/// 盤面の高さ（セル）。240 px のうち上の 8 px は FPS の表示に使うので 29 行。
pub const ROWS: usize = 29;

/// 1 行 = `u64` の下位 `COLS` ビット（ビット x がセル (x, y)）。
const ROW_MASK: u64 = (1 << COLS) - 1;

/// 40×29 の盤面。端は上下左右で繋がる（トーラス）。
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Board {
    rows: [u64; ROWS],
}

impl Board {
    /// 全部死んでいる盤面。
    pub const EMPTY: Board = Board { rows: [0; ROWS] };

    /// (x, y) が生きているか。
    #[must_use]
    pub fn get(&self, x: usize, y: usize) -> bool {
        self.rows[y] >> x & 1 == 1
    }

    /// (x, y) を生かす / 殺す（テストで盤面を組むのに使う）。
    #[cfg(test)]
    pub fn set(&mut self, x: usize, y: usize, alive: bool) {
        if alive {
            self.rows[y] |= 1 << x;
        } else {
            self.rows[y] &= !(1 << x);
        }
    }

    /// 1 行ぶんのビット。
    #[must_use]
    pub fn row(&self, y: usize) -> u64 {
        self.rows[y]
    }

    /// 生きているセルの数（テスト用）。
    #[cfg(test)]
    #[must_use]
    pub fn population(&self) -> u32 {
        self.rows.iter().map(|r| r.count_ones()).sum()
    }

    /// 乱数で埋める。おおよそ 4 つに 1 つが生きる。
    pub fn randomize(&mut self, rng: &mut Rng) {
        for y in 0..ROWS {
            let mut row = 0;
            for x in 0..COLS {
                if rng.next() & 3 == 0 {
                    row |= 1 << x;
                }
            }
            self.rows[y] = row;
        }
    }

    /// 次の世代（B3/S23）。
    ///
    /// **1 行（40 ビット）をまとめてビット演算で数える。** 近傍 8 方向の行を
    /// 回転で作り、ビットごとの加算器で「近傍の数」を下位 2 ビット + 4 以上の
    /// 印で持つ。1 セルずつループで数えるより桁違いに速い（インタプリタは
    /// 1 命令に約 90 サイクルかかる）。
    #[must_use]
    pub fn step(&self) -> Board {
        let mut next = Board::EMPTY;
        for y in 0..ROWS {
            let up = self.rows[(y + ROWS - 1) % ROWS];
            let mid = self.rows[y];
            let down = self.rows[(y + 1) % ROWS];
            let neighbours = [
                rol(up),
                up,
                ror(up),
                rol(mid),
                ror(mid),
                rol(down),
                down,
                ror(down),
            ];
            // b0, b1 = 数の下位 2 ビット、four = 4 以上になった印（以後は外す）。
            let (mut b0, mut b1, mut four) = (0u64, 0u64, 0u64);
            for n in neighbours {
                let c0 = b0 & n;
                b0 ^= n;
                let c1 = b1 & c0;
                b1 ^= c0;
                four |= c1;
            }
            let three = b0 & b1 & !four;
            let two = !b0 & b1 & !four;
            next.rows[y] = (three | (mid & two)) & ROW_MASK;
        }
        next
    }
}

/// 40 ビットの中で左に 1 回す（セル x の値が x + 1 に来る）。
fn rol(r: u64) -> u64 {
    ((r << 1) | (r >> (COLS - 1))) & ROW_MASK
}

/// 40 ビットの中で右に 1 回す（セル x の値が x - 1 に来る）。
fn ror(r: u64) -> u64 {
    ((r >> 1) | (r << (COLS - 1))) & ROW_MASK
}

/// xorshift32。HAL に乱数は無いので自前で持つ。種が同じなら同じ列を返すので、
/// トレースが決定的になる（host と実機で突き合わせられる）。
pub struct Rng(u32);

impl Rng {
    /// 種から作る。0 は xorshift の不動点なので避ける。
    #[must_use]
    pub fn new(seed: u32) -> Self {
        Rng(if seed == 0 { 0x9E37_79B9 } else { seed })
    }

    /// 次の値。
    pub fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board(cells: &[(usize, usize)]) -> Board {
        let mut b = Board::EMPTY;
        for &(x, y) in cells {
            b.set(x, y, true);
        }
        b
    }

    #[test]
    fn a_block_stays() {
        let b = board(&[(5, 5), (6, 5), (5, 6), (6, 6)]);
        assert!(b.step() == b);
    }

    #[test]
    fn a_blinker_has_period_two() {
        let b = board(&[(4, 5), (5, 5), (6, 5)]);
        let s = b.step();
        assert!(s == board(&[(5, 4), (5, 5), (5, 6)]));
        assert!(s.step() == b);
    }

    #[test]
    fn a_glider_moves_one_cell_diagonally_every_four_generations() {
        let g = board(&[(1, 0), (2, 1), (0, 2), (1, 2), (2, 2)]);
        let moved = board(&[(2, 1), (3, 2), (1, 3), (2, 3), (3, 3)]);
        assert!(g.step().step().step().step() == moved);
    }

    #[test]
    fn the_edges_wrap_around() {
        // 右端と左端、下端と上端にまたがるブリンカー。
        let b = board(&[(COLS - 1, 0), (0, 0), (1, 0)]);
        assert!(b.step() == board(&[(0, ROWS - 1), (0, 0), (0, 1)]));
    }

    #[test]
    fn nothing_leaks_past_the_last_column() {
        let mut b = Board::EMPTY;
        b.randomize(&mut Rng::new(1));
        for _ in 0..50 {
            b = b.step();
            for y in 0..ROWS {
                assert_eq!(b.row(y) & !ROW_MASK, 0);
            }
        }
    }

    #[test]
    fn the_same_seed_gives_the_same_board() {
        let (mut a, mut b) = (Board::EMPTY, Board::EMPTY);
        a.randomize(&mut Rng::new(42));
        b.randomize(&mut Rng::new(42));
        assert!(a == b);
        assert!(a.population() > 0);
    }
}
