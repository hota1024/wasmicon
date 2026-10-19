//! Board and generation step, kept separate from drawing so it can be
//! unit-tested on the host.

/// Board width in cells: 320 px / 8 px.
pub const COLS: usize = 40;
/// Board height in cells: 29 rows, because the top 8 px of the 240 show the
/// FPS.
pub const ROWS: usize = 29;

/// One row is the low `COLS` bits of a `u64` (bit x is cell (x, y)).
const ROW_MASK: u64 = (1 << COLS) - 1;

/// A 40x29 board whose edges wrap around (a torus).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Board {
    rows: [u64; ROWS],
}

impl Board {
    /// A board with every cell dead.
    pub const EMPTY: Board = Board { rows: [0; ROWS] };

    /// Returns whether (x, y) is alive.
    #[must_use]
    pub fn get(&self, x: usize, y: usize) -> bool {
        self.rows[y] >> x & 1 == 1
    }

    /// Sets (x, y) alive or dead (used by tests to build boards).
    #[cfg(test)]
    pub fn set(&mut self, x: usize, y: usize, alive: bool) {
        if alive {
            self.rows[y] |= 1 << x;
        } else {
            self.rows[y] &= !(1 << x);
        }
    }

    /// Returns the bits of one row.
    #[must_use]
    pub fn row(&self, y: usize) -> u64 {
        self.rows[y]
    }

    /// Number of live cells (for tests).
    #[cfg(test)]
    #[must_use]
    pub fn population(&self) -> u32 {
        self.rows.iter().map(|r| r.count_ones()).sum()
    }

    /// Fills the board at random; about one cell in four is alive.
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

    /// Computes the next generation (B3/S23).
    ///
    /// Counts a whole row (40 bits) at once with bit operations. The eight
    /// neighbor rows are made by rotating, and a bitwise adder keeps the
    /// neighbor count as its low 2 bits plus a "4 or more" flag. Orders of
    /// magnitude faster than looping over cells (the interpreter spends
    /// about 90 cycles per instruction).
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
            // b0, b1 are the low 2 bits of the count; four marks a count of
            // 4 or more (those cells are excluded from then on).
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

/// Rotates left by 1 within 40 bits (cell x moves to x + 1).
fn rol(r: u64) -> u64 {
    ((r << 1) | (r >> (COLS - 1))) & ROW_MASK
}

/// Rotates right by 1 within 40 bits (cell x moves to x - 1).
fn ror(r: u64) -> u64 {
    ((r >> 1) | (r << (COLS - 1))) & ROW_MASK
}

/// xorshift32. The HAL has no random source, so the app brings its own. The
/// same seed gives the same sequence, which keeps the trace deterministic
/// (comparable between the host and hardware).
pub struct Rng(u32);

impl Rng {
    /// Creates a generator from a seed. 0 is a fixed point of xorshift, so
    /// it is replaced.
    #[must_use]
    pub fn new(seed: u32) -> Self {
        Rng(if seed == 0 { 0x9E37_79B9 } else { seed })
    }

    /// Returns the next value.
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
        // A blinker that spans the right and left edges and the bottom and
        // top edges.
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
